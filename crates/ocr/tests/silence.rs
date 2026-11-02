//! Nothing reaches fds 1 or 2 (spec 0002 O14, O40; §4.8 "Silence").
//!
//! The parent run re-executes this binary as a child with stdout and stderr
//! captured; the child builds engines that fail (missing directory, a
//! 0-byte model Tesseract cannot load — "Error opening data file" /
//! "Failed loading language" otherwise), one that succeeds, and recognises
//! a page, a page the noise pre-check downsamples and one it skips (run
//! 0003 §4.2); then, with process isolation, a helper whose `Init` fails, a
//! helper that aborts mid-page and its replacement (O40: the helper's
//! stderr is the null device, a crash and a restart print nothing), and the
//! downsampled page again. It must write nothing. `harness = false` so
//! libtest's banner is not mistaken for library output.

mod common;

use std::process::{Command, ExitCode};

use pdf_extract_core::BlockSeparator;
use pdf_extract_ocr::isolation::client::set_helper_test_env;
use pdf_extract_ocr::{OcrConfig, OcrEngine, OcrOptions};

const CHILD_ENV: &str = "PDF_EXTRACT_OCR_SILENCE_CHILD";

fn child() {
    let nowhere = common::nowhere();
    let missing = OcrConfig {
        tessdata: Some(common::scratch("silence-missing").join("absent")),
        ..OcrConfig::default()
    };
    assert!(OcrEngine::tesseract_with_candidates(missing, &nowhere).is_err());
    let empty = common::scratch("silence-empty");
    std::fs::write(empty.join("eng.traineddata"), b"").expect("write");
    let broken = OcrConfig {
        tessdata: Some(empty),
        ..OcrConfig::default()
    };
    assert!(OcrEngine::tesseract_with_candidates(broken, &nowhere).is_err());

    let engine = OcrEngine::tesseract_with_candidates(common::config(), &nowhere).expect("engine");
    let doc = common::doc(common::body_page());
    let text = engine
        .ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(150) })
        .expect("ocr")
        .to_text(BlockSeparator::Newline);
    assert!(text.contains("quick"));
    let blank = common::doc(common::blank_page());
    engine
        .ocr(&blank.page(0).expect("page"), &OcrOptions::default())
        .expect("blank");
    let speckled = common::doc(common::sparse_noise_page());
    let at300 = OcrOptions { dpi: Some(300) };
    let halved = engine
        .ocr(&speckled.page(0).expect("page"), &at300)
        .expect("downsampled");
    assert_eq!(halved.dpi(), Some(150));
    let noise = common::doc(common::pure_noise_pdf());
    let skipped = engine
        .ocr(&noise.page(0).expect("page"), &at300)
        .expect("skipped");
    assert_eq!(skipped.to_text(BlockSeparator::Newline), "");

    let empty = common::scratch("silence-process-empty");
    std::fs::write(empty.join("eng.traineddata"), b"").expect("write");
    let broken = OcrConfig {
        tessdata: Some(empty),
        ..common::process_config()
    };
    assert!(OcrEngine::tesseract_with_candidates(broken, &nowhere).is_err());
    set_helper_test_env(&[
        ("PDF_EXTRACT_OCR_HELPER_FAULT", "abort-on-recognize"),
        ("PDF_EXTRACT_OCR_HELPER_FAULT_ARMED", "1"),
    ]);
    let process =
        OcrEngine::tesseract_with_candidates(common::process_config(), &nowhere).expect("engine");
    let page = doc.page(0).expect("page");
    let err = process
        .ocr(&page, &OcrOptions { dpi: Some(150) })
        .expect_err("aborted");
    assert!(err.to_string().contains("SIGABRT"));
    set_helper_test_env(&[]);
    process
        .ocr(&page, &OcrOptions { dpi: Some(150) })
        .expect("restarted helper");
    let halved = process
        .ocr(&speckled.page(0).expect("page"), &at300)
        .expect("downsampled in the parent");
    assert_eq!(halved.dpi(), Some(150));
}

fn main() -> ExitCode {
    if std::env::var_os(CHILD_ENV).is_some() {
        child();
        return ExitCode::SUCCESS;
    }
    if std::env::args().any(|a| a == "--list") {
        return ExitCode::SUCCESS;
    }
    let out = Command::new(std::env::current_exe().expect("own path"))
        .env(CHILD_ENV, "1")
        .output()
        .expect("run child");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() || !stdout.is_empty() || !stderr.is_empty() {
        eprintln!(
            "silence: FAILED (status {})\n--- child stdout ---\n{stdout}\n--- child stderr ---\n{stderr}",
            out.status
        );
        return ExitCode::FAILURE;
    }
    println!("silence: child wrote nothing to fd 1 or fd 2: ok");
    ExitCode::SUCCESS
}
