//! `Page` handles (spec §3.3 `page.rs`, T16, T17).
//!
//! A [`Page`] is `(Arc<DocShared>, index)`: a few words, cheap to clone,
//! holding no parsed data. Everything a page operation needs is fetched when
//! the operation starts, through [`Page::with_loaded`], which implements the
//! operation half of the close protocol (see `document.rs`). Text extraction
//! (`text::extract`, and `Page::text_page` / `Page::text` in `text::page`)
//! is built on it.
//!
//! With the `render-support` feature, [`Page::with_hayro_page`] exposes the
//! same hook to the rendering crate (run 0002 §3.3): hayro types must not
//! appear in the documented API, so the hook is `#[doc(hidden)]` and compiled
//! only for the crates that enable the feature.

use std::fmt;
use std::sync::Arc;

use crate::document::{DocShared, Loaded};
use crate::error::PageError;
use crate::limits::Limits;

/// A handle to one page of a [`Document`](crate::Document).
///
/// Valid after the document is closed (its index still answers); operations
/// on it then fail with [`PageError::Closed`].
#[derive(Clone)]
pub struct Page {
    doc: Arc<DocShared>,
    index: usize,
}

impl Page {
    /// Callers guarantee `index < doc.page_count`.
    pub(crate) fn new(doc: Arc<DocShared>, index: usize) -> Self {
        Self { doc, index }
    }

    /// The page's 0-based index in its document.
    pub fn index(&self) -> usize {
        self.index
    }

    /// The document's limits, which bound every operation on the page.
    pub(crate) fn limits(&self) -> &Limits {
        self.doc.limits()
    }

    /// Runs one page operation against the parsed document.
    ///
    /// The document's `Arc<Loaded>` is cloned under the read lock and the
    /// lock released before `f` runs, so `f` may take as long as it needs
    /// and a concurrent `close()` neither waits for it nor disturbs it. The
    /// hayro `Page` must be built inside `f`
    /// (`loaded.index.hayro_page(...)`) and never stored (T17).
    ///
    /// hayro repairs a broken xref lazily, on the first lookup that misses,
    /// which can be inside any page operation; after `f` the document's
    /// repair flag is therefore re-read so that
    /// [`Document::warnings`](crate::Document::warnings) reports a repair
    /// whenever it happened (spec §4.10, C2 under repair). The page's own
    /// output does not carry it: which concurrent operation triggers the
    /// repair is a race, and a page's result must not depend on one (T39).
    ///
    /// # Errors
    /// `Closed` if the document was closed before the operation started.
    // Without `render-support` the rendering hook must not exist (run 0002
    // §4.1); this doctest is compiled only in that configuration and fails to
    // build if the method is ever reachable without the feature. It sits on
    // a private method so the hook's name stays out of the rendered docs.
    #[cfg_attr(
        not(feature = "render-support"),
        doc = "```compile_fail,E0599",
        doc = "# let doc = pdf_extract_core::Document::from_bytes(Vec::new(), &Default::default()).unwrap();",
        doc = "doc.page(0).unwrap().with_hayro_page(|_| ());",
        doc = "```"
    )]
    pub(crate) fn with_loaded<R>(
        &self,
        f: impl FnOnce(&Loaded, usize) -> R,
    ) -> Result<R, PageError> {
        let loaded = self.doc.loaded()?;
        let result = f(&loaded, self.index);
        if loaded.pdf.xref_was_repaired() {
            self.doc.note_xref_repaired();
        }
        Ok(result)
    }
}

/// What a renderer needs to draw one page (run 0002 §3.3): the hayro page
/// exactly as the text path builds it, the cross-reference table it reads
/// from, and the document's limits. Borrowed for the duration of one
/// [`Page::with_hayro_page`] call; none of it may be stored (T17).
#[cfg(feature = "render-support")]
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct RenderInput<'a> {
    /// The page's 0-based index.
    pub index: usize,
    /// The hayro page; `None` when the leaf no longer resolves (possible
    /// only after a lazy xref repair changed what its identifier points to;
    /// the text path then returns an empty page with a
    /// `PageTreeNodeUnreadable` warning).
    pub page: Option<&'a hayro_syntax::page::Page<'a>>,
    /// The document's cross-reference table.
    pub xref: &'a hayro_syntax::xref::XRef,
    /// The document's limits.
    pub limits: &'a Limits,
}

#[cfg(feature = "render-support")]
impl fmt::Debug for RenderInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderInput")
            .field("index", &self.index)
            .field("resolved", &self.page.is_some())
            .field("limits", self.limits)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "render-support")]
impl Page {
    /// Runs `f` with the hayro page, built as the text path builds it, under
    /// the same operation protocol as text extraction (the document lock is
    /// not held while `f` runs, and a lazy xref repair is noted afterwards).
    /// For `pdf-extract-render` only.
    ///
    /// # Errors
    /// [`PageError::Closed`] if the document was closed before the call.
    #[doc(hidden)]
    pub fn with_hayro_page<R>(&self, f: impl FnOnce(RenderInput<'_>) -> R) -> Result<R, PageError> {
        let limits = self.limits();
        self.with_loaded(|loaded, index| {
            let xref = loaded.pdf.xref();
            let page = loaded.index.hayro_page(xref, index);
            f(RenderInput {
                index,
                page: page.as_ref(),
                xref,
                limits,
            })
        })
    }
}

impl fmt::Debug for Page {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Page")
            .field("index", &self.index)
            .finish_non_exhaustive()
    }
}

#[cfg(all(test, feature = "render-support"))]
#[path = "page_tests.rs"]
mod tests;
