//! Same results in both isolation modes (spec 0002 O26; §4.9 "Same
//! results"): with the same model and resolution, a `Process` engine's
//! text equals the `InProcess` engine's byte for byte, for every corpus-D
//! clean page (the twelve committed scans of spec 0002 Appendix C) at the
//! default 300 dpi, plus a blank page; and, with the noise pre-check on
//! (run 0003 F17), the page it downsamples and one it skips, whose results
//! (resolution and warnings included) are the same in both modes because
//! the check runs in the parent, in front of either backend.

mod common;

use std::time::Duration;

use pdf_extract_core::BlockSeparator;
use pdf_extract_ocr::{Isolation, OcrConfig, OcrEngine, OcrOptions};

/// The page budget for both engines. This test is about text, not time:
/// at the default 60 s a debug build on a loaded host (or under coverage)
/// timed pages out (internal issue #64), so the budget is only a backstop against a
/// hang.
const PAGE_TIMEOUT: Duration = Duration::from_secs(600);

#[test]
fn process_and_in_process_engines_produce_identical_text() {
    let nowhere = common::nowhere();
    let inproc = OcrEngine::tesseract_with_candidates(
        OcrConfig {
            page_timeout: PAGE_TIMEOUT,
            ..common::config()
        },
        &nowhere,
    )
    .expect("in");
    let process = OcrEngine::tesseract_with_candidates(
        OcrConfig {
            page_timeout: PAGE_TIMEOUT,
            ..common::process_config()
        },
        &nowhere,
    )
    .expect("proc");
    assert_eq!(inproc.info().isolation, Isolation::InProcess);
    assert_eq!(process.info().isolation, Isolation::Process);
    // Same model, same library: only the isolation differs.
    let (mut a, mut b) = (inproc.info(), process.info());
    a.isolation = Isolation::Process;
    b.isolation = Isolation::Process;
    assert_eq!(a, b);
    let mut pages: Vec<(&str, Vec<u8>)> = common::corpus_d_clean()
        .into_iter()
        .map(|name| (name, common::corpus_d(name)))
        .collect();
    assert_eq!(pages.len(), 12);
    pages.push(("blank", common::blank_page()));
    pages.push(("sparse_below_min_chars_noise", common::sparse_noise_page()));
    pages.push(("pure noise", common::pure_noise_pdf()));
    for (name, bytes) in pages {
        let doc = common::doc(bytes);
        let page = doc.page(0).expect("page");
        let opts = OcrOptions { dpi: Some(300) };
        let x = inproc.ocr(&page, &opts).expect("in-process");
        let y = process.ocr(&page, &opts).expect("process");
        assert_eq!(x.dpi(), y.dpi());
        assert_eq!(x.warnings(), y.warnings(), "{name}");
        assert_eq!(
            x.to_text(BlockSeparator::Newline),
            y.to_text(BlockSeparator::Newline),
            "{name}"
        );
        assert_eq!(
            x.to_text(BlockSeparator::BlankLine),
            y.to_text(BlockSeparator::BlankLine)
        );
    }
}
