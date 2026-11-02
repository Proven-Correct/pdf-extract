//! `text_width` against the oracle (spec §4.3): every WinAnsi printable
//! character, and a few strings, must match the oracle to within 0.01 pt.
//!
//! The reference values are a checked-in vector (`tests/data/
//! helvetica_widths_oracle.tsv`, written by `tools/oracle/mkfixtures.py
//! --afm-check`), so this test needs no PyMuPDF. The vector derives its
//! character set from Python's cp1252 codec, independently of the Rust
//! WinAnsi table, so the set comparison below also cross-checks that table.
//!
//! Two oracle measurements per row, because they disagree on 27 characters:
//!
//! - `get_text_length` (the measurement spec §4.3 names) knows only Latin-1
//!   code points. For the WinAnsi 0x80-0x9F block (euro, curly quotes,
//!   dashes, ...) it returns 278 units, which is not the glyph's width.
//! - `rendered`: the advance the oracle actually applies when it renders the
//!   run from a WinAnsi Helvetica font (measured from glyph origins). This is
//!   what fixtures depend on, and it equals the AFM for every character.
//!
//! So `text_width` is held to `rendered` everywhere and to `get_text_length`
//! wherever that function is defined (all-Latin-1 text); for the 0x80-0x9F
//! block the 278-unit fallback is pinned, so a future oracle that fixes it is
//! noticed rather than silently ignored.

use std::collections::BTreeSet;

use pdf_extract_testkit::{text_width, win_ansi_chars};

const VECTOR: &str = include_str!("data/helvetica_widths_oracle.tsv");
const TOLERANCE_PT: f64 = 0.01;

struct Row {
    text: String,
    fontsize: f32,
    /// `get_text_length(text, fontname="helv", fontsize)`.
    text_length: f64,
    /// Advance applied when the oracle renders the run.
    rendered: f64,
}

impl Row {
    fn is_latin1(&self) -> bool {
        self.text.chars().all(|c| u32::from(c) <= 0xFF)
    }
}

fn rows() -> Vec<Row> {
    VECTOR
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert_eq!(cols.len(), 4, "malformed vector row {l:?}");
            let text = cols[0]
                .split(' ')
                .map(|h| {
                    let cp = u32::from_str_radix(h, 16).expect("hex code point");
                    char::from_u32(cp).expect("valid scalar")
                })
                .collect();
            Row {
                text,
                fontsize: cols[1].parse().expect("fontsize"),
                text_length: cols[2].parse().expect("text_length"),
                rendered: cols[3].parse().expect("rendered"),
            }
        })
        .collect()
}

fn ours(r: &Row) -> f64 {
    f64::from(text_width(&r.text, r.fontsize).expect("vector is WinAnsi"))
}

#[test]
fn widths_match_oracle_rendering_for_every_row() {
    let rows = rows();
    assert!(
        rows.len() > 218,
        "vector looks truncated: {} rows",
        rows.len()
    );
    let mut worst = 0.0f64;
    for r in &rows {
        let diff = (ours(r) - r.rendered).abs();
        worst = worst.max(diff);
        assert!(
            diff <= TOLERANCE_PT,
            "{:?} at {} pt: ours {}, oracle rendered {}, diff {diff}",
            r.text,
            r.fontsize,
            ours(r),
            r.rendered
        );
    }
    eprintln!("worst difference against rendered advances: {worst:.6} pt");
}

#[test]
fn widths_match_get_text_length_wherever_it_is_defined() {
    let rows = rows();
    let latin1: Vec<&Row> = rows.iter().filter(|r| r.is_latin1()).collect();
    // 95 ASCII + 96 Latin-1 characters, plus the all-Latin-1 strings.
    assert!(
        latin1.len() >= 191 + 7,
        "only {} Latin-1 rows",
        latin1.len()
    );
    for r in latin1 {
        let diff = (ours(r) - r.text_length).abs();
        assert!(
            diff <= TOLERANCE_PT,
            "{:?} at {} pt: ours {}, get_text_length {}, diff {diff}",
            r.text,
            r.fontsize,
            ours(r),
            r.text_length
        );
    }
}

#[test]
fn get_text_length_fallback_for_the_high_block_is_pinned() {
    let high: Vec<Row> = rows()
        .into_iter()
        .filter(|r| r.text.chars().count() == 1 && !r.is_latin1())
        .collect();
    assert_eq!(high.len(), 27, "the WinAnsi 0x80-0x9F block has 27 glyphs");
    for r in &high {
        let fallback = 0.278 * f64::from(r.fontsize);
        assert!(
            (r.text_length - fallback).abs() <= TOLERANCE_PT,
            "{:?}: get_text_length {} is no longer the 278-unit fallback; \
             re-examine afm_oracle.rs",
            r.text,
            r.text_length
        );
    }
}

#[test]
fn vector_covers_exactly_the_win_ansi_printable_set() {
    let in_vector: BTreeSet<char> = rows()
        .iter()
        .filter(|r| r.text.chars().count() == 1)
        .map(|r| r.text.chars().next().expect("one char"))
        .collect();
    let ours: BTreeSet<char> = win_ansi_chars().collect();
    assert_eq!(in_vector, ours);
}

#[test]
fn alpha_at_11pt() {
    // The spec's named example.
    let row = rows()
        .into_iter()
        .find(|r| r.text == "Alpha" && r.fontsize == 11.0)
        .expect("vector has Alpha at 11 pt");
    let ours = f64::from(text_width("Alpha", 11.0).expect("ASCII"));
    assert!((ours - row.text_length).abs() <= TOLERANCE_PT);
    assert!((ours - row.rendered).abs() <= TOLERANCE_PT);
}
