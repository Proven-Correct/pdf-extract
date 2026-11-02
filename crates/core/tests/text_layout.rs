//! Line-assembly oracle of spec §4.10 through the public API
//! (`Page::text`): the KP1-KP5 sweeps, the perpendicular and diagonal
//! probes, the spacing operators, vertical text, blocks and the separator,
//! rotation, empty and zero-size pages, and `Unmapped::Skip` gaps.
//!
//! The expected strings are the reference extractor's output for the same
//! placement (spec Appendix A.1), except where a test says otherwise: the
//! U+FFFD for NUL/C0 codes under `Unmapped::Cid` is a deliberate difference
//! (D-10), and the `0 Tf` case was not probed (A.1 has 0.001 pt). The
//! fixtures are rebuilt here with the test kit rather than read from corpus
//! C, which unit 13 generates from the same recipes. Runs are placed with the kit's `text_width` (Helvetica AFM), so a
//! "gap" is the distance from the end of one run to the start of the next.

use pdf_extract_core::{BlockSeparator, Document, OpenOptions, TextOptions, TextSource, Unmapped};
use pdf_extract_testkit::{
    CidFontSubtype, FontDescriptor, FontDict, PdfBuilder, TextStyle, Type0Font, Type3Font,
    Type3Glyph, text_width,
};

/// Page 0's text with `opts`.
fn text_with(bytes: Vec<u8>, opts: &TextOptions) -> String {
    let doc = Document::from_bytes(bytes, &OpenOptions::default()).expect("fixture opens");
    doc.page(0)
        .expect("page 0")
        .text(opts)
        .expect("page extracts")
}

/// Page 0's text with the default (compat) options.
fn text(bytes: Vec<u8>) -> String {
    text_with(bytes, &TextOptions::default())
}

/// One A4 page with Helvetica runs `(x, y, text, size)`, y measured down
/// from the top as `insert_text` does.
fn runs(rs: &[(f32, f32, &str, f32)]) -> Vec<u8> {
    runs_on(595.0, rs)
}

/// As [`runs`], on a page `width` points wide (842 high). The position
/// sweeps use a 1000 pt page so that a 22 pt run starting at x = 499.9 is
/// not clipped at the A4 edge; only the x values matter to them.
fn runs_on(width: f32, rs: &[(f32, f32, &str, f32)]) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let page = b.add_page(width, 842.0);
    for &(x, y, t, size) in rs {
        page.insert_text((x, y), t, TextStyle::size(size))
            .expect("WinAnsi text");
    }
    b.to_bytes()
}

/// One A4 page with raw content.
fn raw(content: &str) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(content);
    b.to_bytes()
}

fn width(t: &str, size: f32) -> f32 {
    text_width(t, size).expect("WinAnsi text")
}

/// `first` at x = 100 and `second` `gap` points after its end, same
/// baseline, each in its own size.
fn pair(first: &str, first_size: f32, second: &str, second_size: f32, gap: f32) -> String {
    let x2 = 100.0 + width(first, first_size) + gap;
    text(runs(&[
        (100.0, 100.0, first, first_size),
        (x2, 100.0, second, second_size),
    ]))
}

#[track_caller]
fn sweep(cases: &[f32], expected: &str, f: impl Fn(f32) -> String) {
    for &g in cases {
        assert_eq!(f(g), expected, "offset {g}");
    }
}

// --- KP1: space gap ------------------------------------------------------

#[test]
fn kp1_space_gap_11pt() {
    sweep(&[0.5, 1.0, 1.25, 1.5], "AlphaBeta\n", |g| {
        pair("Alpha", 11.0, "Beta", 11.0, g)
    });
    sweep(&[1.75, 2.0, 2.5, 3.0, 4.0], "Alpha Beta\n", |g| {
        pair("Alpha", 11.0, "Beta", 11.0, g)
    });
}

#[test]
fn kp1_space_gap_22pt() {
    sweep(&[0.5, 1.0, 2.0, 3.0, 3.25], "AlphaBeta\n", |g| {
        pair("Alpha", 22.0, "Beta", 22.0, g)
    });
    sweep(&[3.5, 3.75, 4.0], "Alpha Beta\n", |g| {
        pair("Alpha", 22.0, "Beta", 22.0, g)
    });
}

#[test]
fn kp1_mixed_sizes_scale_with_the_incoming_glyph() {
    // 20 pt then 8 pt: the 8 pt glyph's em decides.
    sweep(&[0.0], "Bigsmall\n", |g| pair("Big", 20.0, "small", 8.0, g));
    sweep(&[2.0, 4.0], "Big small\n", |g| {
        pair("Big", 20.0, "small", 8.0, g)
    });
    sweep(&[8.0], "Big\nsmall\n", |g| {
        pair("Big", 20.0, "small", 8.0, g)
    });
    // 8 pt then 20 pt: the 20 pt glyph's em decides.
    sweep(&[1.0, 1.5, 2.0], "smallBig\n", |g| {
        pair("small", 8.0, "Big", 20.0, g)
    });
    sweep(&[3.0, 4.0, 6.0, 8.0, 12.0], "small Big\n", |g| {
        pair("small", 8.0, "Big", 20.0, g)
    });
    sweep(&[16.0, 18.0], "small\nBig\n", |g| {
        pair("small", 8.0, "Big", 20.0, g)
    });
}

/// `first` at `(x, 100)` and `second` `gap` points after its end.
fn pair_at(
    x: f32,
    first: &str,
    first_size: f32,
    second: &str,
    second_size: f32,
    gap: f32,
) -> String {
    let x2 = x + width(first, first_size) + gap;
    text(runs_on(
        1000.0,
        &[
            (x, 100.0, first, first_size),
            (x2, 100.0, second, second_size),
        ],
    ))
}

/// Page positions for the rows within 0.01 em of a threshold (spec §4.10,
/// revisions 7 and 7b): a row on a threshold would classify differently
/// at some of these, as the f32 positions land either side of it.
const POSITIONS: [f32; 6] = [10.0, 33.3, 72.0, 100.0, 250.0, 499.9];

#[test]
fn kp1_rows_nearest_the_threshold_hold_at_every_position() {
    for x in POSITIONS {
        // 3 pt before a 20 pt glyph: 0.15 em, just above 0.149.
        assert_eq!(
            pair_at(x, "small", 8.0, "Big", 20.0, 3.0),
            "small Big\n",
            "x = {x}"
        );
        // 3.25 pt at 22 pt: 0.1477 em, just below 0.149.
        assert_eq!(
            pair_at(x, "Alpha", 22.0, "Beta", 22.0, 3.25),
            "AlphaBeta\n",
            "x = {x}"
        );
    }
}

// --- KP2: same-baseline break --------------------------------------------

#[test]
fn kp2_gap_break_11pt() {
    sweep(&[6.0, 8.0, 8.5, 8.7], "Alpha Beta\n", |g| {
        pair("Alpha", 11.0, "Beta", 11.0, g)
    });
    sweep(
        &[8.8, 9.0, 10.0, 12.0, 16.0, 20.0, 28.0],
        "Alpha\nBeta\n",
        |g| pair("Alpha", 11.0, "Beta", 11.0, g),
    );
}

#[test]
fn kp2_gap_break_22pt() {
    sweep(&[4.0, 8.0, 12.0, 16.0, 17.5], "Alpha Beta\n", |g| {
        pair("Alpha", 22.0, "Beta", 22.0, g)
    });
    sweep(&[18.0, 20.0, 24.0, 28.0], "Alpha\nBeta\n", |g| {
        pair("Alpha", 22.0, "Beta", 22.0, g)
    });
}

// --- KP3: backward movement ----------------------------------------------

#[test]
fn kp3_backward_movement_11pt() {
    let back = |b: f32| pair("Alpha", 11.0, "Beta", 11.0, -b);
    sweep(&[1.0, 2.0, 4.0, 6.0, 8.0, 8.5], "AlphaBeta\n", back);
    sweep(&[9.0, 12.0, 20.0, 26.0], "Alpha\nBeta\n", back);
}

#[test]
fn kp3_right_then_left_is_two_lines() {
    let got = text(runs(&[
        (300.0, 100.0, "Right", 11.0),
        (100.0, 100.0, "Left", 11.0),
    ]));
    assert_eq!(got, "Right\nLeft\n");
}

// --- Across the baseline: superscripts, subscripts, diagonals -------------

/// `Base` at 11 pt, then `sup` at `size` starting at Base's end shifted
/// `along` forwards and `up` points up (negative: down).
fn base_sup(size: f32, along: f32, up: f32) -> String {
    base_sup_at(100.0, size, along, up)
}

/// As [`base_sup`], with `Base` at `(x, 100)`.
fn base_sup_at(x: f32, size: f32, along: f32, up: f32) -> String {
    let x2 = x + width("Base", 11.0) + along;
    text(runs_on(
        1000.0,
        &[(x, 100.0, "Base", 11.0), (x2, 100.0 - up, "sup", size)],
    ))
}

#[test]
fn rows_nearest_the_break_thresholds_hold_at_every_position() {
    for x in POSITIONS {
        // gap_break = 0.798: 0.7909 em space, 0.8000 em new line,
        // 0.7955 em space, 0.8000 em new line.
        assert_eq!(
            pair_at(x, "Alpha", 11.0, "Beta", 11.0, 8.7),
            "Alpha Beta\n",
            "x = {x}"
        );
        assert_eq!(
            pair_at(x, "Alpha", 11.0, "Beta", 11.0, 8.8),
            "Alpha\nBeta\n",
            "x = {x}"
        );
        assert_eq!(
            pair_at(x, "Alpha", 22.0, "Beta", 22.0, 17.5),
            "Alpha Beta\n",
            "x = {x}"
        );
        assert_eq!(
            pair_at(x, "small", 8.0, "Big", 20.0, 16.0),
            "small\nBig\n",
            "x = {x}"
        );
        // perp_break = 0.809: 0.8000 em joins, 0.8182 em breaks.
        assert_eq!(base_sup_at(x, 11.0, 0.0, 8.8), "Basesup\n", "x = {x}");
        assert_eq!(base_sup_at(x, 11.0, 0.0, 9.0), "Base\nsup\n", "x = {x}");
    }
}

#[test]
fn perp_superscript_7pt_raised() {
    sweep(&[2.0, 3.0, 4.0, 5.0], "Basesup\n", |r| {
        base_sup(7.0, 0.0, r)
    });
    sweep(&[6.0, 8.0, 10.0, 12.0], "Base\nsup\n", |r| {
        base_sup(7.0, 0.0, r)
    });
}

#[test]
fn perp_11pt_raised() {
    sweep(&[2.0, 4.0, 6.0, 8.0, 8.8], "Basesup\n", |r| {
        base_sup(11.0, 0.0, r)
    });
    sweep(&[9.0, 10.0, 12.0], "Base\nsup\n", |r| {
        base_sup(11.0, 0.0, r)
    });
}

#[test]
fn perp_7pt_lowered() {
    sweep(&[2.0, 4.0], "Basesup\n", |r| base_sup(7.0, 0.0, -r));
    sweep(&[6.0, 8.0, 10.0], "Base\nsup\n", |r| base_sup(7.0, 0.0, -r));
}

#[test]
fn diagonals_use_independent_thresholds() {
    // Spec A.1: along and across are tested separately, not as a distance.
    for (along, up) in [
        (5.0, 5.0),
        (6.0, 5.0),
        (6.0, 6.0),
        (7.0, 5.0),
        (5.0, 7.0),
        (7.0, 7.0),
        (8.0, 3.0),
        (3.0, 8.0),
        (8.5, 1.0),
        (8.5, 2.0),
        (2.0, 8.5),
    ] {
        assert_eq!(base_sup(11.0, along, up), "Base sup\n", "({along}, {up})");
    }
    assert_eq!(base_sup(11.0, 1.0, 8.5), "Basesup\n");
}

// --- KP4, KP5 -------------------------------------------------------------

#[test]
fn kp4_fake_bold_duplicates_are_kept() {
    for (dx, dy) in [(0.3, 0.0), (0.3, 0.3), (0.0, 0.0)] {
        let got = text(runs(&[
            (100.0, 100.0, "Bold", 11.0),
            (100.0 + dx, 100.0 - dy, "Bold", 11.0),
        ]));
        assert_eq!(got, "Bold\nBold\n", "offset ({dx}, {dy})");
    }
}

#[test]
fn kp5_whitespace_is_kept_verbatim() {
    let got = text(runs(&[
        (100.0, 100.0, "Hello   ", 11.0),
        (100.0, 120.0, "World", 11.0),
    ]));
    assert_eq!(got, "Hello   \nWorld\n");
    assert_eq!(text(runs(&[(100.0, 100.0, "   Lead", 11.0)])), "   Lead\n");
    // Code 9 has no WinAnsi glyph: unmapped, emitted raw as TAB (kept).
    assert_eq!(
        text(raw("BT /Helv 11 Tf 100 700 Td (A\\tB) Tj ET")),
        "A\tB\n"
    );
    // WinAnsi 0xA0 is read as a space (KP15).
    assert_eq!(text(runs(&[(100.0, 100.0, "A\u{A0}B", 11.0)])), "A B\n");
    // No second space after a trailing one (spec A.1).
    assert_eq!(pair("Alpha ", 11.0, "Beta", 11.0, 3.0), "Alpha Beta\n");
}

// --- Spacing operators (T30) ---------------------------------------------

#[test]
fn character_and_word_spacing_move_the_pen() {
    let tc = |tc: &str| {
        text(raw(&format!(
            "BT /Helv 11 Tf {tc} Tc 100 700 Td (Hello) Tj ET"
        )))
    };
    assert_eq!(tc("1.5"), "Hello\n");
    assert_eq!(tc("2"), "H e l l o\n");
    assert_eq!(tc("9"), "H\ne\nl\nl\no\n");
    assert_eq!(
        text(raw("BT /Helv 11 Tf 20 Tw 100 700 Td (a b c) Tj ET")),
        "a \nb \nc\n"
    );
}

#[test]
fn tj_adjustments_and_horizontal_scaling() {
    assert_eq!(
        text(raw(
            "BT /Helv 11 Tf 100 700 Td [(A) -300 (B) -2000 (C) 500 (D) -9000 (E)] TJ ET"
        )),
        "A B\nCD\nE\n"
    );
    assert_eq!(
        text(raw(
            "BT /Helv 11 Tf 50 Tz 100 700 Td (Alpha) Tj 1 0 Td (Beta) Tj ET"
        )),
        "Alpha\nBeta\n"
    );
}

// --- KP12: vertical text ---------------------------------------------------

#[test]
fn kp12_vertical_text_is_one_line_per_glyph() {
    let vertical = Type0Font {
        encoding: "Identity-V".to_owned(),
        ordering: "Japan1".to_owned(),
        supplement: 6,
        cid_subtype: CidFontSubtype::Type0,
        descriptor: Some(FontDescriptor::default()),
        ..Type0Font::identity_h("MS-Mincho")
    };
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font("V", FontDict::Type0(vertical))
        .raw_content("BT /V 11 Tf 300 600 Td <002200230024> Tj ET")
        .raw_content("BT /Helv 11 Tf 100 300 Td (Horizontal) Tj ET");
    assert_eq!(text(b.to_bytes()), "A\nB\nC\nHorizontal\n");
}

// --- Blocks and the separator (T32, T33) ----------------------------------

fn two_blocks() -> Vec<u8> {
    runs(&[
        (100.0, 100.0, "Line one.", 11.0),
        (100.0, 114.0, "line two.", 11.0),
        (100.0, 314.0, "Block two.", 11.0),
    ])
}

#[test]
fn blocks_change_only_the_blank_line_separator() {
    assert_eq!(text(two_blocks()), "Line one.\nline two.\nBlock two.\n");
    let blank = TextOptions {
        block_separator: BlockSeparator::BlankLine,
        ..TextOptions::default()
    };
    assert_eq!(
        text_with(two_blocks(), &blank),
        "Line one.\nline two.\n\nBlock two.\n"
    );
}

#[test]
fn text_page_structure_and_accessors() {
    let doc = Document::from_bytes(two_blocks(), &OpenOptions::default()).expect("opens");
    let tp = doc
        .page(0)
        .expect("page")
        .text_page(&TextOptions::default())
        .expect("extracts");
    assert_eq!(tp.source(), TextSource::Content);
    assert_eq!(tp.dpi(), None);
    assert!(tp.warnings().is_empty());
    let blocks = tp.blocks();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].lines().len(), 2);
    let line = &blocks[0].lines()[0];
    assert_eq!(line.text(), "Line one.");
    assert_eq!(line.dir(), (1.0, 0.0));
    // The synthetic space before "one." stands for the word gap.
    let space = &line.chars()[4];
    assert_eq!(space.text(), " ");
    assert!(space.advance() > 0.0 && space.size() == 11.0);
    assert_eq!(space.dir(), (1.0, 0.0));
    let first = &line.chars()[0];
    assert_eq!(first.origin(), (100.0, 742.0));
    // Boxes: the line's covers its glyphs, the block's covers its lines.
    let [x0, y0, x1, y1] = line.bbox();
    assert!((x0 - 100.0).abs() < 1e-3 && x1 > 140.0);
    assert!((y0 - (742.0 - 2.2)).abs() < 1e-3 && (y1 - (742.0 + 8.8)).abs() < 1e-3);
    let [_, by0, _, by1] = blocks[0].bbox();
    assert!(by0 < y0 && by1 >= y1);
    assert!(format!("{first:?}").contains("\"L\""));
    // Serialising twice, or after close, gives the same text.
    doc.close();
    assert_eq!(
        tp.to_text(BlockSeparator::Newline),
        tp.to_text(BlockSeparator::Newline)
    );
    assert_eq!(
        tp.to_text(BlockSeparator::Newline),
        "Line one.\nline two.\nBlock two.\n"
    );
}

#[test]
fn text_equals_text_page_to_text() {
    let doc = Document::from_bytes(two_blocks(), &OpenOptions::default()).expect("opens");
    let page = doc.page(0).expect("page");
    for sep in [BlockSeparator::Newline, BlockSeparator::BlankLine] {
        let opts = TextOptions {
            block_separator: sep,
            ..TextOptions::default()
        };
        assert_eq!(
            page.text(&opts).expect("text"),
            page.text_page(&opts).expect("text page").to_text(sep)
        );
    }
}

// --- Rotation, empty pages, zero size --------------------------------------

#[test]
fn rotated_page_gives_the_unrotated_text() {
    let build = |rotate: Option<i32>| {
        let mut b = PdfBuilder::new();
        let p = b.add_page_a4();
        if let Some(r) = rotate {
            p.set_rotation(r);
        }
        for (y, t) in [
            (100.0, "Line one."),
            (114.0, "line two."),
            (314.0, "Block two."),
        ] {
            p.insert_text((100.0, y), t, TextStyle::default())
                .expect("text");
        }
        p.insert_text(
            (100.0 + width("Alpha", 11.0) + 2.0, 400.0),
            "Beta",
            TextStyle::default(),
        )
        .expect("text");
        p.insert_text((100.0, 400.0), "Alpha", TextStyle::default())
            .expect("text");
        b.to_bytes()
    };
    let plain = text(build(None));
    assert_eq!(plain, "Line one.\nline two.\nBlock two.\nBeta\nAlpha\n");
    for r in [90, 180, 270] {
        assert_eq!(text(build(Some(r))), plain, "/Rotate {r}");
    }
}

#[test]
fn empty_page_and_zero_size_font() {
    let mut b = PdfBuilder::new();
    b.add_page_a4();
    assert_eq!(text(b.to_bytes()), "");
    assert_eq!(
        text(raw("BT /Helv 0.001 Tf 100 700 Td (Zero) Tj ET")),
        "Zero\n"
    );
    assert_eq!(text(raw("BT /Helv 0 Tf 100 700 Td (Zero) Tj ET")), "Zero\n");
}

// --- Unmapped::Skip leaves gaps (record-model bullet) -----------------------

/// A Type 3 font whose glyphs are all 600 units wide: code 0x42 is named
/// `whitesquare` (U+25A1); the others have names outside the glyph list.
fn kp6_font() -> FontDict {
    let glyph = |code: u8, name: &str| Type3Glyph {
        code,
        name: name.to_owned(),
        width: 600.0,
        procedure: "600 0 d0 0 0 100 100 re f".to_owned(),
    };
    FontDict::Type3(Type3Font {
        font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        font_bbox: [0.0, 0.0, 600.0, 600.0],
        glyphs: vec![
            glyph(65, "g65"),
            glyph(66, "whitesquare"),
            glyph(0, "g0"),
            glyph(1, "g1"),
            glyph(9, "g9"),
            glyph(10, "g10"),
            glyph(32, "g32"),
            glyph(200, "g200"),
        ],
        to_unicode: None,
    })
}

fn type3_page(codes: &str) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font("F", kp6_font())
        .raw_content(&format!("BT /F 20 Tf 72 700 Td <{codes}> Tj ET"));
    b.to_bytes()
}

#[test]
fn unmapped_policies_on_the_kp6_fixture() {
    let with = |u: Unmapped, codes: &str| {
        text_with(
            type3_page(codes),
            &TextOptions {
                unmapped: u,
                ..TextOptions::default()
            },
        )
    };
    let all = "41420001090A20C8";
    // The LF comes from an unmapped code, the one way a line holds '\n'.
    assert_eq!(
        with(Unmapped::Cid, all),
        "A\u{25A1}\u{FFFD}\u{FFFD}\t\n \u{C8}\n"
    );
    assert_eq!(
        with(Unmapped::Replacement, all),
        format!("\u{FFFD}\u{25A1}{}\n", "\u{FFFD}".repeat(6))
    );
    assert_eq!(with(Unmapped::Skip, all), "\u{25A1}\n");
    // A skipped glyph's advance (0.6 em) is a gap: a space ...
    assert_eq!(with(Unmapped::Skip, "424142"), "\u{25A1} \u{25A1}\n");
    assert_eq!(with(Unmapped::Cid, "424142"), "\u{25A1}A\u{25A1}\n");
    // ... and three of them (1.8 em) a line break.
    assert_eq!(with(Unmapped::Skip, "4241414142"), "\u{25A1}\n\u{25A1}\n");
}
