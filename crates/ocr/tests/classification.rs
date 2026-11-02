//! Failure classification through a directly called `ScriptedBackend`
//! (spec 0002 O15, O33; §4.8 "Classification"). No tessdata is needed:
//! `with_backend` skips resolution and validation.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use pdf_extract_core::{BlockSeparator, OcrError, OcrPageError, TextSource, WarningKind};
use pdf_extract_ocr::{OcrConfig, OcrEngine, OcrOptions, PageOcr, PageSegMode};
use pdf_extract_testkit::ocr::{ScriptedBackend, Step, output};

fn engine(backend: &ScriptedBackend, cfg: OcrConfig) -> OcrEngine {
    OcrEngine::with_backend(Arc::new(backend.clone()), cfg)
}

fn run(backend: &ScriptedBackend, step: Step) -> Result<pdf_extract_core::TextPage, OcrError> {
    backend.push(step);
    let doc = common::doc(common::blank_page());
    engine(backend, OcrConfig::default())
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(36) })
}

#[test]
fn each_backend_outcome_surfaces_as_specified() {
    let b = ScriptedBackend::new();
    assert!(matches!(
        run(&b, Step::recognition("bad")),
        Err(OcrError::Page(OcrPageError::Recognition(m))) if m == "bad"
    ));
    // Whatever duration the backend reports, the error carries the page
    // budget.
    assert!(matches!(
        run(&b, Step::timeout()),
        Err(OcrError::Page(OcrPageError::Timeout(d))) if d == Duration::from_secs(60)
    ));
    let empty = run(&b, Step::empty()).expect("empty is success");
    assert_eq!(empty.to_text(BlockSeparator::Newline), "");
    assert_eq!(empty.source(), TextSource::Ocr);
    let r = std::panic::catch_unwind(|| run(&b, Step::Panic("scripted panic".into())));
    assert!(r.is_err());
    assert_eq!(b.calls(), 4);
    assert_eq!(pdf_extract_ocr::OcrBackend::info(&b).name, "scripted");
}

#[test]
fn the_request_carries_dpi_psm_and_a_deadline_from_before_rendering() {
    let b = ScriptedBackend::new();
    let cfg = OcrConfig {
        psm: PageSegMode::SingleBlock,
        page_timeout: Duration::from_secs(5),
        dpi: 144,
        ..OcrConfig::default()
    };
    let doc = common::doc(common::blank_page());
    let before = Instant::now();
    engine(&b, cfg)
        .ocr(&doc.page(0).expect("page"), &OcrOptions::default())
        .expect("ocr");
    let req = b.last_request().expect("called");
    assert_eq!(req.dpi, 144);
    assert_eq!(req.psm, PageSegMode::SingleBlock);
    assert!(req.deadline >= before + Duration::from_secs(5));
    assert!(req.deadline <= Instant::now() + Duration::from_secs(5));
    // Letter at 144 dpi: 1224 x 1584.
    assert_eq!(b.last_image(), Some((1224, 1584)));
}

#[test]
fn words_become_lines_in_reading_order() {
    let b = ScriptedBackend::new();
    b.push(Step::Output(output(&[
        &[("Hello", [10, 10, 60, 30]), ("world", [70, 10, 130, 30])],
        &[("again\u{0}", [10, 40, 60, 60])],
    ])));
    let doc = common::doc(common::blank_page());
    let tp = doc
        .page(0)
        .expect("page")
        .ocr(
            &engine(&b, OcrConfig::default()),
            &OcrOptions { dpi: Some(72) },
        )
        .expect("ocr");
    // D-10 sanitising applies to OCR text as to the text path.
    assert_eq!(
        tp.to_text(BlockSeparator::Newline),
        "Hello world\nagain\u{FFFD}\n"
    );
    assert_eq!(tp.dpi(), Some(72));
}

#[test]
fn a_render_that_eats_the_budget_times_out_before_recognition() {
    let b = ScriptedBackend::new();
    let cfg = OcrConfig {
        page_timeout: Duration::from_nanos(1),
        ..OcrConfig::default()
    };
    let doc = common::doc(common::dense_page());
    let err = engine(&b, cfg)
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(72) })
        .expect_err("budget gone");
    assert!(
        matches!(err, OcrError::Page(OcrPageError::Timeout(_))),
        "{err:?}"
    );
    assert_eq!(b.calls(), 0);
}

#[test]
fn renderer_warnings_are_the_pages_warnings() {
    // An image too large for the pixel cap: skipped with a warning (O4,
    // O19), and the page is still recognised.
    let mut builder = pdf_extract_testkit::PdfBuilder::new();
    builder
        .add_page(100.0, 100.0)
        .raw_content("q 100 0 0 100 0 0 cm BI /W 100000 /H 100000 /BPC 8 /CS /G ID \u{0} EI Q");
    let limits = pdf_extract_core::Limits {
        max_image_pixels: 1_000,
        ..pdf_extract_core::Limits::default()
    };
    let doc = common::open(builder.to_bytes(), limits);
    let b = ScriptedBackend::new();
    let tp = engine(&b, OcrConfig::default())
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(36) })
        .expect("ocr");
    assert!(
        tp.warnings()
            .iter()
            .any(|w| w.kind == WarningKind::ImagePixelLimit),
        "{:?}",
        tp.warnings()
    );
    assert_eq!(b.calls(), 1);
}

#[test]
fn a_codec_memory_refusal_is_one_page_warning() {
    // Run 0003 H14: a JBIG2 image whose decoder would exceed its memory
    // budget is skipped with one `image: memory limit` warning, and the page
    // is still recognised.
    let limits = pdf_extract_core::Limits {
        max_image_pixels: 1_000_000,
        ..pdf_extract_core::Limits::default()
    };
    let doc = common::open(pdf_extract_testkit::jbig2::symbol_export_bomb_pdf(), limits);
    let b = ScriptedBackend::new();
    let tp = engine(&b, OcrConfig::default())
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(36) })
        .expect("ocr");
    let kinds: Vec<_> = tp.warnings().iter().map(|w| (w.kind, w.count)).collect();
    assert_eq!(kinds, [(WarningKind::ImageMemoryLimit, 1)]);
    assert!(
        tp.warnings()[0]
            .to_string()
            .starts_with("image: memory limit")
    );
    assert_eq!(b.calls(), 1);
}

#[test]
fn a_jpx_memory_refusal_is_one_page_warning() {
    // Run 0003 H15: the same for a JPEG 2000 image whose tile records alone
    // exceed its decoder's budget.
    let limits = pdf_extract_core::Limits {
        max_image_pixels: 1_000_000,
        ..pdf_extract_core::Limits::default()
    };
    let doc = common::open(pdf_extract_testkit::jpx::tile_bomb_pdf(), limits);
    let b = ScriptedBackend::new();
    let tp = engine(&b, OcrConfig::default())
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(36) })
        .expect("ocr");
    let kinds: Vec<_> = tp.warnings().iter().map(|w| (w.kind, w.count)).collect();
    assert_eq!(kinds, [(WarningKind::ImageMemoryLimit, 1)]);
    assert_eq!(b.calls(), 1);
}

#[test]
fn engines_are_cheap_clones_sharing_one_backend() {
    let b = ScriptedBackend::new();
    let e = engine(&b, OcrConfig::default());
    let e2 = e.clone();
    let doc = common::doc(common::blank_page());
    let page = doc.page(0).expect("page");
    e.ocr(&page, &OcrOptions { dpi: Some(36) }).expect("ocr");
    e2.ocr(&page, &OcrOptions { dpi: Some(36) }).expect("ocr");
    assert_eq!(b.calls(), 2);
    assert_eq!(e2.pool_len(), None);
    assert_eq!(e2.tessdata(), None);
    assert_eq!(e2.config().dpi, 300);
}
