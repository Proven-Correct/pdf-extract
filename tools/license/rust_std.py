#!/usr/bin/env python3
"""The Rust standard library that every shipped binary links statically
(design D1-D2 of the complete-license-pipeline change; licensing spec
"Generated third-party notices", packaging spec "Every wheel carries a
CycloneDX SBOM").

cargo-about and ``cargo tree`` see only our own graph, never the sysroot, yet
``_native`` and the OCR helper both link ``std``, ``core``, ``alloc``,
``compiler_builtins`` and std's own crates.io dependencies. This module is
the one place that says which crates those are, for both consumers:
``sbom.py`` (components of ``rust.cdx.json``) and ``gen-notices.sh`` (Part 5
of THIRD_PARTY_NOTICES, plus a verbatim copy of the toolchain's
``COPYRIGHT-library.html``).

The set is derived, never typed: ``cargo tree -p std --features
backtrace,panic-unwind`` over the ``library/`` workspace of the pinned
toolchain's ``rust-src`` component, the features the Rust project's own
distribution builds std with. When the target's ``rust-std`` component is
installed, the result is cross-checked against the rlibs it ships, which is
what proves that feature choice right. The manifest uses an unstable cargo
feature, so that one ``cargo tree`` call (a metadata read, nothing is
compiled) runs with ``RUSTC_BOOTSTRAP=1``; the variable is set in that
subprocess's environment only.

The toolchain is always the ``rust-toolchain.toml`` pin, run through
``rustup run``: a ``rustc`` earlier on PATH (a system package) would
describe a different standard library.

Usage:
    rust_std.py notices               # Part 5 of THIRD_PARTY_NOTICES, to stdout
    rust_std.py copyright DEST        # copy COPYRIGHT-library.html to DEST
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tomllib
from collections.abc import Callable, Sequence
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

#: The targets wheels are built for (verify_wheel.PLATFORMS; a test keeps the
#: two equal). Part 5 lists the union of their std sets.
SHIPPED_TARGETS = (
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
)
#: The features the Rust distribution builds std with (library/sysroot's
#: default ``panic-unwind``, plus ``backtrace``, which x.py enables).
STD_FEATURES = "backtrace,panic-unwind"
#: rlibs that rust-std ships but no cdylib or non-test binary links: the test
#: harness and its option parser, the proc-macro bridge, profiling support,
#: and the sysroot umbrella with its own-std shim.
UNLINKED_RLIBS = frozenset(
    {
        "test",
        "getopts",
        "proc_macro",
        "profiler_builtins",
        "sysroot",
        "rustc_literal_escaper",
        "rustc_std_workspace_std",
    }
)
#: The Rust project's standard-library notice inside the sysroot, and the
#: committed copy that ships as a licence file (pyproject ``license-files``).
COPYRIGHT_IN_SYSROOT = Path("share/doc/rust/COPYRIGHT-library.html")
COPYRIGHT_COMMITTED = "tools/license/rust-std-COPYRIGHT.html"
#: Licence-file name tokens per SPDX id, for picking a vendored crate's
#: file for the licence chosen from its expression. Matched as whole tokens
#: of the upper-cased name, so ``BSD`` does not match ``LICENSE-0BSD``.
_FILE_TOKENS = {
    "MIT": "MIT",
    "Apache-2.0": "APACHE",
    "Unlicense": "UNLICENSE",
    "Zlib": "ZLIB",
    "0BSD": "0BSD",
    "BSL-1.0": "BSL",
    "ISC": "ISC",
    "BSD-2-Clause": "BSD",
    "BSD-3-Clause": "BSD",
}


class StdError(Exception):
    """The toolchain, its sources or a licence the generator cannot use."""


@dataclass(frozen=True)
class Toolchain:
    version: str
    sysroot: Path

    @property
    def library(self) -> Path:
        return self.sysroot / "lib" / "rustlib" / "src" / "rust" / "library"


@dataclass(frozen=True)
class StdCrate:
    """A standard-library crate. ``source`` is relative to ``library/``: the
    crate's directory in the Rust repository (``std``, ``compiler-builtins/
    compiler-builtins``) or, for a crate vendored from crates.io,
    ``vendor/NAME-VERSION``. ``targets`` are the shipped targets linking it."""

    name: str
    version: str
    license: str
    source: str
    targets: frozenset[str] = frozenset()

    @property
    def vendored(self) -> bool:
        return self.source.startswith("vendor/")


Runner = Callable[..., subprocess.CompletedProcess[str]]


def _run(cmd: Sequence[str], **kw: object) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, check=False, capture_output=True, text=True, **kw)  # type: ignore[call-overload,no-any-return]


def pinned_channel(repo: Path) -> str:
    with (repo / "rust-toolchain.toml").open("rb") as f:
        channel = tomllib.load(f).get("toolchain", {}).get("channel")
    if not isinstance(channel, str) or not re.fullmatch(r"\d+\.\d+\.\d+", channel):
        raise StdError(f"rust-toolchain.toml pins {channel!r}, not a release version")
    return channel


def toolchain(repo: Path, run: Runner = _run) -> Toolchain:
    """The pinned toolchain's version and sysroot, which must have rust-src."""
    channel = pinned_channel(repo)
    out = run(["rustup", "run", channel, "rustc", "--print", "sysroot"])
    if out.returncode != 0:
        raise StdError(f"rustup cannot run toolchain {channel}:\n{out.stderr.strip()}")
    tc = Toolchain(channel, Path(out.stdout.strip()))
    if not (tc.library / "Cargo.lock").is_file():
        raise StdError(f"toolchain {channel} has no rust-src component (rust-toolchain.toml)")
    return tc


# `cargo tree -f '{p}|{l}'`: "name vX.Y.Z[ (path)]|licence[ (*)]".
_TREE_LINE = re.compile(r"(?P<name>\S+) v(?P<version>\S+)(?: \((?P<path>/[^)]*)\))?")


def normalise_license(lic: str) -> str:
    """cargo's legacy ``A/B`` form (no spaces) means ``A OR B``."""
    lic = lic.strip()
    if "/" in lic and " " not in lic:
        return " OR ".join(lic.split("/"))
    return lic


def parse_tree_line(line: str, library: Path) -> StdCrate:
    line = line.removesuffix(" (*)")
    pkg, sep, lic = line.partition("|")
    m = _TREE_LINE.fullmatch(pkg.strip())
    if not sep or m is None:
        raise StdError(f"unexpected cargo tree line: {line!r}")
    name, version, path = m.group("name"), m.group("version"), m.group("path")
    if not lic.strip():
        raise StdError(f"standard library crate {name} {version} declares no licence")
    if path is None:
        source = f"vendor/{name}-{version}"
        if not (library / source).is_dir():
            raise StdError(f"{name} {version} is not vendored in rust-src")
    else:
        try:
            source = Path(path).resolve().relative_to(library.resolve()).as_posix()
        except ValueError:
            raise StdError(f"{name} {version} lies outside library/: {path}") from None
    return StdCrate(name, version, normalise_license(lic), source)


def rlib_problems(crates: Sequence[StdCrate], rlib_dir: Path) -> list[str]:
    """Agreement between the derived set and the rlibs rust-std ships."""
    rlibs = {p.name[3:].rsplit("-", 1)[0] for p in rlib_dir.glob("lib*-*.rlib")}
    names = {c.name.replace("-", "_") for c in crates}
    problems = [f"no rlib for standard library crate {n}" for n in sorted(names - rlibs)]
    problems += [
        f"rust-std ships {n}, which the derived set lacks"
        for n in sorted(rlibs - names - UNLINKED_RLIBS)
    ]
    return problems


def std_crates(tc: Toolchain, target: str, run: Runner = _run) -> list[StdCrate]:
    """The standard-library crates linked into a binary for ``target``."""
    env = {**os.environ, "RUSTC_BOOTSTRAP": "1"}
    cmd = [
        "rustup", "run", tc.version, "cargo", "tree", "--offline", "--locked",
        "--color", "never", "-p", "std", "--features", STD_FEATURES,
        "-e", "normal", "--target", target, "--prefix", "none", "-f", "{p}|{l}",
    ]  # fmt: skip
    out = run(cmd, cwd=tc.library, env=env)
    if out.returncode != 0:
        raise StdError(f"cargo tree over the standard library failed:\n{out.stderr.strip()}")
    found: dict[tuple[str, str], StdCrate] = {}
    for line in out.stdout.splitlines():
        if line.strip():
            c = parse_tree_line(line, tc.library)
            found[(c.name, c.version)] = StdCrate(
                c.name, c.version, c.license, c.source, frozenset({target})
            )
    crates = sorted(found.values(), key=lambda c: (c.name, c.version))
    rlib_dir = tc.sysroot / "lib" / "rustlib" / target / "lib"
    if rlib_dir.is_dir():
        problems = rlib_problems(crates, rlib_dir)
        if problems:
            raise StdError("; ".join(problems))
    return crates


def merge_targets(sets: Sequence[Sequence[StdCrate]]) -> list[StdCrate]:
    merged: dict[tuple[str, str], StdCrate] = {}
    for crates in sets:
        for c in crates:
            prior = merged.get((c.name, c.version))
            targets = c.targets | (prior.targets if prior else frozenset())
            merged[(c.name, c.version)] = StdCrate(c.name, c.version, c.license, c.source, targets)
    return sorted(merged.values(), key=lambda c: (c.name, c.version))


# --- SPDX expressions -----------------------------------------------------

_TOKEN = re.compile(r"\s*(\(|\)|[A-Za-z0-9.+-]+)")


def _tokens(expr: str) -> list[str]:
    out, pos = [], 0
    expr = expr.rstrip()
    while pos < len(expr):
        m = _TOKEN.match(expr, pos)
        if m is None:
            raise StdError(f"cannot parse licence expression {expr!r}")
        out.append(m.group(1))
        pos = m.end()
    return out


def satisfied(expr: str, allowed: Sequence[str]) -> bool:
    """Whether ``expr`` can be met using only ``allowed`` licences (an id,
    or ``id WITH exception``, must be listed as such)."""
    toks = _tokens(expr)
    pos = 0

    def peek() -> str | None:
        return toks[pos] if pos < len(toks) else None

    def take() -> str:
        nonlocal pos
        if pos >= len(toks):
            raise StdError(f"truncated licence expression {expr!r}")
        pos += 1
        return toks[pos - 1]

    def atom() -> bool:
        tok = take()
        if tok == "(":
            value = disjunction()
            if take() != ")":
                raise StdError(f"unbalanced licence expression {expr!r}")
            return value
        if tok in {")", "AND", "OR", "WITH"}:
            raise StdError(f"unexpected {tok!r} in licence expression {expr!r}")
        if peek() == "WITH":
            take()
            tok = f"{tok} WITH {take()}"
        return tok in allowed

    def conjunction() -> bool:
        value = atom()
        while peek() == "AND":
            take()
            value = atom() and value
        return value

    def disjunction() -> bool:
        value = conjunction()
        while peek() == "OR":
            take()
            value = conjunction() or value
        return value

    result = disjunction()
    if pos != len(toks):
        raise StdError(f"trailing tokens in licence expression {expr!r}")
    return result


def accepted_licenses(repo: Path) -> list[str]:
    """about.toml's ``accepted`` list: deny.toml's allow-list, in its order
    (the licence test keeps them equal)."""
    with (repo / "about.toml").open("rb") as f:
        accepted = tomllib.load(f).get("accepted")
    if not isinstance(accepted, list) or not all(isinstance(a, str) for a in accepted):
        raise StdError("about.toml has no `accepted` list")
    return accepted


def chosen_license(crate: StdCrate, accepted: Sequence[str]) -> str:
    """The first accepted licence that alone satisfies the crate's
    expression, as cargo-about picks for Part 1."""
    for lic in accepted:
        if satisfied(crate.license, [lic]):
            return lic
    raise StdError(f"{crate.name} {crate.version}: no single accepted licence in {crate.license!r}")


def license_file(directory: Path, lic: str) -> Path:
    token = _FILE_TOKENS.get(lic)
    if token is None:
        raise StdError(f"no licence-file rule for {lic}")
    candidates = sorted(
        p
        for p in directory.iterdir()
        if p.is_file()
        and re.match(r"(LICEN[CS]E|COPYING|UNLICENSE)", p.name, re.IGNORECASE)
        and re.search(rf"(?<![A-Z0-9]){token}(?![A-Z0-9])", p.name.upper())
    )
    if not candidates:
        raise StdError(f"{directory.name}: no {lic} licence file")
    return candidates[0]


# --- THIRD_PARTY_NOTICES Part 5 -------------------------------------------


def _heading(title: str, char: str) -> str:
    return f"{title}\n{char * len(title)}\n\n"


def _verbatim(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    return text if text.endswith("\n") else text + "\n"


def notices(repo: Path, tc: Toolchain, crates: Sequence[StdCrate]) -> str:
    accepted = accepted_licenses(repo)
    for c in crates:
        if not satisfied(c.license, accepted):
            raise StdError(f"{c.name} {c.version}: {c.license!r} is outside the allow-list")
    every = frozenset(SHIPPED_TARGETS)
    out = [_heading(f"Part 5. Rust standard library (Rust {tc.version})", "=")]
    out.append(
        f"The extension module and the OCR helper link the standard library of\n"
        f"the toolchain they are built with, Rust {tc.version}, statically. Its crates,\n"
        f"as that toolchain's rust-src resolves them for the shipped targets\n"
        f"({', '.join(SHIPPED_TARGETS)}):\n\n"
    )
    width = max(len(f"{c.name} {c.version}") for c in crates)
    for c in crates:
        where = "crates.io" if c.vendored else "rust-lang/rust"
        line = f"    {f'{c.name} {c.version}':<{width}}  {where:<14}  {c.license}"
        if c.targets != every:
            line += f"  [{', '.join(sorted(c.targets))} only]"
        out.append(line.rstrip() + "\n")
    out.append(
        "\nThe crates marked rust-lang/rust are covered by the Rust project's\n"
        "copyright notices for the standard library, which ship among the\n"
        "package's licence files as tools/license/rust-std-COPYRIGHT.html,\n"
        "verbatim from the toolchain's\n"
        f"{COPYRIGHT_IN_SYSROOT.as_posix()}. The crates.io crates' licences follow,\n"
        "each the first accepted licence of its expression (as in Part 1).\n\n"
        "libc and the C++ runtime (libstdc++ on Linux, libc++ on macOS) are linked\n"
        "dynamically from the host system and are not shipped. On Linux the\n"
        "manylinux toolchain links small parts of its newer libstdc++ statically;\n"
        "the GCC Runtime Library Exception covers them and requires no notice.\n\n"
    )
    for c in crates:
        if not c.vendored:
            continue
        lic = chosen_license(c, accepted)
        path = license_file(tc.library / c.source, lic)
        out.append(_heading(f"{c.name} {c.version} ({lic})", "-"))
        out.append(f"{c.name}-{c.version}/{path.name} verbatim:\n\n")
        out.append(_verbatim(path))
        out.append("\n")
    return "".join(out).rstrip("\n") + "\n"


def main(argv: Sequence[str] | None = None) -> int:
    args = list(sys.argv[1:] if argv is None else argv)
    try:
        tc = toolchain(REPO)
        if args == ["notices"]:
            crates = merge_targets([std_crates(tc, t) for t in SHIPPED_TARGETS])
            sys.stdout.write(notices(REPO, tc, crates))
        elif len(args) == 2 and args[0] == "copyright":
            shutil.copyfile(tc.sysroot / COPYRIGHT_IN_SYSROOT, args[1])
        else:
            print("usage: rust_std.py notices | rust_std.py copyright DEST", file=sys.stderr)
            return 2
    except (StdError, OSError) as e:
        print(f"rust_std: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":  # pragma: no cover - the tests call main()
    sys.exit(main())
