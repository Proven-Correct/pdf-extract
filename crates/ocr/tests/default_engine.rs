//! `default_ocr_engine()` builds one engine per process (spec 0002 O12;
//! §4.8). One test per binary: the default engine is per process.

mod common;

use pdf_extract_core::BlockSeparator;
use pdf_extract_ocr::{
    Candidates, Isolation, OcrConfig, OcrOptions, default_ocr_engine, set_candidates_override,
};

#[test]
fn both_calls_return_the_same_engine() {
    set_candidates_override(Some(Candidates {
        prefix: Some(common::tessdata()),
        known: Vec::new(),
    }));
    let first = default_ocr_engine().expect("engine");
    let second = default_ocr_engine().expect("engine");
    assert!(std::ptr::eq(first, second));
    set_candidates_override(None);
    assert_eq!(first.tessdata(), Some(common::tessdata().as_path()));
    assert_eq!(first.config(), &OcrConfig::default());
    // Rust's default stays in-process (internal #53): only the Python binding, which
    // has a package directory to find the helper in, defaults to Process.
    assert_eq!(first.config().isolation, Isolation::InProcess);
    assert_eq!(first.info().isolation, Isolation::InProcess);
    let doc = common::doc(common::body_page());
    let text = first
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(150) })
        .expect("ocr")
        .to_text(BlockSeparator::Newline);
    assert!(text.contains("quick"), "{text}");
}
