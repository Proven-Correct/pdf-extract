//! Error types (spec §3.3 `error.rs`, §2.2 trust boundary, T47b).
//!
//! Two enums for the two boundaries a Rust caller crosses: [`OpenError`] for
//! everything that can go wrong turning bytes into a [`Document`], and
//! [`PageError`] for per-page operations. OCR adds a third, [`OcrError`]
//! (run 0002 §1.2 O15), split by *who* can fix the failure: the host
//! ([`OcrUnavailable`]: no model data, an engine that will not start) or the
//! page ([`OcrPageError`]: this page failed, the next may not). The engine
//! itself lives in `pdf-extract-ocr`; the vocabulary lives here so that
//! `Document::extract` and the Python binding can name it without depending
//! on the Tesseract build (run 0002 D-2). Malformed content *inside* an
//! opened document is not an error: it surfaces as a [`Warning`] and partial
//! output (spec §2.2), so these enums stay small and each variant maps to one
//! Python exception class.
//!
//! There is no `Internal`/panic variant: the Rust API lets panics propagate
//! and only the Python boundary catches them (D-16).
//!
//! [`Document`]: crate::Document
//! [`Warning`]: crate::Warning

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::limits::LimitKind;

/// Why a document could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The path does not exist.
    #[error("file not found: {}", .0.display())]
    NotFound(PathBuf),
    /// Any other I/O failure while reading the file.
    #[error("I/O error while reading the document: {0}")]
    Io(#[from] std::io::Error),
    /// The input has zero bytes.
    #[error("the document is empty (zero bytes)")]
    Empty,
    /// The input is larger than `Limits::max_file_size`; it was not read.
    #[error("the document is {size} bytes, over the limit of {limit} bytes")]
    TooLarge {
        /// Size of the input, in bytes.
        size: u64,
        /// The configured limit, in bytes.
        limit: u64,
    },
    /// No `%PDF-` header in the first 1024 bytes. Detection looks only at
    /// content, never at the file name (spec T3).
    #[error("not a PDF: no %PDF- header in the first 1024 bytes")]
    NotPdf,
    /// The input has a PDF header but could not be parsed.
    #[error("corrupt document: {0}")]
    Corrupt(String),
    /// The document is encrypted with a user password. Password
    /// authentication is not supported in this release.
    #[error("the document is encrypted and requires a password")]
    PasswordRequired,
    /// The document is encrypted with a scheme the parser does not support.
    #[error("unsupported encryption: {0}")]
    UnsupportedEncryption(String),
    /// The page tree has more leaves than `OpenOptions::max_pages`. The walk
    /// stopped at the first leaf over the limit, so only a lower bound is
    /// known.
    #[error("the document has at least {at_least} pages, over the limit of {limit}")]
    TooManyPages {
        /// The configured `max_pages`.
        limit: usize,
        /// Pages found before the walk stopped (`limit + 1`).
        at_least: usize,
    },
    /// A document-level limit other than size or page count was exceeded
    /// (in this release, `Limits::max_objects`). The Python binding reports
    /// it as `CorruptDocumentError` naming the limit (spec §10).
    #[error("limit {kind} exceeded: the document has {value}")]
    Limit {
        /// Which limit.
        kind: LimitKind,
        /// The observed value that exceeded it.
        value: u64,
    },
}

/// Why a per-page operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PageError {
    /// [`Document::close`](crate::Document::close) was called before the
    /// operation started.
    #[error("the document is closed")]
    Closed,
    /// The page index is not below the page count.
    #[error("page index {index} is out of range for a document with {count} pages")]
    IndexOutOfRange {
        /// The requested index.
        index: usize,
        /// The document's page count.
        count: usize,
    },
    /// A per-page budget ran out (operators or time).
    #[error("limit {kind} exceeded while processing the page ({value})")]
    Limit {
        /// Which limit.
        kind: LimitKind,
        /// The configured limit (operators, or milliseconds for time).
        value: u64,
    },
}

/// Why an OCR operation on a page failed.
///
/// The variants are the three ways a caller reacts: give up on OCR for the
/// whole run ([`Unavailable`](Self::Unavailable)), give up on this page
/// ([`Page`](Self::Page)), or stop because the document went away
/// ([`Closed`](Self::Closed)).
#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    /// OCR cannot run in this process at all; retrying another page will
    /// fail the same way.
    #[error(transparent)]
    Unavailable(#[from] OcrUnavailable),
    /// This page could not be recognised.
    #[error(transparent)]
    Page(#[from] OcrPageError),
    /// [`Document::close`](crate::Document::close) was called before the
    /// operation started.
    #[error("the document is closed")]
    Closed,
}

/// The OCR engine cannot run on this host (run 0002 O7, O15).
///
/// Raised once, when an engine is built (or, for a pool member created
/// later, by the page that needed it); never a property of the document.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("OCR unavailable ({reason}): {detail}")]
pub struct OcrUnavailable {
    /// The category of the failure.
    pub reason: UnavailableReason,
    /// The model directory that was chosen, if resolution got that far.
    pub tessdata: Option<PathBuf>,
    /// The languages that were requested.
    pub languages: Vec<String>,
    /// Every directory tried while resolving the model directory, in order.
    pub searched: Vec<PathBuf>,
    /// Human-readable detail (the engine's own message where there is one).
    pub detail: String,
}

/// The category of an [`OcrUnavailable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnavailableReason {
    /// No model directory holds the first requested language.
    NoTessdata,
    /// The chosen model directory lacks one of the other languages.
    LanguageDataMissing,
    /// The engine refused to start (bad configuration, unreadable model,
    /// or a pool member that could not be created).
    InitFailed,
    /// The engine, or its helper process, is not part of this build or
    /// installation.
    EngineNotBuilt,
}

impl UnavailableReason {
    /// The snake-case name the Python binding reports as
    /// `OcrUnavailableError.reason` (`"no_tessdata"`, ...), so the two never
    /// drift.
    pub fn name(self) -> &'static str {
        match self {
            Self::NoTessdata => "no_tessdata",
            Self::LanguageDataMissing => "language_data_missing",
            Self::InitFailed => "init_failed",
            Self::EngineNotBuilt => "engine_not_built",
        }
    }
}

impl fmt::Display for UnavailableReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// OCR of one page failed; other pages may still succeed (run 0002 O15).
///
/// A recognition that runs and finds no text is *not* an error: it is an
/// empty `TextPage`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OcrPageError {
    /// The page could not be rasterised.
    #[error("rendering failed: {0}")]
    Render(String),
    /// The engine failed on the rasterised page (including a crashed or
    /// misbehaving helper process).
    #[error("recognition failed: {0}")]
    Recognition(String),
    /// Rendering and recognition together took longer than the engine's
    /// page timeout, which this carries.
    #[error("OCR exceeded the page timeout of {0:?}")]
    Timeout(Duration),
    /// A document limit ran out while rendering the page; the partial raster
    /// was not recognised.
    #[error("limit {kind} exceeded ({value})")]
    Limit {
        /// Which limit.
        kind: LimitKind,
        /// The configured limit (operators, pixels, or milliseconds for
        /// time).
        value: u64,
    },
}

impl From<PageError> for OcrError {
    /// `Closed` stays `Closed` and a page budget becomes
    /// [`OcrPageError::Limit`]. `IndexOutOfRange` cannot reach OCR (it
    /// operates on a `Page`, whose index is valid by construction); it is
    /// mapped to `Render` with its message rather than panicking, so a
    /// future caller that does produce one gets an error, not an abort.
    fn from(e: PageError) -> Self {
        match e {
            PageError::Closed => Self::Closed,
            PageError::Limit { kind, value } => Self::Page(OcrPageError::Limit { kind, value }),
            other @ PageError::IndexOutOfRange { .. } => {
                Self::Page(OcrPageError::Render(other.to_string()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_name_what_went_wrong() {
        let cases: Vec<(OpenError, &str)> = vec![
            (OpenError::NotFound(PathBuf::from("/x/y.pdf")), "/x/y.pdf"),
            (
                OpenError::Io(std::io::Error::other("disk on fire")),
                "disk on fire",
            ),
            (OpenError::Empty, "empty"),
            (OpenError::TooLarge { size: 10, limit: 5 }, "10 bytes"),
            (OpenError::NotPdf, "%PDF-"),
            (OpenError::Corrupt("bad xref".into()), "bad xref"),
            (OpenError::PasswordRequired, "password"),
            (OpenError::UnsupportedEncryption("V 9".into()), "V 9"),
            (
                OpenError::TooManyPages {
                    limit: 10,
                    at_least: 11,
                },
                "at least 11",
            ),
            (
                OpenError::Limit {
                    kind: LimitKind::Objects,
                    value: 7,
                },
                "objects",
            ),
        ];
        for (err, needle) in cases {
            let msg = err.to_string();
            assert!(msg.contains(needle), "{msg:?} lacks {needle:?}");
        }
        for (err, needle) in [
            (PageError::Closed, "closed"),
            (PageError::IndexOutOfRange { index: 3, count: 2 }, "3"),
            (
                PageError::Limit {
                    kind: LimitKind::OpsPerPage,
                    value: 9,
                },
                "ops_per_page",
            ),
        ] {
            let msg = err.to_string();
            assert!(msg.contains(needle), "{msg:?} lacks {needle:?}");
        }
    }

    fn unavailable(reason: UnavailableReason) -> OcrUnavailable {
        OcrUnavailable {
            reason,
            tessdata: Some(PathBuf::from("/td")),
            languages: vec!["eng".into()],
            searched: vec![PathBuf::from("/a")],
            detail: "the detail".into(),
        }
    }

    /// Run 0002 §4.1: every OCR variant's message names its cause.
    #[test]
    fn ocr_messages_name_the_cause() {
        let cases: Vec<(OcrError, &[&str])> = vec![
            (
                unavailable(UnavailableReason::NoTessdata).into(),
                &["OCR unavailable", "no_tessdata", "the detail"],
            ),
            (
                unavailable(UnavailableReason::LanguageDataMissing).into(),
                &["language_data_missing"],
            ),
            (
                unavailable(UnavailableReason::InitFailed).into(),
                &["init_failed"],
            ),
            (
                unavailable(UnavailableReason::EngineNotBuilt).into(),
                &["engine_not_built"],
            ),
            (
                OcrPageError::Render("bad raster".into()).into(),
                &["rendering failed", "bad raster"],
            ),
            (
                OcrPageError::Recognition("helper exited: 3".into()).into(),
                &["recognition failed", "helper exited: 3"],
            ),
            (
                OcrPageError::Timeout(Duration::from_secs(60)).into(),
                &["page timeout", "60s"],
            ),
            (
                OcrPageError::Limit {
                    kind: LimitKind::ImagePixels,
                    value: 5,
                }
                .into(),
                &["image_pixels", "5"],
            ),
            (OcrError::Closed, &["closed"]),
        ];
        for (err, needles) in cases {
            let msg = err.to_string();
            for needle in needles {
                assert!(msg.contains(needle), "{msg:?} lacks {needle:?}");
            }
        }
    }

    #[test]
    fn transparent_variants_show_the_inner_message() {
        let u = unavailable(UnavailableReason::InitFailed);
        assert_eq!(OcrError::from(u.clone()).to_string(), u.to_string());
        let p = OcrPageError::Render("x".into());
        assert_eq!(OcrError::from(p.clone()).to_string(), p.to_string());
    }

    #[test]
    fn unavailable_reason_names_are_snake_case() {
        let all = [
            (UnavailableReason::NoTessdata, "no_tessdata"),
            (
                UnavailableReason::LanguageDataMissing,
                "language_data_missing",
            ),
            (UnavailableReason::InitFailed, "init_failed"),
            (UnavailableReason::EngineNotBuilt, "engine_not_built"),
        ];
        for (reason, name) in all {
            assert_eq!(reason.name(), name);
            assert_eq!(reason.to_string(), name);
        }
    }

    #[test]
    fn page_errors_convert_to_ocr_errors() {
        assert!(matches!(
            OcrError::from(PageError::Closed),
            OcrError::Closed
        ));
        assert!(matches!(
            OcrError::from(PageError::Limit {
                kind: LimitKind::PageTimeBudget,
                value: 30_000,
            }),
            OcrError::Page(OcrPageError::Limit {
                kind: LimitKind::PageTimeBudget,
                value: 30_000,
            })
        ));
        match OcrError::from(PageError::IndexOutOfRange { index: 4, count: 2 }) {
            OcrError::Page(OcrPageError::Render(msg)) => assert!(msg.contains("4"), "{msg}"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn io_errors_convert() {
        let e: OpenError = std::io::Error::other("x").into();
        assert!(matches!(e, OpenError::Io(_)));
    }
}
