"""The wheel verifier and the installed-wheel smoke test,
``tools/build/verify_wheel.py`` and ``tools/build/smoke.py`` (packaging spec
"Wheel matrix", "Every wheel ships the OCR helper", "Every wheel ships its
license files", "Every wheel carries a CycloneDX SBOM", "Every built wheel is
verified before release"; design D6).

The verifier is run on synthetic wheels: a correct one built here from the
repository's licence files, real SBOMs (``sbom.generate``) and a hand-made
ELF or Mach-O helper, then one variant per failure the spec names. The
smoke test is run in a child interpreter, both from the checkout (where it
must refuse the development tree) and from a copy outside it.
"""

from __future__ import annotations

import json
import shutil
import struct
import subprocess
import sys
import zipfile
from collections.abc import Callable
from pathlib import Path

import pytest

import pdf_extract

from ocrkit import needs_helper
from pdfs import REPO

sys.path.insert(0, str(REPO / "tools" / "build"))
sys.path.insert(0, str(REPO / "tools" / "license"))
import rust_std
import sbom
import verify_wheel as vw

VERSION = sbom.workspace_version(REPO)
DIST = f"pdf_extract-{VERSION}.dist-info"


# --- synthetic binaries ---------------------------------------------------


def make_elf(  # noqa: PLR0913 - one switch per malformation under test
    needed: list[str],
    versions: list[str],
    *,
    terminated: bool = True,
    machine: int = 0x3E,
    sections_present: bool = True,
    dynamic: bool = True,
) -> bytes:
    """A minimal 64-bit little-endian ELF with .dynstr, .dynamic and
    .gnu.version_r sections (all one library's version needs)."""
    strtab = bytearray(b"\0")

    def s(text: str) -> int:
        off = len(strtab)
        strtab.extend(text.encode() + b"\0")
        return off

    # DT_NEEDED entries, an unrelated DT_FLAGS entry, then DT_NULL.
    dyn = b"".join(struct.pack("<qQ", 1, s(n)) for n in needed) + struct.pack("<qQ", 30, 0)
    if terminated:
        dyn += struct.pack("<qQ", 0, 0)
    aux = b"".join(
        struct.pack("<IHHII", 0, 0, 0, s(v), 16 if i + 1 < len(versions) else 0)
        for i, v in enumerate(versions)
    )
    verneed = (
        struct.pack("<HHIII", 1, len(versions), s("libc.so.6"), 16, 0) + aux if versions else b""
    )
    body = bytes(strtab) + dyn + verneed
    base = 64
    str_off, dyn_off, vn_off = base, base + len(strtab), base + len(strtab) + len(dyn)
    shoff = base + len(body)
    sections = [
        struct.pack("<IIQQQQIIQQ", 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
        struct.pack("<IIQQQQIIQQ", 0, 3, 0, 0, str_off, len(strtab), 0, 0, 1, 0),
        # SHT_DYNAMIC, or SHT_PROGBITS for an executable without one.
        struct.pack("<IIQQQQIIQQ", 0, 6 if dynamic else 1, 0, 0, dyn_off, len(dyn), 1, 0, 8, 16),
        struct.pack(
            "<IIQQQQIIQQ", 0, 0x6FFFFFFE, 0, 0, vn_off, len(verneed), 1, 1 if versions else 0, 4, 0
        ),
    ]
    header = bytearray(64)
    header[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<H", header, 0x12, machine)
    struct.pack_into("<Q", header, 0x28, shoff)
    struct.pack_into("<HHH", header, 0x3A, 64, len(sections) if sections_present else 0, 0)
    return bytes(header) + body + b"".join(sections)


def make_macho(  # noqa: PLR0913 - one switch per malformation under test
    minos: tuple[int, int, int] = (11, 0, 0),
    dylibs: tuple[str, ...] = ("/usr/lib/libSystem.B.dylib",),
    *,
    legacy: bool = False,
    cputype: int = 0x0100000C,
    with_version: bool = True,
    platform: int = 1,
) -> bytes:
    """A minimal thin Mach-O: a minimum-OS command and LC_LOAD_DYLIBs."""
    raw = (minos[0] << 16) | (minos[1] << 8) | minos[2]
    cmds = [struct.pack("<II16s", 0x1B, 24, bytes(16))]  # LC_UUID: not read
    if with_version:
        cmds.append(
            struct.pack("<IIII", 0x24, 16, raw, raw)
            if legacy
            else struct.pack("<IIIIII", 0x32, 24, platform, raw, raw, 0)
        )
    for lib in dylibs:
        name = lib.encode() + b"\0"
        name += b"\0" * (-len(name) % 8)
        cmds.append(struct.pack("<IIIIII", 0xC, 24 + len(name), 24, 0, 0, 0) + name)
    body = b"".join(cmds)
    header = struct.pack("<IiiIIIII", 0xFEEDFACF, cputype, 0, 2, len(cmds), len(body), 0, 0)
    return header + body


GOOD_ELF = make_elf(
    ["libstdc++.so.6", "libc.so.6", "ld-linux-x86-64.so.2"],
    ["GLIBC_2.28", "GLIBCXX_3.4.21", "CXXABI_1.3.11", "GCC_7.0.0", "CXXABI_TM_1"],
)


# --- synthetic wheels -----------------------------------------------------


Members = dict[str, tuple[bytes, int]]


def good_members(tmp: Path, platform: str, abi: str, helper: bytes) -> Members:
    plat = vw.PLATFORMS[platform]
    ext = (
        "pdf_extract/_native.abi3.so"
        if abi == "abi3"
        else "pdf_extract/_native.cpython-314t-darwin.so"
    )
    tag = f"{vw.ABIS[abi][0]}-{plat.tag}"
    metadata = (
        f"Metadata-Version: 2.4\nName: pdf-extract\nVersion: {VERSION}\nRequires-Python: >=3.13\n"
    )
    wheel_file = (
        f"Wheel-Version: 1.0\nGenerator: maturin (1.15.0)\nRoot-Is-Purelib: false\nTag: {tag}\n"
    )
    helper_file = tmp / "helper"
    helper_file.write_bytes(helper)
    rust, native = sbom.generate(
        REPO, plat.triple, ["abi3"] if abi == "abi3" else [], helper_file, tmp / "sb"
    )
    members: Members = {
        "pdf_extract/__init__.py": (b"", 0o100644),
        ext: (helper, 0o100755),
        f"{DIST}/METADATA": (metadata.encode(), 0o100644),
        f"{DIST}/WHEEL": (wheel_file.encode(), 0o100644),
        vw.HELPER: (helper, 0o100755),
        f"{DIST}/sboms/rust.cdx.json": (rust.read_bytes(), 0o100644),
        f"{DIST}/sboms/native.cdx.json": (native.read_bytes(), 0o100644),
    }
    for in_wheel, in_repo in vw.LICENSE_FILES.items():
        members[f"{DIST}/licenses/{in_wheel}"] = ((REPO / in_repo).read_bytes(), 0o100644)
    return members


def write_wheel(path: Path, members: Members) -> Path:
    with zipfile.ZipFile(path, "w") as zf:
        for name, (data, mode) in members.items():
            info = zipfile.ZipInfo(name)
            info.external_attr = mode << 16
            zf.writestr(info, data)
    return path


def linux_name() -> str:
    return f"pdf_extract-{VERSION}-cp313-abi3-manylinux_2_28_x86_64.whl"


@pytest.fixture(scope="module")
def linux_members(tmp_path_factory: pytest.TempPathFactory) -> Members:
    return good_members(tmp_path_factory.mktemp("lin"), "linux-x86_64", "abi3", GOOD_ELF)


@pytest.fixture(scope="module")
def linux_crates() -> list[sbom.Crate]:
    triple = vw.PLATFORMS["linux-x86_64"].triple
    return sbom.merge_graphs(
        {
            "extension": sbom.cargo_tree(REPO, sbom.EXTENSION_CRATE, triple, ["abi3"]),
            "helper": sbom.cargo_tree(REPO, sbom.HELPER_CRATE, triple, ()),
        }
    )


@pytest.fixture(scope="module")
def linux_std() -> tuple[str, list[rust_std.StdCrate]]:
    return vw.shipped_std(REPO, "linux-x86_64")


def verify_linux(
    tmp_path: Path,
    members: Members,
    crates: list[sbom.Crate],
    name: str | None = None,
    std: tuple[str, list[rust_std.StdCrate]] | None = None,
) -> list[str]:
    wheel = write_wheel(tmp_path / (name or linux_name()), members)
    return vw.verify(wheel, "linux-x86_64", "abi3", expected_crates=crates, expected_std=std)


# --- the correct wheel ----------------------------------------------------


def test_a_correct_linux_wheel_passes(tmp_path: Path, linux_members: Members) -> None:
    # expected_crates left to its default: a fresh cargo tree.
    wheel = write_wheel(tmp_path / linux_name(), linux_members)
    assert vw.verify(wheel, "linux-x86_64", "abi3") == []


def test_a_correct_macos_ft_wheel_passes(tmp_path: Path) -> None:
    members = good_members(tmp_path, "macos-arm64", "ft", make_macho())
    wheel = write_wheel(
        tmp_path / f"pdf_extract-{VERSION}-cp314-cp314t-macosx_11_0_arm64.whl", members
    )
    assert vw.verify(wheel, "macos-arm64", "ft") == []


# --- one variant per failure ----------------------------------------------

Edit = Callable[[Members], None]


def _drop(name: str) -> Edit:
    def edit(m: Members) -> None:
        del m[name]

    return edit


def _set(name: str, data: bytes | None = None, mode: int | None = None) -> Edit:
    def edit(m: Members) -> None:
        old = m.get(name, (b"", 0o100644))
        m[name] = (old[0] if data is None else data, old[1] if mode is None else mode)

    return edit


def _sbom(name: str, change: Callable[[sbom.Json], None]) -> Edit:
    def edit(m: Members) -> None:
        member = f"{DIST}/sboms/{name}"
        doc = json.loads(m[member][0])
        change(doc)
        m[member] = (json.dumps(doc).encode(), 0o100644)

    return edit


def _plant_loom(doc: sbom.Json) -> None:
    doc["components"].append(
        dict(doc["components"][0], name="loom", version="0.7.2", **{"bom-ref": "loom"})
    )


def _plant_licence(doc: sbom.Json) -> None:
    pyo3 = next(c for c in doc["components"] if c["name"] == "pyo3")
    pyo3["licenses"] = [{"expression": "GPL-3.0-only"}]


def _plant_purl(doc: sbom.Json) -> None:
    next(c for c in doc["components"] if c["name"] == "pyo3")["purl"] = "pkg:cargo/x@1"


def _drop_std_core(doc: sbom.Json) -> None:
    doc["components"] = [
        c for c in doc["components"] if not (c["name"] == "core" and "github" in c["purl"])
    ]


def _plant_unnoticed(doc: sbom.Json) -> None:
    """A crate the notices do not name, as if the helper had gained a
    dependency the extension lacks (Part 1 covers only the extension)."""
    doc["components"].append(
        dict(doc["components"][0], name="zeroize", version="1.8.1", **{"bom-ref": "z"})
    )


def _plant_hash(doc: sbom.Json) -> None:
    next(c for c in doc["components"] if c["name"] == "leptonica")["hashes"][0]["content"] = (
        "0" * 64
    )


def _plant_path(doc: sbom.Json) -> None:
    doc["metadata"]["component"]["description"] = "/home/runner/work/x"


def _break_schema(doc: sbom.Json) -> None:
    doc["specVersion"] = 7


@pytest.mark.parametrize(
    ("edit", "expected"),
    [
        (_drop(vw.HELPER), "no OCR helper"),
        (_set(vw.HELPER, mode=0o100644), "OCR helper mode"),
        (_set(vw.HELPER, data=make_elf(["libc.so.6"], ["GLIBC_2.34"])), "needs GLIBC_2.34, newer"),
        (_set(vw.HELPER, data=make_elf(["libc.so.6"], ["GLIBCXX_3.4.30"])), "needs GLIBCXX_3.4.30"),
        (_set(vw.HELPER, data=make_elf(["libtesseract.so.5"], [])), "needs libtesseract.so.5"),
        (_set(vw.HELPER, data=make_elf([], ["OPENSSL_3.0.0"])), "does not list"),
        (_set(vw.HELPER, data=b"#!/bin/sh\n"), "not a usable binary"),
        (_set(vw.HELPER, data=make_elf(["libc.so.6"], [], machine=0xB7)), "ELF machine 0xb7"),
        (
            _set(vw.HELPER, data=make_elf(["libc.so.6"], [], sections_present=False)),
            "no section headers",
        ),
        (_set(vw.HELPER, data=make_elf([], [], dynamic=False)), "no dynamic section"),
        (
            _set("pdf_extract/_native.abi3.so", data=make_elf(["libpython3.13.so.1.0"], [])),
            "extension module needs libpython3.13.so.1.0",
        ),
        (_set(f"{DIST}/METADATA", data=b"Requires-Python: >=3.11\n"), "Requires-Python"),
        (_set(f"{DIST}/WHEEL", data=b"Tag: cp313-abi3-linux_x86_64\n"), "WHEEL tags"),
        (_drop(f"{DIST}/WHEEL"), "wheel metadata missing"),
        (_sbom("rust.cdx.json", _plant_licence), "licence ['GPL-3.0-only']"),
        (_sbom("rust.cdx.json", _plant_purl), "purl 'pkg:cargo/x@1'"),
        (_drop(f"{DIST}/licenses/THIRD_PARTY_NOTICES"), "THIRD_PARTY_NOTICES missing"),
        (_set(f"{DIST}/licenses/LICENSE", data=b"MIT"), "differs from LICENSE"),
        (_set("pdf_extract.libs/libfoo.so"), "grafted libraries"),
        (_set("pdf_extract/_native.cpython-313-x86_64-linux-gnu.so"), "extension modules"),
        (_drop(f"{DIST}/sboms/native.cdx.json"), "SBOM documents"),
        (_drop(f"{DIST}/sboms/rust.cdx.json"), "SBOM documents"),
        (_set(f"{DIST}/sboms/extra.cdx.json", data=b"{}"), "SBOM documents"),
        (_sbom("rust.cdx.json", _plant_loom), "lists loom 0.7.2, which is not shipped"),
        (_sbom("rust.cdx.json", _drop_std_core), "rust SBOM lacks std crate core 0.0.0"),
        (
            _sbom("rust.cdx.json", _plant_unnoticed),
            "THIRD_PARTY_NOTICES does not name shipped crate zeroize 1.8.1",
        ),
        (
            _drop(f"{DIST}/licenses/tools/license/rust-std-COPYRIGHT.html"),
            "rust-std-COPYRIGHT.html missing",
        ),
        (_sbom("native.cdx.json", _plant_hash), "leptonica: SHA-256"),
        (_sbom("native.cdx.json", _plant_path), "build-machine path"),
        (_sbom("rust.cdx.json", _break_schema), "not valid CycloneDX 1.5"),
    ],
)
def test_each_defect_is_reported(  # noqa: PLR0913, PLR0917 - fixtures plus the case
    tmp_path: Path,
    linux_members: Members,
    linux_crates: list[sbom.Crate],
    linux_std: tuple[str, list[rust_std.StdCrate]],
    edit: Edit,
    expected: str,
) -> None:
    members = dict(linux_members)
    edit(members)
    problems = verify_linux(tmp_path, members, linux_crates, std=linux_std)
    assert any(expected in p for p in problems), problems


def test_the_auditwheel_sbom_is_allowed(
    tmp_path: Path, linux_members: Members, linux_crates: list[sbom.Crate]
) -> None:
    members = dict(linux_members)
    doc = json.loads(members[f"{DIST}/sboms/native.cdx.json"][0])
    doc["components"], doc["dependencies"] = [], []
    members[f"{DIST}/sboms/auditwheel.cdx.json"] = (json.dumps(doc).encode(), 0o100644)
    assert verify_linux(tmp_path, members, linux_crates) == []


def test_a_wrong_tag_is_reported(
    tmp_path: Path, linux_members: Members, linux_crates: list[sbom.Crate]
) -> None:
    name = f"pdf_extract-{VERSION}-cp311-abi3-manylinux_2_28_x86_64.whl"
    problems = verify_linux(tmp_path, linux_members, linux_crates, name)
    assert any("expected pdf_extract-" in p for p in problems), problems


# --- the binary readers ---------------------------------------------------


def test_elf_reader_round_trip() -> None:
    assert vw.elf_requirements(GOOD_ELF) == (
        ["libstdc++.so.6", "libc.so.6", "ld-linux-x86-64.so.2"],
        ["GLIBC_2.28", "GLIBCXX_3.4.21", "CXXABI_1.3.11", "GCC_7.0.0", "CXXABI_TM_1"],
    )
    assert vw.manylinux_problems(*vw.elf_requirements(GOOD_ELF)) == []


def test_elf_reader_without_a_null_terminator() -> None:
    assert vw.elf_requirements(make_elf(["libc.so.6"], [], terminated=False)) == (["libc.so.6"], [])


def test_elf_reader_refuses_other_formats() -> None:
    with pytest.raises(vw.FormatError):
        vw.elf_requirements(b"\x7fELF\x01\x01" + bytes(60))


def test_macho_reader_and_checks() -> None:
    assert vw.macho_requirements(make_macho()) == ((11, 0, 0), ["/usr/lib/libSystem.B.dylib"])
    assert vw.macho_requirements(make_macho((10, 15, 0), legacy=True))[0] == (10, 15, 0)
    minos, libs = vw.macho_requirements(
        make_macho((12, 3, 0), ("/opt/homebrew/lib/libtesseract.5.dylib",))
    )
    problems = vw.macos_problems(minos, libs, "macosx_11_0_arm64")
    assert any("12.3.0 is newer" in p for p in problems)
    assert any("not a system library" in p for p in problems)
    assert vw.macos_problems((11, 0, 0), [], "manylinux_2_28_x86_64") == [
        "not a macOS platform tag: manylinux_2_28_x86_64"
    ]


@pytest.mark.parametrize(
    "data",
    [
        b"short",
        make_macho(cputype=0x01000007),
        make_macho(with_version=False),
        make_macho(platform=2),  # iOS
    ],
)
def test_macho_reader_refuses_what_it_cannot_check(data: bytes) -> None:
    with pytest.raises(vw.FormatError):
        vw.macho_requirements(data)


def test_a_macos_extension_with_a_newer_minimum_os_is_reported(tmp_path: Path) -> None:
    members = good_members(tmp_path, "macos-arm64", "ft", make_macho())
    members["pdf_extract/_native.cpython-314t-darwin.so"] = (make_macho((13, 0, 0)), 0o100755)
    name = f"pdf_extract-{VERSION}-cp314-cp314t-macosx_11_0_arm64.whl"
    problems = vw.verify(write_wheel(tmp_path / name, members), "macos-arm64", "ft")
    assert any("extension module minimum macOS 13.0.0" in p for p in problems), problems


def test_the_readers_accept_this_platforms_real_helper() -> None:
    # The synthetic binaries above come from this file's own writers; the
    # staged helper is a real linker's output (CI stages it in every Python
    # job, and the installed wheel's in the `wheel` job).
    helper = Path(pdf_extract._native.__file__).resolve().parent / "pdf-extract-ocr-helper"
    if not helper.is_file():
        pytest.skip("no staged helper (tools/build/stage-helper.sh)")
    data = helper.read_bytes()
    if data[:4] == b"\xcf\xfa\xed\xfe":
        minos, dylibs = vw.macho_requirements(data)
        assert minos >= (10, 0, 0)
        assert any(lib.startswith("/usr/lib/libSystem") for lib in dylibs), dylibs
    elif data[:4] == b"\x7fELF":
        needed, versions = vw.elf_requirements(data)
        assert "libc.so.6" in needed, needed
        assert any(v.startswith("GLIBC_") for v in versions), versions
    else:
        pytest.skip("the helper is in neither format the readers support")


# --- the command line -----------------------------------------------------


def test_main_reports_ok_and_failures(
    tmp_path: Path, linux_members: Members, capsys: pytest.CaptureFixture[str]
) -> None:
    good = write_wheel(tmp_path / linux_name(), linux_members)
    assert vw.main(["--platform", "linux-x86_64", "--abi", "abi3", str(good)]) == 0
    assert "ok" in capsys.readouterr().out
    bad = tmp_path / "bad" / linux_name()
    bad.parent.mkdir()
    bad.write_bytes(b"not a zip")
    assert vw.main(["--platform", "linux-x86_64", "--abi", "abi3", str(bad)]) == 1
    assert "verify-wheel:" in capsys.readouterr().err


# --- the smoke test -------------------------------------------------------


def run_smoke(script: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(script), *args],
        capture_output=True,
        text=True,
        check=False,
        cwd=script.parent,
    )


@pytest.mark.skipif(
    Path(pdf_extract.__file__).resolve().parent != (REPO / "python" / "pdf_extract").resolve(),
    reason="testing an installed wheel (CI `wheel` job): there is no development tree to refuse",
)
def test_smoke_refuses_the_development_tree() -> None:
    out = run_smoke(REPO / "tools" / "build" / "smoke.py")
    assert out.returncode == 1
    assert "imported the development tree" in out.stderr


@needs_helper
def test_smoke_passes_from_outside_the_checkout(tmp_path: Path) -> None:
    script = tmp_path / "smoke.py"
    shutil.copy(REPO / "tools" / "build" / "smoke.py", script)
    out = run_smoke(script)  # the default expectation: process isolation (internal #53)
    assert out.returncode == 0, out.stderr
    assert "smoke: ocr ok (process," in out.stdout
    assert "smoke: all checks passed" in out.stdout


@needs_helper
def test_smoke_reports_the_wrong_isolation(tmp_path: Path) -> None:
    script = tmp_path / "smoke.py"
    shutil.copy(REPO / "tools" / "build" / "smoke.py", script)
    out = run_smoke(script, "--expect-isolation", "in_process")
    assert out.returncode == 1
    assert "default engine isolation is 'process'" in out.stderr


def test_a_toolchain_failure_is_reported(
    tmp_path: Path,
    linux_members: Members,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    def fail(repo: Path) -> rust_std.Toolchain:
        raise rust_std.StdError("toolchain 1.98.0 has no rust-src component")

    monkeypatch.setattr(rust_std, "toolchain", fail)
    wheel = write_wheel(tmp_path / linux_name(), linux_members)
    assert vw.main(["--platform", "linux-x86_64", "--abi", "abi3", str(wheel)]) == 1
    assert "no rust-src component" in capsys.readouterr().err


def test_notices_must_name_every_shipped_crate() -> None:
    notices = (
        "Used by:\n    - pyo3 0.28.0\n\n    std 0.0.0   rust-lang/rust  MIT\n"
        # A crate named only inside licence prose is not acknowledged.
        "Portions of serde 1.0.0 are used with permission.\n"
    )
    doc: sbom.Json = {
        "components": [
            {"name": "pyo3", "version": "0.28.0"},
            {"name": "std", "version": "0.0.0"},
            # Our own crates are covered by LICENSE.
            {"name": "pdf-extract-core", "version": "0.1.0"},
            # A prefix of a named version is not a match.
            {"name": "pyo3", "version": "0.28"},
            {"name": "yo3", "version": "0.28.0"},
            {"name": "serde", "version": "1.0.0"},
            "not a component",
        ]
    }
    assert vw.notices_problems(doc, notices) == [
        "THIRD_PARTY_NOTICES does not name shipped crate pyo3 0.28",
        "THIRD_PARTY_NOTICES does not name shipped crate yo3 0.28.0",
        "THIRD_PARTY_NOTICES does not name shipped crate serde 1.0.0",
    ]
    assert vw.notices_problems({"components": 3}, notices) == []
