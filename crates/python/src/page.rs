//! `Page` and `TextPage` (spec §3.5, T19-T36, T43).
//!
//! Extraction (`Page.text`, `Page.text_page`) and serialisation
//! (`TextPage.text`) run under `py.detach`: they interpret content streams
//! or walk every character, and other Python threads must keep running
//! meanwhile (T43). The result string is created after re-attaching, which
//! is one allocation and one copy.
//!
//! The string options are the `Literal` values of the stub; anything else is
//! a `ValueError` naming the accepted values.
//!
//! `Page.ocr` (run 0002 O16, O21) renders and recognises the page under
//! `py.detach`; with no engine it uses `default_ocr_engine()`, whose first
//! build also runs detached.

use pdf_extract_core as core;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyTuple;

use crate::errors::{ocr_error, page_error};
use crate::guard::guarded;
use crate::ocr::{OcrEngine, default_engine, dpi as checked_dpi};

pub(crate) fn block_separator(value: &str) -> PyResult<core::BlockSeparator> {
    match value {
        "newline" => Ok(core::BlockSeparator::Newline),
        "blank_line" => Ok(core::BlockSeparator::BlankLine),
        other => Err(PyValueError::new_err(format!(
            "block_separator must be 'newline' or 'blank_line', not {other:?}"
        ))),
    }
}

fn unmapped(value: &str) -> PyResult<core::Unmapped> {
    match value {
        "cid" => Ok(core::Unmapped::Cid),
        "replacement" => Ok(core::Unmapped::Replacement),
        "skip" => Ok(core::Unmapped::Skip),
        other => Err(PyValueError::new_err(format!(
            "unmapped must be 'cid', 'replacement' or 'skip', not {other:?}"
        ))),
    }
}

/// One page of a `Document`. Obtained from `doc[i]` or by iterating; valid
/// after the document is closed, but extraction then raises
/// `DocumentClosedError`.
#[pyclass(module = "pdf_extract", name = "Page", frozen)]
pub(crate) struct Page {
    inner: core::Page,
}

impl Page {
    pub(crate) fn new(inner: core::Page) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl Page {
    /// The page's 0-based index.
    #[getter]
    fn index(&self) -> PyResult<usize> {
        guarded(|| Ok(self.inner.index()))
    }

    /// The page's text: every line ends in `"\n"`; an empty page gives `""`.
    #[pyo3(signature = (*, block_separator = "newline", unmapped = "cid"))]
    fn text(&self, py: Python<'_>, block_separator: &str, unmapped: &str) -> PyResult<String> {
        guarded(|| {
            let opts = core::TextOptions {
                unmapped: self::unmapped(unmapped)?,
                block_separator: self::block_separator(block_separator)?,
                ..core::TextOptions::default()
            };
            py.detach(|| self.inner.text(&opts))
                .map_err(|e| page_error(py, e))
        })
    }

    /// Extracts the page once; the result can be serialised repeatedly and
    /// carries the page's warnings.
    #[pyo3(signature = (*, unmapped = "cid"))]
    fn text_page(&self, py: Python<'_>, unmapped: &str) -> PyResult<TextPage> {
        guarded(|| {
            let opts = core::TextOptions {
                unmapped: self::unmapped(unmapped)?,
                ..core::TextOptions::default()
            };
            py.detach(|| self.inner.text_page(&opts))
                .map(|inner| TextPage { inner })
                .map_err(|e| page_error(py, e))
        })
    }

    /// Renders the page and recognises it with `engine` (the default engine
    /// when `None`) at `dpi` (the engine's own when `None`). The result's
    /// `source` is `"ocr"` and its `dpi` the resolution actually used.
    #[pyo3(signature = (engine = None, *, dpi = None))]
    fn ocr(
        &self,
        py: Python<'_>,
        engine: Option<PyRef<'_, OcrEngine>>,
        dpi: Option<i128>,
    ) -> PyResult<TextPage> {
        guarded(|| {
            let opts = pdf_extract_ocr::OcrOptions {
                dpi: dpi.map(checked_dpi).transpose()?,
            };
            // A clone of the engine handle (an `Arc`), so nothing borrowed
            // from a Python object crosses into the detached section.
            let engine = match engine {
                Some(e) => e.inner.clone(),
                None => default_engine(py)?,
            };
            py.detach(|| engine.ocr(&self.inner, &opts))
                .map(|inner| TextPage { inner })
                .map_err(|e| ocr_error(py, e))
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        guarded(|| Ok(format!("<pdf_extract.Page index={}>", self.inner.index())))
    }
}

/// The extracted text of one page. Immutable; stays readable after the
/// document is closed.
#[pyclass(module = "pdf_extract", name = "TextPage", frozen)]
pub(crate) struct TextPage {
    inner: core::TextPage,
}

#[pymethods]
impl TextPage {
    /// Where the text came from: `"content"` (the text layer) or `"ocr"`.
    #[getter]
    fn source(&self) -> PyResult<&'static str> {
        guarded(|| Ok(source_name(self.inner.source())))
    }

    /// The resolution OCR ran at; `None` for text read from the content.
    #[getter]
    fn dpi(&self) -> PyResult<Option<u32>> {
        guarded(|| Ok(self.inner.dpi()))
    }

    /// Problems worked around while extracting this page, as
    /// `"<category>: <detail>"` strings.
    #[getter]
    fn warnings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        guarded(|| PyTuple::new(py, self.inner.warnings().iter().map(ToString::to_string)))
    }

    /// The text, serialised with the given block separator.
    #[pyo3(signature = (*, block_separator = "newline"))]
    fn text(&self, py: Python<'_>, block_separator: &str) -> PyResult<String> {
        guarded(|| {
            let sep = self::block_separator(block_separator)?;
            Ok(py.detach(|| self.inner.to_text(sep)))
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        guarded(|| {
            Ok(format!(
                "<pdf_extract.TextPage source='{}' warnings={}>",
                source_name(self.inner.source()),
                self.inner.warnings().len()
            ))
        })
    }
}

/// The Python spelling of a [`core::TextSource`].
fn source_name(source: core::TextSource) -> &'static str {
    match source {
        core::TextSource::Content => "content",
        core::TextSource::Ocr => "ocr",
    }
}
