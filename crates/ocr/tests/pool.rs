//! The production pool driven through an engine with the test kit's
//! `ScriptedBackend` (spec 0002 O10; §4.8 "Pool"): the concurrency bound,
//! drop-on-error and clear-per-page, without Tesseract.

mod common;

use std::sync::Arc;
use std::time::Duration;

use pdf_extract_core::{OcrError, OcrPageError, UnavailableReason};
use pdf_extract_ocr::{OcrConfig, OcrEngine, OcrOptions};
use pdf_extract_testkit::ocr::{ScriptedBackend, Step, output};

fn pooled(max_instances: usize, backend: &ScriptedBackend) -> OcrEngine {
    OcrEngine::with_pool(
        Arc::new(backend.clone()),
        OcrConfig {
            max_instances,
            ..OcrConfig::default()
        },
    )
}

const LOW: OcrOptions = OcrOptions { dpi: Some(36) };

#[test]
fn at_most_max_instances_recognise_at_once_and_all_complete() {
    let backend = ScriptedBackend::with_default(Step::Sleep(
        Duration::from_millis(150),
        Box::new(Step::Output(output(&[&[("ok", [1, 1, 5, 5])]]))),
    ));
    let engine = pooled(2, &backend);
    let doc = common::doc(common::blank_page());
    let done = std::thread::scope(|s| {
        let handles: Vec<_> = (0..4)
            .map(|_| s.spawn(|| engine.ocr(&doc.page(0).expect("page"), &LOW).expect("ocr")))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect::<Vec<_>>()
            .len()
    });
    assert_eq!(done, 4);
    assert_eq!(backend.calls(), 4);
    assert_eq!(backend.max_in_flight(), 2, "the pool bounds concurrency");
    assert_eq!(backend.creates(), 2);
    assert_eq!(engine.pool_len(), Some(2));
    assert_eq!(backend.clears(), 4, "one Clear per page");
}

#[test]
fn an_error_drops_the_instance_and_the_next_page_rebuilds_it() {
    let backend = ScriptedBackend::new();
    let engine = pooled(1, &backend);
    let doc = common::doc(common::blank_page());
    let page = doc.page(0).expect("page");
    engine.ocr(&page, &LOW).expect("first page");
    assert_eq!(engine.pool_len(), Some(1));
    assert_eq!(backend.clears(), 1);

    backend.push(Step::recognition("engine fault"));
    let err = engine.ocr(&page, &LOW).expect_err("scripted failure");
    assert!(matches!(err, OcrError::Page(OcrPageError::Recognition(ref m)) if m == "engine fault"));
    assert_eq!(engine.pool_len(), Some(0), "the failed instance is dropped");
    assert_eq!(backend.drops(), 1);
    assert_eq!(backend.clears(), 1, "a failed instance is not cleared");

    engine.ocr(&page, &LOW).expect("rebuilt");
    assert_eq!(engine.pool_len(), Some(1));
    assert_eq!(backend.creates(), 2);
}

#[test]
fn a_timeout_or_panic_drops_the_instance_too() {
    let backend = ScriptedBackend::new();
    let engine = pooled(1, &backend);
    let doc = common::doc(common::blank_page());
    let page = doc.page(0).expect("page");
    backend.push(Step::timeout());
    assert!(matches!(
        engine.ocr(&page, &LOW),
        Err(OcrError::Page(OcrPageError::Timeout(_)))
    ));
    assert_eq!(engine.pool_len(), Some(0));
    backend.push(Step::Panic("boom".into()));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| engine.ocr(&page, &LOW)));
    assert!(r.is_err(), "a backend panic propagates to Rust callers");
    assert_eq!(engine.pool_len(), Some(0));
    assert_eq!(backend.drops(), 2);
    engine.ocr(&page, &LOW).expect("the pool still works");
}

#[test]
fn a_replacement_that_cannot_be_created_is_unavailable() {
    let backend = ScriptedBackend::new();
    let engine = pooled(1, &backend);
    let doc = common::doc(common::blank_page());
    let page = doc.page(0).expect("page");
    backend.fail_next_creates(1);
    let err = engine.ocr(&page, &LOW).expect_err("cannot create");
    match err {
        OcrError::Unavailable(u) => assert_eq!(u.reason, UnavailableReason::InitFailed),
        other => panic!("expected Unavailable, got {other:?}"),
    }
    assert_eq!(engine.pool_len(), Some(0));
    engine.ocr(&page, &LOW).expect("next creation succeeds");
}

#[test]
fn info_and_tessdata_come_from_the_factory() {
    let engine = pooled(1, &ScriptedBackend::new());
    assert_eq!(engine.info().name, "scripted");
    assert_eq!(engine.tessdata(), None);
}
