//! Every generator's output against hayro's `Pdf::new` (spec §4.3): builder
//! samples and hostile files open with the stated page counts, and the eight
//! encrypted fixtures open (owner-only) or fail with `PasswordProtected`
//! (user password), as spec Appendix A.7 records.
//!
//! This runs against the vendored hayro as it stands. Before units 04/05 the
//! fork is pristine: its page-tree walk and object parser recurse, so the
//! deep fixtures run on a thread with a deliberately huge stack (1 GiB of
//! address space, committed lazily). Running them on 512 KiB is the business
//! of the units that remove the recursion; here the question is only whether
//! the files are what they claim to be.

#[allow(dead_code)]
mod common {
    pub mod samples;
}

use common::samples::{Hayro, Sample, builder_samples, hostile_samples};
use hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use pdf_extract_testkit::hostile::{self, TreeShape};

const BIG_STACK: usize = 1 << 30;

fn on_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(BIG_STACK)
        .spawn(f)
        .expect("spawn test thread")
        .join()
        .expect("test thread panicked")
}

fn page_count(bytes: Vec<u8>) -> Result<usize, LoadPdfError> {
    Pdf::new(bytes).map(|pdf| pdf.pages().len())
}

fn check(samples: Vec<Sample>) {
    let failures: Vec<String> = on_big_stack(move || {
        samples
            .into_iter()
            .filter_map(|s| {
                let Hayro::Pages(want) = s.hayro;
                match page_count(s.bytes) {
                    Ok(n) if n == want => None,
                    other => Some(format!("{}: expected {want} pages, got {other:?}", s.name)),
                }
            })
            .collect()
    });
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn builder_samples_open_in_hayro() {
    check(builder_samples());
}

#[test]
fn hostile_samples_open_in_hayro() {
    check(hostile_samples());
}

/// The runtime-only sizes of Appendix C (never committed). `form_fanout(22)`
/// and the 1M-page tree are left to the units that use them: opening does
/// not interpret content, so they would only re-test the same bytes at a
/// larger size, and the 1M tree takes ~108 MB of temp space.
#[test]
fn runtime_sizes_open_in_hayro() {
    let failures: Vec<String> = on_big_stack(|| {
        let cases: Vec<(&str, Vec<u8>, usize)> = vec![
            // Unreadable page dictionary since H2a (see `deep_obj_20k`).
            (
                "deep_obj_200k",
                hostile::deep_nesting_in_page_dict(200_000),
                0,
            ),
            (
                "deep_content_200k",
                hostile::deep_nesting_in_content(200_000),
                1,
            ),
            (
                "deep_resources_200k",
                hostile::deep_nesting_in_resources(200_000),
                1,
            ),
            (
                "deep_type4_200k",
                hostile::deep_nesting_in_type4_function(200_000),
                1,
            ),
            ("deep_tree_3000", hostile::deep_page_tree(3_000), 1),
            ("flate_bomb_50mib", hostile::flate_bomb(50 << 20), 1),
            ("many_ops_2m", hostile::operator_flood(2_000_000), 1),
            ("form_fanout_16", hostile::form_fanout(16), 1),
            ("nested_forms_49", hostile::nested_forms(49), 1),
            (
                "tree_flat_100k",
                hostile::page_tree(TreeShape::Flat(100_000)),
                100_000,
            ),
            ("deep_tree_0", hostile::deep_page_tree(0), 0),
            ("nested_forms_0", hostile::nested_forms(0), 1),
        ];
        cases
            .into_iter()
            .filter_map(|(name, bytes, want)| match page_count(bytes) {
                Ok(n) if n == want => None,
                other => Some(format!("{name}: expected {want} pages, got {other:?}")),
            })
            .collect()
    });
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn big_tree_to_path_counts_and_balances() {
    let dir = std::env::temp_dir().join(format!("testkit-bigtree-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    // 32^2 + 1 exercises a partial last node and a third level.
    for leaves in [0usize, 1, 31, 32, 33, 1_025, 100_000] {
        let path = dir.join(format!("{leaves}.pdf"));
        hostile::big_tree_to_path(leaves, &path).expect("write");
        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(page_count(bytes).ok(), Some(leaves), "{leaves} leaves");
    }
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn encrypted_fixtures_open_or_require_a_password() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/corpus-c");
    let mut seen = 0;
    for alg in ["rc4_40", "rc4_128", "aes_128", "aes_256"] {
        let owner = std::fs::read(dir.join(format!("enc_{alg}_owner.pdf"))).expect("owner fixture");
        let pdf = Pdf::new(owner).unwrap_or_else(|e| panic!("{alg} owner: {e:?}"));
        assert_eq!(pdf.pages().len(), 1, "{alg} owner");
        let user = std::fs::read(dir.join(format!("enc_{alg}_user.pdf"))).expect("user fixture");
        assert!(
            matches!(
                Pdf::new(user),
                Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected))
            ),
            "{alg} user must need a password"
        );
        seen += 2;
    }
    assert_eq!(seen, 8);
}
