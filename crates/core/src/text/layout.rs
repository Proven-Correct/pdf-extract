//! The line assembler: character records to lines and blocks (design spec
//! §6.4 as corrected by spec T29-T32 and D-9).
//!
//! **Lines.** Records are taken in content-stream order and each is compared
//! only with the line being built, never with earlier lines (the reference's
//! unsorted mode behaves the same way). With `em` the size of the *incoming*
//! glyph and `pen` the end of the line's last glyph (origin plus advance):
//!
//! ```text
//! if no line, or angle(c.dir, line.dir) > angle_tolerance:  new line
//! δ = c.origin − pen;  along = δ·line.dir;  perp = |δ × line.dir|
//! if perp > PERP_BREAK·em or along > GAP_BREAK·em
//!    or −along > BACK_BREAK·em:                             new line
//! else:
//!     if along > SPACE_GAP·em and the line does not end in U+0020,
//!        U+1680, U+3000 or U+FEFF and c does not start with
//!        U+0020 (T30, KP24: the reference's sets):           append ' '
//!     append c
//! ```
//!
//! Each direction has its own threshold (across, forwards, backwards; KP2,
//! KP3): the reference's data put the forward and across brackets on either
//! side of 0.8 em, so no shared value fits (spec revision 7b). The
//! along/across tests are independent rather than a distance (spec A.1's
//! diagonal probes). Backward movement within
//! the threshold joins without a space, which is also how fake-bold
//! overprints are treated: they are kept, landing on the same line or a new
//! one purely by this rule (KP4). Because `dir` is the text matrix's x axis
//! even for vertical writing, vertical glyphs have no advance along it and
//! each becomes its own line (KP12).
//!
//! **Blocks (T32).** A new line also starts a new block when the direction
//! changed, when it lies more than `BLOCK_GAP·em` across the previous line,
//! when it starts more than `BLOCK_INDENT·em` left of the block (along the
//! block's direction, measured from the leftmost line start), or when a Form
//! XObject or annotation boundary lies between the previous line's first
//! record and this line's first record. Only *where* boundaries are matters,
//! not which kind: the device's marker list is capped and may have lost a
//! start whose end survived, so begin/end pairs are never matched up and an
//! unbalanced list is as good as a balanced one. Boundaries never break a
//! line; with the compat separator, blocks do not change the output at all.
//!
//! Arithmetic is in `f64` on the records' `f32` values, so the comparisons
//! at the thresholds are not further blurred by `f32` rounding.

use super::options::LayoutParams;
use super::page::{BBox, Block, Char, Line, union};
use super::record::{CharRecord, CharText, Marker};

/// A 2-D vector in page space.
#[derive(Clone, Copy, Debug, PartialEq)]
struct V {
    x: f64,
    y: f64,
}

impl V {
    fn of(p: (f32, f32)) -> Self {
        Self {
            x: f64::from(p.0),
            y: f64::from(p.1),
        }
    }

    fn sub(self, o: Self) -> Self {
        Self {
            x: self.x - o.x,
            y: self.y - o.y,
        }
    }

    fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y
    }

    fn cross(self, o: Self) -> f64 {
        self.x * o.y - self.y * o.x
    }
}

/// Angle between two directions, in degrees (0-180).
fn angle_deg(a: V, b: V) -> f64 {
    a.cross(b).atan2(a.dot(b)).abs().to_degrees()
}

/// The line being built.
struct OpenLine {
    chars: Vec<Char>,
    dir: V,
    /// End of the last glyph's advance.
    pen: V,
}

/// The block being built.
struct OpenBlock {
    lines: Vec<Line>,
    dir: V,
    /// Origin of the block's first character; line starts are measured
    /// along `dir` from here.
    anchor: V,
    /// Leftmost line start seen so far, along `dir` from `anchor`.
    left: f64,
}

/// Why a new line is starting, for the block decision.
enum Break {
    /// First record of the page.
    First,
    /// The baseline direction changed.
    Direction,
    /// Same direction, moved too far; `perp` is the distance across the
    /// previous line.
    Moved { perp: f64 },
}

struct Assembler<'a> {
    params: &'a LayoutParams,
    markers: &'a [Marker],
    /// Markers before this index have been looked at.
    next_marker: usize,
    /// Record index of the current line's first record.
    line_start: usize,
    blocks: Vec<Block>,
    block: Option<OpenBlock>,
    line: Option<OpenLine>,
}

/// Assembles a page's records into blocks of lines (module docs). `markers`
/// are the device's form/annotation boundaries, in record order.
pub(super) fn assemble(
    records: Vec<CharRecord>,
    markers: &[Marker],
    params: &LayoutParams,
) -> Vec<Block> {
    let mut a = Assembler {
        params,
        markers,
        next_marker: 0,
        line_start: 0,
        blocks: Vec::new(),
        block: None,
        line: None,
    };
    for (i, record) in records.into_iter().enumerate() {
        a.place(i, record);
    }
    a.finish_block();
    a.blocks
}

impl Assembler<'_> {
    fn place(&mut self, i: usize, c: CharRecord) {
        let em = f64::from(c.size);
        let origin = V::of(c.origin);
        let dir = V::of(c.dir);
        let p = self.params;
        let Some(line) = &mut self.line else {
            return self.start_line(i, c, Break::First);
        };
        if angle_deg(dir, line.dir) > f64::from(p.angle_tolerance_deg) {
            return self.start_line(i, c, Break::Direction);
        }
        let delta = origin.sub(line.pen);
        let along = delta.dot(line.dir);
        let perp = delta.cross(line.dir).abs();
        if perp > f64::from(p.perp_break) * em
            || along > f64::from(p.gap_break) * em
            || -along > f64::from(p.back_break) * em
        {
            return self.start_line(i, c, Break::Moved { perp });
        }
        if along > f64::from(p.space_gap) * em
            && !ends_in_space_like(line.chars.last())
            && !c.text.as_str().starts_with(' ')
        {
            line.chars.push(Char {
                text: CharText::from_char(' '),
                origin: (line.pen.x as f32, line.pen.y as f32),
                dir: (line.dir.x as f32, line.dir.y as f32),
                size: c.size,
                advance: along as f32,
            });
        }
        line.pen = pen_after(&c);
        line.chars.push(into_char(c));
    }

    /// Ends the current line (if any) and starts one with `c`, in the
    /// current block or a new one (module docs).
    fn start_line(&mut self, i: usize, c: CharRecord, why: Break) {
        let boundary = self.boundary_before(i);
        self.finish_line();
        let em = f64::from(c.size);
        let origin = V::of(c.origin);
        let p = self.params;
        let new_block = match (why, &self.block) {
            (Break::First | Break::Direction, _) | (_, None) => true,
            (Break::Moved { perp }, Some(block)) => {
                let start = origin.sub(block.anchor).dot(block.dir);
                boundary
                    || perp > f64::from(p.block_gap) * em
                    || start < block.left - f64::from(p.block_indent) * em
            }
        };
        if new_block {
            self.finish_block();
            self.block = Some(OpenBlock {
                lines: Vec::new(),
                dir: V::of(c.dir),
                anchor: origin,
                left: 0.0,
            });
        } else if let Some(block) = &mut self.block {
            let start = origin.sub(block.anchor).dot(block.dir);
            block.left = block.left.min(start);
        }
        self.line_start = i;
        self.line = Some(OpenLine {
            dir: V::of(c.dir),
            pen: pen_after(&c),
            chars: vec![into_char(c)],
        });
    }

    /// Whether a boundary marker lies after the current line's first record
    /// and at or before record `i`. Consumes the markers up to `i`; markers
    /// are in record order, so each is looked at once.
    fn boundary_before(&mut self, i: usize) -> bool {
        let mut crossed = false;
        while let Some(m) = self.markers.get(self.next_marker) {
            if m.at > i {
                break;
            }
            crossed |= m.at > self.line_start;
            self.next_marker += 1;
        }
        crossed
    }

    fn finish_line(&mut self) {
        let Some(line) = self.line.take() else {
            return;
        };
        let bbox = boxes(line.chars.iter().map(Char::bbox));
        let dir = (line.dir.x as f32, line.dir.y as f32);
        let block = self
            .block
            .as_mut()
            .expect("a line is only started inside a block");
        block.lines.push(Line {
            chars: line.chars,
            bbox,
            dir,
        });
    }

    fn finish_block(&mut self) {
        self.finish_line();
        if let Some(block) = self.block.take() {
            let bbox = boxes(block.lines.iter().map(Line::bbox));
            self.blocks.push(Block {
                lines: block.lines,
                bbox,
            });
        }
    }
}

/// Union of non-empty boxes (callers never pass none).
fn boxes(mut it: impl Iterator<Item = BBox>) -> BBox {
    let first = it.next().unwrap_or_default();
    it.fold(first, union)
}

/// Whether a line ending in `c` already separates the next glyph, so no
/// synthetic space is needed (T30, KP24).
fn ends_in_space_like(c: Option<&Char>) -> bool {
    c.and_then(|c| c.text().chars().next_back())
        .is_some_and(is_space_like)
}

/// The code points that already separate words (ADR 0048). The set is the
/// reference extractor's, probed over the whole Unicode whitespace set:
/// U+0020, OGHAM SPACE MARK, IDEOGRAPHIC SPACE and U+FEFF. Deliberately not
/// `char::is_whitespace`: TAB, LF, NBSP, the U+2000 spaces and the line
/// separators do not count there, and U+FEFF (not whitespace to Rust) does.
/// The OCR builder drops words made only of these, so both sources share one
/// definition of a space.
pub(super) fn is_space_like(c: char) -> bool {
    matches!(c, ' ' | '\u{1680}' | '\u{3000}' | '\u{FEFF}')
}

/// The pen position after `c`: its origin plus its advance along its
/// direction.
fn pen_after(c: &CharRecord) -> V {
    let advance = f64::from(c.advance);
    V {
        x: f64::from(c.origin.0) + f64::from(c.dir.0) * advance,
        y: f64::from(c.origin.1) + f64::from(c.dir.1) * advance,
    }
}

fn into_char(c: CharRecord) -> Char {
    Char {
        text: c.text,
        origin: c.origin,
        dir: c.dir,
        size: c.size,
        advance: c.advance,
    }
}
