//! `pdf-extract-ocr-helper`: the child process behind
//! `OcrEngine` with `Isolation::Process` (spec 0002 §1.5, §3.5). It speaks
//! the frame protocol on stdin/stdout and runs one Tesseract instance; all
//! of its logic lives in `pdf_extract_ocr::isolation::helper` so the
//! library's tests and this binary share it.

#![forbid(unsafe_code)]

use std::process::ExitCode;

#[cfg(unix)]
fn main() -> ExitCode {
    ExitCode::from(pdf_extract_ocr::isolation::helper::main())
}

/// Process isolation is Unix-only; elsewhere the library never spawns this.
#[cfg(not(unix))]
fn main() -> ExitCode {
    ExitCode::from(2)
}
