//! `default_ocr_engine()` stores a construction failure (spec 0002 O12;
//! §4.8): the second call returns the *same* error object without resolving
//! again. One test per binary: the default engine is per process.

mod common;

use pdf_extract_core::UnavailableReason;
use pdf_extract_ocr::{Candidates, default_ocr_engine, resolutions, set_candidates_override};

#[test]
fn a_failure_is_stored_and_not_retried() {
    // The injectable resolver stands in for `TESSDATA_PREFIX` pointing at a
    // directory without models.
    let bad = common::scratch("default-bad");
    set_candidates_override(Some(Candidates {
        prefix: Some(bad.clone()),
        known: Vec::new(),
    }));
    let before = resolutions();
    let first = default_ocr_engine().expect_err("no model there");
    assert_eq!(first.reason, UnavailableReason::NoTessdata);
    assert_eq!(first.searched, vec![bad.clone(), bad.join("tessdata")]);
    assert_eq!(resolutions(), before + 1);
    // Even once a model would be found, the stored failure stands.
    set_candidates_override(Some(Candidates {
        prefix: Some(common::tessdata()),
        known: Vec::new(),
    }));
    let second = default_ocr_engine().expect_err("stored");
    assert!(std::ptr::eq(first, second), "the same error object");
    assert_eq!(resolutions(), before + 1, "no second resolution");
    set_candidates_override(None);
}
