//! `tests/fixtures/manifest.toml` is complete and correct (spec §3.6, §4.3):
//! every committed fixture file has exactly one entry with a matching SHA-256,
//! every entry names a file that exists, the per-gate and per-corpus required
//! keys are present, and every fixture not produced by the test kit has a
//! PROVENANCE.md row carrying the same hash.
//!
//! The walk covers all of `tests/fixtures/`, so a stray file anywhere (a new
//! corpus directory, a file at the top level, a dotfile) must be listed or the
//! test fails. The only exemptions are explicit: the manifest and provenance
//! files themselves, `.gitkeep` placeholders, `hostile/` (runtime-generated
//! only; its `.gitignore` keeps it out of git), and `expectations/` and
//! `snapshots/` (oracle outputs keyed by fixture hash, checked against the
//! manifest by unit 13). Symlinks are rejected rather than followed.
//!
//! `[[built]]` rows (run 0002 §3.8) pin corpus D's noise variants, which the
//! test kit builds at test time and which are never committed; they are
//! checked the other way round (see `check_built_rows`).
//!
//! The fixture counts that README.md and ARCHITECTURE.md quote are checked
//! against the manifest too (`docs_quote_the_manifest_fixture_counts`), and
//! ARCHITECTURE.md's vendor patch counts against `vendor/`
//! (`docs_quote_the_vendor_patch_counts`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use toml::{Table, Value};

/// Top-level files and directories that are not fixtures.
const EXEMPT_TOP: [&str; 5] = [
    "manifest.toml",
    "PROVENANCE.md",
    "hostile",
    "expectations",
    "snapshots",
];
// `hostile` (D-27, spec revision 10): outcome class, warning kinds and our
// pinned snapshot, never the oracle's text.
// `ocr` (run 0002 §3.8): corpus D, scored by crates/ocr/tests/quality.rs.
const GATES: [&str; 6] = [
    "exact",
    "similarity",
    "divergence",
    "skip",
    "hostile",
    "ocr",
];

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every non-exempt file under `root`, as a `/`-separated relative path.
fn committed_files(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("readable fixtures directory") {
            let e = e.expect("readable directory entry");
            let p = e.path();
            let rel = p
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            if EXEMPT_TOP.contains(&rel.as_str()) || e.file_name() == ".gitkeep" {
                continue;
            }
            let meta = std::fs::symlink_metadata(&p).expect("stat fixture");
            assert!(
                !meta.file_type().is_symlink(),
                "{rel}: symlinks are not allowed"
            );
            if meta.is_dir() {
                stack.push(p);
            } else {
                out.insert(rel);
            }
        }
    }
    out
}

fn str_key<'a>(entry: &'a Table, key: &str, path: &str) -> &'a str {
    entry
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{path}: missing string key `{key}`"))
}

#[test]
fn manifest_lists_every_committed_fixture_with_matching_hash() {
    let root = fixtures_dir();
    let text = std::fs::read_to_string(root.join("manifest.toml")).expect("manifest.toml");
    let manifest: Table = toml::from_str(&text).expect("manifest parses as TOML");
    let entries: Vec<&Table> = match manifest.get("fixture") {
        None => Vec::new(),
        Some(v) => v
            .as_array()
            .expect("`fixture` is an array of tables")
            .iter()
            .map(|e| e.as_table().expect("fixture entry is a table"))
            .collect(),
    };
    let provenance = std::fs::read_to_string(root.join("PROVENANCE.md")).expect("PROVENANCE.md");

    let mut listed: BTreeMap<String, String> = BTreeMap::new();
    for e in &entries {
        let path = str_key(e, "path", "<entry>").to_owned();
        let sha = str_key(e, "sha256", &path).to_owned();
        let generator = str_key(e, "generator", &path);
        let gate = str_key(e, "gate", &path);
        assert!(GATES.contains(&gate), "{path}: unknown gate {gate:?}");
        assert!(
            e.get("compat_points").and_then(Value::as_array).is_some(),
            "{path}: compat_points must be an array"
        );
        if gate == "divergence" {
            str_key(e, "divergence", &path);
        }
        if gate == "divergence" || gate == "hostile" {
            let snap = str_key(e, "snapshot", &path);
            assert_eq!(
                snap,
                format!("snapshots/{sha}.txt"),
                "{path}: snapshot name"
            );
            assert!(root.join(snap).is_file(), "{path}: snapshot {snap} missing");
        }
        if gate == "ocr" {
            str_key(e, "subset", &path);
            str_key(e, "truth", &path);
        }
        if path.starts_with("corpus-b/") {
            for k in ["origin", "license", "license_note"] {
                str_key(e, k, &path);
            }
        }
        // Anything the test kit did not generate needs a provenance row.
        if !generator.starts_with("testkit::") {
            let row = provenance
                .lines()
                .find(|l| l.contains(&format!("`{path}`")))
                .unwrap_or_else(|| panic!("{path}: no PROVENANCE.md row"));
            assert!(
                row.contains(&sha),
                "{path}: PROVENANCE.md row has a different hash"
            );
        }
        assert!(
            listed.insert(path.clone(), sha).is_none(),
            "{path}: listed twice"
        );
    }

    let files = committed_files(&root);
    let listed_paths: BTreeSet<String> = listed.keys().cloned().collect();
    let unlisted: Vec<_> = files.difference(&listed_paths).collect();
    let missing: Vec<_> = listed_paths.difference(&files).collect();
    assert!(
        unlisted.is_empty(),
        "files not in manifest.toml: {unlisted:?}"
    );
    assert!(
        missing.is_empty(),
        "manifest entries without a file: {missing:?}"
    );

    for e in &entries {
        if e.get("gate").and_then(Value::as_str) == Some("ocr") {
            let path = str_key(e, "path", "<entry>");
            let truth = str_key(e, "truth", path);
            assert!(
                listed.contains_key(truth),
                "{path}: truth {truth} is not a fixture"
            );
        }
    }
    check_built_rows(&manifest, &listed);

    for (path, sha) in &listed {
        let actual = sha256_hex(&std::fs::read(root.join(path)).expect("fixture readable"));
        assert_eq!(&actual, sha, "{path}: SHA-256 mismatch");
    }
}

#[test]
fn eight_encrypted_fixtures_exist() {
    let files = committed_files(&fixtures_dir());
    let enc: Vec<_> = files.iter().filter(|f| f.contains("/enc_")).collect();
    assert_eq!(enc.len(), 8, "{enc:?}");
}

/// README.md and ARCHITECTURE.md quote fixture counts; they are computed here
/// from the manifest, so a corpus change that forgets the docs fails instead
/// of drifting.
#[test]
fn docs_quote_the_manifest_fixture_counts() {
    let root = fixtures_dir();
    let text = std::fs::read_to_string(root.join("manifest.toml")).expect("manifest.toml");
    let manifest: Table = toml::from_str(&text).expect("manifest parses as TOML");
    let rows = |key: &str| -> Vec<Table> {
        manifest
            .get(key)
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("manifest has no [[{key}]] rows"))
            .iter()
            .map(|e| e.as_table().expect("manifest row is a table").clone())
            .collect()
    };
    let fixtures = rows("fixture");
    let count = |corpus: &str, keep: &dyn Fn(&Table) -> bool| {
        fixtures
            .iter()
            .filter(|e| str_key(e, "path", "<entry>").starts_with(&format!("{corpus}/")) && keep(e))
            .count()
    };
    let corpus_c = count("corpus-c", &|_| true);
    let encrypted = count("corpus-c", &|e| {
        str_key(e, "generator", "<entry>") == "tools/oracle/mkfixtures.py::enc"
    });
    let corpus_b = count("corpus-b", &|_| true);
    let scans = count("corpus-d", &|e| str_key(e, "gate", "<entry>") == "ocr");
    let built = rows("built").len();

    let generated = corpus_c - encrypted;
    let expected = [
        (
            "README.md",
            "real-world",
            format!("{corpus_c} synthetic and {corpus_b} real-world"),
        ),
        (
            "ARCHITECTURE.md",
            "| `tests/fixtures/` |",
            format!(
                "`corpus-c/` ({encrypted} encrypted + {generated} generated, {corpus_c} manifest entries)"
            ),
        ),
        (
            "ARCHITECTURE.md",
            "| `tests/fixtures/` |",
            format!("`corpus-b/` ({corpus_b} imported files)"),
        ),
        (
            "ARCHITECTURE.md",
            "| `tests/fixtures/` |",
            format!("(OCR: {scans} committed image-only scans"),
        ),
        (
            "ARCHITECTURE.md",
            "| `tests/fixtures/` |",
            format!("; {built} noise variants"),
        ),
        (
            "README.md",
            "noise variants",
            format!("corpus D's {} noise variants", in_words(built)),
        ),
    ];
    assert_docs_quote(&expected, "tests/fixtures/manifest.toml");
}

/// ARCHITECTURE.md quotes the size of the hayro patch series and the number of
/// crates `check-patches.sh` fetches; both are counted here from
/// `vendor/patches/hayro/` and the hayro rows of `vendor/PROVENANCE.md`, so a
/// new patch or crate that forgets the docs fails instead of drifting (internal #99).
#[test]
fn docs_quote_the_vendor_patch_counts() {
    let vendor = fixtures_dir().join("../../vendor");
    let patches = std::fs::read_dir(vendor.join("patches/hayro"))
        .expect("vendor/patches/hayro")
        .filter(|e| {
            e.as_ref()
                .expect("dir entry")
                .path()
                .extension()
                .is_some_and(|x| x == "patch")
        })
        .count();
    let provenance =
        std::fs::read_to_string(vendor.join("PROVENANCE.md")).expect("vendor/PROVENANCE.md");
    let crates = provenance
        .lines()
        .filter(|l| l.starts_with("| hayro-"))
        .count();
    assert!(
        patches > 0 && crates > 0,
        "{patches} patches, {crates} crates"
    );
    let expected = [
        (
            "ARCHITECTURE.md",
            "| `vendor/` |",
            format!("patches 0001\u{2013}{patches:04} ("),
        ),
        (
            "ARCHITECTURE.md",
            "| `vendor-patches` |",
            format!("{} hayro rows, {patches} patches)", in_words(crates)),
        ),
    ];
    assert_docs_quote(&expected, "vendor/patches/hayro and vendor/PROVENANCE.md");
}

/// A small count as the docs spell it in running prose.
fn in_words(n: usize) -> &'static str {
    const WORDS: [&str; 13] = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve",
    ];
    WORDS
        .get(n)
        .unwrap_or_else(|| panic!("{n} is past the spelled-out counts; quote it in digits"))
}

/// Asserts that each `(doc, anchor, phrase)` holds: `doc` (relative to the
/// repository root) contains `phrase` with runs of whitespace collapsed, so
/// re-wrapping a paragraph does not matter. The anchor is a string that stays
/// on the line to update, so a failure can name that line.
fn assert_docs_quote(expected: &[(&str, &str, String)], source: &str) {
    for (doc, anchor, phrase) in expected {
        let raw = std::fs::read_to_string(fixtures_dir().join("../..").join(doc))
            .unwrap_or_else(|e| panic!("{doc}: {e}"));
        let flat = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let line = raw.lines().position(|l| l.contains(anchor)).map_or_else(
            || format!("the line containing {anchor:?}"),
            |i| format!("line {}", i + 1),
        );
        assert!(
            flat.contains(phrase.as_str()),
            "{doc}: counts are stale; {line} should say {phrase:?} (counted from {source})"
        );
    }
}

/// `[[built]]` rows (run 0002 §3.8 "Pinning"): files the test kit builds at
/// test time, so the opposite of `[[fixture]]`: they must **not** exist under
/// `tests/fixtures/`, their `source` and `truth` must be committed fixtures,
/// and the set of paths is exactly the test kit's noise variants.
fn check_built_rows(manifest: &Table, listed: &BTreeMap<String, String>) {
    let rows: Vec<&Table> = manifest
        .get("built")
        .and_then(Value::as_array)
        .expect("manifest has [[built]] rows (corpus D noise variants)")
        .iter()
        .map(|r| r.as_table().expect("built row is a table"))
        .collect();
    let root = fixtures_dir();
    let mut paths = BTreeSet::new();
    for r in rows {
        let path = str_key(r, "path", "<built>");
        let sha = str_key(r, "sha256", path);
        assert!(
            sha.len() == 64
                && sha
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{path}: sha256 {sha:?}"
        );
        assert_eq!(
            str_key(r, "generator", path),
            "testkit::corpus_d::noise",
            "{path}"
        );
        assert_eq!(str_key(r, "gate", path), "ocr", "{path}");
        assert_eq!(str_key(r, "subset", path), "noise", "{path}");
        for key in ["source", "truth"] {
            let target = str_key(r, key, path);
            assert!(
                listed.contains_key(target),
                "{path}: {key} {target} is not a fixture"
            );
        }
        assert!(
            !root.join("corpus-d").join(path).exists(),
            "{path}: a built file must not be committed"
        );
        assert!(paths.insert(path.to_owned()), "{path}: listed twice");
    }
    let want: BTreeSet<String> = pdf_extract_testkit::corpus_d::noise_names()
        .into_iter()
        .map(|n| format!("{n}.pdf"))
        .collect();
    assert_eq!(
        paths, want,
        "[[built]] paths differ from corpus_d::noise_names()"
    );
}
