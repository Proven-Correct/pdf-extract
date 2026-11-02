//! `Document::extract` through a real `OcrEngine` (run 0002 §4.10): the
//! engine's `PageRecognizer` impl, with `ScriptedBackend` standing in for
//! Tesseract. Core's own tests cover the policy against a fake recogniser;
//! these show the engine plugs into it with the same outcomes, including
//! `Unavailable` raised by the production pool.
//!
//! Each page has its own width, so the raster width the backend receives
//! identifies the page it was asked to recognise.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use pdf_extract_core::{
    Document, ExtractOptions, OcrPageError, OcrStatus, OpenOptions, PageSource, WarningKind,
};
use pdf_extract_ocr::{
    BackendInfo, GrayImage, OcrBackend, OcrConfig, OcrEngine, OcrOutput, OcrRequest,
};
use pdf_extract_testkit::ocr::{ScriptedBackend, Step, output};
use pdf_extract_testkit::{PdfBuilder, TextStyle};

/// Rendering resolution: low, so 40 pages render quickly.
const DPI: u32 = 36;

/// Page `i` is `width_pt(i)` points wide.
fn width_pt(i: usize) -> f32 {
    100.0 + 10.0 * i as f32
}

/// The page index whose raster is `px` pixels wide at [`DPI`].
fn page_of(px: u32) -> usize {
    (0..40)
        .find(|&i| (width_pt(i) * DPI as f32 / 72.0).ceil() as u32 == px)
        .expect("a page of that width")
}

/// 40 pages; those in `sparse` carry a short label, the rest six lines of
/// ten digits (60 code points, never sparse).
fn doc(sparse: &[usize]) -> Document {
    let mut b = PdfBuilder::new();
    for i in 0..40 {
        let page = b.add_page(width_pt(i), 300.0);
        if sparse.contains(&i) {
            page.insert_text((10.0, 40.0), &format!("p{i}"), TextStyle::default())
                .expect("label");
        } else {
            for line in 0..6 {
                let y = 40.0 + 20.0 * line as f32;
                page.insert_text((10.0, y), "0123456789", TextStyle::default())
                    .expect("body");
            }
        }
    }
    Document::from_bytes(b.to_bytes(), &OpenOptions::default()).expect("opens")
}

/// A `ScriptedBackend` that also records which page each call was for.
struct Recording {
    inner: ScriptedBackend,
    pages: Mutex<Vec<usize>>,
}

impl Recording {
    fn new(inner: ScriptedBackend) -> Arc<Self> {
        Arc::new(Self {
            inner,
            pages: Mutex::new(Vec::new()),
        })
    }

    fn pages(&self) -> Vec<usize> {
        self.pages.lock().expect("pages").clone()
    }
}

impl OcrBackend for Recording {
    fn recognize(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        self.pages.lock().expect("pages").push(page_of(img.width));
        self.inner.recognize(img, req)
    }

    fn info(&self) -> BackendInfo {
        self.inner.info()
    }
}

fn config() -> OcrConfig {
    OcrConfig {
        dpi: DPI,
        ..OcrConfig::default()
    }
}

fn found() -> Step {
    Step::Output(output(&[&[("recognised", [2, 2, 40, 12])]]))
}

fn opts(engine: &OcrEngine, max_ocr_pages: usize, workers: usize) -> ExtractOptions<'_> {
    ExtractOptions {
        ocr: Some(engine),
        max_ocr_pages,
        workers,
        ..ExtractOptions::default()
    }
}

#[test]
fn budget_selects_pages_in_page_order() {
    let d = doc(&[3, 7, 12]);
    for workers in [1, 4] {
        let rec = Recording::new(ScriptedBackend::with_default(found()));
        let engine = OcrEngine::with_backend(rec.clone(), config());
        let r = d.extract(&opts(&engine, 2, workers)).expect("extract");
        assert_eq!(r.pages[3].source, PageSource::Ocr);
        assert_eq!(r.pages[3].text, "recognised\n");
        assert_eq!(r.pages[7].source, PageSource::Ocr);
        assert_eq!(r.pages[12].source, PageSource::Text);
        assert_eq!(r.pages[12].ocr_error, None);
        assert_eq!(r.ocr_status, OcrStatus::BudgetExhausted);
        if workers == 1 {
            assert_eq!(rec.pages(), [3, 7]);
        } else {
            let set: BTreeSet<_> = rec.pages().into_iter().collect();
            assert_eq!(set, BTreeSet::from([3, 7]));
        }
        // The text pass is unaffected by OCR.
        let plain = d
            .extract(&ExtractOptions {
                workers,
                ..ExtractOptions::default()
            })
            .expect("extract");
        assert_eq!(plain.ocr_status, OcrStatus::NotNeeded);
        assert_eq!(plain.pages[0].text, r.pages[0].text);
    }
}

#[test]
fn a_recognition_error_falls_back_and_is_partial() {
    let d = doc(&[3, 7, 12]);
    let b = ScriptedBackend::new();
    b.push(found())
        .push(Step::recognition("scripted"))
        .push(found());
    let rec = Recording::new(b);
    let engine = OcrEngine::with_backend(rec.clone(), config());
    let r = d.extract(&opts(&engine, 500, 1)).expect("extract");
    assert_eq!(rec.pages(), [3, 7, 12]);
    assert_eq!(r.pages[7].source, PageSource::TextFallback);
    assert_eq!(r.pages[7].text, "p7\n");
    assert_eq!(
        r.pages[7].ocr_error.as_deref(),
        Some("recognition failed: scripted")
    );
    assert_eq!(r.pages[12].source, PageSource::Ocr);
    assert_eq!(r.ocr_status, OcrStatus::Partial);
}

#[test]
fn an_empty_result_falls_back_without_an_error() {
    let d = doc(&[3, 7, 12]);
    let b = ScriptedBackend::with_default(found());
    b.push(found()).push(Step::empty());
    let engine = OcrEngine::with_backend(Arc::new(b), config());
    let r = d.extract(&opts(&engine, 500, 1)).expect("extract");
    assert_eq!(r.pages[7].source, PageSource::TextFallback);
    assert_eq!(r.pages[7].ocr_error, None);
    assert_eq!(r.ocr_status, OcrStatus::Ok);
}

#[test]
fn a_whitespace_only_result_counts_as_empty() {
    let d = doc(&[3]);
    let b = ScriptedBackend::with_default(Step::Output(output(&[&[(" ", [2, 2, 8, 12])]])));
    let engine = OcrEngine::with_backend(Arc::new(b), config());
    let r = d.extract(&opts(&engine, 500, 1)).expect("extract");
    assert_eq!(r.pages[3].source, PageSource::TextFallback);
    assert_eq!(r.pages[3].text, "p3\n");
}

#[test]
fn a_timeout_is_a_page_failure() {
    let d = doc(&[3]);
    let engine = OcrEngine::with_backend(
        Arc::new(ScriptedBackend::with_default(Step::timeout())),
        config(),
    );
    let r = d.extract(&opts(&engine, 500, 1)).expect("extract");
    assert_eq!(r.pages[3].source, PageSource::TextFallback);
    assert!(
        r.pages[3]
            .ocr_error
            .as_deref()
            .is_some_and(|m| m.starts_with("OCR exceeded the page timeout")),
        "{:?}",
        r.pages[3].ocr_error
    );
    assert_eq!(r.ocr_status, OcrStatus::Partial);
}

/// `Unavailable` as the production pool raises it: the first instance
/// cannot be created.
#[test]
fn an_unavailable_engine_stops_all_further_ocr() {
    let d = doc(&[3, 7, 12]);
    for workers in [1, 4] {
        let b = ScriptedBackend::with_default(found());
        b.fail_next_creates(1);
        let engine = OcrEngine::with_pool(Arc::new(b.clone()), config());
        let r = d.extract(&opts(&engine, 500, workers)).expect("extract");
        assert_eq!(r.ocr_status, OcrStatus::Unavailable);
        if workers == 1 {
            assert_eq!(b.calls(), 0, "no recognition after the failure");
            assert_eq!(b.creates(), 0, "no instance creation after the failure");
            for i in [3, 7, 12] {
                assert_eq!(r.pages[i].source, PageSource::TextFallback, "page {i}");
                assert_eq!(r.pages[i].text, format!("p{i}\n"));
                let msg = r.pages[i].ocr_error.as_deref().unwrap_or_default();
                assert!(
                    msg.starts_with("OCR unavailable (init_failed)"),
                    "page {i}: {msg}"
                );
            }
        }
    }
}

/// With several workers, a recognition already in flight elsewhere may
/// finish, but none starts after the pool reported OCR unavailable.
#[test]
fn no_recognition_starts_after_the_pool_is_unavailable() {
    let d = doc(&(0..30).collect::<Vec<_>>());
    let b = ScriptedBackend::with_default(Step::Sleep(
        std::time::Duration::from_millis(200),
        Box::new(found()),
    ));
    b.fail_next_creates(1);
    let engine = OcrEngine::with_pool(Arc::new(b.clone()), config());
    let workers = 3;
    let r = d.extract(&opts(&engine, 500, workers)).expect("extract");
    assert_eq!(r.ocr_status, OcrStatus::Unavailable);
    // One worker hit the failed creation; each other one had at most one
    // page in flight.
    assert!(b.calls() < workers, "{}", b.calls());
    let ocr = r
        .pages
        .iter()
        .filter(|p| p.source == PageSource::Ocr)
        .count();
    assert_eq!(ocr, b.calls());
    for p in r.pages[..30].iter().filter(|p| p.source != PageSource::Ocr) {
        assert_eq!(p.source, PageSource::TextFallback);
        let msg = p.ocr_error.as_deref().unwrap_or_default();
        assert!(
            msg.starts_with("OCR unavailable (init_failed)"),
            "page {}: {msg}",
            p.index
        );
    }
}

#[test]
fn the_pool_bounds_concurrency_under_parallel_extract() {
    let d = doc(&(0..40).collect::<Vec<_>>());
    let b = ScriptedBackend::with_default(Step::Sleep(
        std::time::Duration::from_millis(2),
        Box::new(found()),
    ));
    let engine = OcrEngine::with_pool(
        Arc::new(b.clone()),
        OcrConfig {
            max_instances: 2,
            ..config()
        },
    );
    let r = d.extract(&opts(&engine, 500, 4)).expect("extract");
    assert_eq!(r.ocr_status, OcrStatus::Ok);
    assert!(r.pages.iter().all(|p| p.source == PageSource::Ocr));
    assert_eq!(b.calls(), 40);
    assert!(b.max_in_flight() <= 2, "{}", b.max_in_flight());
}

#[test]
fn a_backend_panic_propagates_from_extract() {
    let d = doc(&[3, 7, 12]);
    for workers in [1, 4] {
        let b = ScriptedBackend::with_default(Step::Panic("scripted backend panic".into()));
        let engine = OcrEngine::with_backend(Arc::new(b), config());
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            d.extract(&opts(&engine, 500, workers))
        }))
        .expect_err("the panic propagates");
        let msg = payload
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()));
        assert_eq!(msg.as_deref(), Some("scripted backend panic"));
    }
}

#[test]
fn page_recognizer_uses_the_engine_dpi_unless_given() {
    use pdf_extract_core::PageRecognizer;
    let d = doc(&[]);
    let b = ScriptedBackend::new();
    let engine = OcrEngine::with_backend(Arc::new(b.clone()), config());
    let page = d.page(0).expect("page");
    let tp = engine.recognize(&page, None).expect("ocr");
    assert_eq!(tp.dpi(), Some(DPI));
    let tp = engine.recognize(&page, Some(18)).expect("ocr");
    assert_eq!(tp.dpi(), Some(18));
    assert_eq!(b.calls(), 2);
}

/// Six image-only pages at the engine's default 300 dpi: 0–2 and 4 small
/// white scans, 3 the pure noise field (skipped by the noise pre-check),
/// 5 corpus D's sparse page under noise (recognised at 150 dpi).
fn noisy_doc() -> Document {
    use pdf_extract_testkit::corpus_d::{ScanPage, noise_variant_page, pure_noise_page, scan_pdf};
    let white = ScanPage {
        media_box: [0.0, 0.0, 72.0, 72.0],
        crop_box: None,
        rotate: 0,
        width: 1,
        height: 1,
        pixels: vec![255],
    };
    let clean = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/corpus-d/sparse_below_min_chars.pdf"),
    )
    .expect("committed corpus-D file");
    let pages = vec![
        white.clone(),
        white.clone(),
        white.clone(),
        pure_noise_page(),
        white,
        noise_variant_page(&clean).expect("variant page"),
    ];
    Document::from_bytes(scan_pdf(&pages), &OpenOptions::default()).expect("opens")
}

#[test]
fn noise_notes_reach_the_extraction_in_page_order() {
    let d = noisy_doc();
    for workers in [1, 4] {
        // Every recognition that happens finds a word; the skipped page
        // never reaches the backend.
        let backend = ScriptedBackend::with_default(found());
        let engine = OcrEngine::with_backend(Arc::new(backend.clone()), OcrConfig::default());
        let r = d.extract(&opts(&engine, 500, workers)).expect("extract");
        assert_eq!(backend.calls(), 5, "workers = {workers}");
        assert_eq!(r.ocr_status, OcrStatus::Ok);
        assert_eq!(r.pages[3].source, PageSource::TextFallback);
        assert_eq!(r.pages[3].ocr_error, None);
        assert_eq!(r.pages[3].text, "");
        assert_eq!(r.pages[5].source, PageSource::Ocr);
        assert_eq!(r.pages[5].text, "recognised\n");
        let notes: Vec<_> = r
            .warnings
            .iter()
            .filter(|w| {
                matches!(
                    w.kind,
                    WarningKind::OcrNoiseSkipped | WarningKind::OcrNoiseDownsampled
                )
            })
            .collect();
        assert_eq!(notes.len(), 2);
        // They end the list, skipped page first.
        assert_eq!(
            r.warnings[r.warnings.len() - 2..],
            [notes[0].clone(), notes[1].clone()]
        );
        assert_eq!(notes[0].kind, WarningKind::OcrNoiseSkipped);
        assert!(
            notes[0].detail.starts_with("page 3: "),
            "{}",
            notes[0].detail
        );
        assert_eq!(notes[1].kind, WarningKind::OcrNoiseDownsampled);
        assert_eq!(
            notes[1].to_string(),
            "page 5: ocr: noise-dominated raster recognised at 150 dpi (rendered at 300)"
        );
    }
}
