#!/usr/bin/env python3
"""Static verification of a built pdf-extract wheel, before it is uploaded
or released (design D6 of the ship-wheels-sbom-process-default change;
packaging spec "Wheel matrix", "Every wheel ships the OCR helper", "Every
wheel ships its license files", "Every wheel carries a CycloneDX SBOM").

    verify_wheel.py --platform linux-x86_64|linux-aarch64|macos-arm64
                    --abi abi3|ft WHEEL

Checks, all of them reported (not just the first):

- the filename's version is the workspace version and its tags are the
  matrix entry's;
- exactly one extension module, of the ABI's kind; no grafted-libraries
  directory (Tesseract and Leptonica are linked statically);
- the OCR helper is present, a regular file with mode 0755 in the zip, and
  for its platform:
    Linux  every ``DT_NEEDED`` library is on manylinux_2_28's allow-list and
           every versioned symbol it needs is within the policy (GLIBC 2.28,
           GLIBCXX 3.4.24, CXXABI 1.3.11, GCC 7.0.0, as in auditwheel's
           policy file). maturin's auditwheel step audits only the
           extension, never an included file, so the helper is checked here;
    macOS  a thin arm64 Mach-O whose minimum OS is no newer than the wheel's
           platform tag, linking only system libraries;
- ``LICENSE``, ``THIRD_PARTY_NOTICES``, ``README-AFM.txt`` and
  ``rust-std-COPYRIGHT.html`` are in ``.dist-info/licenses/``,
  byte-identical to the repository's;
- every crate of the Rust SBOM except our own ``pdf-extract-*`` ones is
  named in the wheel's ``THIRD_PARTY_NOTICES``;
- ``.dist-info/sboms/`` holds exactly ``rust.cdx.json`` and
  ``native.cdx.json`` (plus auditwheel's optional document), each valid
  CycloneDX 1.5; the Rust document lists exactly the crates of a fresh
  ``cargo tree`` for the wheel's target, and the pinned toolchain's
  standard-library crates for it (``rust_std.py``) as their own group;
  the native document agrees with ``vendor/PROVENANCE.md`` and the
  helper's hash; no document contains a build-machine path.

The ELF and Mach-O readers are minimal and dependency-free: only the fields
these checks need. Dev-only: CycloneDX validation uses cyclonedx-python-lib
from the development dependency group.
"""

from __future__ import annotations

import argparse
import functools
import hashlib
import json
import re
import stat
import struct
import sys
import zipfile
from collections.abc import Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "tools" / "license"))
import rust_std  # noqa: E402
import sbom  # noqa: E402

HELPER = "pdf_extract/pdf-extract-ocr-helper"
LICENSE_FILES = {
    "LICENSE": "LICENSE",
    "THIRD_PARTY_NOTICES": "THIRD_PARTY_NOTICES",
    "crates/testkit/data/README-AFM.txt": "crates/testkit/data/README-AFM.txt",
    "tools/license/rust-std-COPYRIGHT.html": "tools/license/rust-std-COPYRIGHT.html",
}
SBOM_FILES = {"rust.cdx.json", "native.cdx.json"}
#: maturin's auditwheel step writes this when it grafts libraries; allowed,
#: although the grafted-libraries check means it should never appear.
OPTIONAL_SBOM = "auditwheel.cdx.json"


@dataclass(frozen=True)
class Platform:
    tag: str
    triple: str
    kind: str  # "elf" or "macho"
    #: ELF e_machine (EM_X86_64, EM_AARCH64); Mach-O checks its own cputype.
    machine: int = 0


PLATFORMS = {
    "linux-x86_64": Platform("manylinux_2_28_x86_64", "x86_64-unknown-linux-gnu", "elf", 0x3E),
    "linux-aarch64": Platform("manylinux_2_28_aarch64", "aarch64-unknown-linux-gnu", "elf", 0xB7),
    "macos-arm64": Platform("macosx_11_0_arm64", "aarch64-apple-darwin", "macho"),
}
#: The wheel's ``Requires-Python`` (pyproject.toml; packaging spec
#: "Supported Python versions").
REQUIRES_PYTHON = ">=3.13"
#: Interpreter/ABI tags and the extension module's file name per ABI.
ABIS = {
    "abi3": ("cp313-abi3", re.compile(r"pdf_extract/_native\.abi3\.so")),
    "ft": ("cp314-cp314t", re.compile(r"pdf_extract/_native\.cpython-314t-[^/]+\.so")),
}

# manylinux_2_28 (auditwheel's manylinux-policy.json). The dynamic loader is
# not on auditwheel's list because it is checked separately; it is allowed.
MANYLINUX_LIBS = frozenset(
    {
        "libatomic.so.1",
        "libgcc_s.so.1",
        "libstdc++.so.6",
        "libm.so.6",
        "libmvec.so.1",
        "libanl.so.1",
        "libdl.so.2",
        "librt.so.1",
        "libc.so.6",
        "libnsl.so.1",
        "libutil.so.1",
        "libpthread.so.0",
        "libresolv.so.2",
        "libz.so.1",
        "ld-linux-x86-64.so.2",
        "ld-linux-aarch64.so.1",
    }
)
MANYLINUX_MAX = {"GLIBC": (2, 28), "GLIBCXX": (3, 4, 24), "CXXABI": (1, 3, 11), "GCC": (7, 0, 0)}
#: Non-numeric symbol versions the policy lists.
MANYLINUX_NAMED = frozenset({"CXXABI_FLOAT128", "CXXABI_TM_1"})


class FormatError(Exception):
    """A binary the minimal readers cannot interpret."""


# --- ELF ------------------------------------------------------------------

_SHT_DYNAMIC = 6
_SHT_GNU_VERNEED = 0x6FFFFFFE
_DT_NEEDED = 1


def _cstr(data: bytes, offset: int) -> str:
    end = data.index(b"\0", offset)
    return data[offset:end].decode("utf-8", "replace")


def elf_requirements(data: bytes, machine: int | None = None) -> tuple[list[str], list[str]]:
    """``(DT_NEEDED libraries, required symbol versions)`` of a 64-bit
    little-endian, dynamically linked ELF file for ``machine`` (when given),
    read through its section headers. A file it cannot read that way (no
    section headers, no dynamic section, another architecture) is a
    ``FormatError``, never an empty and therefore passing answer."""
    if data[:4] != b"\x7fELF" or data[4] != 2 or data[5] != 1:
        raise FormatError("not a 64-bit little-endian ELF file")
    e_machine = struct.unpack_from("<H", data, 0x12)[0]
    if machine is not None and e_machine != machine:
        raise FormatError(f"ELF machine {e_machine:#x}, expected {machine:#x}")
    shoff = struct.unpack_from("<Q", data, 0x28)[0]
    shentsize, shnum = struct.unpack_from("<HH", data, 0x3A)
    if shnum == 0:
        raise FormatError("ELF file has no section headers")
    sections = [
        struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * shentsize) for i in range(shnum)
    ]

    def strtab(index: int) -> tuple[int, int]:
        sh = sections[index]
        return sh[4], sh[5]

    needed: list[str] = []
    versions: list[str] = []
    if not any(sh[1] == _SHT_DYNAMIC for sh in sections):
        raise FormatError("ELF file has no dynamic section")
    for _name, sh_type, _f, _a, offset, size, link, info, _al, _es in sections:
        if sh_type == _SHT_DYNAMIC:
            str_off, _ = strtab(link)
            for pos in range(offset, offset + size, 16):
                tag, val = struct.unpack_from("<qQ", data, pos)
                if tag == 0:
                    break
                if tag == _DT_NEEDED:
                    needed.append(_cstr(data, str_off + val))
        elif sh_type == _SHT_GNU_VERNEED:
            str_off, _ = strtab(link)
            pos = offset
            for _ in range(info):
                _ver, cnt, _file, aux, nxt = struct.unpack_from("<HHIII", data, pos)
                apos = pos + aux
                for _ in range(cnt):
                    _hash, _flags, _other, name, anext = struct.unpack_from("<IHHII", data, apos)
                    versions.append(_cstr(data, str_off + name))
                    apos += anext
                pos += nxt
    return needed, versions


def _version_tuple(text: str) -> tuple[int, ...] | None:
    return tuple(int(p) for p in text.split(".")) if re.fullmatch(r"\d+(\.\d+)*", text) else None


def manylinux_problems(needed: Sequence[str], versions: Sequence[str]) -> list[str]:
    problems = [
        f"needs {lib}, which manylinux_2_28 does not allow"
        for lib in needed
        if lib not in MANYLINUX_LIBS
    ]
    for v in versions:
        family, _, number = v.partition("_")
        if v in MANYLINUX_NAMED:
            continue
        limit = MANYLINUX_MAX.get(family)
        parsed = _version_tuple(number)
        if limit is None or parsed is None:
            problems.append(f"needs symbol version {v}, which manylinux_2_28 does not list")
        elif parsed > limit:
            most = ".".join(map(str, limit))
            problems.append(f"needs {v}, newer than manylinux_2_28 allows ({family}_{most})")
    return problems


# --- Mach-O ---------------------------------------------------------------

_MH_MAGIC_64 = 0xFEEDFACF
_CPU_TYPE_ARM64 = 0x0100000C
_LC_BUILD_VERSION = 0x32
_PLATFORM_MACOS = 1
_LC_VERSION_MIN_MACOSX = 0x24
_DYLIB_COMMANDS = frozenset({0xC, 0x80000018, 0x8000001F, 0x20, 0x80000023})


def macho_requirements(data: bytes) -> tuple[tuple[int, int, int], list[str]]:
    """``(minimum macOS version, linked dylibs)`` of a thin arm64 Mach-O."""
    if len(data) < 32:
        raise FormatError("not a Mach-O file")
    magic, cputype, _sub, _ft, ncmds, _size, _flags, _res = struct.unpack_from("<IiiIIIII", data, 0)
    if magic != _MH_MAGIC_64 or cputype != _CPU_TYPE_ARM64:
        raise FormatError("not a thin 64-bit arm64 Mach-O file")
    pos = 32
    minos: tuple[int, int, int] | None = None
    dylibs: list[str] = []
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", data, pos)
        if cmd == _LC_BUILD_VERSION:
            platform, raw = struct.unpack_from("<II", data, pos + 8)
            if platform != _PLATFORM_MACOS:
                raise FormatError(f"built for Mach-O platform {platform}, not macOS")
            minos = (raw >> 16, (raw >> 8) & 0xFF, raw & 0xFF)
        elif cmd == _LC_VERSION_MIN_MACOSX and minos is None:
            raw = struct.unpack_from("<I", data, pos + 8)[0]
            minos = (raw >> 16, (raw >> 8) & 0xFF, raw & 0xFF)
        elif cmd in _DYLIB_COMMANDS:
            name_off = struct.unpack_from("<I", data, pos + 8)[0]
            dylibs.append(_cstr(data, pos + name_off))
        pos += size
    if minos is None:
        raise FormatError("no minimum OS version load command")
    return minos, dylibs


def macos_problems(minos: tuple[int, int, int], dylibs: Sequence[str], tag: str) -> list[str]:
    m = re.fullmatch(r"macosx_(\d+)_(\d+)_\w+", tag)
    if m is None:
        return [f"not a macOS platform tag: {tag}"]
    allowed = (int(m.group(1)), int(m.group(2)), 0)
    problems = []
    if minos > allowed:
        have = ".".join(map(str, minos))
        problems.append(f"minimum macOS {have} is newer than the tag's {m.group(1)}.{m.group(2)}")
    problems += [
        f"links {lib}, which is not a system library"
        for lib in dylibs
        if not lib.startswith(("/usr/lib/", "/System/Library/"))
    ]
    return problems


# --- the wheel ------------------------------------------------------------


@functools.cache
def _validator() -> Any:
    # Built once: loading the CycloneDX schemas is most of a validation's cost.
    from cyclonedx.schema import SchemaVersion  # noqa: PLC0415 - dev-only dependency
    from cyclonedx.validation.json import JsonStrictValidator  # noqa: PLC0415

    return JsonStrictValidator(SchemaVersion.V1_5)


def _schema_problems(name: str, doc: sbom.Json) -> list[str]:
    error = _validator().validate_str(json.dumps(doc))
    return [] if error is None else [f"{name}: not valid CycloneDX 1.5: {error}"]


def binary_problems(label: str, data: bytes, platform: str) -> list[str]:
    """Portability of one native binary of the wheel (the helper or the
    extension) on ``platform``."""
    plat = PLATFORMS[platform]
    try:
        if plat.kind == "elf":
            found = manylinux_problems(*elf_requirements(data, plat.machine))
        else:
            found = macos_problems(*macho_requirements(data), plat.tag)
    except (FormatError, struct.error, ValueError, IndexError) as e:
        return [f"{label} is not a usable binary for {platform}: {e}"]
    return [f"{label} {p}" for p in found]


def _helper_problems(info: zipfile.ZipInfo, data: bytes, platform: str) -> list[str]:
    problems = []
    mode = info.external_attr >> 16
    if not stat.S_ISREG(mode) or stat.S_IMODE(mode) != 0o755:
        problems.append(f"OCR helper mode is {oct(mode)}, expected a regular file 0o755")
    return problems + binary_problems("OCR helper", data, platform)


def metadata_problems(metadata: str, wheel_file: str, tag: str) -> list[str]:
    """``METADATA`` declares the supported Pythons; ``WHEEL`` carries
    exactly the filename's tag."""
    problems = []
    requires = [
        line.split(":", 1)[1].strip()
        for line in metadata.splitlines()
        if line.startswith("Requires-Python:")
    ]
    if requires != [REQUIRES_PYTHON]:
        problems.append(f"METADATA Requires-Python is {requires}, expected [{REQUIRES_PYTHON!r}]")
    tags = [
        line.split(":", 1)[1].strip() for line in wheel_file.splitlines() if line.startswith("Tag:")
    ]
    if tags != [tag]:
        problems.append(f"WHEEL tags are {tags}, expected [{tag!r}]")
    return problems


def _extensions(names: Sequence[str]) -> list[str]:
    return [n for n in names if re.fullmatch(r"pdf_extract/_native\..*\.so", n)]


def _layout_problems(names: Sequence[str], ext_pattern: re.Pattern[str]) -> list[str]:
    problems = []
    extensions = _extensions(names)
    if len(extensions) != 1 or not ext_pattern.fullmatch(extensions[0]):
        problems.append(
            f"extension modules {extensions}, expected one matching {ext_pattern.pattern}"
        )
    grafted = sorted({n.split("/")[0] for n in names if n.split("/")[0].endswith(".libs")})
    if grafted:
        problems.append(
            f"grafted libraries directory {grafted}: native code must be linked statically"
        )
    return problems


def shipped_crates(repo: Path, platform: str, abi: str) -> list[sbom.Crate]:
    """A fresh ``cargo tree`` of the extension and the helper for the
    wheel's target: what its Rust SBOM must list."""
    triple = PLATFORMS[platform].triple
    features = ["abi3"] if abi == "abi3" else []
    return sbom.merge_graphs(
        {
            "extension": sbom.cargo_tree(repo, sbom.EXTENSION_CRATE, triple, features),
            "helper": sbom.cargo_tree(repo, sbom.HELPER_CRATE, triple, ()),
        }
    )


#: Our own workspace crates: covered by LICENSE, not by THIRD_PARTY_NOTICES.
OWN_CRATE_PREFIX = "pdf-extract-"


#: The lines of THIRD_PARTY_NOTICES that name a crate: Part 1's "Used by:"
#: entries ("    - NAME VERSION") and Part 5's table rows ("    NAME VERSION
#: <origin>  <licence>"). Anything else (licence texts quoting crate names)
#: does not count as acknowledging a crate.
_NOTICE_ENTRY = re.compile(
    r"^    (?:- (?P<n1>\S+) (?P<v1>\S+)"
    r"|(?P<n5>\S+) (?P<v5>\S+) +(?:crates\.io|rust-lang/rust) .*)$",
    re.MULTILINE,
)


def notice_entries(notices: str) -> set[tuple[str, str]]:
    return {
        (m["n1"], m["v1"]) if m["n1"] else (m["n5"], m["v5"])
        for m in _NOTICE_ENTRY.finditer(notices)
    }


def notices_problems(rust_doc: sbom.Json, notices: str) -> list[str]:
    """Every non-workspace crate of the wheel's Rust SBOM is named in the
    wheel's THIRD_PARTY_NOTICES, as a Part 1 "Used by:" entry or a Part 5
    row (licensing spec "Notices cover every shipped crate"). The two are
    generated from different graphs (crates/python's for all targets, and
    each target's extension plus helper), so this is what catches them
    drifting apart."""
    named = notice_entries(notices)
    problems = []
    comps = rust_doc.get("components")
    for comp in comps if isinstance(comps, list) else []:
        if not isinstance(comp, dict):
            continue
        name, version = str(comp.get("name")), str(comp.get("version"))
        if name.startswith(OWN_CRATE_PREFIX):
            continue
        if (name, version) not in named:
            problems.append(f"THIRD_PARTY_NOTICES does not name shipped crate {name} {version}")
    return problems


def _shipped_notices_problems(
    zf: zipfile.ZipFile, licenses: str, docs: dict[str, sbom.Json]
) -> list[str]:
    """notices_problems on the wheel's own files; a missing notices file or
    Rust SBOM is reported by the presence checks instead."""
    names = set(zf.namelist())
    member = f"{licenses}THIRD_PARTY_NOTICES"
    if "rust.cdx.json" not in docs or member not in names:
        return []
    return notices_problems(docs["rust.cdx.json"], zf.read(member).decode("utf-8"))


def shipped_std(repo: Path, platform: str) -> tuple[str, list[rust_std.StdCrate]]:
    """The pinned toolchain's version and the standard-library crates it
    links for the wheel's target: the SBOM's std group."""
    try:
        tc = rust_std.toolchain(repo)
        return tc.version, rust_std.std_crates(tc, PLATFORMS[platform].triple)
    except rust_std.StdError as e:
        raise sbom.SbomError(str(e)) from None


def _sbom_problems(
    docs: dict[str, sbom.Json],
    repo: Path,
    *,
    helper_sha: str | None,
    expected_crates: list[sbom.Crate],
    expected_std: tuple[str, list[rust_std.StdCrate]],
) -> list[str]:
    problems = []
    for name, doc in sorted(docs.items()):
        problems += _schema_problems(name, doc)
        problems += [f"{name}: {p}" for p in sbom.absolute_path_problems(doc)]
    if "rust.cdx.json" in docs:
        clarified = sbom.clarified_licenses(repo / "about.toml")
        problems += sbom.rust_problems(docs["rust.cdx.json"], expected_crates, clarified)
        toolchain_version, std = expected_std
        problems += sbom.std_problems(docs["rust.cdx.json"], std, toolchain_version)
    if "native.cdx.json" in docs:
        text = (repo / "vendor" / "PROVENANCE.md").read_text(encoding="utf-8")
        provenance = sbom.parse_provenance(text)
        problems += sbom.native_problems(docs["native.cdx.json"], provenance, helper_sha)
    return problems


def verify(  # noqa: PLR0913 - the matrix entry plus test overrides
    wheel: Path,
    platform: str,
    abi: str,
    *,
    repo: Path = REPO,
    expected_crates: list[sbom.Crate] | None = None,
    expected_std: tuple[str, list[rust_std.StdCrate]] | None = None,
) -> list[str]:
    """Every problem with ``wheel`` as the ``(platform, abi)`` matrix entry.
    ``expected_crates`` defaults to a fresh ``cargo tree`` for the target,
    ``expected_std`` (toolchain version, crates) to the pinned toolchain's
    standard library for it."""
    plat = PLATFORMS[platform]
    abi_tag, ext_pattern = ABIS[abi]
    version = sbom.workspace_version(repo)
    dist_info = f"pdf_extract-{version}.dist-info"
    problems: list[str] = []

    want_name = f"pdf_extract-{version}-{abi_tag}-{plat.tag}.whl"
    if wheel.name != want_name:
        problems.append(f"file name is {wheel.name}, expected {want_name}")

    with zipfile.ZipFile(wheel) as zf:
        infos = {i.filename: i for i in zf.infolist()}
        problems += _layout_problems(list(infos), ext_pattern)
        for ext in _extensions(list(infos)):
            problems += binary_problems("extension module", zf.read(ext), platform)
        try:
            problems += metadata_problems(
                zf.read(f"{dist_info}/METADATA").decode(),
                zf.read(f"{dist_info}/WHEEL").decode(),
                f"{abi_tag}-{plat.tag}",
            )
        except KeyError as e:
            problems.append(f"wheel metadata missing: {e}")

        helper_sha: str | None = None
        info = infos.get(HELPER)
        if info is None:
            problems.append(f"no OCR helper at {HELPER}")
        else:
            data = zf.read(info)
            helper_sha = hashlib.sha256(data).hexdigest()
            problems += _helper_problems(info, data, platform)

        for in_wheel, in_repo in LICENSE_FILES.items():
            member = f"{dist_info}/licenses/{in_wheel}"
            if member not in infos:
                problems.append(f"license file {member} missing")
            elif zf.read(member) != (repo / in_repo).read_bytes():
                problems.append(f"license file {member} differs from {in_repo}")

        prefix = f"{dist_info}/sboms/"
        present = {n[len(prefix) :] for n in infos if n.startswith(prefix)}
        if present - {OPTIONAL_SBOM} != SBOM_FILES:
            problems.append(f"SBOM documents {sorted(present)}, expected {sorted(SBOM_FILES)}")
        docs = {n: json.loads(zf.read(prefix + n)) for n in present}
        problems += _shipped_notices_problems(zf, f"{dist_info}/licenses/", docs)

    if "rust.cdx.json" not in docs:
        expected_crates, expected_std = [], ("", [])
    if expected_crates is None:
        expected_crates = shipped_crates(repo, platform, abi)
    if expected_std is None:
        expected_std = shipped_std(repo, platform)
    return problems + _sbom_problems(
        docs,
        repo,
        helper_sha=helper_sha,
        expected_crates=expected_crates,
        expected_std=expected_std,
    )


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Verify a built pdf-extract wheel.")
    parser.add_argument("--platform", required=True, choices=sorted(PLATFORMS))
    parser.add_argument("--abi", required=True, choices=sorted(ABIS))
    parser.add_argument("wheel", type=Path)
    args = parser.parse_args(argv)
    try:
        problems = verify(args.wheel, args.platform, args.abi)
    except (sbom.SbomError, zipfile.BadZipFile, OSError, ValueError, KeyError) as e:
        # ValueError covers json.JSONDecodeError (a malformed SBOM document).
        problems = [str(e)]
    for p in problems:
        print(f"verify-wheel: {p}", file=sys.stderr)
    if problems:
        return 1
    print(f"verify-wheel: {args.wheel.name} ok")
    return 0


if __name__ == "__main__":  # pragma: no cover - the tests call main()
    sys.exit(main())
