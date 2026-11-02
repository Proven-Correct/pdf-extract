//! Rust errors to Python exceptions (spec T47, T47b; design spec §9.2).
//!
//! The exception classes are defined in pure Python (`pdf_extract/errors.py`)
//! so that they can use multiple inheritance (`DocumentNotFoundError` is
//! also a `FileNotFoundError`). As design spec §9.2 says, the native module
//! imports them at initialisation ([`init`]) and keeps the class objects;
//! raising never looks anything up. That matters for R1: a lookup at raise
//! time (PyO3's `import_exception!`) panics when it fails, and it would fail
//! while PyO3 restores the error, outside `guard::guarded` and outside any
//! unwinding frame, so the process would abort. With the classes cached, a
//! broken `pdf_extract.errors` makes `import pdf_extract` fail instead.
//! (`pdf_extract/__init__.py` imports `errors` before `_native`; importing
//! `errors` here is therefore a dictionary lookup, not work.)
//!
//! Mapping (T47b): every `OpenError` variant has its own class, except
//! `Limit { .. }` (only `max_objects` in this release), which is a
//! `CorruptDocumentError` whose message names the limit (design spec §10).
//! `Io` becomes the base `OpenError` with the original `OSError` as
//! `__cause__` (T47). `PageError::Closed` → `DocumentClosedError`,
//! `IndexOutOfRange` → the builtin `IndexError`, `Limit` →
//! `LimitExceededError(message, limit_name, value)`.
//!
//! OCR (run 0002 O38): `OcrError::Unavailable` → `OcrUnavailableError(message,
//! reason, tessdata, languages, searched)`, the one `RuntimeError` subclass;
//! `OcrError::Page(Render | Recognition)` → `OcrPageError(message)`;
//! `Page(Timeout(d))` → `OcrTimeoutError(message, seconds)`;
//! `Page(Limit { kind, value })` → `OcrPageError(message, limit_name, value)`;
//! `OcrError::Closed` → `DocumentClosedError`.

use std::path::{Path, PathBuf};

use pdf_extract_core as core;
use pyo3::exceptions::{PyIndexError, PyOSError, PyRuntimeError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;

/// The classes of `pdf_extract.errors` the binding raises.
struct Classes {
    open_error: Py<PyType>,
    document_not_found: Py<PyType>,
    empty_file: Py<PyType>,
    not_a_pdf: Py<PyType>,
    corrupt_document: Py<PyType>,
    password_required: Py<PyType>,
    unsupported_encryption: Py<PyType>,
    file_too_large: Py<PyType>,
    too_many_pages: Py<PyType>,
    document_closed: Py<PyType>,
    limit_exceeded: Py<PyType>,
    ocr_unavailable: Py<PyType>,
    ocr_page: Py<PyType>,
    ocr_timeout: Py<PyType>,
    internal: Py<PyType>,
}

static CLASSES: PyOnceLock<Classes> = PyOnceLock::new();

/// Imports `pdf_extract.errors` and caches its classes. Called from module
/// initialisation; an error here fails the import of `pdf_extract._native`.
pub(crate) fn init(py: Python<'_>) -> PyResult<()> {
    CLASSES.get_or_try_init(py, || {
        let module = py.import("pdf_extract.errors")?;
        let class = |name: &str| -> PyResult<Py<PyType>> {
            Ok(module.getattr(name)?.cast_into::<PyType>()?.unbind())
        };
        Ok::<_, PyErr>(Classes {
            open_error: class("OpenError")?,
            document_not_found: class("DocumentNotFoundError")?,
            empty_file: class("EmptyFileError")?,
            not_a_pdf: class("NotAPdfError")?,
            corrupt_document: class("CorruptDocumentError")?,
            password_required: class("PasswordRequiredError")?,
            unsupported_encryption: class("UnsupportedEncryptionError")?,
            file_too_large: class("FileTooLargeError")?,
            too_many_pages: class("TooManyPagesError")?,
            document_closed: class("DocumentClosedError")?,
            limit_exceeded: class("LimitExceededError")?,
            ocr_unavailable: class("OcrUnavailableError")?,
            ocr_page: class("OcrPageError")?,
            ocr_timeout: class("OcrTimeoutError")?,
            internal: class("InternalError")?,
        })
    })?;
    Ok(())
}

/// Builds an exception of the cached class `pick`, with `args`.
///
/// If the cache is somehow empty (module initialisation always fills it, so
/// this is unreachable in practice) the error degrades to a `RuntimeError`
/// with the same message rather than panicking.
fn raise<A>(py: Python<'_>, pick: fn(&Classes) -> &Py<PyType>, args: A, message: &str) -> PyErr
where
    A: for<'py> IntoPyObject<'py> + Send + Sync + 'static,
{
    match CLASSES.get(py) {
        Some(classes) => PyErr::from_type(pick(classes).bind(py).clone(), args),
        None => PyRuntimeError::new_err(message.to_owned()),
    }
}

/// `InternalError(message)`, for the panic boundary.
pub(crate) fn internal_error(py: Python<'_>, message: String) -> PyErr {
    let text = message.clone();
    raise(py, |c| &c.internal, (message,), &text)
}

/// `errno` value for "no such file" (ENOENT is 2 on every platform CPython
/// supports), so `DocumentNotFoundError.errno` matches what a builtin
/// `FileNotFoundError` would carry.
const ENOENT: i32 = 2;

/// The `OSError` an I/O failure becomes as `__cause__`. An OS error is
/// rebuilt as `OSError(errno, strerror, filename)`, which CPython turns
/// into the matching subclass (`PermissionError`, `NotADirectoryError`, ...)
/// with `errno` and `filename` set, as `builtins.open` would; a non-OS error
/// (such as core's "not a regular file") keeps PyO3's conversion. Also used
/// by the test kit's `PdfBuilder.save`.
pub(crate) fn os_error(err: std::io::Error, path: Option<&Path>) -> PyErr {
    match (err.raw_os_error(), path) {
        (Some(errno), Some(p)) => {
            let text = err.to_string();
            // std appends " (os error N)" to the OS description; strerror
            // must be the description alone.
            let strerror = text
                .rsplit_once(" (os error ")
                .map_or(text.as_str(), |(head, _)| head)
                .to_owned();
            PyOSError::new_err((errno, strerror, p.as_os_str().to_owned()))
        }
        _ => PyErr::from(err),
    }
}

/// Converts an open failure. `path` is the file the caller named, if any; it
/// is added to I/O messages so the exception names the object it failed on.
pub(crate) fn open_error(py: Python<'_>, err: core::OpenError, path: Option<&Path>) -> PyErr {
    let message = err.to_string();
    match err {
        core::OpenError::NotFound(p) => raise(
            py,
            |c| &c.document_not_found,
            // The path as an OS string, so a non-UTF-8 name round-trips
            // (surrogate escapes) and `filename` equals what was passed.
            (ENOENT, "No such file or directory", p.into_os_string()),
            &message,
        ),
        core::OpenError::Io(io) => {
            let message = match path {
                Some(p) => format!("{message}: {}", p.display()),
                None => message,
            };
            let err = raise(py, |c| &c.open_error, (message.clone(),), &message);
            err.set_cause(py, Some(os_error(io, path)));
            err
        }
        core::OpenError::Empty => raise(py, |c| &c.empty_file, (message.clone(),), &message),
        core::OpenError::TooLarge { .. } => {
            raise(py, |c| &c.file_too_large, (message.clone(),), &message)
        }
        core::OpenError::NotPdf => raise(py, |c| &c.not_a_pdf, (message.clone(),), &message),
        core::OpenError::Corrupt(_) | core::OpenError::Limit { .. } => {
            raise(py, |c| &c.corrupt_document, (message.clone(),), &message)
        }
        core::OpenError::PasswordRequired => {
            raise(py, |c| &c.password_required, (message.clone(),), &message)
        }
        core::OpenError::UnsupportedEncryption(_) => raise(
            py,
            |c| &c.unsupported_encryption,
            (message.clone(),),
            &message,
        ),
        core::OpenError::TooManyPages { limit, at_least } => raise(
            py,
            |c| &c.too_many_pages,
            (message.clone(), limit, at_least),
            &message,
        ),
    }
}

/// Converts a per-page failure.
pub(crate) fn page_error(py: Python<'_>, err: core::PageError) -> PyErr {
    let message = err.to_string();
    match err {
        core::PageError::Closed => raise(py, |c| &c.document_closed, (message.clone(),), &message),
        core::PageError::IndexOutOfRange { .. } => PyIndexError::new_err(message),
        core::PageError::Limit { kind, value } => raise(
            py,
            |c| &c.limit_exceeded,
            (message.clone(), kind.name(), value),
            &message,
        ),
    }
}

/// The `TooLarge` error for a buffer refused before it is copied; the same
/// text and class core would produce for it.
pub(crate) fn too_large(py: Python<'_>, size: u64, limit: u64) -> PyErr {
    open_error(py, core::OpenError::TooLarge { size, limit }, None)
}

/// `OcrUnavailableError(message, reason, tessdata, languages, searched)`.
///
/// Paths go over as `OsString`, which PyO3 converts to `str` with surrogate
/// escapes, so a non-UTF-8 directory name round-trips through
/// `os.fsencode`.
pub(crate) fn ocr_unavailable(py: Python<'_>, err: core::OcrUnavailable) -> PyErr {
    let message = err.to_string();
    let args = (
        message.clone(),
        err.reason.name(),
        err.tessdata.map(PathBuf::into_os_string),
        err.languages,
        err.searched
            .into_iter()
            .map(PathBuf::into_os_string)
            .collect::<Vec<_>>(),
    );
    raise(py, |c| &c.ocr_unavailable, args, &message)
}

/// Converts an OCR failure of one page (or of the engine behind it).
pub(crate) fn ocr_error(py: Python<'_>, err: core::OcrError) -> PyErr {
    match err {
        core::OcrError::Unavailable(u) => ocr_unavailable(py, u),
        core::OcrError::Closed => {
            let message = err.to_string();
            raise(py, |c| &c.document_closed, (message.clone(),), &message)
        }
        core::OcrError::Page(page) => {
            let message = page.to_string();
            match page {
                core::OcrPageError::Timeout(d) => raise(
                    py,
                    |c| &c.ocr_timeout,
                    (message.clone(), d.as_secs_f64()),
                    &message,
                ),
                core::OcrPageError::Limit { kind, value } => raise(
                    py,
                    |c| &c.ocr_page,
                    (message.clone(), Some(kind.name()), Some(value)),
                    &message,
                ),
                core::OcrPageError::Render(_) | core::OcrPageError::Recognition(_) => {
                    raise(py, |c| &c.ocr_page, (message.clone(),), &message)
                }
            }
        }
    }
}
