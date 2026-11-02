//! The `detail` text of the warnings the text path and the renderer both
//! raise (spec 0002 O3, O4).
//!
//! One definition per detail, used by `extract.rs` here and, through the
//! doc-hidden `render_support` module (feature `render-support`), by
//! `pdf-extract-render`'s `signals.rs`. A warning therefore reads the same
//! whichever path raised it by construction, not by two literals kept in step
//! by hand. Python callers and tests match on the `kind: ` prefix and a few
//! leading words of these, so a reword keeps those.

use crate::limits::Limits;

/// `ObjectDepth`: the parser's nesting cap (H2a).
pub const OBJECT_DEPTH_DETAIL: &str = "nesting limit: an over-deep object was treated as missing";

/// `StreamDecodeLimit` (H10f): names the cap so a reader knows which limit to
/// raise.
#[must_use]
pub fn stream_decode_limit_detail(limits: &Limits) -> String {
    format!(
        "decoded size limit: a stream over {} bytes was treated as absent",
        limits.max_stream_decoded_bytes
    )
}

/// `ContentDamaged` (H10a, T35).
pub const CONTENT_DAMAGED_DETAIL: &str =
    "damaged: bytes that start no operator or operand were skipped";

/// `FontLoadFailed` (H10e, T37).
pub const FONT_SUBSTITUTED_DETAIL: &str =
    "substituted: a font that could not be loaded was drawn with a standard font";

/// `InterpreterNesting`: the interpreter's content-nesting cap.
pub const INTERPRETER_NESTING_DETAIL: &str =
    "nesting limit: nested content beyond 50 levels was skipped";

/// `FormXObjectCycle`.
pub const FORM_CYCLE_DETAIL: &str = "a form XObject that draws itself was skipped";

/// `UnsupportedFont`.
pub const UNSUPPORTED_FONT_DETAIL: &str = "unsupported font kind; its glyphs were dropped";

/// `ImageDecodeFailure` (run 0002 O4). The text device asks for no pixels, so
/// only the renderer raises it in practice.
pub const IMAGE_DECODE_FAILURE_DETAIL: &str =
    "decode failed: an image could not be decoded and was skipped";

/// `ImagePixelLimit` (run 0002 O4): the cap, so a reader can tell which limit
/// to raise. H13 refuses an image at three points (as declared, as a codec
/// reports it from its header, and as decoded), so the wording does not claim
/// one of them.
#[must_use]
pub fn image_pixel_limit_detail(limits: &Limits) -> String {
    format!(
        "pixel limit: an image over {} pixels (declared, reported by its codec, or decoded) was skipped",
        limits.max_image_pixels
    )
}

/// `ImageMemoryLimit` (run 0003 F8). The budget is derived from
/// `max_image_pixels`, so that is the limit to raise.
pub const IMAGE_MEMORY_LIMIT_DETAIL: &str = concat!(
    "memory limit: an image whose decoder would exceed its memory budget ",
    "(set by max_image_pixels) was skipped"
);
