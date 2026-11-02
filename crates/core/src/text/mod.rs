//! The text path: from a page to its text (design spec §6.1-§6.5; run spec
//! T19-T37, §4.9, §4.10).
//!
//! `Page::extract_chars` (in `extract`) interprets one page with hayro into
//! the `TextDevice` (in `device`), which filters, de-duplicates and maps each
//! glyph to Unicode (`unicode`, backed by the data tables `agl` and
//! `mac_glyphs`) and returns `CharRecord`s (`record`) in content-stream
//! order. The line assembler (`layout`) turns those into blocks of lines,
//! and `page` wraps them in the public [`TextPage`], serialises them and
//! provides `Page::text_page` / `Page::text`. `options` holds the public
//! [`TextOptions`].
//!
//! Besides the records, a page yields boundary markers (Form XObjects and
//! annotations, T32) for the assembler's blocks, and warnings built from the
//! fork's signals: content the lexer skipped (`ContentDamaged`, T35), fonts
//! drawn with a substitute (`FontLoadFailed`, T37), and the limits.
//!
//! With the `render-support` feature, `ocr` builds a [`TextPage`] from an OCR
//! engine's word boxes instead (run 0002 O17, O18), so both sources share one
//! model, serialisation and sanitising.

mod agl;
mod details;
mod device;
mod extract;
mod layout;
mod mac_glyphs;
#[cfg(feature = "render-support")]
mod ocr;
mod options;
mod page;
mod record;
mod unicode;

#[cfg(feature = "render-support")]
pub use details::{
    CONTENT_DAMAGED_DETAIL, FONT_SUBSTITUTED_DETAIL, FORM_CYCLE_DETAIL,
    IMAGE_DECODE_FAILURE_DETAIL, IMAGE_MEMORY_LIMIT_DETAIL, INTERPRETER_NESTING_DETAIL,
    OBJECT_DEPTH_DETAIL, UNSUPPORTED_FONT_DETAIL, image_pixel_limit_detail,
    stream_decode_limit_detail,
};
#[cfg(feature = "render-support")]
pub use ocr::TextPageBuilder;
pub use options::{BlockSeparator, LayoutParams, TextOptions, Unmapped};
pub use page::{Block, Char, Line, TextPage, TextSource};
// The font-to-Unicode fallbacks, for the crate-level `fuzz_api` only.
#[cfg(feature = "fuzzing")]
pub(crate) use unicode::{
    agl_lookup, decode_text_string, kp15_position, name_to_unicode, sanitise, unicode_from_name,
    unmapped_text,
};

#[cfg(test)]
mod tests;
