//! The corpus-D gate's model-pin check, done once up front (internal #97 B5).
//!
//! The committed expectations hold for one model, tessdata_fast
//! `eng.traineddata` (spec D-17), and `model_dir()` follows the library's
//! discovery order (ADRs 0057, 0069), so on a machine with another
//! installation (MacPorts' combined set, say) and no `TESSDATA_PREFIX` it
//! finds the wrong one. The per-page SHA comparison catches that too, but as
//! 52 identical lines that never say which directory was found or why. This
//! module turns the same fact into one message naming the cause and the
//! remedy. It only reports; the discovery order is deliberately untouched
//! (ADR 0069 fixes it, ADR 0072 requires a mismatch to fail).

use std::collections::BTreeSet;
use std::path::Path;

use pdf_extract_ocr::Candidates;

/// How `dir` was chosen by the discovery order, for the failure message.
pub fn model_source(dir: &Path, candidates: &Candidates) -> String {
    match &candidates.prefix {
        Some(p) if dir == p.as_path() || dir == p.join("tessdata") => {
            format!("TESSDATA_PREFIX={}", p.display())
        }
        Some(p) => format!(
            "a known install location, because TESSDATA_PREFIX={} holds no eng.traineddata",
            p.display()
        ),
        None => "a known install location, because TESSDATA_PREFIX is unset".to_owned(),
    }
}

/// `Some(message)` when the model found (`found_sha`, in `dir`, chosen as
/// `source`) is not the one every expectation was made with (`pinned`, the
/// distinct `tessdata_sha256` values over the corpus); `None` when it is.
pub fn model_pin_failure(
    found_sha: &str,
    pinned: &BTreeSet<String>,
    dir: &Path,
    source: &str,
    remedy_dir: &Path,
) -> Option<String> {
    if pinned.len() == 1 && pinned.contains(found_sha) {
        return None;
    }
    let pinned_list = pinned.iter().cloned().collect::<Vec<_>>().join(", ");
    Some(format!(
        "quality: the model found is not the pinned one (spec D-17: corpus D's numbers hold \
         for tessdata_fast only). Found {dir} (SHA-256 {found_sha}), chosen via {source}; the \
         expectations were made with {pinned_list}. Point the gate at the pinned model: \
         TESSDATA_PREFIX={remedy} (tools/oracle/mkcorpusd.py --fetch-model downloads it)",
        dir = dir.display(),
        remedy = remedy_dir.display(),
    ))
}

#[test]
fn matching_model_passes() {
    let pinned = BTreeSet::from(["aa".to_owned()]);
    let none = model_pin_failure("aa", &pinned, Path::new("/m"), "x", Path::new("/r"));
    assert_eq!(none, None);
}

#[test]
fn mismatch_message_names_directory_source_both_shas_and_remedy() {
    let pinned = BTreeSet::from(["7d4322bd".to_owned()]);
    let msg = model_pin_failure(
        "ffff0000",
        &pinned,
        Path::new("/opt/local/share/tessdata"),
        "a known install location, because TESSDATA_PREFIX is unset",
        Path::new("/repo/tools/oracle/.cache/tessdata_fast"),
    )
    .expect("a different SHA must fail");
    for part in [
        "/opt/local/share/tessdata",
        "TESSDATA_PREFIX is unset",
        "7d4322bd",
        "ffff0000",
        "TESSDATA_PREFIX=/repo/tools/oracle/.cache/tessdata_fast",
    ] {
        assert!(msg.contains(part), "{part:?} missing from: {msg}");
    }
}

#[test]
fn expectations_that_disagree_with_each_other_fail_even_if_one_matches() {
    let pinned = BTreeSet::from(["aa".to_owned(), "bb".to_owned()]);
    let msg = model_pin_failure("aa", &pinned, Path::new("/m"), "x", Path::new("/r"));
    assert!(msg.is_some_and(|m| m.contains("aa, bb")));
}

#[test]
fn source_names_the_prefix_or_the_known_location() {
    let c = |prefix: Option<&str>| Candidates {
        prefix: prefix.map(Into::into),
        known: vec!["/opt/local/share/tessdata".into()],
    };
    assert_eq!(
        model_source(Path::new("/p/tessdata"), &c(Some("/p"))),
        "TESSDATA_PREFIX=/p"
    );
    assert_eq!(
        model_source(Path::new("/p"), &c(Some("/p"))),
        "TESSDATA_PREFIX=/p"
    );
    let s = model_source(Path::new("/opt/local/share/tessdata"), &c(Some("/q")));
    assert!(
        s.contains("known install location") && s.contains("/q"),
        "{s}"
    );
    let s = model_source(Path::new("/opt/local/share/tessdata"), &c(None));
    assert!(s.contains("TESSDATA_PREFIX is unset"), "{s}");
}
