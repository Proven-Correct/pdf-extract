#!/usr/bin/env python3
"""Writes the CycloneDX SBOMs that every wheel carries in
``.dist-info/sboms/`` (PEP 770; design D5 of the
ship-wheels-sbom-process-default change, packaging spec "Every wheel carries a
CycloneDX SBOM").

Two documents, generated at build time by ``tools/build/build-wheel.sh`` and
handed to maturin with ``--sbom-include``. They cannot be committed, because
they depend on the wheel's target and carry the staged helper's hash:

``rust.cdx.json``
    Every crate linked into the extension (``pdf-extract-python`` with the
    wheel's features) or the OCR helper (``pdf-extract-ocr``), as
    ``cargo tree -e normal,no-proc-macro --target <triple>`` resolves them:
    normal dependencies only, evaluated for the real target, so dev-only and
    build-only crates, proc-macros and other platforms' or ``cfg(loom)``
    crates are absent. maturin's own cargo-cyclonedx document is turned off
    (``[tool.maturin.sbom] rust = false``) because it resolves all targets
    and embeds the build machine's paths. The patched hayro crates under
    ``vendor/`` carry a ``pedigree``: the crates.io release they came from
    (its package SHA-256 from ``vendor/PROVENANCE.md``) and the patches of
    ``vendor/patches/hayro/`` that touch them. The Rust standard library the
    binaries link statically follows as its own component group
    (``rust_std.py``; design D3 of the complete-license-pipeline change):
    each crate carries the property ``pdf-extract:origin`` =
    ``rust-toolchain-<version>``; crates.io ones keep their ``pkg:cargo``
    purl with a ``bom-ref`` suffixed ``?origin=rust-std`` (std's hashbrown
    can share a version with ours), in-tree ones get a
    ``pkg:github/rust-lang/rust@<version>#library/<dir>`` purl.

``native.cdx.json``
    What cargo cannot see: the vendored Tesseract and Leptonica (version,
    licence, archive URL and SHA-256 from ``vendor/PROVENANCE.md``, and their
    patch series under ``vendor/patches/{tesseract,leptonica}/`` if any), and
    the helper binary as an ``application`` with its SHA-256, depending on
    both libraries and on the Rust components.

No absolute path appears in either document: path crates are recorded
relative to the repository. Standard library only, because it runs inside the
manylinux build image before any tool is installed.

Usage:
    sbom.py --target TRIPLE --helper PATH --out-dir DIR [--features F,...]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tomllib
import uuid
from collections.abc import Sequence
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import rust_std  # noqa: E402

SPEC_VERSION = "1.5"
SCHEMA = "http://cyclonedx.org/schema/bom-1.5.schema.json"

#: The vendored C/C++ sources and their licences (vendor/PROVENANCE.md, the
#: "Licences:" line; the licensing spec's "Vendored C/C++ license check" keeps
#: the trees to these).
C_LICENSES = {"tesseract": "Apache-2.0", "leptonica": "BSD-2-Clause"}

#: The package the documents describe (the Python distribution).
PACKAGE = "pdf-extract"
#: The crates whose graphs are shipped: the extension and the helper's crate.
EXTENSION_CRATE = "pdf-extract-python"
HELPER_CRATE = "pdf-extract-ocr"
HELPER_NAME = "pdf-extract-ocr-helper"

#: A CycloneDX JSON document (or part of one), as json.load returns it.
Json = dict[str, Any]


class SbomError(Exception):
    """A provenance row, a cargo report or an input the generator cannot use."""


@dataclass(frozen=True)
class CrateRow:
    """One row of PROVENANCE.md's vendored-crate table."""

    name: str
    version: str
    sha256: str


@dataclass(frozen=True)
class SourceRow:
    """One row of PROVENANCE.md's vendored C/C++ table."""

    name: str
    version: str
    url: str
    sha256: str
    commit: str


@dataclass(frozen=True)
class Provenance:
    crates: dict[str, CrateRow]
    sources: dict[str, SourceRow]


@dataclass(frozen=True)
class Crate:
    """A crate in the shipped graph. ``path`` is repository-relative for path
    crates (ours and the vendored hayro crates) and ``None`` for registry
    crates."""

    name: str
    version: str
    license: str | None
    repository: str | None
    path: str | None = None
    used_by: frozenset[str] = field(default_factory=frozenset)


_SHA256 = re.compile(r"[0-9a-f]{64}")
_COMMIT = re.compile(r"[0-9a-f]{40}")


def _cells(line: str) -> list[str]:
    return [c.strip() for c in line.strip().strip("|").split("|")]


def parse_provenance(text: str) -> Provenance:
    """Reads both tables of PROVENANCE.md. Rows are recognised by their
    first cell (``hayro-*``, ``tesseract``, ``leptonica``) as the import
    scripts do; a recognised row that does not parse is an error, never
    skipped."""
    crates: dict[str, CrateRow] = {}
    sources: dict[str, SourceRow] = {}
    for line in text.splitlines():
        if not line.startswith("| "):
            continue
        cells = _cells(line)
        first = cells[0]
        if first.startswith("hayro-"):
            if len(cells) != 4 or not _SHA256.fullmatch(cells[2]):
                raise SbomError(f"malformed vendored-crate row: {line!r}")
            crates[first] = CrateRow(first, cells[1], cells[2])
        elif first in C_LICENSES:
            if (
                len(cells) != 6
                or not cells[2].startswith("https://")
                or not _SHA256.fullmatch(cells[3])
                or not _COMMIT.fullmatch(cells[4])
            ):
                raise SbomError(f"malformed C/C++ source row: {line!r}")
            sources[first] = SourceRow(first, cells[1], cells[2], cells[3], cells[4])
    missing = sorted(set(C_LICENSES) - set(sources))
    if missing:
        raise SbomError(f"PROVENANCE.md has no row for {', '.join(missing)}")
    if not crates:
        raise SbomError("PROVENANCE.md has no vendored-crate rows")
    return Provenance(crates, sources)


# `cargo tree -f '{p}|{l}|{r}'`: "name vX.Y.Z[ (path)]|licence|repo[ (*)]";
# cargo appends the "(*)" (already shown) marker to the end of the line.
_TREE_LINE = re.compile(r"(?P<name>\S+) v(?P<version>\S+)(?: \((?P<path>/[^)]*)\))?")
_SEEN = " (*)"


def parse_tree_line(line: str, repo: Path) -> Crate:
    """One line of ``cargo tree --prefix none -f '{p}|{l}|{r}'``."""
    line = line.removesuffix(_SEEN)
    try:
        pkg, lic, url = line.split("|")
    except ValueError:
        raise SbomError(f"unexpected cargo tree line: {line!r}") from None
    m = _TREE_LINE.fullmatch(pkg.strip())
    if m is None:
        raise SbomError(f"unexpected cargo tree package: {pkg!r}")
    path = m.group("path")
    rel: str | None = None
    if path is not None:
        try:
            rel = Path(path).resolve().relative_to(repo.resolve()).as_posix()
        except ValueError:
            raise SbomError(f"path crate outside the repository: {path}") from None
    return Crate(m.group("name"), m.group("version"), lic.strip() or None, url.strip() or None, rel)


def cargo_tree(repo: Path, package: str, target: str, features: Sequence[str]) -> list[Crate]:
    """The normal, non-proc-macro dependency graph of ``package`` for
    ``target``, its own crate included."""
    cmd = [
        "cargo",
        "tree",
        "--locked",
        # CI sets CARGO_TERM_COLOR=always; colour codes would end up in the
        # document (they did: escape sequences inside repository URLs).
        "--color",
        "never",
        "-p",
        package,
        "-e",
        "normal,no-proc-macro",
        "--target",
        target,
        "--prefix",
        "none",
        "-f",
        "{p}|{l}|{r}",
    ]
    if features:
        cmd += ["--features", ",".join(features)]
    run = subprocess.run(cmd, cwd=repo, check=False, capture_output=True, text=True)
    if run.returncode != 0:
        raise SbomError(f"cargo tree for {package} failed:\n{run.stderr.strip()}")
    out = run.stdout
    return [parse_tree_line(line, repo) for line in out.splitlines() if line.strip()]


def merge_graphs(graphs: dict[str, list[Crate]]) -> list[Crate]:
    """The union of the graphs, one entry per (name, version), recording which
    shipped binary uses each crate."""
    merged: dict[tuple[str, str], Crate] = {}
    for user, crates in graphs.items():
        for c in crates:
            key = (c.name, c.version)
            prior = merged.get(key)
            users = (prior.used_by if prior else frozenset()) | {user}
            merged[key] = Crate(c.name, c.version, c.license, c.repository, c.path, users)
    return sorted(merged.values(), key=lambda c: (c.name, c.version))


def clarified_licenses(about_toml: Path) -> dict[str, str]:
    """``about.toml``'s licence clarifications (crate name -> SPDX), the same
    corrections THIRD_PARTY_NOTICES uses, so the two never disagree."""
    with about_toml.open("rb") as f:
        about = tomllib.load(f)
    out: dict[str, str] = {}
    for name, entry in about.items():
        if isinstance(entry, dict) and isinstance(entry.get("clarify"), dict):
            lic = entry["clarify"].get("license")
            if isinstance(lic, str):
                out[name] = lic
    return out


def license_expression(crate: Crate, clarified: dict[str, str]) -> str:
    """The crate's SPDX expression: a clarification wins; cargo's legacy
    ``A/B`` form (no spaces, ``/`` only) means ``A OR B``. Anything else is
    taken as SPDX already."""
    if crate.name in clarified:
        return clarified[crate.name]
    if crate.license is None:
        raise SbomError(
            f"{crate.name} {crate.version} declares no licence and has no clarification"
        )
    lic = crate.license.strip()
    if "/" in lic and " " not in lic:
        return " OR ".join(lic.split("/"))
    return lic


def patches_touching(repo: Path, patch_dir: Path, prefix: str | None) -> list[str]:
    """Repository-relative paths of the patches in ``patch_dir`` (sorted, as
    applied), restricted to those that touch ``prefix/`` when given. A
    missing directory means no patches."""
    if not patch_dir.is_dir():
        return []
    found = []
    for patch in sorted(patch_dir.glob("*.patch")):
        if prefix is not None:
            text = patch.read_text(encoding="utf-8", errors="replace")
            if f"diff --git a/{prefix}/" not in text:
                continue
        found.append(patch.relative_to(repo).as_posix())
    return found


def _patch_list(paths: list[str]) -> list[Json]:
    return [{"type": "unofficial", "diff": {"url": p}} for p in paths]


def _timestamp() -> str:
    # SOURCE_DATE_EPOCH (reproducible builds) when set.
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    when = datetime.fromtimestamp(int(epoch), UTC) if epoch else datetime.now(UTC)
    return when.strftime("%Y-%m-%dT%H:%M:%SZ")


def _document(version: str, components: list[Json], deps: list[Json]) -> Json:
    return {
        "$schema": SCHEMA,
        "bomFormat": "CycloneDX",
        "specVersion": SPEC_VERSION,
        "serialNumber": f"urn:uuid:{uuid.uuid4()}",
        "version": 1,
        "metadata": {
            "timestamp": _timestamp(),
            "tools": {
                "components": [{"type": "application", "name": "pdf-extract tools/license/sbom.py"}]
            },
            "component": {
                "type": "library",
                "bom-ref": f"{PACKAGE}@{version}",
                "name": PACKAGE,
                "version": version,
                "purl": f"pkg:pypi/{PACKAGE}@{version}",
                "licenses": [{"expression": "GPL-3.0-or-later"}],
            },
        },
        "components": components,
        "dependencies": deps,
    }


def crate_ref(c: Crate) -> str:
    return f"pkg:cargo/{c.name}@{c.version}"


#: Property naming where a component comes from; set on std crates only.
ORIGIN = "pdf-extract:origin"


def std_origin(toolchain_version: str) -> str:
    return f"rust-toolchain-{toolchain_version}"


def std_purl(c: rust_std.StdCrate, toolchain_version: str) -> str:
    if c.vendored:
        return f"pkg:cargo/{c.name}@{c.version}"
    return f"pkg:github/rust-lang/rust@{toolchain_version}#library/{c.source}"


def std_ref(c: rust_std.StdCrate, toolchain_version: str) -> str:
    purl = std_purl(c, toolchain_version)
    return f"{purl}?origin=rust-std" if c.vendored else purl


def std_components(std: list[rust_std.StdCrate], toolchain_version: str) -> list[Json]:
    return [
        {
            "type": "library",
            "bom-ref": std_ref(c, toolchain_version),
            "name": c.name,
            "version": c.version,
            "scope": "required",
            "licenses": [{"expression": c.license}],
            "purl": std_purl(c, toolchain_version),
            "properties": [
                {"name": ORIGIN, "value": std_origin(toolchain_version)},
                {"name": "pdf-extract:rust-source", "value": f"library/{c.source}"},
            ],
        }
        for c in std
    ]


def rust_document(  # noqa: PLR0913 - the document's independent inputs
    version: str,
    crates: list[Crate],
    provenance: Provenance,
    clarified: dict[str, str],
    repo: Path,
    *,
    std: list[rust_std.StdCrate] | None = None,
    toolchain_version: str = "",
) -> Json:
    components: list[Json] = []
    for c in crates:
        comp: Json = {
            "type": "library",
            "bom-ref": crate_ref(c),
            "name": c.name,
            "version": c.version,
            "scope": "required",
            "licenses": [{"expression": license_expression(c, clarified)}],
            "purl": crate_ref(c),
            "properties": [{"name": "pdf-extract:used-by", "value": ",".join(sorted(c.used_by))}],
        }
        if c.repository:
            comp["externalReferences"] = [{"type": "vcs", "url": c.repository}]
        if c.path is not None:
            comp["properties"].append({"name": "pdf-extract:source-path", "value": c.path})
            row = provenance.crates.get(c.name)
            if c.path.startswith("vendor/"):
                if row is None or row.version != c.version:
                    raise SbomError(
                        f"vendored crate {c.name} {c.version} has no matching PROVENANCE.md row"
                    )
                comp["pedigree"] = {
                    "ancestors": [
                        {
                            "type": "library",
                            "name": c.name,
                            "version": row.version,
                            "purl": f"pkg:cargo/{c.name}@{row.version}",
                            "hashes": [{"alg": "SHA-256", "content": row.sha256}],
                        }
                    ],
                    "patches": _patch_list(
                        patches_touching(repo, repo / "vendor" / "patches" / "hayro", c.name)
                    ),
                    "notes": "crates.io release patched in-tree; see vendor/PROVENANCE.md",
                }
        components.append(comp)
    std_comps = std_components(std or [], toolchain_version)
    components += std_comps
    root = f"{PACKAGE}@{version}"
    refs = [crate_ref(c) for c in crates] + [str(c["bom-ref"]) for c in std_comps]
    deps = [{"ref": root, "dependsOn": refs}]
    return _document(version, components, deps)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def native_document(
    version: str, provenance: Provenance, helper: Path, rust_refs: list[str], repo: Path
) -> Json:
    components: list[Json] = []
    lib_refs = []
    for name in sorted(C_LICENSES):
        row = provenance.sources[name]
        ref = f"vendored:{name}@{row.version}"
        lib_refs.append(ref)
        comp: Json = {
            "type": "library",
            "bom-ref": ref,
            "name": name,
            "version": row.version,
            "scope": "required",
            "licenses": [{"expression": C_LICENSES[name]}],
            "hashes": [{"alg": "SHA-256", "content": row.sha256}],
            "externalReferences": [{"type": "distribution", "url": row.url}],
            "properties": [
                {"name": "pdf-extract:source-path", "value": f"vendor/{name}"},
                {"name": "pdf-extract:upstream-commit", "value": row.commit},
                {"name": "pdf-extract:linkage", "value": "static"},
            ],
        }
        patches = patches_touching(repo, repo / "vendor" / "patches" / name, None)
        if patches:
            comp["pedigree"] = {"patches": _patch_list(patches)}
        components.append(comp)
    helper_ref = f"{HELPER_NAME}@{version}"
    components.append(
        {
            "type": "application",
            "bom-ref": helper_ref,
            "name": HELPER_NAME,
            "version": version,
            "scope": "required",
            "licenses": [{"expression": "GPL-3.0-or-later"}],
            "hashes": [{"alg": "SHA-256", "content": sha256_file(helper)}],
            "properties": [
                {"name": "pdf-extract:install-path", "value": f"pdf_extract/{HELPER_NAME}"}
            ],
        }
    )
    root = f"{PACKAGE}@{version}"
    deps: list[Json] = [
        {"ref": root, "dependsOn": [helper_ref, *lib_refs]},
        {"ref": helper_ref, "dependsOn": [*lib_refs, *rust_refs]},
    ]
    return _document(version, components, deps)


# Checks shared with tools/build/verify_wheel.py: each returns a list of
# problems (empty when the document agrees).


def _components(doc: Json) -> list[Json]:
    comps = doc.get("components")
    if not isinstance(comps, list):
        return []
    return [c for c in comps if isinstance(c, dict)]


def _origin(comp: Json) -> str | None:
    for prop in comp.get("properties", []):
        if isinstance(prop, dict) and prop.get("name") == ORIGIN:
            return str(prop.get("value"))
    return None


def std_problems(doc: Json, expected: list[rust_std.StdCrate], toolchain_version: str) -> list[str]:
    """The document's standard-library group lists exactly ``expected``,
    each with the toolchain's origin, its purl, bom-ref and licence."""
    origin = std_origin(toolchain_version)
    comps = [c for c in _components(doc) if _origin(c) is not None]
    keys = [(str(c.get("name")), str(c.get("version"))) for c in comps]
    by_key = dict(zip(keys, comps, strict=True))
    want = {(c.name, c.version): c for c in expected}
    twice = sorted({k for k in keys if keys.count(k) > 1})
    problems = [f"rust SBOM lists std crate {n} {v} twice" for n, v in twice]
    problems += [
        f"rust SBOM lacks std crate {n} {v}" for n, v in sorted(want.keys() - by_key.keys())
    ]
    problems += [
        f"rust SBOM lists std crate {n} {v}, which is not linked"
        for n, v in sorted(by_key.keys() - want.keys())
    ]
    for key in sorted(want.keys() & by_key.keys()):
        comp, crate = by_key[key], want[key]
        label = f"std {crate.name} {crate.version}"
        if _origin(comp) != origin:
            problems.append(f"{label}: origin {_origin(comp)!r}, expected {origin!r}")
        if comp.get("purl") != std_purl(crate, toolchain_version):
            problems.append(f"{label}: purl {comp.get('purl')!r}")
        if comp.get("bom-ref") != std_ref(crate, toolchain_version):
            problems.append(f"{label}: bom-ref {comp.get('bom-ref')!r}")
        lic = [x.get("expression") for x in comp.get("licenses", []) if isinstance(x, dict)]
        if lic != [crate.license]:
            problems.append(f"{label}: licence {lic}, expected {crate.license!r}")
    return problems


def rust_problems(
    doc: Json, expected: list[Crate], clarified: dict[str, str] | None = None
) -> list[str]:
    """The Rust document's lockfile crates (those without a standard-library
    origin; ``std_problems`` checks the others) are exactly ``expected``
    (name and version), each with its ``pkg:cargo`` purl and, when
    ``clarified`` is given, the licence expression the generator derives for
    it."""
    by_key = {
        (str(c.get("name")), str(c.get("version"))): c
        for c in _components(doc)
        if _origin(c) is None
    }
    want = {(c.name, c.version): c for c in expected}
    problems = [f"rust SBOM lacks {n} {v}" for n, v in sorted(want.keys() - by_key.keys())]
    problems += [
        f"rust SBOM lists {n} {v}, which is not shipped"
        for n, v in sorted(by_key.keys() - want.keys())
    ]
    for key in sorted(want.keys() & by_key.keys()):
        comp, crate = by_key[key], want[key]
        if comp.get("purl") != crate_ref(crate):
            problems.append(f"{crate.name} {crate.version}: purl {comp.get('purl')!r}")
        if clarified is not None:
            lic = [x.get("expression") for x in comp.get("licenses", []) if isinstance(x, dict)]
            want_lic = license_expression(crate, clarified)
            if lic != [want_lic]:
                problems.append(
                    f"{crate.name} {crate.version}: licence {lic}, expected {want_lic!r}"
                )
    return problems


def native_problems(
    doc: Json, provenance: Provenance, helper_sha256: str | None = None
) -> list[str]:
    """The native document agrees with PROVENANCE.md and, when given, the
    helper's hash."""
    by_name = {str(c.get("name")): c for c in _components(doc)}
    problems = []
    for name, row in sorted(provenance.sources.items()):
        comp = by_name.get(name)
        if comp is None:
            problems.append(f"native SBOM lacks {name}")
            continue
        if comp.get("version") != row.version:
            problems.append(f"{name}: version {comp.get('version')} != PROVENANCE.md {row.version}")
        hashes = {
            h.get("alg"): h.get("content") for h in comp.get("hashes", []) if isinstance(h, dict)
        }
        if hashes.get("SHA-256") != row.sha256:
            problems.append(
                f"{name}: SHA-256 {hashes.get('SHA-256')} != PROVENANCE.md {row.sha256}"
            )
        urls = [r.get("url") for r in comp.get("externalReferences", []) if isinstance(r, dict)]
        if row.url not in urls:
            problems.append(f"{name}: download URL is not {row.url}")
    helper = by_name.get(HELPER_NAME)
    if helper is None:
        problems.append(f"native SBOM lacks {HELPER_NAME}")
    elif helper_sha256 is not None:
        hashes = {
            h.get("alg"): h.get("content") for h in helper.get("hashes", []) if isinstance(h, dict)
        }
        if hashes.get("SHA-256") != helper_sha256:
            problems.append(f"{HELPER_NAME}: SHA-256 does not match the shipped helper")
    return problems


def absolute_path_problems(doc: object, where: str = "$") -> list[str]:
    """String values that look like build-machine paths: absolute paths or
    ``file:`` URLs."""
    if isinstance(doc, dict):
        return [p for k, v in doc.items() for p in absolute_path_problems(v, f"{where}.{k}")]
    if isinstance(doc, list):
        return [p for i, v in enumerate(doc) for p in absolute_path_problems(v, f"{where}[{i}]")]
    if isinstance(doc, str) and doc.startswith(("/", "file:")):
        return [f"{where}: build-machine path {doc!r}"]
    return []


def workspace_version(repo: Path) -> str:
    with (repo / "Cargo.toml").open("rb") as f:
        v = tomllib.load(f)["workspace"]["package"]["version"]
    if not isinstance(v, str):
        raise SbomError("workspace version is not a string")
    return v


def generate(
    repo: Path, target: str, features: Sequence[str], helper: Path, out_dir: Path
) -> tuple[Path, Path]:
    if not helper.is_file():
        raise SbomError(f"helper not found: {helper}")
    provenance = parse_provenance((repo / "vendor" / "PROVENANCE.md").read_text(encoding="utf-8"))
    version = workspace_version(repo)
    crates = merge_graphs(
        {
            "extension": cargo_tree(repo, EXTENSION_CRATE, target, features),
            "helper": cargo_tree(repo, HELPER_CRATE, target, ()),
        }
    )
    try:
        tc = rust_std.toolchain(repo)
        std = rust_std.std_crates(tc, target)
    except rust_std.StdError as e:
        raise SbomError(str(e)) from None
    clarified = clarified_licenses(repo / "about.toml")
    rust = rust_document(
        version, crates, provenance, clarified, repo, std=std, toolchain_version=tc.version
    )
    rust_refs = [crate_ref(c) for c in crates] + [std_ref(c, tc.version) for c in std]
    native = native_document(version, provenance, helper, rust_refs, repo)
    out_dir.mkdir(parents=True, exist_ok=True)
    paths = (out_dir / "rust.cdx.json", out_dir / "native.cdx.json")
    for path, doc in zip(paths, (rust, native), strict=True):
        path.write_text(json.dumps(doc, indent=2, sort_keys=False) + "\n", encoding="utf-8")
    return paths


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--target", required=True, help="Rust target triple of the wheel")
    parser.add_argument("--features", default="", help="extension features, comma-separated")
    parser.add_argument("--helper", required=True, type=Path, help="the staged helper binary")
    parser.add_argument("--out-dir", required=True, type=Path)
    args = parser.parse_args(argv)
    features = [f for f in args.features.split(",") if f]
    try:
        for path in generate(REPO, args.target, features, args.helper, args.out_dir):
            print(path)
    except SbomError as e:
        print(f"sbom: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":  # pragma: no cover - the tests call main()
    sys.exit(main())
