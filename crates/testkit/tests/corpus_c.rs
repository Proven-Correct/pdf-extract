//! The committed corpus-C fixtures are exactly what the catalogue in
//! `tests/common/corpus/` builds (spec §4.13): every generated file is
//! byte-identical to a fresh build, the generated section of
//! `manifest.toml` is exactly what `examples/corpus_c.rs` would write
//! (hashes, compat points, gates, divergence reasons, warning kinds), and
//! `corpus-c/` holds nothing else except the oracle-made encrypted files.
//!
//! This is the provenance of the generated fixtures: a reviewer changes a
//! recipe or a gate in the catalogue, reruns the example, and this test
//! proves the committed bytes and manifest followed.

#[allow(dead_code)]
mod common {
    pub mod corpus;
}

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::corpus::{self, Gate};
use sha2::{Digest, Sha256};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn committed_files_match_the_catalogue() {
    let dir = fixtures_dir().join("corpus-c");
    let fixtures = corpus::catalogue();
    let mut names = BTreeSet::new();
    for f in &fixtures {
        assert!(names.insert(f.file.clone()), "{}: listed twice", f.file);
        let committed = std::fs::read(dir.join(&f.file))
            .unwrap_or_else(|e| panic!("{}: {e} (run the corpus_c example)", f.file));
        assert!(
            committed == f.bytes,
            "{}: committed bytes differ from the recipe (run the corpus_c example)",
            f.file
        );
    }
    for item in std::fs::read_dir(&dir).expect("corpus-c") {
        let name = item
            .expect("entry")
            .file_name()
            .into_string()
            .expect("UTF-8");
        assert!(
            names.contains(&name) || name.starts_with("enc_") || name == ".gitkeep",
            "corpus-c/{name}: not in the catalogue"
        );
    }
    // Hashes key the expectations and snapshots, so no two recipes may
    // produce the same bytes.
    let hashes: BTreeSet<String> = fixtures.iter().map(|f| sha256_hex(&f.bytes)).collect();
    assert_eq!(
        hashes.len(),
        fixtures.len(),
        "two recipes produce identical bytes"
    );
}

#[test]
fn manifest_section_matches_the_catalogue() {
    let manifest =
        std::fs::read_to_string(fixtures_dir().join("manifest.toml")).expect("manifest.toml");
    let start = manifest
        .find(corpus::MANIFEST_BEGIN)
        .expect("generated section");
    let end = manifest[start..]
        .find(corpus::MANIFEST_END)
        .expect("generated section end")
        + start
        + corpus::MANIFEST_END.len();
    let mut want = format!("{}\n", corpus::MANIFEST_BEGIN);
    for f in corpus::catalogue() {
        want.push('\n');
        want.push_str(&corpus::manifest_block(&f, &sha256_hex(&f.bytes)));
    }
    want.push('\n');
    want.push_str(corpus::MANIFEST_END);
    assert!(
        manifest[start..end] == want,
        "manifest.toml's generated section is stale (run the corpus_c example)"
    );
}

#[test]
fn every_compat_point_has_a_fixture() {
    // Spec §4.13: one or more fixtures per KP1-KP20 (KP17 through the KP9
    // divergence rows), plus the KP21, KP22 and KP24 rows. KP23 is the
    // runtime `deep_tree_3000` (Appendix C), never committed.
    let covered: BTreeSet<&str> = corpus::catalogue()
        .iter()
        .flat_map(|f| f.compat_points.clone())
        .collect();
    for n in (1..=22).chain([24]) {
        let kp = format!("KP{n}");
        assert!(
            covered.contains(kp.as_str()),
            "{kp} has no corpus-C fixture"
        );
    }
}

#[test]
fn divergences_cite_a_reason_and_hostile_rows_list_warnings() {
    for f in corpus::catalogue() {
        if let Gate::Divergence(reason) = f.gate {
            assert!(reason.len() > 20, "{}: reason too short", f.file);
        }
        if f.gate == Gate::Hostile {
            assert!(f.warnings.is_some(), "{}: hostile without warnings", f.file);
        }
        if let Some(kinds) = &f.warnings {
            let mut sorted = kinds.clone();
            sorted.sort_unstable();
            assert_eq!(&sorted, kinds, "{}: warnings unsorted", f.file);
        }
    }
}
