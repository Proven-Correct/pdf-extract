//! Unit tests of the OCR page builder (run 0002 §4.1, `TextPageBuilder`
//! bullet). Inside the crate because the character boxes the builder sets
//! (`Char::bbox`) are not public.

use super::TextPageBuilder;
use crate::text::{BlockSeparator, TextSource};
use crate::warning::{Warning, WarningKind};

/// One block per inner list, one line per list of words; each word gets a
/// 10-pt-high box of 6 pt per character, lines 12 pt apart.
fn build(blocks: &[&[&[&str]]]) -> crate::TextPage {
    let mut b = TextPageBuilder::new(300, Vec::new());
    let mut y = 800.0;
    for lines in blocks {
        b.begin_block();
        for words in *lines {
            b.begin_line([10.0, y, 500.0, y + 10.0]);
            let mut x = 10.0;
            for w in *words {
                let width = 6.0 * w.chars().count() as f32;
                b.word(w, [x, y, x + width, y + 10.0]);
                x += width + 4.0;
            }
            y -= 12.0;
        }
    }
    b.finish()
}

#[test]
fn source_and_dpi_mark_an_ocr_page() {
    let warning = Warning {
        page: Some(0),
        kind: WarningKind::ImagePixelLimit,
        detail: "pixel limit".into(),
        count: 1,
    };
    let page = TextPageBuilder::new(144, vec![warning.clone()]).finish();
    assert_eq!(page.source(), TextSource::Ocr);
    assert_eq!(page.dpi(), Some(144));
    assert_eq!(page.warnings(), &[warning]);
}

#[test]
fn no_words_is_the_empty_string() {
    let page = build(&[]);
    assert_eq!(page.to_text(BlockSeparator::Newline), "");
    assert_eq!(page.to_text(BlockSeparator::BlankLine), "");
    assert!(page.blocks().is_empty());
}

#[test]
fn every_line_ends_in_a_newline_and_blank_line_separates_blocks() {
    let page = build(&[&[&["Hello", "world"], &["second"]], &[&["third", "block"]]]);
    assert_eq!(
        page.to_text(BlockSeparator::Newline),
        "Hello world\nsecond\nthird block\n"
    );
    assert_eq!(
        page.to_text(BlockSeparator::BlankLine),
        "Hello world\nsecond\n\nthird block\n"
    );
}

#[test]
fn text_is_sanitised_like_the_text_path() {
    let page = build(&[&[&["a\u{0}b", "\u{FFFE}"]]]);
    assert_eq!(
        page.to_text(BlockSeparator::Newline),
        "a\u{FFFD}b \u{FFFD}\n"
    );
}

#[test]
fn empty_words_lines_and_blocks_are_dropped() {
    let mut b = TextPageBuilder::new(300, Vec::new());
    b.begin_block();
    b.begin_line([0.0, 0.0, 10.0, 10.0]);
    b.word("", [0.0, 0.0, 5.0, 10.0]);
    b.begin_line([0.0, 20.0, 10.0, 30.0]);
    b.word("x", [0.0, 20.0, 5.0, 30.0]);
    b.word("", [6.0, 20.0, 9.0, 30.0]);
    b.word("y", [6.0, 20.0, 9.0, 30.0]);
    b.begin_block(); // a block with no lines at all
    b.begin_block();
    b.begin_line([0.0, 40.0, 10.0, 50.0]); // a line with no words
    b.begin_block();
    b.begin_line([0.0, 60.0, 10.0, 70.0]);
    b.word("z", [0.0, 60.0, 5.0, 70.0]);
    let page = b.finish();
    assert_eq!(page.blocks().len(), 2);
    assert_eq!(page.blocks()[0].lines().len(), 1);
    // The empty word between "x" and "y" adds no second space.
    assert_eq!(page.to_text(BlockSeparator::BlankLine), "x y\n\nz\n");
}

#[test]
fn space_only_words_are_dropped_like_empty_ones() {
    // A word of nothing but the text path's space-like code points (ADR
    // 0048: U+0020, U+1680, U+3000, U+FEFF) would put a second space next to
    // the joining one, or make a line of only spaces; it is dropped. Other
    // whitespace (NBSP here) is text the engine reported and is kept, as the
    // text path keeps such glyphs.
    let mut b = TextPageBuilder::new(300, Vec::new());
    b.begin_block();
    b.begin_line([0.0, 0.0, 10.0, 10.0]);
    b.word(" ", [0.0, 0.0, 5.0, 10.0]);
    b.word("\u{3000}\u{FEFF}", [5.0, 0.0, 9.0, 10.0]);
    b.begin_line([0.0, 20.0, 10.0, 30.0]);
    b.word("x", [0.0, 20.0, 5.0, 30.0]);
    b.word(" \u{1680}", [6.0, 20.0, 9.0, 30.0]);
    b.word("y", [6.0, 20.0, 9.0, 30.0]);
    b.word("\u{A0}", [9.0, 20.0, 10.0, 30.0]);
    let page = b.finish();
    assert_eq!(page.blocks().len(), 1);
    assert_eq!(page.blocks()[0].lines().len(), 1);
    assert_eq!(page.to_text(BlockSeparator::Newline), "x y \u{A0}\n");
}

#[test]
fn words_and_lines_without_an_open_container_open_one() {
    let mut b = TextPageBuilder::new(300, Vec::new());
    b.word("lone", [0.0, 0.0, 24.0, 10.0]);
    b.begin_line([0.0, 20.0, 30.0, 30.0]);
    b.word("next", [0.0, 20.0, 24.0, 30.0]);
    let page = b.finish();
    assert_eq!(page.blocks().len(), 1);
    assert_eq!(page.to_text(BlockSeparator::Newline), "lone\nnext\n");
    // The implicit line's box is the word's.
    assert_eq!(page.blocks()[0].lines()[0].bbox(), [0.0, 0.0, 24.0, 10.0]);
}

#[test]
fn block_box_is_the_union_of_its_lines() {
    let mut b = TextPageBuilder::new(300, Vec::new());
    b.begin_block();
    b.begin_line([10.0, 100.0, 50.0, 110.0]);
    b.word("ab", [10.0, 100.0, 22.0, 110.0]);
    // Corners given in the other order are normalised.
    b.begin_line([60.0, 95.0, 5.0, 80.0]);
    b.word("cd", [5.0, 80.0, 17.0, 95.0]);
    // A word reaching outside its line widens the line.
    b.begin_line([0.0, 50.0, 10.0, 60.0]);
    b.word("e", [0.0, 50.0, 70.0, 61.0]);
    let page = b.finish();
    let block = &page.blocks()[0];
    let lines = block.lines();
    assert_eq!(lines[0].bbox(), [10.0, 100.0, 50.0, 110.0]);
    assert_eq!(lines[1].bbox(), [5.0, 80.0, 60.0, 95.0]);
    assert_eq!(lines[2].bbox(), [0.0, 50.0, 70.0, 61.0]);
    assert_eq!(block.bbox(), [0.0, 50.0, 70.0, 110.0]);
    for line in lines {
        assert_eq!(line.dir(), (1.0, 0.0));
    }
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

#[test]
fn a_word_box_is_split_evenly_between_its_characters() {
    let mut b = TextPageBuilder::new(300, Vec::new());
    b.begin_line([100.0, 200.0, 220.0, 212.0]);
    b.word("abcd", [100.0, 200.0, 140.0, 212.0]);
    b.word("é\u{1F600}", [160.0, 200.0, 180.0, 212.0]);
    let page = b.finish();
    let chars = page.blocks()[0].lines()[0].chars();
    let texts: Vec<&str> = chars.iter().map(|c| c.text()).collect();
    assert_eq!(texts, ["a", "b", "c", "d", " ", "é", "\u{1F600}"]);
    for (i, c) in chars[..4].iter().enumerate() {
        let x0 = 100.0 + 10.0 * i as f32;
        assert!(close(c.bbox(), [x0, 200.0, x0 + 10.0, 212.0]), "{c:?}");
        assert!((c.advance() - 10.0).abs() < 1e-4);
        assert_eq!(c.size(), 12.0);
        assert_eq!(c.dir(), (1.0, 0.0));
    }
    // The joining space spans the gap between the two words.
    let space = &chars[4];
    assert_eq!(space.origin().0, 140.0);
    assert_eq!(space.advance(), 20.0);
    assert!(close(chars[5].bbox(), [160.0, 200.0, 170.0, 212.0]));
    assert!(close(chars[6].bbox(), [170.0, 200.0, 180.0, 212.0]));
}

#[test]
fn overlapping_words_get_a_zero_width_space() {
    let mut b = TextPageBuilder::new(300, Vec::new());
    b.word("ab", [0.0, 0.0, 20.0, 10.0]);
    b.word("cd", [15.0, 0.0, 35.0, 10.0]);
    let page = b.finish();
    let chars = page.blocks()[0].lines()[0].chars();
    assert_eq!(chars[2].text(), " ");
    assert_eq!(chars[2].advance(), 0.0);
}

/// A small deterministic generator for the sweeps below. The workspace has
/// no property-testing crate and these invariants do not justify adding
/// one; a fixed seed keeps the cases reproducible, and the failing case is
/// printed in the assertion message.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    /// Uniform in `[lo, hi)`.
    fn f32_in(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * (self.next() % 1_000_000) as f32 / 1_000_000.0
    }

    /// Uniform in `0..n`.
    fn below(&mut self, n: u64) -> usize {
        usize::try_from(self.next() % n).expect("small")
    }
}

/// The split is exact enough at any position and length: the character
/// boxes tile the word box from left to right.
#[test]
fn character_boxes_tile_the_word_box() {
    let mut rng = Lcg(0x5eed_0001);
    for case in 0..2000 {
        let x0 = rng.f32_in(-1.0e4, 1.0e4);
        let y0 = rng.f32_in(-1.0e4, 1.0e4);
        let w = rng.f32_in(0.0, 2000.0);
        let h = rng.f32_in(0.0, 500.0);
        let n = 1 + rng.below(40);
        let mut b = TextPageBuilder::new(300, Vec::new());
        b.word(&"w".repeat(n), [x0, y0, x0 + w, y0 + h]);
        let page = b.finish();
        let chars = page.blocks()[0].lines()[0].chars();
        assert_eq!(chars.len(), n);
        let tol = 1e-3 * (1.0 + x0.abs() + y0.abs() + w + h);
        for (i, c) in chars.iter().enumerate() {
            let bb = c.bbox();
            let want = [
                x0 + w * i as f32 / n as f32,
                y0,
                x0 + w * (i + 1) as f32 / n as f32,
                y0 + h,
            ];
            assert!(
                bb.iter().zip(want).all(|(a, b)| (a - b).abs() <= tol),
                "case {case}: x0={x0} y0={y0} w={w} h={h} n={n} char {i}: {bb:?} vs {want:?}"
            );
        }
    }
}

/// Serialisation shape for any script of words: one line per line with a
/// non-empty word, words joined by single spaces, blocks with such a line
/// separated by one blank line under `BlankLine`; every block's box is the
/// union of its lines'.
#[test]
fn serialisation_matches_the_script() {
    const WORDS: [&str; 6] = ["", "a", "bc", "def", "ghij", "k"];
    let mut rng = Lcg(0x5eed_0002);
    for case in 0..2000 {
        let script: Vec<Vec<Vec<&str>>> = (0..rng.below(4))
            .map(|_| {
                (0..rng.below(4))
                    .map(|_| (0..rng.below(4)).map(|_| WORDS[rng.below(6)]).collect())
                    .collect()
            })
            .collect();
        let mut b = TextPageBuilder::new(300, Vec::new());
        for (bi, lines) in script.iter().enumerate() {
            b.begin_block();
            for (li, words) in lines.iter().enumerate() {
                let y = -((bi * 10 + li) as f32) * 12.0;
                b.begin_line([0.0, y, 100.0, y + 10.0]);
                for (wi, w) in words.iter().enumerate() {
                    let x = wi as f32 * 30.0;
                    b.word(w, [x, y, x + 20.0, y + 10.0]);
                }
            }
        }
        let page = b.finish();
        let blocks: Vec<Vec<String>> = script
            .iter()
            .map(|lines| {
                lines
                    .iter()
                    .map(|ws| {
                        let kept: Vec<&str> =
                            ws.iter().copied().filter(|w| !w.is_empty()).collect();
                        kept.join(" ")
                    })
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|b| !b.is_empty())
            .collect();
        let newline: String = blocks.iter().flatten().map(|l| format!("{l}\n")).collect();
        let blank = blocks
            .iter()
            .map(|b| b.iter().map(|l| format!("{l}\n")).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            page.to_text(BlockSeparator::Newline),
            newline,
            "case {case}: {script:?}"
        );
        assert_eq!(
            page.to_text(BlockSeparator::BlankLine),
            blank,
            "case {case}: {script:?}"
        );
        assert_eq!(page.blocks().len(), blocks.len());
        for block in page.blocks() {
            let union = block.lines().iter().fold(block.lines()[0].bbox(), |a, l| {
                let b = l.bbox();
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            });
            assert_eq!(block.bbox(), union);
        }
    }
}

#[test]
fn non_finite_boxes_are_ignored_and_degenerate_ones_kept() {
    let mut b = TextPageBuilder::new(300, Vec::new());
    b.begin_line([0.0, 0.0, f32::NAN, 10.0]);
    b.word("inf", [0.0, 0.0, f32::INFINITY, 10.0]);
    b.word("nan", [f32::NAN, 0.0, 5.0, 10.0]);
    b.word("ok", [2.0, 1.0, 8.0, 9.0]);
    // A zero-height, zero-width word is kept: its characters are points.
    b.word("pt", [9.0, 5.0, 9.0, 5.0]);
    let page = b.finish();
    assert_eq!(page.clone(), page, "no NaN anywhere");
    assert_eq!(page.to_text(BlockSeparator::Newline), "ok pt\n");
    // The line's box comes from its words, the bad line box ignored.
    assert_eq!(page.blocks()[0].lines()[0].bbox(), [2.0, 1.0, 9.0, 9.0]);
    let pt = &page.blocks()[0].lines()[0].chars()[3];
    assert_eq!((pt.size(), pt.advance()), (0.0, 0.0));
}
