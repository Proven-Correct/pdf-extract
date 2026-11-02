//! The text device's output: one [`CharRecord`] per emitted glyph, in
//! content-stream order (design spec §6.1), plus the page's warnings.
//!
//! Geometry is stored as `f32`: page coordinates are at most a few times
//! 10^4 points, where `f32` still resolves well below a hundredth of a point,
//! and a page may hold up to `max_chars_per_page` (two million) records, so
//! the halved size matters. All values are in *page space*: PDF user space
//! after the CTM and the page's `initial_transform(false)` (crop-box origin,
//! `/Rotate` applied, y up).

use std::fmt;

use crate::warning::Warning;

/// Longest text (in UTF-8 bytes) stored without a heap allocation. A glyph is
/// almost always one code point (at most four bytes); 22 keeps the enum at 24
/// bytes, the size of a `String`, while covering most ToUnicode ligatures.
const INLINE_CAP: usize = 22;

/// The text of one record: one or more code points (a ToUnicode entry or an
/// `/ActualText` span can map one glyph to several).
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum CharText {
    /// Short text, held in place. Invariant: `bytes[..len]` is valid UTF-8.
    Inline { len: u8, bytes: [u8; INLINE_CAP] },
    /// Longer text.
    Heap(Box<str>),
}

impl CharText {
    /// Stores `s`.
    pub(crate) fn new(s: &str) -> Self {
        match u8::try_from(s.len()) {
            Ok(len) if s.len() <= INLINE_CAP => {
                let mut bytes = [0; INLINE_CAP];
                bytes[..s.len()].copy_from_slice(s.as_bytes());
                Self::Inline { len, bytes }
            }
            _ => Self::Heap(s.into()),
        }
    }

    /// Stores one code point.
    pub(crate) fn from_char(c: char) -> Self {
        Self::new(c.encode_utf8(&mut [0; 4]))
    }

    /// The text.
    pub(crate) fn as_str(&self) -> &str {
        match self {
            // Built only from a `&str` in `new`, so the bytes are UTF-8.
            Self::Inline { len, bytes } => std::str::from_utf8(&bytes[..usize::from(*len)])
                .expect("inline text is copied from a str"),
            Self::Heap(s) => s,
        }
    }

    /// Number of code points; what `max_chars_per_page` counts.
    pub(crate) fn char_count(&self) -> usize {
        self.as_str().chars().count()
    }
}

impl fmt::Debug for CharText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

/// One emitted glyph (design spec §6.1).
///
/// The line assembler (unit 10) needs only these fields: where the glyph
/// starts, which way its baseline runs, how big it is and how far it moves
/// the pen. Hidden-content filtering, de-duplication and Unicode mapping have
/// already happened when a record exists.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CharRecord {
    /// The glyph's text (never empty).
    pub(crate) text: CharText,
    /// Baseline start, page space.
    pub(crate) origin: (f32, f32),
    /// Unit vector along the baseline: the text matrix's x axis after the
    /// CTM, whatever the writing mode (T29: vertical text therefore gets one
    /// line per glyph, as the reference does, KP12).
    pub(crate) dir: (f32, f32),
    /// Effective font size: the height of the text-space em square measured
    /// perpendicular to `dir`, so horizontal scaling (`Tz`) does not change
    /// it.
    pub(crate) size: f32,
    /// Distance the glyph's own width moves the pen along `dir`, page units
    /// (from `/Widths`, `/W` or the font). Character and word spacing and
    /// `TJ` offsets are not included: they show up in the next record's
    /// origin. For an `/ActualText` span it is the sum of the advances of the
    /// span's glyphs (T28: later glyphs emit nothing but still advance the
    /// pen). Zero for
    /// vertical writing, whose advance runs across `dir`.
    pub(crate) advance: f32,
    /// Drawn with `GlyphDrawMode::Invisible`: text render mode 3 or 7.
    pub(crate) invisible: bool,
}

/// A Form XObject or annotation boundary (T32), seen through hayro's device
/// hooks (fork change H10d).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Boundary {
    FormStart,
    FormEnd,
    AnnotationStart,
    AnnotationEnd,
}

/// A boundary in the record stream. Markers are kept beside the records
/// rather than among them, so that the two million records a page may hold
/// stay one compact array; `at` places the marker between records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Marker {
    /// Index of the first record after the boundary (the number of records
    /// before it); `chars.len()` for a boundary after the last record.
    pub(crate) at: usize,
    pub(crate) boundary: Boundary,
}

/// What [`extract_chars`](crate::Page::extract_chars) produced for one page.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PageChars {
    /// The emitted glyphs, in content-stream order (annotations after the
    /// page content).
    pub(crate) chars: Vec<CharRecord>,
    /// Form and annotation boundaries, in order (only those with records
    /// inside; see the text device).
    pub(crate) markers: Vec<Marker>,
    /// Problems worked around on this page, one per kind.
    pub(crate) warnings: Vec<Warning>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_inline_up_to_the_cap_and_heap_beyond() {
        for s in [
            "",
            "A",
            "é",
            "日本",
            "\u{1F600}",
            "ffi",
            "a".repeat(22).as_str(),
        ] {
            let t = CharText::new(s);
            assert!(matches!(t, CharText::Inline { .. }), "{s:?}");
            assert_eq!(t.as_str(), s);
        }
        let long = "ab".repeat(12);
        let t = CharText::new(&long);
        assert!(matches!(t, CharText::Heap(_)));
        assert_eq!(t.as_str(), long);
        assert_eq!(t.char_count(), 24);
        assert_eq!(CharText::from_char('\u{2603}').as_str(), "\u{2603}");
        assert_eq!(CharText::new("日本").char_count(), 2);
        assert_eq!(format!("{:?}", CharText::new("x")), "\"x\"");
        assert_eq!(size_of::<CharText>(), 24);
    }
}
