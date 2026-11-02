//! Core library of `pdf-extract`: opening PDFs and extracting their text layer.
//!
//! Opens documents, indexes their pages, and extracts each page's text layer
//! in the reference extractor's `'text'` shape.
//!
//! ```
//! use pdf_extract_core::{Document, OpenOptions, TextOptions};
//! # let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),
//! #     "/../../tests/fixtures/corpus-c/enc_aes_128_owner.pdf")).unwrap();
//! let doc = Document::from_bytes(bytes, &OpenOptions::default())?;
//! for page in doc.pages() {
//!     let text = page.text(&TextOptions::default())?;
//!     println!("page {}: {} bytes of text", page.index(), text.len());
//! }
//! doc.close();
//! assert_eq!(doc.page_count(), 1); // still answers after close
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Module map (internal modules are private; their headers explain them):
//!
//! - `limits`, `error`, `warning`: the public vocabulary, including OCR's
//!   errors (the OCR engine itself is in `pdf-extract-ocr`).
//! - `source`: reading files and content-only type detection.
//! - `open`: the open pipeline, including how hayro's out-of-band signals
//!   become warnings.
//! - `index`: the page-tree walk and the per-page records it keeps.
//! - `document`, `page`: the handles; `close_cell`: the close protocol
//!   they share; `sync`: the `std`/loom shim it is built on (`--cfg loom`
//!   builds re-export `CloseCell` for tests/loom_close.rs).
//! - `text`: the text path: the text device (page content to character
//!   records), the line assembler, and the public `TextPage`/`TextOptions`.
//! - `extract`: the options and results of the batch helper and the
//!   [`PageRecognizer`] seam it calls OCR through.
//! - [`parity`] (public): page-by-page agreement metrics between a
//!   reference extractor's text and ours, for migrations (design spec §13.2).
//!
//! The cargo feature `render-support` adds three `#[doc(hidden)]` hooks for
//! the rendering and OCR crates only: `Page::with_hayro_page` (with
//! `RenderInput`), `ocr_support::TextPageBuilder`, and `render_support`, the
//! warning details the renderer shares with the text path. The
//! first two expose hayro types and the internal page model, which the
//! documented API never does.
//!
//! The cargo feature `test-hooks` adds the `#[doc(hidden)]`
//! `arm_codec_panic_for_test` for the Python binding's test hook.
//!
//! The cargo feature `fuzzing` adds the `#[doc(hidden)]` module `fuzz_api`:
//! the tightened fuzzing limits and the drivers shared by the `fuzz/` targets
//! and the regression replay tests.
//!
//! Panics are not caught here (D-16); the Python binding catches them. The
//! one exception is a panic inside an image codec, which the fork's codec
//! boundary (H18) turns into an `image: decode failed` warning; this crate
//! never installs or replaces the process's panic hook.

// The safety story of the whole project rests on this crate being free of
// `unsafe`; forbidding it (rather than denying) means it cannot be overridden
// locally with an `allow`. The library as built for every other crate, for
// doctests and for the integration tests is `forbid`. Only the unit-test
// build (`cfg(test)`) relaxes it to `deny`, so that one explicitly
// `#[allow]`ed test module can include the measuring small-stack runner,
// which needs pthread FFI (ADR 0089).
#![cfg_attr(not(test), forbid(unsafe_code))]
#![cfg_attr(test, deny(unsafe_code))]

mod close_cell;
mod document;
mod error;
mod extract;
#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub mod fuzz_api;
mod index;
mod limits;
mod open;
mod page;
pub mod parity;
mod source;
mod sync;
mod text;
mod warning;

pub use document::{Document, OpenOptions, Pages};
pub use error::{OcrError, OcrPageError, OcrUnavailable, OpenError, PageError, UnavailableReason};
pub use extract::{ExtractOptions, Extraction, OcrStatus, PageRecognizer, PageResult, PageSource};
pub use limits::{LimitKind, Limits};
pub use page::Page;
#[cfg(feature = "render-support")]
#[doc(hidden)]
pub use page::RenderInput;
#[doc(hidden)]
pub use text::{Block, Char, Line};
pub use text::{BlockSeparator, LayoutParams, TextOptions, TextPage, TextSource, Unmapped};
pub use warning::{Warning, WarningKind};

// The close protocol's cell, reachable only from the loom build so that
// tests/loom_close.rs model-checks the shipped code (design D5).
#[cfg(loom)]
#[doc(hidden)]
pub use close_cell::CloseCell;

/// Building an OCR [`TextPage`] from word boxes; for `pdf-extract-ocr` only
/// (feature `render-support`). Not a stable interface.
#[cfg(feature = "render-support")]
#[doc(hidden)]
pub mod ocr_support {
    pub use crate::text::TextPageBuilder;
}

/// The warning details `pdf-extract-render` shares with the text path, so
/// a warning reads the same from both (feature `render-support`). Not a
/// stable interface.
#[cfg(feature = "render-support")]
#[doc(hidden)]
pub mod render_support {
    pub use crate::text::{
        CONTENT_DAMAGED_DETAIL, FONT_SUBSTITUTED_DETAIL, FORM_CYCLE_DETAIL,
        IMAGE_DECODE_FAILURE_DETAIL, IMAGE_MEMORY_LIMIT_DETAIL, INTERPRETER_NESTING_DETAIL,
        OBJECT_DEPTH_DETAIL, UNSUPPORTED_FONT_DETAIL, image_pixel_limit_detail,
        stream_decode_limit_detail,
    };
}

/// Make the next image-codec decode on this thread panic inside the fork's
/// codec boundary (H18), as a codec bug would; the image is then skipped with
/// an `image: decode failed` warning. Feature `test-hooks`, for the Python
/// binding's `_arm_codec_panic_for_test`. Not a stable interface.
#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn arm_codec_panic_for_test() {
    hayro_syntax::arm_codec_panic_for_test();
}

/// Version of the `pdf-extract-core` crate, as declared in `Cargo.toml`.
///
/// The Python binding re-exports this as `pdf_extract.__version__` so the two
/// can never drift.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn version_is_semver_shaped() {
        let parts: Vec<&str> = VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "unexpected version {VERSION:?}");
        assert!(parts.iter().all(|p| p.parse::<u32>().is_ok()));
    }
}
