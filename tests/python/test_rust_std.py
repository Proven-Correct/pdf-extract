"""The standard-library resolver, ``tools/license/rust_std.py`` (licensing
spec "Generated third-party notices", scenarios "Standard library
acknowledged" and "Standard library licenses on the allow-list"; design D1-D2
of the complete-license-pipeline change).

Most cases run against a planted sysroot and a stand-in for ``cargo tree``,
so each rule is exercised in isolation; one case resolves the real pinned
toolchain, which CI installs with rust-src from rust-toolchain.toml.
"""

from __future__ import annotations

import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path

import pytest

from pdfs import REPO

sys.path.insert(0, str(REPO / "tools" / "license"))
sys.path.insert(0, str(REPO / "tools" / "build"))
import rust_std
import verify_wheel

MIT_TEXT = "Permission is hereby granted, free of charge\n"


def completed(
    stdout: str = "", returncode: int = 0, stderr: str = ""
) -> subprocess.CompletedProcess[str]:
    return subprocess.CompletedProcess([], returncode, stdout, stderr)


@pytest.fixture
def sysroot(tmp_path: Path) -> rust_std.Toolchain:
    """A sysroot with rust-src (std in-tree, libc vendored) and a target's
    rlibs for exactly those two crates plus the known unlinked ones."""
    tc = rust_std.Toolchain("1.98.0", tmp_path / "sysroot")
    lib = tc.library
    (lib / "std").mkdir(parents=True)
    (lib / "Cargo.lock").write_text("")
    vendored = lib / "vendor" / "libc-0.2.185"
    vendored.mkdir(parents=True)
    (vendored / "LICENSE-MIT").write_text(MIT_TEXT)
    (vendored / "LICENSE-APACHE").write_text("Apache License\n")
    rlibs = tc.sysroot / "lib" / "rustlib" / "x86_64-unknown-linux-gnu" / "lib"
    rlibs.mkdir(parents=True)
    for name in ["std", "libc", "test", "getopts"]:
        (rlibs / f"lib{name}-0123abcd.rlib").write_text("")
    copyright_doc = tc.sysroot / rust_std.COPYRIGHT_IN_SYSROOT
    copyright_doc.parent.mkdir(parents=True)
    copyright_doc.write_text("<html>notices</html>\n")
    return tc


def tree_output(tc: rust_std.Toolchain) -> str:
    return (
        f"std v0.0.0 ({tc.library}/std)|MIT OR Apache-2.0\n"
        "libc v0.2.185|MIT/Apache-2.0\n"
        "libc v0.2.185|MIT/Apache-2.0 (*)\n"
        "\n"
    )


def runner(stdout: str, returncode: int = 0) -> rust_std.Runner:
    def run(cmd: Sequence[str], **kw: object) -> subprocess.CompletedProcess[str]:
        return completed(stdout, returncode, "boom")

    return run


# --- the toolchain ---------------------------------------------------------


def test_pinned_channel_is_rust_toolchain_toml() -> None:
    text = (REPO / "rust-toolchain.toml").read_text()
    assert f'channel = "{rust_std.pinned_channel(REPO)}"' in text
    assert '"rust-src"' in text


def test_a_non_release_channel_is_refused(tmp_path: Path) -> None:
    (tmp_path / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "stable"\n')
    with pytest.raises(rust_std.StdError, match="not a release version"):
        rust_std.pinned_channel(tmp_path)


def test_toolchain_runs_the_pin_through_rustup(tmp_path: Path, sysroot: rust_std.Toolchain) -> None:
    (tmp_path / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.0"\n')
    seen: list[Sequence[str]] = []

    def run(cmd: Sequence[str], **kw: object) -> subprocess.CompletedProcess[str]:
        seen.append(cmd)
        return completed(f"{sysroot.sysroot}\n")

    tc = rust_std.toolchain(tmp_path, run)
    assert tc == sysroot
    assert seen == [["rustup", "run", "1.98.0", "rustc", "--print", "sysroot"]]


def test_toolchain_failures(tmp_path: Path) -> None:
    (tmp_path / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.0"\n')
    with pytest.raises(rust_std.StdError, match="rustup cannot run"):
        rust_std.toolchain(tmp_path, runner("", 1))
    with pytest.raises(rust_std.StdError, match="no rust-src"):
        rust_std.toolchain(tmp_path, runner(f"{tmp_path}\n"))


# --- the crate set -----------------------------------------------------------


def test_std_crates_parses_in_tree_and_vendored(sysroot: rust_std.Toolchain) -> None:
    seen: dict[str, object] = {}

    def run(cmd: Sequence[str], **kw: object) -> subprocess.CompletedProcess[str]:
        seen.update(kw, cmd=cmd)
        return completed(tree_output(sysroot))

    crates = rust_std.std_crates(sysroot, "x86_64-unknown-linux-gnu", run)
    assert [(c.name, c.version, c.license, c.source, c.vendored) for c in crates] == [
        ("libc", "0.2.185", "MIT OR Apache-2.0", "vendor/libc-0.2.185", True),
        ("std", "0.0.0", "MIT OR Apache-2.0", "std", False),
    ]
    assert all(c.targets == {"x86_64-unknown-linux-gnu"} for c in crates)
    # RUSTC_BOOTSTRAP only in the subprocess, the pinned toolchain, in library/.
    env = seen["env"]
    assert isinstance(env, dict)
    assert env["RUSTC_BOOTSTRAP"] == "1"
    assert seen["cwd"] == sysroot.library
    cmd = seen["cmd"]
    assert isinstance(cmd, list)
    assert cmd[:4] == ["rustup", "run", "1.98.0", "cargo"]
    assert rust_std.STD_FEATURES in cmd


def test_a_missing_rlib_fails(sysroot: rust_std.Toolchain) -> None:
    out = tree_output(sysroot) + "hashbrown v0.17.1|MIT OR Apache-2.0\n"
    (sysroot.library / "vendor" / "hashbrown-0.17.1").mkdir()
    with pytest.raises(rust_std.StdError, match="no rlib for standard library crate hashbrown"):
        rust_std.std_crates(sysroot, "x86_64-unknown-linux-gnu", runner(out))


def test_an_unaccounted_rlib_fails(sysroot: rust_std.Toolchain) -> None:
    rlibs = sysroot.sysroot / "lib" / "rustlib" / "x86_64-unknown-linux-gnu" / "lib"
    (rlibs / "libgimli-feedface.rlib").write_text("")
    with pytest.raises(rust_std.StdError, match="rust-std ships gimli"):
        rust_std.std_crates(sysroot, "x86_64-unknown-linux-gnu", runner(tree_output(sysroot)))


def test_without_rust_std_for_the_target_no_cross_check(sysroot: rust_std.Toolchain) -> None:
    crates = rust_std.std_crates(sysroot, "aarch64-apple-darwin", runner(tree_output(sysroot)))
    assert [c.name for c in crates] == ["libc", "std"]


def test_cargo_tree_failure_is_reported(sysroot: rust_std.Toolchain) -> None:
    with pytest.raises(rust_std.StdError, match="failed:\nboom"):
        rust_std.std_crates(sysroot, "x86_64-unknown-linux-gnu", runner("", 101))


@pytest.mark.parametrize(
    ("line", "message"),
    [
        ("garbage", "unexpected cargo tree line"),
        ("libc v0.2.185|", "declares no licence"),
        ("serde v1.0.0|MIT", "not vendored in rust-src"),
        ("std v0.0.0 (/elsewhere/std)|MIT", "outside library/"),
    ],
)
def test_bad_tree_lines_are_refused(sysroot: rust_std.Toolchain, line: str, message: str) -> None:
    with pytest.raises(rust_std.StdError, match=message):
        rust_std.parse_tree_line(line, sysroot.library)


def test_merge_records_targets() -> None:
    a = rust_std.StdCrate("std", "0.0.0", "MIT", "std", frozenset({"t1"}))
    b = rust_std.StdCrate("std", "0.0.0", "MIT", "std", frozenset({"t2"}))
    c = rust_std.StdCrate("libc", "0.2.0", "MIT", "vendor/libc-0.2.0", frozenset({"t2"}))
    merged = rust_std.merge_targets([[a], [b, c]])
    assert [(m.name, m.targets) for m in merged] == [
        ("libc", frozenset({"t2"})),
        ("std", frozenset({"t1", "t2"})),
    ]


def test_shipped_targets_match_the_wheel_matrix() -> None:
    assert set(rust_std.SHIPPED_TARGETS) == {p.triple for p in verify_wheel.PLATFORMS.values()}


# --- SPDX ------------------------------------------------------------------

ALLOW = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "Zlib"]


@pytest.mark.parametrize(
    ("expr", "ok"),
    [
        ("MIT", True),
        ("GPL-3.0-only", False),
        ("MIT OR GPL-3.0-only", True),
        ("MIT AND GPL-3.0-only", False),
        ("MIT AND Apache-2.0 WITH LLVM-exception AND (MIT OR Apache-2.0)", True),
        ("GPL-2.0-only WITH Classpath-exception-2.0", False),
        ("(LGPL-2.1-only OR Zlib) AND (MIT)", True),
    ],
)
def test_spdx_satisfaction(expr: str, ok: bool) -> None:
    assert rust_std.satisfied(expr, ALLOW) is ok


@pytest.mark.parametrize("expr", ["", "MIT OR", "(MIT", "(MIT Zlib)", "MIT)", "AND MIT", "MIT $"])
def test_malformed_spdx_is_refused(expr: str) -> None:
    with pytest.raises(rust_std.StdError):
        rust_std.satisfied(expr, ALLOW)


def test_legacy_slash_licence_means_or() -> None:
    assert rust_std.normalise_license("MIT/Apache-2.0") == "MIT OR Apache-2.0"
    assert rust_std.normalise_license("MIT OR Apache-2.0") == "MIT OR Apache-2.0"


def test_accepted_licenses_come_from_about_toml(tmp_path: Path) -> None:
    assert rust_std.accepted_licenses(REPO)[0] == "MIT"
    (tmp_path / "about.toml").write_text("accepted = 3\n")
    with pytest.raises(rust_std.StdError, match="no `accepted` list"):
        rust_std.accepted_licenses(tmp_path)


def test_chosen_license_follows_the_accepted_order() -> None:
    crate = rust_std.StdCrate("memchr", "2.7.6", "Unlicense OR MIT", "vendor/memchr-2.7.6")
    assert rust_std.chosen_license(crate, ["MIT", "Unlicense"]) == "MIT"
    assert rust_std.chosen_license(crate, ["Unlicense", "MIT"]) == "Unlicense"
    both = rust_std.StdCrate("x", "1.0.0", "MIT AND Zlib", "vendor/x-1.0.0")
    with pytest.raises(rust_std.StdError, match="no single accepted licence"):
        rust_std.chosen_license(both, ALLOW)


def test_license_file_lookup(tmp_path: Path) -> None:
    (tmp_path / "LICENSE-MIT.md").write_text(MIT_TEXT)
    (tmp_path / "README.md").write_text("MIT")
    assert rust_std.license_file(tmp_path, "MIT").name == "LICENSE-MIT.md"
    with pytest.raises(rust_std.StdError, match="no Zlib licence file"):
        rust_std.license_file(tmp_path, "Zlib")
    # Whole tokens only: BSD-3-Clause must not take the 0BSD file.
    (tmp_path / "LICENSE-0BSD").write_text("0BSD")
    with pytest.raises(rust_std.StdError, match="no BSD-3-Clause licence file"):
        rust_std.license_file(tmp_path, "BSD-3-Clause")
    (tmp_path / "LICENSE-BSD").write_text("BSD")
    assert rust_std.license_file(tmp_path, "BSD-3-Clause").name == "LICENSE-BSD"
    assert rust_std.license_file(tmp_path, "0BSD").name == "LICENSE-0BSD"
    with pytest.raises(rust_std.StdError, match="no licence-file rule"):
        rust_std.license_file(tmp_path, "OFL-1.1")


# --- Part 5 ----------------------------------------------------------------


def test_notices_part_5(sysroot: rust_std.Toolchain) -> None:
    crates = rust_std.std_crates(sysroot, "x86_64-unknown-linux-gnu", runner(tree_output(sysroot)))
    text = rust_std.notices(REPO, sysroot, crates)
    assert text.startswith("Part 5. Rust standard library (Rust 1.98.0)\n")
    only = "[x86_64-unknown-linux-gnu only]"
    assert f"libc 0.2.185  crates.io       MIT OR Apache-2.0  {only}" in text
    assert f"std 0.0.0     rust-lang/rust  MIT OR Apache-2.0  {only}" in text
    assert "rust-std-COPYRIGHT.html" in text
    assert "GCC Runtime Library Exception" in text
    assert f"libc-0.2.185/LICENSE-MIT verbatim:\n\n{MIT_TEXT}" in text
    # In-tree crates carry no licence section of their own.
    assert "std 0.0.0 (MIT)" not in text
    assert text.endswith("\n")
    assert not text.endswith("\n\n")


def test_notices_refuse_a_licence_outside_the_allow_list(sysroot: rust_std.Toolchain) -> None:
    gpl = rust_std.StdCrate("evil", "1.0.0", "GPL-3.0-only", "vendor/evil-1.0.0")
    with pytest.raises(rust_std.StdError, match="outside the allow-list"):
        rust_std.notices(REPO, sysroot, [gpl])


def test_verbatim_adds_a_missing_final_newline(sysroot: rust_std.Toolchain) -> None:
    (sysroot.library / "vendor" / "libc-0.2.185" / "LICENSE-MIT").write_text("no newline")
    crates = rust_std.std_crates(sysroot, "x86_64-unknown-linux-gnu", runner(tree_output(sysroot)))
    assert "no newline\n" in rust_std.notices(REPO, sysroot, crates)


# --- main and the real toolchain ---------------------------------------------


def test_main_usage(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch, sysroot: rust_std.Toolchain
) -> None:
    monkeypatch.setattr(rust_std, "toolchain", lambda repo: sysroot)
    assert rust_std.main(["bogus"]) == 2
    assert "usage" in capsys.readouterr().err


def test_main_copyright(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, sysroot: rust_std.Toolchain
) -> None:
    monkeypatch.setattr(rust_std, "toolchain", lambda repo: sysroot)
    dest = tmp_path / "out.html"
    assert rust_std.main(["copyright", str(dest)]) == 0
    assert dest.read_text() == "<html>notices</html>\n"


def test_main_reports_errors(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch
) -> None:
    def fail(repo: Path) -> rust_std.Toolchain:
        raise rust_std.StdError("no toolchain")

    monkeypatch.setattr(rust_std, "toolchain", fail)
    assert rust_std.main(["notices"]) == 1
    assert "rust_std: no toolchain" in capsys.readouterr().err


def test_main_notices(
    capsys: pytest.CaptureFixture[str], monkeypatch: pytest.MonkeyPatch, sysroot: rust_std.Toolchain
) -> None:
    real = rust_std.std_crates
    monkeypatch.setattr(rust_std, "toolchain", lambda repo: sysroot)
    monkeypatch.setattr(
        rust_std, "std_crates", lambda tc, target: real(tc, target, runner(tree_output(sysroot)))
    )
    assert rust_std.main(["notices"]) == 0
    out = capsys.readouterr().out
    assert out.startswith("Part 5.")
    # Every shipped target links the same set, so no row is target-only.
    assert "only]" not in out


def test_real_toolchain_resolves_std() -> None:
    """The pinned toolchain's std, as the wheels link it: the backtrace
    crates are there, the test harness is not, and the committed notices
    carry exactly this Part 5."""
    tc = rust_std.toolchain(REPO)
    crates = rust_std.merge_targets([rust_std.std_crates(tc, t) for t in rust_std.SHIPPED_TARGETS])
    names = {c.name for c in crates}
    assert {"std", "core", "alloc", "compiler_builtins", "gimli", "libc"} <= names
    assert not names & {"test", "getopts", "proc_macro"}
    notices = (REPO / "THIRD_PARTY_NOTICES").read_text(encoding="utf-8")
    assert rust_std.notices(REPO, tc, crates) in notices
