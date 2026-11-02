//! `TesseractBackend` used directly as an `OcrBackend` (spec 0002 O33):
//! the error mapping of a raster Tesseract refuses, an expired deadline,
//! and a replacement instance that cannot be created.

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use pdf_extract_core::{BlockSeparator, OcrPageError};
use pdf_extract_ocr::{
    GrayImage, OcrBackend, OcrConfig, OcrEngine, OcrOptions, OcrRequest, PageSegMode,
    TesseractBackend,
};

fn request(deadline: Instant) -> OcrRequest {
    OcrRequest {
        dpi: 300,
        deadline,
        psm: PageSegMode::Auto,
    }
}

fn soon() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

#[test]
fn refused_rasters_and_expired_deadlines_are_page_errors() {
    let backend = TesseractBackend::new(&common::config()).expect("backend");
    let empty = GrayImage {
        width: 0,
        height: 0,
        data: Vec::new(),
    };
    let err = backend
        .recognize(&empty, &request(soon()))
        .expect_err("0 x 0");
    assert!(matches!(err, OcrPageError::Recognition(_)), "{err:?}");
    let white = GrayImage {
        width: 8,
        height: 8,
        data: vec![255; 64],
    };
    let err = backend
        .recognize(&white, &request(Instant::now()))
        .expect_err("already expired");
    assert!(matches!(err, OcrPageError::Timeout(_)), "{err:?}");
    let out = backend.recognize(&white, &request(soon())).expect("white");
    assert!(out.blocks.is_empty());
    assert_eq!(backend.info().name, "tesseract");
}

#[cfg(unix)]
#[test]
fn a_replacement_that_cannot_load_its_model_is_reported() {
    let dir = common::scratch("vanishing-model");
    common::link_model(&dir);
    let cfg = OcrConfig {
        tessdata: Some(dir.clone()),
        max_instances: 1,
        ..OcrConfig::default()
    };
    let backend = Arc::new(TesseractBackend::new(&cfg).expect("backend"));
    let empty = GrayImage {
        width: 0,
        height: 0,
        data: Vec::new(),
    };
    // The failure drops the only instance; then the model goes away.
    assert!(backend.recognize(&empty, &request(soon())).is_err());
    std::fs::remove_file(dir.join("eng.traineddata")).expect("rm model");
    let white = GrayImage {
        width: 8,
        height: 8,
        data: vec![255; 64],
    };
    let err = backend
        .recognize(&white, &request(soon()))
        .expect_err("no model");
    match err {
        OcrPageError::Recognition(m) => assert!(m.contains("OCR unavailable"), "{m}"),
        other => panic!("expected Recognition, got {other:?}"),
    }
    // Through an engine the same failure is `OcrError::Unavailable`.
    let engine = OcrEngine::with_backend(backend, OcrConfig::default());
    let doc = common::doc(common::blank_page());
    let text = engine
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(36) })
        .map(|tp| tp.to_text(BlockSeparator::Newline));
    assert!(text.is_err());
}
