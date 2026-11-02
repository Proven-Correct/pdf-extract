"""The wheel SBOM generator, ``tools/license/sbom.py`` (packaging spec "Every
wheel carries a CycloneDX SBOM"; design D5).

The generator is tested directly: the provenance and ``cargo tree`` parsers,
the licence and patch rules, the agreement checks that ``verify_wheel.py``
reuses, and a real generation for this host's target, validated against the
CycloneDX 1.5 schema and compared with an independent ``cargo tree`` run.
"""

from __future__ import annotations

import hashlib
import json
import subprocess
import sys
from pathlib import Path

import pytest
from cyclonedx.schema import SchemaVersion
from cyclonedx.validation.json import JsonStrictValidator

from pdfs import REPO

sys.path.insert(0, str(REPO / "tools" / "license"))
import rust_std
import sbom

PROVENANCE = (REPO / "vendor" / "PROVENANCE.md").read_text(encoding="utf-8")


def host_triple() -> str:
    out = subprocess.run(["rustc", "-vV"], check=True, capture_output=True, text=True).stdout
    return next(line.split(": ", 1)[1] for line in out.splitlines() if line.startswith("host: "))


@pytest.fixture(scope="module")
def generated(
    tmp_path_factory: pytest.TempPathFactory,
) -> tuple[sbom.Json, sbom.Json, Path]:
    """One real generation for the host target, with a stand-in helper."""
    out = tmp_path_factory.mktemp("sbom")
    helper = out / "helper"
    helper.write_bytes(b"not really a helper")
    rust_path, native_path = sbom.generate(REPO, host_triple(), ["abi3"], helper, out)
    return (
        json.loads(rust_path.read_text(encoding="utf-8")),
        json.loads(native_path.read_text(encoding="utf-8")),
        helper,
    )


# --- PROVENANCE.md --------------------------------------------------------


def test_provenance_rows_are_read() -> None:
    prov = sbom.parse_provenance(PROVENANCE)
    assert set(prov.crates) == {
        "hayro-syntax",
        "hayro-interpret",
        "hayro-postscript",
        "hayro-jbig2",
        "hayro-jpeg2000",
    }
    assert set(prov.sources) == {"tesseract", "leptonica"}
    tess = prov.sources["tesseract"]
    assert tess.version == "5.5.3"
    assert tess.url.endswith("5.5.3.tar.gz")
    assert len(tess.sha256) == 64
    assert len(tess.commit) == 40


@pytest.mark.parametrize(
    ("needle", "replacement"),
    [
        # A crate row whose checksum is not 64 hex digits.
        ("| hayro-syntax | 0.7.2 | 0edeafd7", "| hayro-syntax | 0.7.2 | zzedeafd7"),
        # A source row with a column missing.
        (
            "| db0ec62f81b0737fbbe184d8fea40af5738f8eef | 2026-10-03 |",
            "| 2026-10-03 |",
        ),
        # A source row whose URL is not https.
        ("| https://github.com/tesseract-ocr", "| http://github.com/tesseract-ocr"),
    ],
)
def test_a_malformed_provenance_row_is_refused(needle: str, replacement: str) -> None:
    assert needle in PROVENANCE
    with pytest.raises(sbom.SbomError, match="malformed"):
        sbom.parse_provenance(PROVENANCE.replace(needle, replacement, 1))


def test_a_missing_source_row_is_refused() -> None:
    text = "\n".join(line for line in PROVENANCE.splitlines() if not line.startswith("| leptonica"))
    with pytest.raises(sbom.SbomError, match="no row for leptonica"):
        sbom.parse_provenance(text)


def test_no_crate_rows_is_refused() -> None:
    text = "\n".join(line for line in PROVENANCE.splitlines() if not line.startswith("| hayro-"))
    with pytest.raises(sbom.SbomError, match="no vendored-crate rows"):
        sbom.parse_provenance(text)


# --- cargo tree lines -----------------------------------------------------


def test_registry_and_path_tree_lines() -> None:
    reg = sbom.parse_tree_line("adler2 v2.0.1|0BSD OR MIT|https://github.com/oyvindln/adler2", REPO)
    assert (reg.name, reg.version, reg.license, reg.path) == (
        "adler2",
        "2.0.1",
        "0BSD OR MIT",
        None,
    )
    path = sbom.parse_tree_line(
        f"hayro-syntax v0.7.2 ({REPO}/vendor/hayro-syntax)|Apache-2.0 OR MIT|https://x (*)", REPO
    )
    assert path.path == "vendor/hayro-syntax"
    assert path.repository == "https://x"


def test_a_crate_without_licence_or_repository() -> None:
    c = sbom.parse_tree_line("x v1.0.0||", REPO)
    assert c.license is None
    assert c.repository is None


@pytest.mark.parametrize("line", ["no separators", "x|y", "|MIT|r", "x 1.0|MIT|r"])
def test_an_unexpected_tree_line_is_refused(line: str) -> None:
    with pytest.raises(sbom.SbomError, match="unexpected cargo tree"):
        sbom.parse_tree_line(line, REPO)


def test_a_path_crate_outside_the_repository_is_refused(tmp_path: Path) -> None:
    with pytest.raises(sbom.SbomError, match="outside the repository"):
        sbom.parse_tree_line(f"x v1.0.0 ({tmp_path})|MIT|", REPO)


def test_cargo_tree_output_carries_no_colour(monkeypatch: pytest.MonkeyPatch) -> None:
    # CI sets CARGO_TERM_COLOR=always; the codes once leaked into the URLs.
    monkeypatch.setenv("CARGO_TERM_COLOR", "always")
    crates = sbom.cargo_tree(REPO, sbom.HELPER_CRATE, host_triple(), ())
    assert crates
    assert not any("\x1b" in f"{c.name}{c.version}{c.license}{c.repository}" for c in crates)


def test_merge_records_which_binary_uses_a_crate() -> None:
    a = sbom.Crate("a", "1.0.0", "MIT", None)
    b = sbom.Crate("b", "1.0.0", "MIT", None)
    merged = sbom.merge_graphs({"extension": [a, b], "helper": [a]})
    assert [(c.name, sorted(c.used_by)) for c in merged] == [
        ("a", ["extension", "helper"]),
        ("b", ["extension"]),
    ]


# --- licences and patches -------------------------------------------------


def test_licence_clarifications_come_from_about_toml() -> None:
    clarified = sbom.clarified_licenses(REPO / "about.toml")
    assert clarified["brotli-decompressor"] == "BSD-3-Clause"
    crate = sbom.Crate("brotli-decompressor", "5.0.3", "BSD-3-Clause/MIT", None)
    assert sbom.license_expression(crate, clarified) == "BSD-3-Clause"


def test_clarifications_without_a_licence_string_are_ignored(tmp_path: Path) -> None:
    about = tmp_path / "about.toml"
    about.write_text(
        'accepted = ["Zlib"]\n[a.clarify]\nlicense = 3\n[b.clarify]\nlicense = "Zlib"\n'
    )
    assert sbom.clarified_licenses(about) == {"b": "Zlib"}


def test_a_non_string_workspace_version_is_refused(tmp_path: Path) -> None:
    (tmp_path / "Cargo.toml").write_text("[workspace.package]\nversion = 1\n")
    with pytest.raises(sbom.SbomError, match="not a string"):
        sbom.workspace_version(tmp_path)


def test_legacy_slash_licence_means_or() -> None:
    crate = sbom.Crate("x", "1.0.0", "MIT/Apache-2.0", None)
    assert sbom.license_expression(crate, {}) == "MIT OR Apache-2.0"


def test_a_spdx_expression_is_kept_as_is() -> None:
    crate = sbom.Crate("x", "1.0.0", "(MIT OR Apache-2.0) AND Unicode-3.0", None)
    assert sbom.license_expression(crate, {}) == "(MIT OR Apache-2.0) AND Unicode-3.0"


def test_a_crate_without_licence_is_refused() -> None:
    with pytest.raises(sbom.SbomError, match="declares no licence"):
        sbom.license_expression(sbom.Crate("x", "1.0.0", None, None), {})


def test_patches_touching_filters_by_crate(tmp_path: Path) -> None:
    patches = tmp_path / "vendor" / "patches" / "hayro"
    patches.mkdir(parents=True)
    (patches / "0002-b.patch").write_text("diff --git a/hayro-b/src/x.rs b/hayro-b/src/x.rs\n")
    (patches / "0001-a.patch").write_text("diff --git a/hayro-a/src/x.rs b/hayro-a/src/x.rs\n")
    assert sbom.patches_touching(tmp_path, patches, "hayro-a") == [
        "vendor/patches/hayro/0001-a.patch"
    ]
    assert sbom.patches_touching(tmp_path, patches, None) == [
        "vendor/patches/hayro/0001-a.patch",
        "vendor/patches/hayro/0002-b.patch",
    ]
    assert sbom.patches_touching(tmp_path, tmp_path / "absent", None) == []


def test_a_vendored_crate_needs_its_provenance_row() -> None:
    prov = sbom.parse_provenance(PROVENANCE)
    stray = sbom.Crate("hayro-new", "1.0.0", "MIT", None, "vendor/hayro-new")
    with pytest.raises(sbom.SbomError, match="no matching PROVENANCE\\.md row"):
        sbom.rust_document("0.1.0", [stray], prov, {}, REPO)


def test_c_patch_series_become_pedigree(tmp_path: Path) -> None:
    patches = tmp_path / "vendor" / "patches" / "tesseract"
    patches.mkdir(parents=True)
    (patches / "0001-fix.patch").write_text("diff --git a/x b/x\n")
    helper = tmp_path / "helper"
    helper.write_bytes(b"h")
    doc = sbom.native_document("0.1.0", sbom.parse_provenance(PROVENANCE), helper, [], tmp_path)
    by_name = {c["name"]: c for c in doc["components"]}
    assert by_name["tesseract"]["pedigree"] == {
        "patches": [
            {"type": "unofficial", "diff": {"url": "vendor/patches/tesseract/0001-fix.patch"}}
        ]
    }
    assert "pedigree" not in by_name["leptonica"]


# --- a real generation ----------------------------------------------------


@pytest.mark.parametrize("which", [0, 1])
def test_documents_validate_against_cyclonedx_1_5(
    generated: tuple[sbom.Json, sbom.Json, Path], which: int
) -> None:
    error = JsonStrictValidator(SchemaVersion.V1_5).validate_str(json.dumps(generated[which]))
    assert error is None, error


def test_rust_document_is_the_shipped_graph(
    generated: tuple[sbom.Json, sbom.Json, Path],
) -> None:
    rust = generated[0]
    names = {c["name"] for c in rust["components"]}
    # Shipped: the binding, the vendored parser, the hashing used for models.
    assert {"pyo3", "hayro-syntax", "sha2", "pdf-extract-core"} <= names
    # Not shipped: model checking, build scripts, proc-macros.
    assert not names & {"loom", "cmake", "cc", "pyo3-build-config", "syn", "bytemuck_derive"}
    # An independent cargo tree run (the module's parser is not reused).
    expected = set()
    for package, feats in (("pdf-extract-python", ["--features", "abi3"]), ("pdf-extract-ocr", [])):
        out = subprocess.run(
            [
                "cargo",
                "tree",
                "--locked",
                "--color",
                "never",
                "-p",
                package,
                "-e",
                "normal,no-proc-macro",
                "--target",
                host_triple(),
                "--prefix",
                "none",
                "-f",
                "{p}",
                *feats,
            ],
            cwd=REPO,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        expected |= {tuple(line.split()[:2]) for line in out.splitlines() if line.strip()}
    # The lockfile group; the standard library is checked below.
    have = {(c["name"], "v" + c["version"]) for c in rust["components"] if not is_std(c)}
    assert have == expected


def is_std(comp: sbom.Json) -> bool:
    return any(p["name"] == sbom.ORIGIN for p in comp.get("properties", []))


# --- the standard library (licensing change, design D3) -------------------


def test_rust_document_carries_the_standard_library(
    generated: tuple[sbom.Json, sbom.Json, Path],
) -> None:
    rust = generated[0]
    tc = rust_std.toolchain(REPO)
    want = rust_std.std_crates(tc, host_triple())
    assert sbom.std_problems(rust, want, tc.version) == []
    std = {c["name"]: c for c in rust["components"] if is_std(c)}
    assert {"std", "core", "alloc", "compiler_builtins", "gimli"} <= set(std)
    assert std["core"]["purl"] == f"pkg:github/rust-lang/rust@{tc.version}#library/core"
    assert std["libc"]["purl"].startswith("pkg:cargo/libc@")
    # Every std crate is a dependency of the package, like the lockfile ones.
    deps = rust["dependencies"][0]["dependsOn"]
    assert all(c["bom-ref"] in deps for c in std.values())


def test_a_std_crate_sharing_a_lockfile_version_stays_distinct(
    generated: tuple[sbom.Json, sbom.Json, Path],
) -> None:
    """std's hashbrown 0.17.1 is also in Cargo.lock: two components, two
    bom-refs, each checked against its own group."""
    rust = generated[0]
    refs = [c["bom-ref"] for c in rust["components"]]
    assert len(refs) == len(set(refs))
    std = rust_std.StdCrate("hashbrown", "0.17.1", "MIT OR Apache-2.0", "vendor/hashbrown-0.17.1")
    lock = sbom.Crate("hashbrown", "0.17.1", "MIT OR Apache-2.0", None)
    doc = sbom.rust_document(
        "0.1.0",
        [lock],
        sbom.parse_provenance(PROVENANCE),
        {},
        REPO,
        std=[std],
        toolchain_version="1.98.0",
    )
    assert [c["bom-ref"] for c in doc["components"]] == [
        "pkg:cargo/hashbrown@0.17.1",
        "pkg:cargo/hashbrown@0.17.1?origin=rust-std",
    ]
    assert sbom.rust_problems(doc, [lock]) == []
    assert sbom.std_problems(doc, [std], "1.98.0") == []


def test_std_problems_name_each_disagreement() -> None:
    std = rust_std.StdCrate("core", "0.0.0", "MIT OR Apache-2.0", "core")
    extra = rust_std.StdCrate("test", "0.0.0", "MIT OR Apache-2.0", "test")
    doc = {"components": sbom.std_components([std, extra], "1.98.0")}
    twice = {"components": sbom.std_components([std, std], "1.98.0")}
    assert sbom.std_problems(twice, [std], "1.98.0") == [
        "rust SBOM lists std crate core 0.0.0 twice"
    ]
    gone = rust_std.StdCrate("alloc", "0.0.0", "MIT OR Apache-2.0", "alloc")
    assert sbom.std_problems(doc, [std, gone], "1.98.0") == [
        "rust SBOM lacks std crate alloc 0.0.0",
        "rust SBOM lists std crate test 0.0.0, which is not linked",
    ]
    comp = doc["components"][0]
    comp["properties"][0]["value"] = "rust-toolchain-1.97.0"
    comp["purl"] = comp["bom-ref"] = "pkg:cargo/core@0.0.0"
    comp["licenses"] = [{"expression": "MIT"}]
    assert sbom.std_problems(doc, [std, extra], "1.98.0") == [
        "std core 0.0.0: origin 'rust-toolchain-1.97.0', expected 'rust-toolchain-1.98.0'",
        "std core 0.0.0: purl 'pkg:cargo/core@0.0.0'",
        "std core 0.0.0: bom-ref 'pkg:cargo/core@0.0.0'",
        "std core 0.0.0: licence ['MIT'], expected 'MIT OR Apache-2.0'",
    ]


def test_a_toolchain_failure_is_an_sbom_error(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    def fail(repo: Path) -> rust_std.Toolchain:
        raise rust_std.StdError("no rust-src")

    monkeypatch.setattr(rust_std, "toolchain", fail)
    helper = tmp_path / "helper"
    helper.write_bytes(b"h")
    with pytest.raises(sbom.SbomError, match="no rust-src"):
        sbom.generate(REPO, host_triple(), [], helper, tmp_path)


def test_vendored_hayro_crates_carry_their_pedigree(
    generated: tuple[sbom.Json, sbom.Json, Path],
) -> None:
    prov = sbom.parse_provenance(PROVENANCE)
    comps = {c["name"]: c for c in generated[0]["components"]}
    syntax = comps["hayro-syntax"]
    ancestor = syntax["pedigree"]["ancestors"][0]
    assert ancestor["hashes"] == [{"alg": "SHA-256", "content": prov.crates["hayro-syntax"].sha256}]
    patches = [p["diff"]["url"] for p in syntax["pedigree"]["patches"]]
    assert patches, "hayro-syntax is patched, so its pedigree lists patches"
    for rel in patches:
        assert "diff --git a/hayro-syntax/" in (REPO / rel).read_text(encoding="utf-8")
    assert "pedigree" not in comps["pyo3"]


def test_native_document_agrees_with_provenance_and_helper(
    generated: tuple[sbom.Json, sbom.Json, Path],
) -> None:
    _, native, helper = generated
    prov = sbom.parse_provenance(PROVENANCE)
    digest = hashlib.sha256(helper.read_bytes()).hexdigest()
    assert sbom.native_problems(native, prov, digest) == []
    deps = {d["ref"]: d["dependsOn"] for d in native["dependencies"]}
    helper_deps = deps[f"pdf-extract-ocr-helper@{sbom.workspace_version(REPO)}"]
    assert "vendored:tesseract@5.5.3" in helper_deps
    assert "pkg:cargo/hayro-syntax@0.7.2" in helper_deps


def test_a_planted_archive_hash_fails_the_agreement_check(
    generated: tuple[sbom.Json, sbom.Json, Path],
) -> None:
    native = json.loads(json.dumps(generated[1]))
    tess = next(c for c in native["components"] if c["name"] == "tesseract")
    tess["hashes"][0]["content"] = "0" * 64
    tess["version"] = "5.5.2"
    tess["externalReferences"] = []
    problems = sbom.native_problems(native, sbom.parse_provenance(PROVENANCE))
    assert any("tesseract: SHA-256" in p for p in problems)
    assert any("tesseract: version" in p for p in problems)
    assert any("download URL" in p for p in problems)


def test_a_missing_component_or_wrong_helper_hash_fails(
    generated: tuple[sbom.Json, sbom.Json, Path],
) -> None:
    native = json.loads(json.dumps(generated[1]))
    native["components"] = [c for c in native["components"] if c["name"] != "leptonica"]
    prov = sbom.parse_provenance(PROVENANCE)
    problems = sbom.native_problems(native, prov, "0" * 64)
    assert "native SBOM lacks leptonica" in problems
    assert any("does not match the shipped helper" in p for p in problems)
    native["components"] = []
    assert "native SBOM lacks pdf-extract-ocr-helper" in sbom.native_problems(native, prov)
    assert sbom.native_problems({}, prov)  # no components at all


def test_rust_problems_name_missing_and_unshipped_crates() -> None:
    doc = {
        "components": [
            {"name": "a", "version": "1", "purl": "pkg:cargo/a@1"},
            {"name": "loom", "version": "0.7.2"},
        ]
    }
    expected = [sbom.Crate("a", "1", "MIT", None), sbom.Crate("b", "2", "MIT", None)]
    assert sbom.rust_problems(doc, expected) == [
        "rust SBOM lacks b 2",
        "rust SBOM lists loom 0.7.2, which is not shipped",
    ]


def test_no_build_machine_paths(
    generated: tuple[sbom.Json, sbom.Json, Path],
) -> None:
    assert sbom.absolute_path_problems(generated[0]) == []
    assert sbom.absolute_path_problems(generated[1]) == []
    planted = {"a": [{"b": "/Users/someone/x"}, "file:///tmp/y", "fine", "see profile:x"]}
    assert len(sbom.absolute_path_problems(planted)) == 2


# --- the command line -----------------------------------------------------


def test_main_writes_both_documents(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    helper = tmp_path / "helper"
    helper.write_bytes(b"h")
    assert (
        sbom.main(
            ["--target", host_triple(), "--helper", str(helper), "--out-dir", str(tmp_path / "o")]
        )
        == 0
    )
    assert sorted(p.name for p in (tmp_path / "o").iterdir()) == [
        "native.cdx.json",
        "rust.cdx.json",
    ]
    assert "rust.cdx.json" in capsys.readouterr().out


def test_main_reports_a_missing_helper(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    code = sbom.main(
        ["--target", host_triple(), "--helper", str(tmp_path / "nope"), "--out-dir", str(tmp_path)]
    )
    assert code == 1
    assert "helper not found" in capsys.readouterr().err


def test_main_reports_a_cargo_failure(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    helper = tmp_path / "helper"
    helper.write_bytes(b"h")
    code = sbom.main(
        ["--target", "no-such-target", "--helper", str(helper), "--out-dir", str(tmp_path)]
    )
    assert code == 1
    assert "cargo tree for pdf-extract-python failed" in capsys.readouterr().err


def test_source_date_epoch_fixes_the_timestamp(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("SOURCE_DATE_EPOCH", "0")
    assert sbom._timestamp() == "1970-01-01T00:00:00Z"
