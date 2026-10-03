//! The starting point for reading PDF files.

use crate::PdfData;
use crate::filter::{DecodeBudget, DecodeLimits, DecodeStats};
use crate::object::Object;
use crate::object::ObjectIdentifier;
use crate::object::dict::Dict;
use crate::page::Pages;
use crate::page::cached::CachedPages;
use crate::reader::{Reader, ReaderContext};
use crate::sync::{Arc, OnceLock};
use crate::xref::{PagesLocation, XRef, XRefError, fallback, root_xref};

pub use crate::crypto::DecryptionError;
use crate::metadata::Metadata;

/// A PDF file.
pub struct Pdf {
    xref: Arc<XRef>,
    header_version: PdfVersion,
    // Built on the first call to `pages()`, not when the document is opened:
    // walking the tree resolves every page dictionary and holds a `Page` for
    // each, which dominates the cost of opening a large document and is
    // wasted on callers that only need a few pages (or build them with
    // `Page::from_dict`). Once built, the pages live as long as the `Pdf`.
    pages: OnceLock<CachedPages>,
    data: PdfData,
}

/// The root of a document's page tree, as returned by [`Pdf::pages_root`].
///
/// Fork change H11: the catalog's `/Pages` entry is normally a reference, but
/// it may also be a dictionary written directly in the catalog. The two are
/// kept apart (rather than an `Option` of an identifier) because an inline
/// root is a valid page tree, not a missing one.
#[derive(Debug, Clone)]
pub enum PagesRoot<'a> {
    /// `/Pages` references this object.
    Ref(ObjectIdentifier),
    /// `/Pages` is this dictionary, written inline in the catalog.
    Inline(Dict<'a>),
}

impl PagesRoot<'_> {
    /// The root's object identifier, or `None` for an inline root.
    pub fn id(&self) -> Option<ObjectIdentifier> {
        match self {
            Self::Ref(id) => Some(*id),
            Self::Inline(_) => None,
        }
    }
}

/// An error that occurred while loading a PDF file.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LoadPdfError {
    /// An error occurred while processing an encrypted document.
    Decryption(DecryptionError),
    /// The PDF was invalid or could not be parsed due to some other unknown reason.
    Invalid,
}

#[allow(clippy::len_without_is_empty)]
impl Pdf {
    /// Try to read the given PDF file.
    ///
    /// Returns `Err` if it was unable to read it.
    pub fn new(data: impl Into<PdfData>) -> Result<Self, LoadPdfError> {
        Self::new_with_password(data, "")
    }

    /// Try to read the given PDF file with a password.
    ///
    /// Returns `Err` if it was unable to read it or if the password is incorrect.
    pub fn new_with_password(
        data: impl Into<PdfData>,
        password: &str,
    ) -> Result<Self, LoadPdfError> {
        Self::new_with_options(data, password, DecodeLimits::default())
    }

    /// Try to read the given PDF file with a password, decoding its streams
    /// within `limits`.
    ///
    /// Returns `Err` if it was unable to read it or if the password is incorrect.
    pub fn new_with_options(
        data: impl Into<PdfData>,
        password: &str,
        limits: DecodeLimits,
    ) -> Result<Self, LoadPdfError> {
        let data = data.into();
        let password = password.as_bytes();
        let version = find_version(data.as_ref()).unwrap_or(PdfVersion::Pdf10);
        let budget = Arc::new(DecodeBudget::new(limits));
        let xref = match root_xref(data.clone(), password, budget.clone()) {
            Ok(x) => x,
            Err(e) => match e {
                XRefError::Unknown => {
                    fallback(data.clone(), password, budget).ok_or(LoadPdfError::Invalid)?
                }
                XRefError::Encryption(e) => return Err(LoadPdfError::Decryption(e)),
            },
        };
        let xref = Arc::new(xref);

        // The page tree is not read here (see `pages`), so a document whose
        // tree is unreadable opens and has no pages. The catalog and its
        // `/Pages` entry (a reference or, since H11, an inline dictionary)
        // are still required: resolving the trailer above fails without them.
        Ok(Self {
            xref,
            header_version: version,
            pages: OnceLock::new(),
            data,
        })
    }

    /// Return the number of objects present in the PDF file.
    pub fn len(&self) -> usize {
        self.xref.len()
    }

    /// Return an iterator over all objects defined in the PDF file.
    pub fn objects(&self) -> impl IntoIterator<Item = Object<'_>> {
        self.xref.objects()
    }

    /// Return the version of the PDF file.
    pub fn version(&self) -> PdfVersion {
        self.xref
            .trailer_data()
            .version
            .unwrap_or(self.header_version)
    }

    /// Return the underlying data of the PDF file.
    pub fn data(&self) -> &PdfData {
        &self.data
    }

    /// Return the pages of the PDF file.
    ///
    /// The page tree is walked on the first call and the result is cached, so
    /// the first call costs time and memory proportional to the number of
    /// pages. If neither the page tree nor a search for page objects yields a
    /// page, the result is empty.
    pub fn pages(&self) -> &Pages<'_> {
        self.pages
            .get_or_init(|| CachedPages::new(self.xref.clone()))
            .get()
    }

    /// Return whether the pages have been built, i.e. whether
    /// [`Pdf::pages`] has been called.
    pub fn pages_built(&self) -> bool {
        self.pages.get().is_some()
    }

    /// Return the root of the page tree (the catalog's `/Pages` entry): the
    /// object it references or, when the catalog writes the root inline, the
    /// dictionary itself.
    ///
    /// Together with [`XRef::get`] and [`Page::from_dict`](crate::page::Page::from_dict),
    /// this lets a caller walk the page tree on its own terms instead of
    /// calling [`Pdf::pages`]. An inline root has no identifier, so a caller
    /// that deduplicates nodes by identifier cannot do so for it (nothing can
    /// refer back to it either).
    pub fn pages_root(&self) -> PagesRoot<'_> {
        match self.xref.trailer_data().pages {
            PagesLocation::Ref(id) => PagesRoot::Ref(id),
            // Read again from the catalog, which `Pdf::new` already parsed;
            // an empty dictionary (no `/Kids`, so no pages) stands in if a
            // lazy repair has since replaced the catalog.
            PagesLocation::Inline => PagesRoot::Inline(
                self.xref
                    .inline_pages_root(&ReaderContext::new(&self.xref, false))
                    .unwrap_or_default(),
            ),
        }
    }

    /// Return whether the xref table was broken and had to be rebuilt by
    /// scanning the file.
    ///
    /// A broken table is usually detected while opening, but can also be
    /// detected later, the first time an object is not found where the table
    /// says it is; the result reflects the state at the time of the call.
    pub fn xref_was_repaired(&self) -> bool {
        self.xref.is_repaired()
    }

    /// Return whether the document is encrypted.
    ///
    /// An encrypted document that opened (with an empty password when only
    /// an owner password is set, or with the right password) returns `true`;
    /// its objects are decrypted transparently when read.
    pub fn is_encrypted(&self) -> bool {
        self.xref.is_encrypted()
    }

    /// Return the xref of the PDF file.
    pub fn xref(&self) -> &XRef {
        &self.xref
    }

    /// Return the metadata in the document information dictionary of the document.
    pub fn metadata(&self) -> &Metadata {
        self.xref.metadata()
    }

    /// Return what has happened so far while decoding the document's streams,
    /// such as how many hit the [`DecodeLimits`] it was opened with.
    pub fn decode_stats(&self) -> DecodeStats {
        self.xref.decode_stats()
    }
}

fn find_version(data: &[u8]) -> Option<PdfVersion> {
    let data = &data[..data.len().min(2000)];
    let mut r = Reader::new(data);

    while r.forward_tag(b"%PDF-").is_none() {
        r.read_byte()?;
    }

    PdfVersion::from_bytes(r.tail()?)
}

/// The version of a PDF document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PdfVersion {
    /// PDF 1.0.
    Pdf10,
    /// PDF 1.1.
    Pdf11,
    /// PDF 1.2.
    Pdf12,
    /// PDF 1.3.
    Pdf13,
    /// PDF 1.4.
    Pdf14,
    /// PDF 1.5.
    Pdf15,
    /// PDF 1.6.
    Pdf16,
    /// PDF 1.7.
    Pdf17,
    /// PDF 2.0.
    Pdf20,
}

impl PdfVersion {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Option<Self> {
        match bytes.get(..3)? {
            b"1.0" => Some(Self::Pdf10),
            b"1.1" => Some(Self::Pdf11),
            b"1.2" => Some(Self::Pdf12),
            b"1.3" => Some(Self::Pdf13),
            b"1.4" => Some(Self::Pdf14),
            b"1.5" => Some(Self::Pdf15),
            b"1.6" => Some(Self::Pdf16),
            b"1.7" => Some(Self::Pdf17),
            b"2.0" => Some(Self::Pdf20),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::pdf::{Pdf, PdfVersion};

    #[test]
    fn issue_49() {
        let _ = Pdf::new(Vec::new());
    }

    #[test]
    fn pdf_version_header() {
        let data = std::fs::read("../hayro-tests/downloads/pdfjs/alphatrans.pdf").unwrap();
        let pdf = Pdf::new(data).unwrap();

        assert_eq!(pdf.version(), PdfVersion::Pdf17);
    }

    #[test]
    fn pdf_version_catalog() {
        let data = std::fs::read("../hayro-tests/downloads/pdfbox/2163.pdf").unwrap();
        let pdf = Pdf::new(data).unwrap();

        assert_eq!(pdf.version(), PdfVersion::Pdf14);
    }
}
