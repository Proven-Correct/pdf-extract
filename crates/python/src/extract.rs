//! `extract`, `PageResult` and `Extraction` (run 0002 §3.6, O22–O25; design
//! spec §12.4): the batch helper that reads every page's text layer and
//! recognises the sparse ones.
//!
//! `extract` opens the file and runs `Document::extract` in one detached
//! section, so the text pass, the OCR pass and their worker threads (scoped,
//! joined before the call returns) all run without the GIL. A panic in a
//! worker is resumed on the calling thread after the scope and becomes
//! `InternalError` at the guard, like any other panic (O24, O39).
//!
//! The results are plain immutable values. `Extraction.pages` is built once,
//! when the result is converted, so repeated reads return the same tuple.

use std::path::PathBuf;

use pdf_extract_core as core;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::document::options;
use crate::errors::{open_error, page_error};
use crate::guard::guarded;
use crate::limits::{Limits, non_negative};
use crate::ocr::OcrEngine;
use crate::page::block_separator;

/// Why `extract` failed: opening, or the extraction itself.
enum Failure {
    Open(core::OpenError),
    Page(core::PageError),
}

/// Opens `path` and returns every page's text, recognising the sparse pages
/// with `ocr` when one is given; see the stub for the policy.
///
/// Forking: with OCR in play, do not call this from a process that forks
/// while another thread is inside `extract()`, `Page.ocr()` or the first
/// `default_ocr_engine()` (`os.fork()`, or `multiprocessing` with the `fork`
/// start method): the child can deadlock on a lock the parent's thread held.
/// Use the `spawn` or `forkserver` start method, or build the engine before
/// forking and pass it as `ocr=`; see `OcrEngine`.
#[pyfunction]
#[pyo3(signature = (
    path,
    *,
    max_pages = None,
    min_chars = 50,
    ocr = None,
    max_ocr_pages = 500,
    workers = 1,
    block_separator = "newline",
    limits = None,
))]
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors the keyword-only Python signature"
)]
pub(crate) fn extract(
    py: Python<'_>,
    path: PathBuf,
    max_pages: Option<i128>,
    min_chars: i128,
    ocr: Option<PyRef<'_, OcrEngine>>,
    max_ocr_pages: i128,
    workers: i128,
    block_separator: &str,
    limits: Option<PyRef<'_, Limits>>,
) -> PyResult<Extraction> {
    guarded(|| {
        let open = options(max_pages, limits.as_deref())?;
        let separator = self::block_separator(block_separator)?;
        let min_chars: usize = non_negative("min_chars", min_chars)?;
        let max_ocr_pages: usize = non_negative("max_ocr_pages", max_ocr_pages)?;
        if workers < 1 {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "workers must be at least 1, got {workers}"
            )));
        }
        let workers: usize = non_negative("workers", workers)?;
        let engine = ocr.map(|e| e.inner.clone());
        let result = py.detach(|| {
            let doc = core::Document::open(&path, &open).map_err(Failure::Open)?;
            let opts = core::ExtractOptions {
                min_chars,
                ocr: engine.as_ref().map(|e| e as &dyn core::PageRecognizer),
                max_ocr_pages,
                workers,
                block_separator: separator,
                ..core::ExtractOptions::default()
            };
            let result = doc.extract(&opts).map_err(Failure::Page);
            // Free the document now rather than when the last reference
            // goes (there is none outside this call).
            doc.close();
            result
        });
        match result {
            Ok(r) => Extraction::new(py, r),
            Err(Failure::Open(e)) => Err(open_error(py, e, Some(&path))),
            Err(Failure::Page(e)) => Err(page_error(py, e)),
        }
    })
}

/// The outcome for one page of `extract`.
#[pyclass(module = "pdf_extract", name = "PageResult", frozen)]
pub(crate) struct PageResult {
    inner: core::PageResult,
}

#[pymethods]
impl PageResult {
    /// The page's 0-based index.
    #[getter]
    fn index(&self) -> PyResult<usize> {
        guarded(|| Ok(self.inner.index))
    }

    /// The page's text (from `source`).
    #[getter]
    fn text(&self) -> PyResult<String> {
        guarded(|| Ok(self.inner.text.clone()))
    }

    /// `"text"`, `"ocr"` or `"text_fallback"` (OCR was tried and did not
    /// replace the text layer).
    #[getter]
    fn source(&self) -> PyResult<&'static str> {
        guarded(|| Ok(source_name(self.inner.source)))
    }

    /// Why OCR did not replace the text layer, when it was tried.
    #[getter]
    fn ocr_error(&self) -> PyResult<Option<String>> {
        guarded(|| Ok(self.inner.ocr_error.clone()))
    }

    fn __repr__(&self) -> PyResult<String> {
        guarded(|| {
            Ok(format!(
                "<pdf_extract.PageResult index={} source='{}' chars={}>",
                self.inner.index,
                source_name(self.inner.source),
                self.inner.text.chars().count()
            ))
        })
    }
}

/// The result of `extract`: one `PageResult` per page, in page order.
#[pyclass(module = "pdf_extract", name = "Extraction", frozen)]
pub(crate) struct Extraction {
    page_count: usize,
    pages: Py<PyTuple>,
    ocr_status: core::OcrStatus,
    warnings: Py<PyTuple>,
}

impl Extraction {
    fn new(py: Python<'_>, r: core::Extraction) -> PyResult<Self> {
        let pages = r
            .pages
            .into_iter()
            .map(|inner| Py::new(py, PageResult { inner }))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(Self {
            page_count: r.page_count,
            pages: PyTuple::new(py, pages)?.unbind(),
            ocr_status: r.ocr_status,
            warnings: PyTuple::new(py, r.warnings.iter().map(ToString::to_string))?.unbind(),
        })
    }
}

#[pymethods]
impl Extraction {
    /// The document's page count (also `len(pages)`).
    #[getter]
    fn page_count(&self) -> PyResult<usize> {
        guarded(|| Ok(self.page_count))
    }

    /// One `PageResult` per page, in page order.
    #[getter]
    fn pages<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        guarded(|| Ok(self.pages.bind(py).clone()))
    }

    /// `"not_needed"`, `"ok"`, `"unavailable"`, `"partial"` or
    /// `"budget_exhausted"`.
    #[getter]
    fn ocr_status(&self) -> PyResult<&'static str> {
        guarded(|| Ok(status_name(self.ocr_status)))
    }

    /// Document-level warnings, then text-pass failures of single pages
    /// (`"page N: ..."`), then the noise pre-check notes of recognised pages
    /// (`"page N: ocr: noise-dominated raster ..."`), in page order.
    #[getter]
    fn warnings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        guarded(|| Ok(self.warnings.bind(py).clone()))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        guarded(|| {
            Ok(format!(
                "<pdf_extract.Extraction page_count={} ocr_status='{}' warnings={}>",
                self.page_count,
                status_name(self.ocr_status),
                self.warnings.bind(py).len()
            ))
        })
    }
}

fn source_name(source: core::PageSource) -> &'static str {
    match source {
        core::PageSource::Text => "text",
        core::PageSource::Ocr => "ocr",
        core::PageSource::TextFallback => "text_fallback",
    }
}

fn status_name(status: core::OcrStatus) -> &'static str {
    match status {
        core::OcrStatus::NotNeeded => "not_needed",
        core::OcrStatus::Ok => "ok",
        core::OcrStatus::Unavailable => "unavailable",
        core::OcrStatus::Partial => "partial",
        core::OcrStatus::BudgetExhausted => "budget_exhausted",
    }
}
