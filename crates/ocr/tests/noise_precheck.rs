//! The noise pre-check inside `OcrEngine::ocr` (spec 0003 F12–F14, §4.2
//! "Engine"), with `ScriptedBackend` standing in for Tesseract: what the
//! backend is given (raster size, resolution, whether it is called at all),
//! the result's resolution, warnings and coordinates, the opt-out, the
//! resolution floor and the page budget. Needs no tessdata.

mod common;

use std::sync::Arc;
use std::time::Duration;

use pdf_extract_core::{BlockSeparator, OcrError, OcrPageError, TextPage, TextSource, WarningKind};
use pdf_extract_ocr::{OcrConfig, OcrEngine, OcrOptions, PageOcr};
use pdf_extract_testkit::ocr::{ScriptedBackend, Step, output};

/// An engine over `backend` called directly, with `cfg`.
fn engine(backend: &ScriptedBackend, cfg: OcrConfig) -> OcrEngine {
    OcrEngine::with_backend(Arc::new(backend.clone()), cfg)
}

fn ocr(engine: &OcrEngine, pdf: Vec<u8>, dpi: Option<u32>) -> Result<TextPage, OcrError> {
    let doc = common::doc(pdf);
    let page = doc.page(0).expect("page 0");
    page.ocr(engine, &OcrOptions { dpi })
}

fn ocr_warnings(tp: &TextPage) -> Vec<String> {
    tp.warnings()
        .iter()
        .map(ToString::to_string)
        .filter(|w| w.starts_with("ocr:"))
        .collect()
}

#[test]
fn the_sparse_noise_page_is_recognised_at_half_resolution() {
    let backend = ScriptedBackend::with_default(Step::Output(output(&[&[(
        "Exhibit",
        [100, 100, 200, 120],
    )]])));
    let e = engine(&backend, OcrConfig::default());
    let tp = ocr(&e, common::sparse_noise_page(), Some(300)).expect("ocr");
    assert_eq!(backend.calls(), 1);
    assert_eq!(backend.last_image(), Some((1240, 1754)));
    assert_eq!(backend.last_request().expect("called").dpi, 150);
    assert_eq!(tp.source(), TextSource::Ocr);
    assert_eq!(tp.dpi(), Some(150));
    assert_eq!(tp.to_text(BlockSeparator::Newline), "Exhibit\n");
    let noise: Vec<_> = tp
        .warnings()
        .iter()
        .filter(|w| w.kind == WarningKind::OcrNoiseDownsampled)
        .collect();
    assert_eq!(noise.len(), 1);
    assert_eq!(noise[0].page, Some(0));
    assert_eq!(
        ocr_warnings(&tp),
        ["ocr: noise-dominated raster recognised at 150 dpi (rendered at 300)"]
    );
    // F13: page points = px x 2 x 72 / 300, flipped against the rendered
    // height (3509 px at 300 dpi).
    let per_px = 72.0f32 / 300.0;
    let height = 3509.0 * per_px;
    assert_eq!(
        tp.blocks()[0].lines()[0].bbox(),
        [
            200.0 * per_px,
            height - 240.0 * per_px,
            400.0 * per_px,
            height - 200.0 * per_px
        ]
    );
    // Which is where a 150-dpi pixel box [100, 100, 200, 120] lies: x at
    // 48-96 pt, 48-57.6 pt below the top of the raster.
    let [x0, y0, x1, y1] = tp.blocks()[0].lines()[0].bbox();
    assert!((x0 - 48.0).abs() < 1e-4 && (x1 - 96.0).abs() < 1e-4);
    assert!((height - y1 - 48.0).abs() < 1e-3 && (height - y0 - 57.6).abs() < 1e-3);
}

#[test]
fn a_pure_noise_page_is_skipped_without_calling_the_backend() {
    let backend = ScriptedBackend::with_default(Step::Panic("must not be called".into()));
    let e = engine(&backend, OcrConfig::default());
    let tp = ocr(&e, common::pure_noise_pdf(), None).expect("skipped, not failed");
    assert_eq!(backend.calls(), 0);
    assert_eq!(tp.source(), TextSource::Ocr);
    assert_eq!(tp.dpi(), Some(300));
    assert_eq!(tp.to_text(BlockSeparator::Newline), "");
    assert!(tp.blocks().is_empty());
    assert_eq!(
        ocr_warnings(&tp),
        ["ocr: noise-dominated raster skipped (no convincing foreground at 300 or 150 dpi)"]
    );
    assert_eq!(
        tp.warnings()
            .iter()
            .filter(|w| w.kind == WarningKind::OcrNoiseSkipped)
            .count(),
        1
    );
}

#[test]
fn the_opt_out_sends_the_raster_as_rendered() {
    let backend = ScriptedBackend::new();
    let cfg = OcrConfig {
        noise_precheck: false,
        ..OcrConfig::default()
    };
    let e = engine(&backend, cfg);
    for pdf in [common::pure_noise_pdf(), common::sparse_noise_page()] {
        let tp = ocr(&e, pdf, Some(300)).expect("ocr");
        assert_eq!(backend.last_image(), Some((2480, 3509)));
        assert_eq!(backend.last_request().expect("called").dpi, 300);
        assert_eq!(tp.dpi(), Some(300));
        assert!(ocr_warnings(&tp).is_empty(), "{:?}", tp.warnings());
    }
    assert_eq!(backend.calls(), 2);
}

#[test]
fn below_the_resolution_floor_nothing_is_screened() {
    let backend = ScriptedBackend::new();
    let e = engine(&backend, OcrConfig::default());
    // 100 dpi: A4 is ceil(595 x 100 / 72) x ceil(842 x 100 / 72).
    let tp = ocr(&e, common::pure_noise_pdf(), Some(100)).expect("ocr");
    assert_eq!(backend.calls(), 1);
    assert_eq!(backend.last_image(), Some((827, 1170)));
    assert_eq!(backend.last_request().expect("called").dpi, 100);
    assert_eq!(tp.dpi(), Some(100));
    assert!(ocr_warnings(&tp).is_empty());
    // 144 is the floor itself: screened.
    let tp = ocr(&e, common::pure_noise_pdf(), Some(144)).expect("ocr");
    assert_eq!(backend.calls(), 1, "skipped at 144 dpi");
    assert_eq!(tp.dpi(), Some(144));
    assert_eq!(
        ocr_warnings(&tp),
        ["ocr: noise-dominated raster skipped (no convincing foreground at 144 or 72 dpi)"]
    );
}

#[test]
fn clean_pages_go_through_unchanged() {
    let backend = ScriptedBackend::new();
    let e = engine(&backend, OcrConfig::default());
    let tp = ocr(&e, common::corpus_d("sparse_below_min_chars"), Some(300)).expect("ocr");
    assert_eq!(backend.last_image(), Some((2480, 3509)));
    assert_eq!(tp.dpi(), Some(300));
    assert!(ocr_warnings(&tp).is_empty());
    let tp = ocr(&e, common::blank_page(), Some(300)).expect("ocr");
    assert_eq!(backend.last_image(), Some((2550, 3300)));
    assert!(ocr_warnings(&tp).is_empty());
}

// A 1 ms budget is spent by the render, so this exercises the render-side
// deadline check and never reaches the pre-check. The post-screen check is
// unit-tested in `engine_tests.rs` (`screen_within`).
#[test]
fn a_spent_budget_skips_the_pre_check_and_the_backend() {
    let backend = ScriptedBackend::new();
    let cfg = OcrConfig {
        page_timeout: Duration::from_millis(1),
        ..OcrConfig::default()
    };
    let e = engine(&backend, cfg);
    for pdf in [common::pure_noise_pdf(), common::sparse_noise_page()] {
        match ocr(&e, pdf, Some(300)) {
            Err(OcrError::Page(OcrPageError::Timeout(t))) => {
                assert_eq!(t, Duration::from_millis(1));
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
    }
    assert_eq!(backend.calls(), 0);
}

#[test]
fn a_downsampled_page_through_the_pool_and_a_backend_error() {
    // The production pool path (`with_pool`) behaves the same, and a
    // backend failure on the half-resolution raster is the page's error,
    // with no warning to carry.
    let backend = ScriptedBackend::new();
    backend.push(Step::recognition("scripted"));
    let e = OcrEngine::with_pool(Arc::new(backend.clone()), OcrConfig::default());
    let err = ocr(&e, common::sparse_noise_page(), Some(300)).expect_err("fails");
    assert!(matches!(err, OcrError::Page(OcrPageError::Recognition(_))));
    assert_eq!(backend.last_image(), Some((1240, 1754)));
    let tp = ocr(&e, common::sparse_noise_page(), Some(300)).expect("ocr");
    assert_eq!(tp.dpi(), Some(150));
    assert_eq!(ocr_warnings(&tp).len(), 1);
}
