//! Building a [`TextPage`] from OCR output (run 0002 §3.3 `ocr_support`,
//! O17, O18). Compiled only with the `render-support` feature and used only
//! by `pdf-extract-ocr`, which owns the engine; the builder lives here because
//! `TextPage`'s model is private to this crate and must stay one shape for
//! both sources.
//!
//! An OCR engine reports blocks of text lines of *words*, each with a box;
//! it has no glyph positions. The builder therefore makes one [`Char`] per
//! code point of a word and splits the word's box evenly along x between
//! them (a word-level box, not glyph-level), and joins the words of a line
//! with one synthetic U+0020 whose advance is the gap between the two words.
//! Coordinates arrive already in page space (points, y up): the caller owns
//! the pixel-to-point transform because only it knows the resolution and the
//! raster's orientation.
//!
//! Empty words, words of only spaces, lines without words and blocks without
//! lines are dropped, so the serialisation invariants of the text path (a
//! line is never empty, a block never empty) hold here too, and a line never
//! holds only the separators the builder itself would insert. "Space" is the
//! text path's set (ADR 0048, `layout::is_space_like`), not
//! `char::is_whitespace`. Text is sanitised with the same D-10 function as
//! the text path, so the invariant is enforced in one place for both sources.

use super::layout::is_space_like;
use super::page::{BBox, Block, Char, Line, TextPage, union};
use super::record::CharText;
use super::unicode::sanitise;
use crate::warning::Warning;

/// The unit baseline direction of every OCR line: the engine reports
/// axis-aligned boxes on an upright raster.
const ACROSS: (f32, f32) = (1.0, 0.0);

/// Fraction of a box's height below the baseline. `Char`'s approximate box
/// runs from −0.2 to +0.8 of its size across the baseline (design spec
/// §6.3), so placing the baseline 0.2 of the height above the bottom edge
/// makes that approximate box reproduce the word box exactly.
const DESCENT: f32 = 0.2;

/// The identity of [`union`]: a box that any real box replaces.
const EMPTY: BBox = [
    f32::INFINITY,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::NEG_INFINITY,
];

/// Builds an OCR [`TextPage`] from word boxes in page space.
///
/// Calls follow the engine's reading order: [`begin_block`](Self::begin_block)
/// opens a block, [`begin_line`](Self::begin_line) a line in it,
/// [`word`](Self::word) adds a word to the open line, and
/// [`finish`](Self::finish) closes everything. A word with no open line (or
/// a line with no open block) opens one implicitly, so no call order panics.
#[derive(Debug)]
pub struct TextPageBuilder {
    dpi: u32,
    warnings: Vec<Warning>,
    blocks: Vec<Block>,
    /// Finished lines of the open block.
    lines: Vec<Line>,
    /// The open line, if any.
    line: Option<OpenLine>,
}

/// A line being filled.
#[derive(Debug)]
struct OpenLine {
    /// The box the engine reported, widened by every word added.
    bbox: BBox,
    chars: Vec<Char>,
    /// Right edge of the last word, where the next word's joining space
    /// starts; `None` before the first word.
    pen: Option<f32>,
}

impl TextPageBuilder {
    /// An empty page recognised at `dpi`, carrying the renderer's
    /// `warnings`.
    pub fn new(dpi: u32, warnings: Vec<Warning>) -> Self {
        Self {
            dpi,
            warnings,
            blocks: Vec::new(),
            lines: Vec::new(),
            line: None,
        }
    }

    /// Starts a new block (closing the open line and block).
    pub fn begin_block(&mut self) {
        self.close_line();
        self.close_block();
    }

    /// Starts a new line with the engine's line box `bbox`
    /// (`[x0, y0, x1, y1]`, page space; the corners may come in any order).
    /// A box with a non-finite coordinate is ignored: the line's box is then
    /// that of its words.
    pub fn begin_line(&mut self, bbox: [f32; 4]) {
        self.close_line();
        self.line = Some(OpenLine {
            bbox: if bbox.iter().all(|v| v.is_finite()) {
                normalise(bbox)
            } else {
                EMPTY
            },
            chars: Vec::new(),
            pen: None,
        });
    }

    /// Adds one word of the open line with its box `bbox` (page space).
    ///
    /// The text is sanitised (run 0001 D-10); an empty word is dropped, as
    /// is a word made only of space-like code points (ADR 0048: it would
    /// add spaces beside the joining U+0020, or leave a line of nothing but
    /// spaces), and so is a word whose box has a non-finite coordinate (its
    /// characters would get NaN positions, and a `TextPage` holding NaN is
    /// not equal to itself). A word after the first in its line is preceded
    /// by one U+0020. D-10 keeps tab, line feed and carriage return; the
    /// engine never puts them inside a word, and if one did, a line feed
    /// would show in the serialised text exactly as one from a content
    /// stream does.
    pub fn word(&mut self, text: &str, bbox: [f32; 4]) {
        let text = sanitise(text);
        let n = text.chars().count();
        // `all` is true for the empty word too, so one test drops both.
        if text.chars().all(is_space_like) || !bbox.iter().all(|v| v.is_finite()) {
            return;
        }
        let [x0, y0, x1, y1] = normalise(bbox);
        let line = self.line.get_or_insert_with(|| OpenLine {
            bbox: [x0, y0, x1, y1],
            chars: Vec::new(),
            pen: None,
        });
        line.bbox = union(line.bbox, [x0, y0, x1, y1]);
        let size = y1 - y0;
        let baseline = y0 + DESCENT * size;
        if let Some(pen) = line.pen {
            line.chars.push(Char {
                text: CharText::from_char(' '),
                origin: (pen, baseline),
                dir: ACROSS,
                size,
                // A word that overlaps its predecessor leaves no gap; the
                // space is still emitted (the engine said these are two
                // words), with no width.
                advance: (x0 - pen).max(0.0),
            });
        }
        let width = x1 - x0;
        // Positions are computed from the box edge for every character
        // (not accumulated), so rounding does not drift along a long word.
        let at = |i: usize| x0 + width * (i as f32) / (n as f32);
        for (i, c) in text.chars().enumerate() {
            let start = at(i);
            line.chars.push(Char {
                text: CharText::from_char(c),
                origin: (start, baseline),
                dir: ACROSS,
                size,
                advance: at(i + 1) - start,
            });
        }
        line.pen = Some(x1);
    }

    /// The finished page: `TextSource::Ocr` at the builder's resolution.
    pub fn finish(mut self) -> TextPage {
        self.close_line();
        self.close_block();
        TextPage::from_ocr(self.blocks, self.warnings, self.dpi)
    }

    /// Moves the open line, if it holds any character, into the open block.
    fn close_line(&mut self) {
        if let Some(line) = self.line.take()
            && !line.chars.is_empty()
        {
            self.lines.push(Line {
                chars: line.chars,
                bbox: line.bbox,
                dir: ACROSS,
            });
        }
    }

    /// Moves the open block, if it holds any line, into the page.
    fn close_block(&mut self) {
        let lines = std::mem::take(&mut self.lines);
        let Some(first) = lines.first() else {
            return;
        };
        let bbox = lines.iter().fold(first.bbox, |b, l| union(b, l.bbox));
        self.blocks.push(Block { lines, bbox });
    }
}

/// `[min x, min y, max x, max y]` of a box given by two opposite corners.
fn normalise([a, b, c, d]: [f32; 4]) -> BBox {
    [a.min(c), b.min(d), a.max(c), b.max(d)]
}

#[cfg(test)]
#[path = "ocr_tests.rs"]
mod tests;
