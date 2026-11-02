// Pins the test helper's candidate list (`common::tessdata_candidates`) to the
// resolver's rules, since this crate's tests cannot call the ocr crate's own
// `Candidates::ordered`. The list of known directories is shared through
// `TESSDATA_KNOWN_DIRS`; what is checked here is how the prefix is treated.

mod common;

use std::ffi::OsString;
use std::path::PathBuf;

use common::tessdata_candidates;
use pdf_extract_tesseract_sys::TESSDATA_KNOWN_DIRS;

fn known() -> Vec<PathBuf> {
    TESSDATA_KNOWN_DIRS.iter().map(PathBuf::from).collect()
}

#[test]
fn unset_prefix_yields_only_the_known_directories() {
    assert_eq!(tessdata_candidates(None), known());
}

/// The regression: an empty `TESSDATA_PREFIX` used to add `""` and the
/// relative `tessdata`, which the ocr resolver (and Tesseract) never try.
#[test]
fn empty_prefix_counts_as_unset() {
    assert_eq!(tessdata_candidates(Some(OsString::new())), known());
}

#[test]
fn prefix_is_tried_as_given_then_with_tessdata_before_the_known_directories() {
    let got = tessdata_candidates(Some(OsString::from("/models")));
    let mut want = vec![PathBuf::from("/models"), PathBuf::from("/models/tessdata")];
    want.extend(known());
    assert_eq!(got, want);
}
