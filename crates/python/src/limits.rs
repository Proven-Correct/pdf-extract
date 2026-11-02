//! `pdf_extract.Limits` (spec §3.5, §1.4): an immutable mirror of
//! `pdf_extract_core::Limits`.
//!
//! Arguments are keyword-only, with the core defaults. Integer arguments are
//! taken as `i128` and range-checked here so that a negative or oversized
//! value raises `ValueError` naming the argument (PyO3's conversion to the
//! field's own type would raise `OverflowError` with no argument name). An
//! integer beyond 128 bits still fails in PyO3's own conversion, with
//! `OverflowError`. `page_time_budget` is
//! seconds as a float, like `time.sleep`.

use std::time::Duration;

use pdf_extract_core as core;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::guard::guarded;

/// Resource limits for one document. Immutable; pass to `open` or
/// `open_bytes`.
#[pyclass(module = "pdf_extract", name = "Limits", frozen)]
pub(crate) struct Limits {
    pub(crate) inner: core::Limits,
}

/// Checks that `value` fits `T` and is not negative.
pub(crate) fn non_negative<T: TryFrom<i128>>(name: &str, value: i128) -> PyResult<T> {
    if value < 0 {
        return Err(PyValueError::new_err(format!(
            "{name} must not be negative, got {value}"
        )));
    }
    T::try_from(value)
        .map_err(|_| PyValueError::new_err(format!("{name} is too large, got {value}")))
}

#[pymethods]
impl Limits {
    // The defaults are written out as literals (not `core::Limits::default()`
    // field reads) because PyO3 shows them in `__text_signature__`; a test
    // checks they equal the core defaults.
    #[new]
    #[pyo3(signature = (
        *,
        max_file_size = 536_870_912,
        max_page_tree_depth = 256,
        max_objects = 10_000_000,
        max_stream_decoded_bytes = 268_435_456,
        max_ops_per_page = 10_000_000,
        page_time_budget = 30.0,
        max_chars_per_page = 2_000_000,
        max_image_pixels = 256_000_000,
    ))]
    #[allow(
        clippy::too_many_arguments,
        reason = "mirrors the keyword-only Python signature"
    )]
    fn new(
        max_file_size: i128,
        max_page_tree_depth: i128,
        max_objects: i128,
        max_stream_decoded_bytes: i128,
        max_ops_per_page: i128,
        page_time_budget: f64,
        max_chars_per_page: i128,
        max_image_pixels: i128,
    ) -> PyResult<Self> {
        guarded(|| {
            let page_time_budget = Duration::try_from_secs_f64(page_time_budget).map_err(|_| {
                PyValueError::new_err(format!(
                    "page_time_budget must be a finite, non-negative number of seconds, \
                     got {page_time_budget}"
                ))
            })?;
            Ok(Self {
                inner: core::Limits {
                    max_file_size: non_negative("max_file_size", max_file_size)?,
                    max_page_tree_depth: non_negative("max_page_tree_depth", max_page_tree_depth)?,
                    max_objects: non_negative("max_objects", max_objects)?,
                    max_stream_decoded_bytes: non_negative(
                        "max_stream_decoded_bytes",
                        max_stream_decoded_bytes,
                    )?,
                    max_ops_per_page: non_negative("max_ops_per_page", max_ops_per_page)?,
                    page_time_budget,
                    max_chars_per_page: non_negative("max_chars_per_page", max_chars_per_page)?,
                    max_image_pixels: non_negative("max_image_pixels", max_image_pixels)?,
                },
            })
        })
    }

    /// Largest input accepted, in bytes.
    #[getter]
    fn max_file_size(&self) -> PyResult<u64> {
        guarded(|| Ok(self.inner.max_file_size))
    }

    /// Deepest page-tree node visited; deeper subtrees are skipped.
    #[getter]
    fn max_page_tree_depth(&self) -> PyResult<u32> {
        guarded(|| Ok(self.inner.max_page_tree_depth))
    }

    /// Most cross-reference entries a document may have.
    #[getter]
    fn max_objects(&self) -> PyResult<usize> {
        guarded(|| Ok(self.inner.max_objects))
    }

    /// Most bytes one stream may decode to.
    #[getter]
    fn max_stream_decoded_bytes(&self) -> PyResult<u64> {
        guarded(|| Ok(self.inner.max_stream_decoded_bytes))
    }

    /// Most content-stream operators interpreted per page.
    #[getter]
    fn max_ops_per_page(&self) -> PyResult<u64> {
        guarded(|| Ok(self.inner.max_ops_per_page))
    }

    /// Wall-clock budget for one page, in seconds.
    #[getter]
    fn page_time_budget(&self) -> PyResult<f64> {
        guarded(|| Ok(self.inner.page_time_budget.as_secs_f64()))
    }

    /// Most characters extracted per page.
    #[getter]
    fn max_chars_per_page(&self) -> PyResult<usize> {
        guarded(|| Ok(self.inner.max_chars_per_page))
    }

    /// Largest image (width × height) rendering will decode, checked as the
    /// image declares it, as its codec reads it from the header, and as the
    /// codec reports it after decoding. Also sets the JBIG2 and JPEG 2000
    /// decoders' memory budgets, 2 and 64 bytes per pixel of this cap, and
    /// JBIG2's allowance for placing symbol instances, 16 pixels per pixel of
    /// it; an image over the cap or its decoder's budget is skipped with an
    /// ``image:`` warning. JPEG 2000 files using 1x1 code-blocks
    /// (non-default encoder settings) are refused above about a quarter of
    /// this value; raise it to admit them.
    #[getter]
    fn max_image_pixels(&self) -> PyResult<u64> {
        guarded(|| Ok(self.inner.max_image_pixels))
    }

    fn __repr__(&self) -> PyResult<String> {
        guarded(|| {
            let l = &self.inner;
            Ok(format!(
                "Limits(max_file_size={}, max_page_tree_depth={}, max_objects={}, \
                 max_stream_decoded_bytes={}, max_ops_per_page={}, page_time_budget={:?}, \
                 max_chars_per_page={}, max_image_pixels={})",
                l.max_file_size,
                l.max_page_tree_depth,
                l.max_objects,
                l.max_stream_decoded_bytes,
                l.max_ops_per_page,
                l.page_time_budget.as_secs_f64(),
                l.max_chars_per_page,
                l.max_image_pixels,
            ))
        })
    }
}
