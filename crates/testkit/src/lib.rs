//! Test kit for `pdf-extract`: builds PDF files for tests (spec §3.4, §1.7).
//!
//! - [`PdfBuilder`] / [`PageBuilder`]: well-formed documents with text placed
//!   at exact positions (top-left origin, y down, Helvetica/WinAnsi with AFM
//!   widths from [`text_width`]), plus raw content, CropBox/Rotate, fonts,
//!   Form XObjects, annotations and optional content for compat fixtures.
//! - [`contract`]: everyday defects for the open-contract tests (garbage
//!   after the header, a junk prefix, a broken `startxref`, a truncated tail,
//!   plain text, a catalog without `/Pages`).
//! - [`hostile`]: deliberately hostile files (deep nesting, page-tree shapes,
//!   flate bombs, operator floods, Form XObject fan-out and recursion, huge
//!   page trees), each targeting one resource bound.
//! - [`jbig2`]: hand-assembled JBIG2 streams in the shapes that exhaust a
//!   decoder's memory (run 0003 H14), and a PDF that draws one.
//! - [`jpx`]: hand-assembled JPEG 2000 codestreams in the shapes that exhaust
//!   a decoder's memory (run 0003 H15), and a PDF that draws one.
//! - [`corpus_d`]: the vector source pages of the OCR quality corpus, which
//!   `tools/oracle/mkcorpusd.py` rasterises (run 0002 §3.8).
//! - `ocr` (feature `ocr`): `ScriptedBackend`, a stand-in OCR backend.
//!
//! Output is deterministic: the same calls produce the same bytes. Fixture
//! SHA-256s in `tests/fixtures/manifest.toml` and the Python test kit's golden
//! test both rely on that. Encrypted files are out of scope (the builder never
//! encrypts); those fixtures come from `tools/oracle/mkfixtures.py`.
//!
//! The Rust library never depends on the kit; the Python extension does, to
//! offer the builder as `pdf_extract.testing` (spec T49).

// Same rule as the core crate: the test kit handles hostile bytes in the same
// process as the code under test, so it gets no exemption.
#![forbid(unsafe_code)]

mod afm;
mod builder;
pub mod contract;
pub mod corpus_d;
mod fonts;
pub mod hostile;
pub mod jbig2;
pub mod jpx;
mod mq;
#[cfg(feature = "ocr")]
pub mod ocr;
mod pdfwriter;

pub use afm::{encode_win_ansi, helvetica_ascent_descent, text_width, win_ansi_chars};
pub use builder::{
    Annotation, Appearance, AppearanceStream, DEFAULT_FONT, OcgRef, PageBuilder, PdfBuilder,
    TextStyle,
};
pub use fonts::{
    BaseEncoding, CidFontSubtype, FontDescriptor, FontDict, SimpleEncoding, Type0Font, Type1Font,
    Type3Font, Type3Glyph, to_unicode_cmap,
};

/// Why the builder refused an input.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BuildError {
    /// The character has no glyph in WinAnsiEncoding (this includes control
    /// characters such as tab and newline). The Python test kit maps this to
    /// `ValueError`.
    #[error("character {ch:?} (U+{:04X}) has no glyph in WinAnsiEncoding", u32::from(*ch))]
    NotWinAnsi {
        /// The first offending character.
        ch: char,
    },
    /// Text rendering modes are 0-7.
    #[error("text rendering mode {0} is outside 0-7")]
    RenderMode(u8),
    /// A corpus-D clean file is not the one-image gray scan the noise
    /// builder expects (spec 0002 §3.8), or a built file could not be
    /// written.
    #[error("corpus D: {0}")]
    CorpusD(String),
    /// A coordinate or font size was NaN or infinite.
    #[error("text position and font size must be finite")]
    NonFinite,
}
