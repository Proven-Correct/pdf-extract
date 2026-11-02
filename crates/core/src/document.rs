//! `Document`, its shared state, and the close protocol (spec §3.3
//! `document.rs`, T16, T38-T40).
//!
//! **Ownership.** A [`Document`] is an `Arc<DocShared>`; cloning it, and every
//! [`Page`] handle, shares that one allocation. `DocShared` keeps the facts
//! that must survive `close()` (page count, encryption flag, open-time
//! warnings) next to `state: CloseCell<Loaded>`, which owns the parsed
//! document.
//!
//! **Close protocol (T39, T40).** Implemented by `CloseCell`
//! (`close_cell.rs`, model-checked by tests/loom_close.rs). An operation
//! takes the read lock only long enough to clone the `Arc<Loaded>`, then
//! releases it and works on its own clone. `close()` takes the write lock
//! and replaces the state with `None`. So:
//!
//! - operations already running keep their clone and finish normally;
//! - operations starting later see `None` and return `PageError::Closed`;
//! - the parsed document is freed when the last in-flight clone drops, which
//!   is immediately if nothing is running;
//! - the lock is never held during parsing or interpretation, so `close()`
//!   never waits for a page to finish.

use std::fmt;
use std::iter::FusedIterator;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use hayro_syntax::Pdf;

use crate::close_cell::CloseCell;
use crate::error::{OpenError, PageError};
use crate::index::PageIndex;
use crate::limits::Limits;
use crate::open;
use crate::page::Page;
use crate::source;
use crate::sync;
use crate::warning::{Warning, WarningKind};

/// How to open a document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OpenOptions {
    /// Refuse documents with more pages than this (`OpenError::TooManyPages`).
    /// The page-tree walk stops at the first page over the limit, so a huge
    /// document costs no more than the limit to reject. Default: no limit.
    pub max_pages: Option<usize>,
    /// Resource limits for this document.
    pub limits: Limits,
}

/// An open PDF document.
///
/// Cheap to clone (one reference count); clones share the document, and
/// closing one closes all. `Send + Sync`: any number of threads may use one
/// document at once.
#[derive(Clone)]
pub struct Document(Arc<DocShared>);

/// State shared by a document's clones and its page handles.
pub(crate) struct DocShared {
    state: CloseCell<Loaded>,
    page_count: usize,
    is_encrypted: bool,
    open_warnings: Vec<Warning>,
    limits: Limits,
    /// Set by a page operation that finds the xref repaired (hayro repairs
    /// lazily, so this can happen long after open); see [`Document::warnings`].
    repaired_later: AtomicBool,
}

/// The parsed document. Lives exactly as long as some operation or the open
/// state holds an `Arc` to it (a `sync::Arc`: loom's under `--cfg loom`).
pub(crate) struct Loaded {
    /// The bytes the document was opened from (shared with `pdf`, not
    /// copied). Nothing reads them through this field (hayro holds its own
    /// clone of the `Arc`); it makes the ownership explicit and lets tests
    /// check that opening did not copy the buffer.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by tests only"))]
    pub(crate) source: Arc<Vec<u8>>,
    pub(crate) pdf: Pdf,
    pub(crate) index: PageIndex,
}

static_assertions::assert_impl_all!(Document: Send, Sync);
static_assertions::assert_impl_all!(Page: Send, Sync);
static_assertions::assert_impl_all!(Pages: Send, Sync);

impl Document {
    /// Opens the PDF file at `path`.
    ///
    /// The file is size-checked from its metadata, read whole, and closed
    /// before this returns (T1); its name and extension are ignored (T3).
    ///
    /// # Errors
    /// `NotFound` or `Io` for file-system failures, `TooLarge` before
    /// reading an oversized file, and every error of [`Self::from_bytes`].
    pub fn open(path: impl AsRef<Path>, opts: &OpenOptions) -> Result<Self, OpenError> {
        let bytes = source::read_file(path.as_ref(), opts.limits.max_file_size)?;
        Self::from_shared(Arc::new(bytes), opts)
    }

    /// Opens a PDF held in memory, taking ownership of the buffer.
    ///
    /// # Errors
    /// `TooLarge`, `Empty`, `NotPdf`, `Corrupt`, `PasswordRequired`,
    /// `UnsupportedEncryption`, `TooManyPages`, or `Limit { Objects }`.
    pub fn from_bytes(bytes: Vec<u8>, opts: &OpenOptions) -> Result<Self, OpenError> {
        Self::from_shared(Arc::new(bytes), opts)
    }

    /// Opens a PDF from a shared buffer without copying it.
    ///
    /// # Errors
    /// As [`Self::from_bytes`].
    pub fn from_shared(bytes: Arc<Vec<u8>>, opts: &OpenOptions) -> Result<Self, OpenError> {
        let opened = open::open(bytes, opts)?;
        Ok(Self(Arc::new(DocShared {
            state: CloseCell::new(opened.loaded),
            page_count: opened.page_count,
            is_encrypted: opened.is_encrypted,
            open_warnings: opened.warnings,
            limits: opts.limits.clone(),
            repaired_later: AtomicBool::new(false),
        })))
    }

    /// Number of pages. Constant time; still answers after [`Self::close`].
    pub fn page_count(&self) -> usize {
        self.0.page_count
    }

    /// Whether the document is encrypted (it opened because its user
    /// password is empty). Still answers after [`Self::close`].
    pub fn is_encrypted(&self) -> bool {
        self.0.is_encrypted
    }

    /// Document-level warnings: those raised while opening (xref repair,
    /// page-tree damage, ...), plus `XrefRepaired` if a page operation later
    /// found the cross-reference table broken and had it rebuilt (repair is
    /// lazy; the warning appears at most once either way). Still answers
    /// after [`Self::close`].
    pub fn warnings(&self) -> Vec<Warning> {
        let mut warnings = self.0.open_warnings.clone();
        if self.0.repaired_later.load(Ordering::Acquire)
            && !warnings.iter().any(|w| w.kind == WarningKind::XrefRepaired)
        {
            warnings.push(Warning {
                page: None,
                kind: WarningKind::XrefRepaired,
                detail: "repaired".into(),
                count: 1,
            });
        }
        warnings
    }

    /// A handle to page `index` (0-based).
    ///
    /// # Errors
    /// `Closed` after [`Self::close`]; `IndexOutOfRange` when
    /// `index >= page_count()`.
    pub fn page(&self, index: usize) -> Result<Page, PageError> {
        if self.is_closed() {
            return Err(PageError::Closed);
        }
        let count = self.0.page_count;
        if index >= count {
            return Err(PageError::IndexOutOfRange { index, count });
        }
        Ok(Page::new(self.0.clone(), index))
    }

    /// An iterator over handles to every page, in order. It yields handles
    /// even after [`Self::close`]; using them then fails with `Closed`.
    pub fn pages(&self) -> Pages {
        Pages {
            doc: self.0.clone(),
            cursor: AtomicUsize::new(0),
        }
    }

    /// Closes the document: later operations on it or its pages fail with
    /// `PageError::Closed`, operations already running finish, and the parsed
    /// document is freed once the last of them ends. Idempotent.
    pub fn close(&self) {
        // Dropped here, after `close` has released the lock: freeing a large
        // document must not stall readers waiting on the lock.
        drop(self.0.state.close());
    }

    /// Whether [`Self::close`] has been called (on this or any clone).
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Document")
            .field("page_count", &self.0.page_count)
            .field("is_encrypted", &self.0.is_encrypted)
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl DocShared {
    /// The parsed document for one operation (see the module docs): clones
    /// the `Arc` under the read lock and releases the lock before returning.
    pub(crate) fn loaded(&self) -> Result<sync::Arc<Loaded>, PageError> {
        self.state.get().ok_or(PageError::Closed)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.state.is_closed()
    }

    pub(crate) fn page_count(&self) -> usize {
        self.page_count
    }

    /// Records that hayro has repaired the xref (see [`Document::warnings`]).
    pub(crate) fn note_xref_repaired(&self) {
        self.repaired_later.store(true, Ordering::Release);
    }

    /// The limits the document was opened with; page operations apply them.
    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }
}

/// Iterator over a document's pages, from [`Document::pages`].
///
/// The position is an atomic counter, so one iterator shared between threads
/// (as the Python binding does) hands each index out exactly once.
pub struct Pages {
    doc: Arc<DocShared>,
    cursor: AtomicUsize,
}

impl Pages {
    /// Claims the next index; safe to call from several threads at once.
    fn next_shared(&self) -> Option<Page> {
        let count = self.doc.page_count();
        let index = self
            .cursor
            .try_update(Ordering::AcqRel, Ordering::Acquire, |c| {
                (c < count).then_some(c + 1)
            })
            .ok()?;
        Some(Page::new(self.doc.clone(), index))
    }

    fn remaining(&self) -> usize {
        self.doc
            .page_count()
            .saturating_sub(self.cursor.load(Ordering::Acquire))
    }
}

impl Iterator for Pages {
    type Item = Page;

    fn next(&mut self) -> Option<Page> {
        self.next_shared()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.remaining();
        (n, Some(n))
    }
}

impl ExactSizeIterator for Pages {}

impl FusedIterator for Pages {}

impl fmt::Debug for Pages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pages")
            .field("remaining", &self.remaining())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
