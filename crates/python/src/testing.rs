//! `PdfBuilder`, `PageBuilder` and `text_width`: the Python test kit (spec
//! §1.7 T49, §3.5 `pdf_extract.testing`, design spec §12.3).
//!
//! These are thin wrappers over `pdf-extract-testkit`, so a Python fixture
//! and a Rust fixture built with the same calls are the same bytes (the
//! golden test in `tests/python/test_testing.py` checks that). They are
//! registered on `_native` and re-exported by `pdf_extract/testing.py`;
//! `import pdf_extract` does not import that module.
//!
//! **Numbers.** The Rust kit works in `f32`; Python floats are narrowed by
//! PyO3's `f32` conversion (`f64 as f32`, round to nearest; out-of-range
//! values become infinite). A coordinate or size that is not finite after
//! narrowing raises `ValueError`, as the Rust kit's `BuildError::NonFinite`
//! does. Page sizes get the same check here, because the Rust kit only
//! asserts them when serialising (a panic is the wrong answer for a
//! caller's typo).
//!
//! **Errors.** Every `BuildError` (character outside WinAnsi, render mode
//! outside 0-7, non-finite number) is a `ValueError` with the kit's message;
//! `save` raises the `OSError` subclass `builtins.open` would.
//!
//! **Pages.** The Rust `PdfBuilder::add_page` hands out a `&mut PageBuilder`
//! borrowed from the document, which a Python object cannot hold. A Python
//! `PageBuilder` is instead (owning document, page index) and reaches its
//! page through `PdfBuilder::page_mut` under the document's mutex.
//!
//! **Locking.** The document sits behind a `Mutex` so the classes can be
//! `frozen` (shared between threads without PyO3 borrow flags). Every method
//! that takes the lock does so inside `py.detach`: a thread blocked on the
//! mutex then never holds the GIL (or blocks a free-threaded stop-the-world
//! pause), and no Python code ever runs while the lock is held, so the lock
//! cannot take part in a deadlock with the interpreter. Serialisation
//! (`to_bytes`, `save`) and `text_width` are the work T43 names; the other
//! methods are short, but take the lock and so detach as well.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use pdf_extract_testkit as testkit;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

use crate::errors::os_error;
use crate::guard::guarded;

/// A `BuildError` as the `ValueError` the spec promises (T49).
fn build_error(err: &testkit::BuildError) -> PyErr {
    PyValueError::new_err(err.to_string())
}

/// Builds a PDF document page by page; see `pdf_extract.testing`.
#[pyclass(module = "pdf_extract.testing", name = "PdfBuilder", frozen)]
pub(crate) struct PdfBuilder {
    inner: Mutex<testkit::PdfBuilder>,
}

impl PdfBuilder {
    fn lock(&self) -> MutexGuard<'_, testkit::PdfBuilder> {
        // A panic while the lock was held (only possible through a bug in
        // the test kit, caught by `guarded`) leaves at worst a half-written
        // page; the builder itself stays valid, so poisoning is ignored.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[pymethods]
impl PdfBuilder {
    /// An empty document (no pages).
    #[new]
    fn new() -> PyResult<Self> {
        guarded(|| {
            Ok(Self {
                inner: Mutex::new(testkit::PdfBuilder::new()),
            })
        })
    }

    /// Appends a page of `width` x `height` points (default A4) and returns
    /// its builder.
    #[pyo3(signature = (width = 595.0, height = 842.0))]
    fn new_page(slf: &Bound<'_, Self>, width: f32, height: f32) -> PyResult<PageBuilder> {
        guarded(|| {
            if !(width.is_finite() && height.is_finite()) {
                return Err(PyValueError::new_err(format!(
                    "page size must be finite, got {width} x {height}"
                )));
            }
            let this = slf.get();
            let index = slf.py().detach(|| {
                let mut doc = this.lock();
                doc.add_page(width, height);
                doc.page_count() - 1
            });
            Ok(PageBuilder {
                doc: slf.clone().unbind(),
                index,
            })
        })
    }

    /// The document as PDF bytes. The same calls always give the same bytes.
    fn to_bytes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyBytes>> {
        guarded(|| {
            let bytes = py.detach(|| self.lock().to_bytes());
            Ok(PyBytes::new(py, &bytes))
        })
    }

    /// Writes `to_bytes()` to `path`.
    fn save(&self, py: Python<'_>, path: PathBuf) -> PyResult<()> {
        guarded(|| {
            py.detach(|| {
                let bytes = self.lock().to_bytes();
                std::fs::write(&path, bytes)
            })
            .map_err(|e| os_error(e, Some(&path)))
        })
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        guarded(|| {
            let pages = py.detach(|| self.lock().page_count());
            Ok(format!("<pdf_extract.testing.PdfBuilder pages={pages}>"))
        })
    }
}

/// One page of a `PdfBuilder`, returned by `PdfBuilder.new_page`.
#[pyclass(module = "pdf_extract.testing", name = "PageBuilder", frozen)]
pub(crate) struct PageBuilder {
    doc: Py<PdfBuilder>,
    index: usize,
}

#[pymethods]
impl PageBuilder {
    /// Draws `text` in Helvetica (WinAnsiEncoding, not embedded) with the
    /// baseline origin of its first glyph at `point`, measured from the
    /// page's top-left corner with y growing downwards. Raises `ValueError`
    /// for characters outside WinAnsi, a render mode outside 0-7 or a
    /// non-finite number; nothing is drawn then.
    #[pyo3(signature = (point, text, *, fontsize = 11.0, render_mode = 0))]
    fn insert_text(
        &self,
        py: Python<'_>,
        point: (f32, f32),
        text: &str,
        fontsize: f32,
        render_mode: i128,
    ) -> PyResult<()> {
        guarded(|| {
            // The kit's own check rejects 8..=255; anything that does not
            // even fit its u8 gets the same message.
            let render_mode = u8::try_from(render_mode).map_err(|_| {
                PyValueError::new_err(format!("text rendering mode {render_mode} is outside 0-7"))
            })?;
            let style = testkit::TextStyle {
                fontsize,
                render_mode,
                ..testkit::TextStyle::default()
            };
            let doc = self.doc.get();
            let index = self.index;
            py.detach(|| {
                let mut guard = doc.lock();
                // Pages are never removed, so the index stays valid.
                let page = guard
                    .page_mut(index)
                    .expect("a PageBuilder's page exists for the builder's lifetime");
                page.insert_text(point, text, style)
            })
            .map_err(|e| build_error(&e))
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        guarded(|| {
            Ok(format!(
                "<pdf_extract.testing.PageBuilder index={}>",
                self.index
            ))
        })
    }
}

/// Width of `text` in Helvetica at `fontsize` points, from Adobe's AFM
/// metrics (what a viewer advances by). Raises `ValueError` for characters
/// outside WinAnsi or a non-finite size.
#[pyfunction]
#[pyo3(signature = (text, fontsize = 11.0))]
pub(crate) fn text_width(py: Python<'_>, text: &str, fontsize: f32) -> PyResult<f32> {
    guarded(|| {
        py.detach(|| testkit::text_width(text, fontsize))
            .map_err(|e| build_error(&e))
    })
}
