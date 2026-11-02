//! `pdf_extract._native`: the CPython binding of `pdf-extract-core`.
//!
//! The public Python API is `pdf_extract` (python/pdf_extract/__init__.py),
//! which re-exports the classes and functions defined here; the exception
//! classes live in `pdf_extract/errors.py` and are imported and cached by
//! `errors` at module initialisation. The stub `_native.pyi` is the typed
//! description of this module (spec §3.5).
//!
//! Module map:
//!
//! - `guard` is the panic boundary. Every exported body runs in
//!   `guard::guarded`, so a Rust panic surfaces as `InternalError`, never as
//!   `pyo3.PanicException` (T45), and the panic hook writes nothing to
//!   stderr (T48). Its test hooks live here: `_panic_for_test` and
//!   `_arm_codec_panic_for_test` (a panic caught at the fork's image-codec
//!   boundary instead, H18, internal #76).
//! - `errors` maps Rust error enums to Python exception classes (T47, T47b).
//! - `limits` holds `Limits`.
//! - `document` holds `open`, `open_bytes`, `Document` and its iterator.
//! - `page` holds `Page` (including `Page.ocr`) and `TextPage`.
//! - `ocr` holds `OcrEngine`, `OcrInfo`, `default_ocr_engine` and the
//!   test-only hooks `_scripted_engine`, `_scripted_stats` and
//!   `_set_tessdata_candidates` (run 0002 §3.6, §4.11).
//! - `extract` holds `extract`, `PageResult` and `Extraction`.
//! - `testing` holds the test kit (`PdfBuilder`, `PageBuilder`,
//!   `text_width`) over `pdf-extract-testkit`; Python reaches it as
//!   `pdf_extract.testing`, which `import pdf_extract` does not load.
//!
//! **Free threading.** The module declares `gil_used = false` (T43, D-24):
//! importing it on a free-threaded CPython does not re-enable the GIL. That
//! is sound because every class is `frozen` around `Send + Sync` core
//! handles and no method keeps Python state outside the objects it is
//! given. Non-constant work runs under `py.detach`.
//!
//! **Import does no work (T42).** Module initialisation only registers the
//! classes and functions and installs the panic hook; no thread is started
//! and nothing is read; the only import is `pdf_extract.errors`, which the
//! package has already imported. The OCR engine, Tesseract, helper discovery
//! and the model checksums all wait for first use (run 0002 O41).

mod document;
mod errors;
mod extract;
mod guard;
mod limits;
mod ocr;
mod page;
mod testing;

use pyo3::prelude::*;

/// Panics on purpose, for the test suite (T45): proves that a panic in Rust
/// arrives as `pdf_extract.InternalError`. With `detached=True` the panic
/// happens while the thread is detached from the interpreter, as it would
/// inside an extraction. With `resumed=True` it is raised with
/// `resume_unwind`, which skips the panic hook, as core's `run_parallel` does
/// when it re-raises a worker's panic.
#[pyfunction]
#[pyo3(signature = (*, detached = false, resumed = false))]
fn _panic_for_test(py: Python<'_>, detached: bool, resumed: bool) -> PyResult<()> {
    guard::guarded(|| {
        if resumed {
            std::panic::resume_unwind(Box::new("deliberate panic for testing (resumed)"));
        }
        if detached {
            py.detach(|| panic!("deliberate panic for testing (detached)"));
        }
        panic!("deliberate panic for testing");
    })
}

/// Makes the next image-codec decode on the calling thread panic inside the
/// fork's codec boundary (H18), for the test suite: the image is skipped with
/// an `image: decode failed` warning and no `InternalError` (internal #76). The flag
/// clears when it fires. Per thread, so the decode must run on this thread,
/// as `Page.ocr` does.
#[pyfunction]
fn _arm_codec_panic_for_test() -> PyResult<()> {
    guard::guarded(|| {
        pdf_extract_core::arm_codec_panic_for_test();
        Ok(())
    })
}

#[pymodule(gil_used = false)]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    guard::install_panic_hook();
    errors::init(m.py())?;
    m.add("__version__", pdf_extract_core::VERSION)?;
    m.add_class::<limits::Limits>()?;
    m.add_class::<document::Document>()?;
    m.add_class::<page::Page>()?;
    m.add_class::<page::TextPage>()?;
    m.add_function(wrap_pyfunction!(document::open, m)?)?;
    m.add_function(wrap_pyfunction!(document::open_bytes, m)?)?;
    m.add_class::<ocr::OcrEngine>()?;
    m.add_class::<ocr::OcrInfo>()?;
    m.add_function(wrap_pyfunction!(ocr::default_ocr_engine, m)?)?;
    m.add_function(wrap_pyfunction!(extract::extract, m)?)?;
    m.add_class::<extract::PageResult>()?;
    m.add_class::<extract::Extraction>()?;
    m.add_function(wrap_pyfunction!(ocr::_scripted_engine, m)?)?;
    m.add_function(wrap_pyfunction!(ocr::_scripted_stats, m)?)?;
    m.add_function(wrap_pyfunction!(ocr::_set_tessdata_candidates, m)?)?;
    m.add_class::<testing::PdfBuilder>()?;
    m.add_class::<testing::PageBuilder>()?;
    m.add_function(wrap_pyfunction!(testing::text_width, m)?)?;
    m.add_function(wrap_pyfunction!(_panic_for_test, m)?)?;
    m.add_function(wrap_pyfunction!(_arm_codec_panic_for_test, m)?)?;
    Ok(())
}
