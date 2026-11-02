//! `open`, `open_bytes`, `Document` and its page iterator (spec §3.5, T2,
//! T16, T40, T43, T46; design spec §8.4).
//!
//! **GIL.** Opening parses the file, so `open` and `open_bytes` convert their
//! arguments while attached and then run `Document::open`/`from_bytes` under
//! `py.detach` (T43): other Python threads run, and on the free-threaded
//! build the thread does not block a stop-the-world pause. Everything else
//! here is constant time and stays attached.
//!
//! **Buffers (T2).** `open_bytes` copies the caller's buffer exactly once
//! and keeps no reference to the Python object. The copy holds a buffer
//! export, which is what stops a `bytearray` from being resized under it.
//! The copy itself runs attached: copying without the interpreter would need
//! a raw pointer into the export (`unsafe`, which spec §4.16 keeps to what
//! PyO3 requires), and a memcpy of even `max_file_size` bytes is short next
//! to the parse that follows, which is detached. The size limit is checked
//! before the copy, so an oversized buffer is refused without being
//! duplicated. A memoryview of a non-byte format (`array('i')`) is viewed as
//! bytes through `memoryview.cast('B')`, which needs it C-contiguous.
//!
//! **Sharing.** Every class is `frozen`: methods take `&self`, so PyO3 never
//! needs a runtime borrow flag and threads can call into one object at once
//! (`pdf_extract_core::Document` is `Send + Sync` and does its own locking).
//! Classes are *not* `immutable_type`, so `unittest.mock.patch.object` can
//! replace their methods (spec §3.5).

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use pdf_extract_core as core;
use pyo3::buffer::PyBuffer;
use pyo3::exceptions::{PyIndexError, PyOverflowError, PyTypeError};
use pyo3::prelude::*;
use pyo3::types::{PyMemoryView, PyTuple};

use crate::errors::{open_error, page_error, too_large};
use crate::guard::guarded;
use crate::limits::{Limits, non_negative};
use crate::page::Page;

/// Builds the core options from the keyword arguments shared by `open` and
/// `open_bytes`.
pub(crate) fn options(
    max_pages: Option<i128>,
    limits: Option<&Limits>,
) -> PyResult<core::OpenOptions> {
    Ok(core::OpenOptions {
        max_pages: max_pages
            .map(|n| non_negative("max_pages", n))
            .transpose()?,
        limits: limits.map(|l| l.inner.clone()).unwrap_or_default(),
    })
}

/// Opens the PDF file at `path`. The file is read whole and closed before
/// this returns; its name and extension are ignored.
#[pyfunction]
#[pyo3(signature = (path, *, max_pages = None, limits = None))]
pub(crate) fn open(
    py: Python<'_>,
    path: PathBuf,
    max_pages: Option<i128>,
    limits: Option<PyRef<'_, Limits>>,
) -> PyResult<Document> {
    guarded(|| {
        let opts = options(max_pages, limits.as_deref())?;
        let result = py.detach(|| core::Document::open(&path, &opts));
        result
            .map(Document::new)
            .map_err(|e| open_error(py, e, Some(&path)))
    })
}

/// Opens a PDF held in memory. The buffer is copied once; no reference to
/// `data` is kept.
#[pyfunction]
#[pyo3(signature = (data, *, max_pages = None, limits = None))]
pub(crate) fn open_bytes(
    py: Python<'_>,
    data: &Bound<'_, PyAny>,
    max_pages: Option<i128>,
    limits: Option<PyRef<'_, Limits>>,
) -> PyResult<Document> {
    guarded(|| {
        let opts = options(max_pages, limits.as_deref())?;
        if data.is_instance_of::<pyo3::types::PyString>() {
            // str supports no buffer protocol either, but say what to do.
            return Err(PyTypeError::new_err(
                "open_bytes() needs a bytes-like object, not str (encode it, or use open() for a path)",
            ));
        }
        let buffer = match PyBuffer::<u8>::get(data) {
            Ok(buffer) => buffer,
            Err(_) if data.is_instance_of::<PyMemoryView>() => {
                let bytes_view = data.call_method1("cast", ("B",)).map_err(|e| {
                    PyTypeError::new_err(format!(
                        "open_bytes() could not view this memoryview as bytes: {e}"
                    ))
                })?;
                PyBuffer::<u8>::get(&bytes_view)?
            }
            Err(_) => {
                return Err(PyTypeError::new_err(format!(
                    "open_bytes() needs bytes, bytearray or a memoryview, not {}",
                    data.get_type()
                        .name()
                        .map_or_else(|_| "?".into(), |n| n.to_string())
                )));
            }
        };
        let size = buffer.len_bytes() as u64;
        if size > opts.limits.max_file_size {
            return Err(too_large(py, size, opts.limits.max_file_size));
        }
        let bytes = buffer.to_vec(py)?;
        // Release the export now: the caller may resize a bytearray as soon
        // as we return, and we no longer need it.
        drop(buffer);
        let result = py.detach(|| core::Document::from_bytes(bytes, &opts));
        result
            .map(Document::new)
            .map_err(|e| open_error(py, e, None))
    })
}

/// An open PDF document. Create with `open` or `open_bytes`.
#[pyclass(module = "pdf_extract", name = "Document", frozen)]
pub(crate) struct Document {
    inner: core::Document,
}

impl Document {
    fn new(inner: core::Document) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl Document {
    /// Number of pages; readable after `close()`.
    #[getter]
    fn page_count(&self) -> PyResult<usize> {
        guarded(|| Ok(self.inner.page_count()))
    }

    /// Whether `close()` has been called.
    #[getter]
    fn closed(&self) -> PyResult<bool> {
        guarded(|| Ok(self.inner.is_closed()))
    }

    /// Whether the file is encrypted (owner-password-only files open and
    /// extract transparently).
    #[getter]
    fn is_encrypted(&self) -> PyResult<bool> {
        guarded(|| Ok(self.inner.is_encrypted()))
    }

    /// Problems worked around while opening, as `"<category>: <detail>"`
    /// strings, plus `"xref: repaired"` if a later operation had to rebuild
    /// the cross-reference table.
    #[getter]
    fn warnings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        guarded(|| {
            let warnings = self.inner.warnings();
            PyTuple::new(py, warnings.iter().map(ToString::to_string))
        })
    }

    fn __len__(&self) -> PyResult<usize> {
        guarded(|| Ok(self.inner.page_count()))
    }

    /// `doc[i]`: negative indices count from the end; out of range raises
    /// `IndexError`, a non-integer `TypeError` (T46), a closed document
    /// `DocumentClosedError`.
    fn __getitem__(&self, index: &Bound<'_, PyAny>) -> PyResult<Page> {
        guarded(|| {
            let count = self.inner.page_count();
            let py = index.py();
            // Only the conversion's own failures are rewritten: an
            // `__index__` that raises something else propagates unchanged.
            let index: isize = index.extract().map_err(|e: PyErr| {
                if e.is_instance_of::<PyOverflowError>(py) {
                    PyIndexError::new_err("page index out of range")
                } else if !e.is_instance_of::<PyTypeError>(py) {
                    e
                } else {
                    PyTypeError::new_err(format!(
                        "page indices must be integers, not {}",
                        index
                            .get_type()
                            .name()
                            .map_or_else(|_| "?".into(), |n| n.to_string())
                    ))
                }
            })?;
            // `count` fits isize: it counts leaves indexed in memory.
            let resolved = if index < 0 {
                index.checked_add_unsigned(count)
            } else {
                Some(index)
            };
            let Some(resolved) = resolved.and_then(|i| usize::try_from(i).ok()) else {
                return Err(PyIndexError::new_err(format!(
                    "page index {index} is out of range for a document with {count} pages"
                )));
            };
            self.inner
                .page(resolved)
                .map(Page::new)
                .map_err(|e| page_error(py, e))
        })
    }

    /// Iterates over the pages. The iterator's position is shared safely
    /// between threads (each page is handed out once).
    fn __iter__(&self) -> PyResult<PageIterator> {
        guarded(|| {
            Ok(PageIterator {
                pages: Mutex::new(self.inner.pages()),
            })
        })
    }

    /// Closes the document and frees its memory once running extractions
    /// finish. Later page operations raise `DocumentClosedError`; calling it
    /// again does nothing.
    fn close(&self) -> PyResult<()> {
        guarded(|| {
            self.inner.close();
            Ok(())
        })
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        guarded(|| Ok(slf))
    }

    /// Always closes; never suppresses the exception (returns `None`).
    #[pyo3(signature = (*_exc_info))]
    fn __exit__(&self, _exc_info: &Bound<'_, PyTuple>) -> PyResult<()> {
        guarded(|| {
            self.inner.close();
            Ok(())
        })
    }

    fn __repr__(&self) -> PyResult<String> {
        guarded(|| {
            Ok(format!(
                "<pdf_extract.Document page_count={} closed={}>",
                self.inner.page_count(),
                if self.inner.is_closed() {
                    "True"
                } else {
                    "False"
                }
            ))
        })
    }
}

/// Iterator returned by `iter(document)`.
///
/// The core iterator's cursor is atomic, but `Iterator::next` takes `&mut`,
/// so it sits behind a mutex held only for one `next` (a few instructions,
/// no Python calls), which keeps the class `frozen` and safe to share
/// between threads.
#[pyclass(module = "pdf_extract._native", name = "PageIterator", frozen)]
pub(crate) struct PageIterator {
    pages: Mutex<core::Pages>,
}

impl PageIterator {
    fn lock(&self) -> std::sync::MutexGuard<'_, core::Pages> {
        // Poisoning cannot leave `Pages` inconsistent: its only state is an
        // atomic cursor, so a panic elsewhere is no reason to stop iterating.
        self.pages.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[pymethods]
impl PageIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        guarded(|| Ok(slf))
    }

    fn __next__(&self) -> PyResult<Option<Page>> {
        guarded(|| Ok(self.lock().next().map(Page::new)))
    }

    fn __length_hint__(&self) -> PyResult<usize> {
        guarded(|| Ok(self.lock().len()))
    }
}
