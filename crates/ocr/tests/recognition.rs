//! Recognition with the real Tesseract (spec 0002 O16–O19; §4.8
//! "Recognition").

mod common;

use std::sync::Arc;
use std::time::Duration;

use pdf_extract_core::{BlockSeparator, LimitKind, Limits, OcrError, OcrPageError, TextSource};
use pdf_extract_ocr::{OcrConfig, OcrEngine, OcrOptions, PageOcr};
use pdf_extract_testkit::ocr::ScriptedBackend;

fn engine() -> OcrEngine {
    OcrEngine::tesseract_with_candidates(common::config(), &common::nowhere()).expect("engine")
}

/// Corpus D's `clean_body_11pt` (an oracle-rasterised A4 scan of 40
/// lines of 11 pt text, spec 0002 Appendix C).
#[test]
fn a_clean_body_page_is_recognised_at_300_dpi() {
    let doc = common::doc(common::corpus_d("clean_body_11pt"));
    let page = doc.page(0).expect("page 0");
    let tp = page.ocr(&engine(), &OcrOptions::default()).expect("ocr");
    assert_eq!(tp.source(), TextSource::Ocr);
    assert_eq!(tp.dpi(), Some(300));
    let text = tp.to_text(BlockSeparator::Newline);
    let truth = common::corpus_d_truth("clean_body_11pt");
    let rate = common::cer(&text, &truth);
    assert!(rate < 0.02, "CER {rate:.4}\n--- got ---\n{text}");
    assert!(text.ends_with('\n'));
    assert!(!text.contains("\n\n"), "{text:?}");
    for line in text.lines() {
        assert!(!line.is_empty());
    }
    // The boxes are in page points: every line lies on the A4 page, and
    // the first line's baseline is near where the test kit put it
    // (72 pt from the top of an 842 pt page).
    let lines: Vec<_> = tp.blocks().iter().flat_map(|b| b.lines()).collect();
    assert!(!lines.is_empty());
    for l in &lines {
        let [x0, y0, x1, y1] = l.bbox();
        assert!(
            x0 >= 0.0 && x1 <= 596.0 && y0 >= 0.0 && y1 <= 842.0,
            "{:?}",
            l.bbox()
        );
    }
    let [_, y0, _, y1] = lines[0].bbox();
    assert!(
        (y0..=y1 + 4.0).contains(&(842.0 - 72.0)),
        "{:?}",
        lines[0].bbox()
    );
}

#[test]
fn a_blank_page_gives_empty_text() {
    let doc = common::doc(common::blank_page());
    let tp = engine()
        .ocr(&doc.page(0).expect("page"), &OcrOptions::default())
        .expect("ocr");
    assert_eq!(tp.to_text(BlockSeparator::Newline), "");
    assert_eq!(tp.source(), TextSource::Ocr);
}

#[test]
fn the_render_clamp_lowers_the_reported_dpi() {
    let cfg = OcrConfig {
        max_render_pixels: 100_000,
        ..common::config()
    };
    let engine = OcrEngine::tesseract_with_candidates(cfg, &common::nowhere()).expect("engine");
    let doc = common::doc(common::body_page());
    let tp = engine
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(72) })
        .expect("ocr");
    // A4 at 72 dpi is 595 x 842 = 501,000 px; 100,000 px allows 32 dpi.
    let dpi = tp.dpi().expect("OCR page has a dpi");
    assert!(dpi < 72, "{dpi}");
    assert_eq!(dpi, 32);
}

#[test]
fn a_render_over_the_operator_budget_is_a_limit_and_is_not_recognised() {
    let limits = Limits {
        max_ops_per_page: 1_000,
        ..Limits::default()
    };
    let doc = common::open(
        pdf_extract_testkit::hostile::operator_flood(100_000),
        limits,
    );
    let scripted = ScriptedBackend::new();
    let engine = OcrEngine::with_backend(Arc::new(scripted.clone()), OcrConfig::default());
    let err = engine
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(36) })
        .expect_err("over budget");
    assert!(
        matches!(
            err,
            OcrError::Page(OcrPageError::Limit {
                kind: LimitKind::OpsPerPage,
                value: 1_000
            })
        ),
        "{err:?}"
    );
    assert_eq!(scripted.calls(), 0, "the backend must not be called");
}

#[test]
fn a_one_millisecond_budget_times_out() {
    let cfg = OcrConfig {
        page_timeout: Duration::from_millis(1),
        ..common::config()
    };
    let engine = OcrEngine::tesseract_with_candidates(cfg, &common::nowhere()).expect("engine");
    let doc = common::doc(common::dense_page());
    let page = doc.page(0).expect("page");
    let err = engine.ocr(&page, &OcrOptions::default()).expect_err("1 ms");
    assert!(
        matches!(err, OcrError::Page(OcrPageError::Timeout(d)) if d == Duration::from_millis(1)),
        "{err:?}"
    );
    // The timed-out instance is dropped (O10) and replaced on demand; a
    // page that runs out of time again still fails cleanly.
    let err = engine
        .ocr(&page, &OcrOptions::default())
        .expect_err("again");
    assert!(matches!(err, OcrError::Page(OcrPageError::Timeout(_))));
}

#[test]
fn a_closed_document_is_closed() {
    let doc = common::doc(common::blank_page());
    let page = doc.page(0).expect("page");
    doc.close();
    let err = engine()
        .ocr(&page, &OcrOptions::default())
        .expect_err("closed");
    assert!(matches!(err, OcrError::Closed), "{err:?}");
}

#[test]
fn instances_are_reused_across_pages() {
    let engine = engine();
    let doc = common::doc(common::blank_page());
    let page = doc.page(0).expect("page");
    for _ in 0..3 {
        engine.ocr(&page, &OcrOptions::default()).expect("ocr");
    }
    assert_eq!(engine.pool_len(), Some(1));
}

#[test]
fn concurrent_pages_share_a_bounded_pool() {
    let cfg = OcrConfig {
        max_instances: 2,
        ..common::config()
    };
    let engine = OcrEngine::tesseract_with_candidates(cfg, &common::nowhere()).expect("engine");
    let doc = common::doc(common::body_page());
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                let page = doc.page(0).expect("page");
                let tp = engine
                    .ocr(&page, &OcrOptions { dpi: Some(150) })
                    .expect("ocr");
                assert!(tp.to_text(BlockSeparator::Newline).contains("quick"));
            });
        }
    });
    let n = engine.pool_len().expect("pooled");
    assert!((1..=2).contains(&n), "{n}");
}

/// A budget that survives rendering but not recognition must end in
/// Tesseract's own deadline monitor (`tesseract.rs`), not in the engine's
/// after-render clock check, which the 1 ms test above reaches.
///
/// The two are told apart by the pool: the clock check fails before any
/// checkout, so the instance stays pooled; a monitor timeout is a failed
/// recognition, so the instance is dropped (O10). Each attempt measures the
/// render S (the engine over a scripted backend: render and pre-check) and
/// the whole page F with Tesseract, both as medians taken just before it,
/// and sets the budget halfway through recognition, at S + (F − S)/2: the
/// attempt still stops in the monitor if its own render runs up to
/// (F − S)/2 late, or its recognition up to twice as fast. Timings on
/// shared runners and under coverage instrumentation (with the other tests
/// of this binary running alongside) drift past even that, so a clock-check
/// timeout or a success only makes that attempt inconclusive, and the next
/// one measures afresh (internal issue #64). With a monitor that never fires every
/// attempt succeeds, so the test fails.
#[test]
fn a_budget_that_ends_during_recognition_is_caught_by_tesseracts_monitor() {
    let doc = common::doc(common::dense_page());
    let page = doc.page(0).expect("page");
    let opts = OcrOptions { dpi: Some(300) };
    let scripted = OcrEngine::with_backend(Arc::new(ScriptedBackend::new()), OcrConfig::default());

    // Every attempt inconclusive: a success each time (no monitor stop) or a
    // kept instance each time (a broken O10).
    let suspect = "Tesseract's monitor never stopped recognition, or a timed-out instance was kept";
    common::timing::retry("monitor timeout", suspect, 8, || {
        let render = common::timing::measure(5, || {
            scripted.ocr(&page, &opts).expect("scripted ocr");
        })
        .median;
        // A fresh engine per sample, as the attempt itself uses one; built
        // outside the timed runs.
        let engines: Vec<_> = (0..3).map(|_| engine()).collect();
        let mut fresh = engines.iter();
        let full = common::timing::measure(3, || {
            let e = fresh.next().expect("one engine per sample");
            e.ocr(&page, &opts).expect("full ocr");
        })
        .median;
        let recognition = full.saturating_sub(render);
        if recognition <= Duration::from_millis(10) {
            return Err(format!(
                "render {render:?}, full {full:?}: recognition of the dense page is too quick \
                 to bound"
            ));
        }
        let budget = render + recognition / 2;
        let cfg = OcrConfig {
            page_timeout: budget,
            max_instances: 1,
            ..common::config()
        };
        let e = OcrEngine::tesseract_with_candidates(cfg, &common::nowhere()).expect("engine");
        let tried = format!("render {render:?}, full {full:?}, budget {budget:?}");
        match e.ocr(&page, &opts) {
            Ok(_) => return Err(format!("{tried}: success")),
            Err(OcrError::Page(OcrPageError::Timeout(d))) => assert_eq!(d, budget),
            Err(other) => panic!("unexpected error {other:?}"),
        }
        if e.pool_len() == Some(1) {
            // Indistinguishable by time from a monitor timeout that kept
            // its instance (a broken O10), so the note names both.
            return Err(format!(
                "{tried}: clock check (or a timed-out instance kept)"
            ));
        }
        assert_eq!(e.pool_len(), Some(0), "the timed-out instance is dropped");
        let blank = common::doc(common::blank_page());
        e.ocr(&blank.page(0).expect("page"), &OcrOptions::default())
            .expect("the next page rebuilds the pool");
        assert_eq!(e.pool_len(), Some(1));
        Ok(())
    });
}
