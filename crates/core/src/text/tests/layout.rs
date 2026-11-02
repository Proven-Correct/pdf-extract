//! The line assembler on hand-made records (spec T29-T32): the rules the
//! builder fixtures of `tests/text_layout.rs` cannot isolate, notably the
//! block rules and boundary markers, including the unbalanced marker lists a
//! capped device can leave behind.

use super::super::layout::assemble;
use super::super::options::LayoutParams;
use super::super::page::{Block, TextPage};
use super::super::record::{Boundary, CharRecord, CharText, Marker};
use crate::BlockSeparator;

/// A horizontal record: `text` at `(x, y)`, size `size`, advancing
/// `advance`.
fn rec(text: &str, x: f32, y: f32, size: f32, advance: f32) -> CharRecord {
    CharRecord {
        text: CharText::new(text),
        origin: (x, y),
        dir: (1.0, 0.0),
        size,
        advance,
        invisible: false,
    }
}

/// Records of `word` at 10 pt, 5 pt per character, from `(x, y)`.
fn word(word: &str, x: f32, y: f32) -> Vec<CharRecord> {
    word.chars()
        .enumerate()
        .map(|(i, c)| rec(&c.to_string(), x + 5.0 * i as f32, y, 10.0, 5.0))
        .collect()
}

fn marker(at: usize, boundary: Boundary) -> Marker {
    Marker { at, boundary }
}

fn run(records: Vec<CharRecord>, markers: &[Marker], params: &LayoutParams) -> Vec<Block> {
    assemble(records, markers, params)
}

fn text(blocks: Vec<Block>, sep: BlockSeparator) -> String {
    TextPage::new(blocks, Vec::new()).to_text(sep)
}

fn blank(records: Vec<CharRecord>, markers: &[Marker]) -> String {
    text(
        run(records, markers, &LayoutParams::default()),
        BlockSeparator::BlankLine,
    )
}

#[test]
fn no_records_no_blocks() {
    assert!(run(Vec::new(), &[], &LayoutParams::default()).is_empty());
    // Markers alone (a form with no text) change nothing.
    let m = [marker(0, Boundary::FormStart), marker(0, Boundary::FormEnd)];
    assert!(run(Vec::new(), &m, &LayoutParams::default()).is_empty());
}

/// Two lines 12 pt apart (1.2 em: a new line, the same block).
fn two_lines() -> Vec<CharRecord> {
    let mut r = word("ab", 100.0, 700.0);
    r.extend(word("cd", 100.0, 688.0));
    r
}

#[test]
fn boundary_between_lines_starts_a_block() {
    assert_eq!(blank(two_lines(), &[]), "ab\ncd\n");
    for b in [
        Boundary::FormStart,
        Boundary::FormEnd,
        Boundary::AnnotationStart,
        Boundary::AnnotationEnd,
    ] {
        // Any kind, alone and unmatched (the device's cap may have dropped
        // its partner), separates the blocks.
        assert_eq!(blank(two_lines(), &[marker(2, b)]), "ab\n\ncd\n", "{b:?}");
        // A boundary inside the first line, after its first record, also
        // counts for the next line.
        assert_eq!(blank(two_lines(), &[marker(1, b)]), "ab\n\ncd\n", "{b:?}");
    }
    // Boundaries at the start of the page or after the last record do not
    // separate anything.
    let m = [marker(0, Boundary::FormStart), marker(4, Boundary::FormEnd)];
    assert_eq!(blank(two_lines(), &m), "ab\ncd\n");
}

#[test]
fn boundaries_never_break_a_line() {
    // Text that joins the line across a form boundary stays on the line,
    // and the markers are consumed in order across several lines.
    let mut r = word("ab", 100.0, 700.0);
    r.extend(word("cd", 110.0, 700.0));
    r.extend(word("ef", 100.0, 688.0));
    r.extend(word("gh", 100.0, 676.0));
    let m = [
        marker(2, Boundary::FormStart),
        marker(4, Boundary::FormEnd),
        marker(4, Boundary::FormEnd),
    ];
    assert_eq!(blank(r, &m), "abcd\n\nef\ngh\n");
}

#[test]
fn block_gap_and_indent() {
    // 16 pt below (1.6 em > 1.5): a new block; 14 pt (1.4 em): same block.
    let mut r = word("ab", 100.0, 700.0);
    r.extend(word("cd", 100.0, 684.0));
    r.extend(word("ef", 100.0, 670.0));
    assert_eq!(blank(r, &[]), "ab\n\ncd\nef\n");
    // A line starting 25 pt (2.5 em) left of the block starts a block; one
    // 15 pt left does not, and then becomes the block's left edge.
    let mut r = word("ab", 100.0, 700.0);
    r.extend(word("cd", 85.0, 688.0));
    r.extend(word("ef", 65.0, 676.0));
    r.extend(word("gh", 40.0, 664.0));
    assert_eq!(blank(r, &[]), "ab\ncd\nef\n\ngh\n");
    // Starting further right never starts a block.
    let mut r = word("ab", 100.0, 700.0);
    r.extend(word("cd", 300.0, 688.0));
    assert_eq!(blank(r, &[]), "ab\ncd\n");
}

#[test]
fn direction_change_starts_a_line_and_a_block() {
    let mut r = word("ab", 100.0, 700.0);
    // Continuing at the pen, but turned by 3° (> 2°).
    let (s, c) = 3f32.to_radians().sin_cos();
    r.push(CharRecord {
        dir: (c, s),
        ..rec("c", 110.0, 700.0, 10.0, 5.0)
    });
    // Within 0.5° of the new line's direction: the same line.
    let (s, c) = 3.5f32.to_radians().sin_cos();
    r.push(CharRecord {
        dir: (c, s),
        ..rec("d", 115.0, 700.0, 10.0, 5.0)
    });
    let blocks = run(r, &[], &LayoutParams::default());
    assert_eq!(
        text(blocks.clone(), BlockSeparator::BlankLine),
        "ab\n\ncd\n"
    );
    let (s3, c3) = 3f32.to_radians().sin_cos();
    assert_eq!(blocks[1].lines()[0].dir(), (c3, s3));
    // Reversed direction (180°) is a change too.
    let mut r = word("ab", 100.0, 700.0);
    r.push(CharRecord {
        dir: (-1.0, 0.0),
        ..rec("c", 110.0, 700.0, 10.0, 5.0)
    });
    assert_eq!(blank(r, &[]), "ab\n\nc\n");
}

#[test]
fn only_the_reference_space_set_suppresses_the_synthetic_space() {
    // Gap of 3 pt (0.3 em > 0.149) after a space glyph: no synthetic
    // space; before a glyph starting with TAB: a space (T30, KP24).
    // Multi-code-point texts look at their last and first characters.
    let r = vec![
        rec("a ", 100.0, 700.0, 10.0, 5.0),
        rec("b", 108.0, 700.0, 10.0, 5.0),
        rec("\tc", 116.0, 700.0, 10.0, 5.0),
        rec("ff", 124.0, 700.0, 10.0, 5.0),
        rec(" g", 132.0, 700.0, 10.0, 5.0),
    ];
    assert_eq!(blank(r, &[]), "a b \tc ff g\n");
}

/// `A`, then `ch` and `B` with a 0.3 em gap before (`gap_first`) or after
/// `ch`; the line text.
fn around_gap(ch: char, gap_first: bool) -> String {
    let (first, second) = if gap_first {
        ("A".to_owned(), format!("{ch}B"))
    } else {
        (format!("A{ch}"), "B".to_owned())
    };
    let r = vec![
        rec(&first, 100.0, 700.0, 10.0, 5.0),
        rec(&second, 108.0, 700.0, 10.0, 5.0),
    ];
    blank(r, &[])
}

#[test]
fn every_probed_code_point_on_both_sides_of_a_gap() {
    // Spec T30 / Appendix A.1 (revision 10), the reference's sets: as the
    // incoming glyph only U+0020 suppresses the space; as the line's last
    // character U+0020, U+1680, U+3000 and U+FEFF do. Everything else in
    // the probe, `char::is_whitespace` or not, suppresses nothing.
    let ends_line = [' ', '\u{1680}', '\u{3000}', '\u{FEFF}'];
    let probed = [
        '\t', '\n', '\u{B}', '\u{C}', '\r', ' ', '\u{85}', '\u{A0}', '\u{1680}', '\u{2000}',
        '\u{2002}', '\u{2003}', '\u{2007}', '\u{2009}', '\u{200A}', '\u{200B}', '\u{2028}',
        '\u{2029}', '\u{202F}', '\u{205F}', '\u{3000}', '\u{FEFF}',
    ];
    for ch in probed {
        let before = if ends_line.contains(&ch) {
            format!("A{ch}B\n")
        } else {
            format!("A{ch} B\n")
        };
        assert_eq!(
            around_gap(ch, false),
            before,
            "U+{:04X} before the gap",
            u32::from(ch)
        );
        let after = if ch == ' ' {
            "A B\n".to_owned()
        } else {
            format!("A {ch}B\n")
        };
        assert_eq!(
            around_gap(ch, true),
            after,
            "U+{:04X} after the gap",
            u32::from(ch)
        );
    }
}

#[test]
fn line_and_block_boxes() {
    let blocks = run(two_lines(), &[], &LayoutParams::default());
    let line = &blocks[0].lines()[0];
    // 100..110 along, −2..+8 across the baseline at y = 700.
    assert_eq!(line.bbox(), [100.0, 698.0, 110.0, 708.0]);
    assert_eq!(blocks[0].bbox(), [100.0, 686.0, 110.0, 708.0]);
    // A vertical baseline: the box turns with it.
    let r = vec![CharRecord {
        dir: (0.0, 1.0),
        ..rec("v", 100.0, 700.0, 10.0, 5.0)
    }];
    let blocks = run(r, &[], &LayoutParams::default());
    assert_eq!(blocks[0].lines()[0].bbox(), [92.0, 700.0, 102.0, 705.0]);
}

#[test]
fn degenerate_parameters_do_not_panic() {
    // NaN makes every threshold comparison false: no breaks, no spaces.
    let nan = LayoutParams {
        space_gap: f32::NAN,
        perp_break: f32::NAN,
        gap_break: f32::NAN,
        back_break: f32::NAN,
        block_gap: f32::NAN,
        block_indent: f32::NAN,
        angle_tolerance_deg: f32::NAN,
    };
    assert_eq!(
        text(run(two_lines(), &[], &nan), BlockSeparator::BlankLine),
        "abcd\n"
    );
    // Zero thresholds: any movement breaks the line; the block rules still
    // need some distance across or to the left, so a line continuing to
    // the right on the same baseline stays in the block.
    let zero = LayoutParams {
        space_gap: 0.0,
        perp_break: 0.0,
        gap_break: 0.0,
        back_break: 0.0,
        block_gap: 0.0,
        block_indent: 0.0,
        angle_tolerance_deg: 0.0,
    };
    let r = vec![
        rec("a", 100.0, 700.0, 10.0, 5.0),
        rec("b", 106.0, 700.0, 10.0, 5.0),
    ];
    assert_eq!(
        text(run(r, &[], &zero), BlockSeparator::BlankLine),
        "a\nb\n"
    );
}

#[test]
fn zero_size_glyphs_join_at_the_pen() {
    // em = 0 makes every threshold 0: glyphs exactly at the pen join, as a
    // size-0 font's glyphs (all at one point) do.
    let r = vec![
        rec("Z", 100.0, 700.0, 0.0, 0.0),
        rec("e", 100.0, 700.0, 0.0, 0.0),
    ];
    assert_eq!(blank(r, &[]), "Ze\n");
}
