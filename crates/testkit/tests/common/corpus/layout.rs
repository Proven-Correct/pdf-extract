//! Line-assembly fixtures (spec Appendix A.1, Appendix C rows KP1-KP5,
//! perp, spacing ops) and the KP24 whitespace probe.
//!
//! Every recipe places Helvetica runs with the AFM widths, so a "gap" is the
//! distance from the end of one run to the start of the next, exactly as the
//! A.1 probes measured it with the oracle's `get_text_length`. The six-position
//! sweeps of the near-threshold rows are `crates/core` §4.10 tests, not
//! fixtures (spec revision 10): at a gap exactly on 0.15 or 0.8 em the
//! oracle's answer depends on its own f32 rounding of the page position.

use pdf_extract_testkit::{
    BaseEncoding, FontDict, PdfBuilder, SimpleEncoding, Type1Font, Type3Font, Type3Glyph,
    to_unicode_cmap,
};

use super::{Fixture, n, raw, runs, runs_on, width};

/// `first` at (100, 100) and `second` `gap` points after its end.
fn pair(first: &str, first_size: f32, second: &str, second_size: f32, gap: f32) -> Vec<u8> {
    let x2 = 100.0 + width(first, first_size) + gap;
    runs(&[
        (100.0, 100.0, first, first_size),
        (x2, 100.0, second, second_size),
    ])
}

/// `Base` at 11 pt at (100, 100), then `sup` at `size` starting at Base's
/// end shifted `along` forwards and `up` points up (negative: down).
fn base_sup(size: f32, along: f32, up: f32) -> Vec<u8> {
    let x2 = 100.0 + width("Base", 11.0) + along;
    runs_on(
        1000.0,
        &[(100.0, 100.0, "Base", 11.0), (x2, 100.0 - up, "sup", size)],
    )
}

pub fn fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    let kp1 = &["KP1"];
    let kp2 = &["KP2"];

    // KP1: space gap. Every row is at least 0.001 em from SPACE_GAP 0.149
    // (Appendix C, revision 7): 22 pt 3.25 is 0.1477 em, 8->20 pt 3 is
    // 0.1500 em, the two ends of the bracket.
    for g in [0.5, 1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0] {
        out.push(Fixture::exact(
            format!("kp01_space_11pt_{}.pdf", n(g)),
            kp1,
            pair("Alpha", 11.0, "Beta", 11.0, g),
        ));
    }
    for g in [0.5, 1.0, 2.0, 3.0, 3.25, 3.5, 3.75, 4.0] {
        out.push(Fixture::exact(
            format!("kp01_space_22pt_{}.pdf", n(g)),
            kp1,
            pair("Alpha", 22.0, "Beta", 22.0, g),
        ));
    }
    for g in [0.0, 2.0, 4.0, 8.0] {
        out.push(Fixture::exact(
            format!("kp01_mixed_20to8_{}.pdf", n(g)),
            kp1,
            pair("Big", 20.0, "small", 8.0, g),
        ));
    }
    for g in [1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 18.0] {
        let points: &[&'static str] = if g >= 16.0 { &["KP1", "KP2"] } else { kp1 };
        out.push(Fixture::exact(
            format!("kp01_mixed_8to20_{}.pdf", n(g)),
            points,
            pair("small", 8.0, "Big", 20.0, g),
        ));
    }
    // KP2: same-baseline break (GAP_BREAK, bracket [0.7955, 0.8)).
    for g in [6.0, 8.0, 8.5, 8.7, 8.8, 9.0, 12.0, 28.0] {
        out.push(Fixture::exact(
            format!("kp02_break_11pt_{}.pdf", n(g)),
            kp2,
            pair("Alpha", 11.0, "Beta", 11.0, g),
        ));
    }
    for g in [16.0, 17.5, 18.0, 28.0] {
        out.push(Fixture::exact(
            format!("kp02_break_22pt_{}.pdf", n(g)),
            kp2,
            pair("Alpha", 22.0, "Beta", 22.0, g),
        ));
    }
    // KP3: backward movement (BACK_BREAK, bracket [0.7727, 0.8182)).
    for b in [1.0, 4.0, 8.0, 8.5, 9.0, 12.0, 26.0] {
        out.push(Fixture::exact(
            format!("kp03_back_11pt_{}.pdf", n(b)),
            &["KP3"],
            pair("Alpha", 11.0, "Beta", 11.0, -b),
        ));
    }
    out.push(Fixture::exact(
        "kp03_right_left.pdf",
        &["KP3"],
        runs(&[(300.0, 100.0, "Right", 11.0), (100.0, 100.0, "Left", 11.0)]),
    ));

    // KP4: fake bold is kept.
    for (file, dx, dy) in [
        ("kp04_fakebold_0.3.pdf", 0.3, 0.0),
        ("kp04_fakebold_0.3x0.3.pdf", 0.3, 0.3),
        ("kp04_fakebold_overprint.pdf", 0.0, 0.0),
    ] {
        out.push(Fixture::exact(
            file,
            &["KP4"],
            runs(&[
                (100.0, 100.0, "Bold", 11.0),
                (100.0 + dx, 100.0 - dy, "Bold", 11.0),
            ]),
        ));
    }

    // KP5: whitespace kept verbatim.
    out.push(Fixture::exact(
        "kp05_trailing.pdf",
        &["KP5"],
        runs(&[
            (100.0, 100.0, "Hello   ", 11.0),
            (100.0, 120.0, "World", 11.0),
        ]),
    ));
    out.push(Fixture::exact(
        "kp05_leading.pdf",
        &["KP5"],
        runs(&[(100.0, 100.0, "   Lead", 11.0)]),
    ));
    out.push(Fixture::exact(
        "kp05_tab.pdf",
        &["KP5"],
        raw("BT /Helv 11 Tf 100 700 Td (A\\tB) Tj ET"),
    ));
    out.push(Fixture::exact(
        "kp05_nbsp_winansi.pdf",
        &["KP5", "KP15"],
        runs(&[(100.0, 100.0, "A\u{A0}B", 11.0)]),
    ));
    // A trailing space then a 3 pt gap: no second space (A.1).
    out.push(Fixture::exact(
        "kp05_trailing_space_then_gap.pdf",
        &["KP1", "KP5"],
        pair("Alpha ", 11.0, "Beta", 11.0, 3.0),
    ));

    // Across the baseline (PERP_BREAK, bracket [0.8, 0.8182)).
    let perp = &["KP2"];
    for r in [2.0, 5.0, 6.0, 12.0] {
        out.push(Fixture::exact(
            format!("perp_7pt_{}.pdf", n(r)),
            perp,
            base_sup(7.0, 0.0, r),
        ));
    }
    for r in [8.8, 9.0] {
        out.push(Fixture::exact(
            format!("perp_11pt_{}.pdf", n(r)),
            perp,
            base_sup(11.0, 0.0, r),
        ));
    }
    for r in [4.0, 6.0] {
        out.push(Fixture::exact(
            format!("perp_low_{}.pdf", n(r)),
            perp,
            base_sup(7.0, 0.0, -r),
        ));
    }
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
        (1.0, 8.5),
    ] {
        out.push(Fixture::exact(
            format!("diag_{}x{}.pdf", n(along), n(up)),
            &["KP1", "KP2"],
            base_sup(11.0, along, up),
        ));
    }

    // Spacing operators (T30, A.1).
    for tc in ["1.5", "2", "9"] {
        out.push(Fixture::exact(
            format!("tc_{tc}.pdf"),
            &["KP1", "KP2"],
            raw(&format!("BT /Helv 11 Tf {tc} Tc 100 700 Td (Hello) Tj ET")),
        ));
    }
    out.push(Fixture::exact(
        "tw_20.pdf",
        &["KP1", "KP2"],
        raw("BT /Helv 11 Tf 20 Tw 100 700 Td (a b c) Tj ET"),
    ));
    out.push(Fixture::exact(
        "tj_adjust.pdf",
        &["KP1", "KP2", "KP3"],
        raw("BT /Helv 11 Tf 100 700 Td [(A) -300 (B) -2000 (C) 500 (D) -9000 (E)] TJ ET"),
    ));
    out.push(Fixture::exact(
        "tz_50.pdf",
        &["KP2"],
        raw("BT /Helv 11 Tf 50 Tz 100 700 Td (Alpha) Tj 1 0 Td (Beta) Tj ET"),
    ));

    out.extend(kp24());
    out
}

/// KP24 rows (spec Appendix C, revision 10): each probed character before
/// and after a 0.3 em gap. The reference's rule, which T30 adopts: as the
/// incoming glyph only U+0020 suppresses the synthetic space; as the line's
/// last character U+0020, U+1680, U+3000 and U+FEFF do; nothing else does.
///
/// `kp24_w_<cp>_*`: WinAnsi Helvetica whose ToUnicode maps the code of `x`
/// to the probed character, so both readers advance by Helvetica's AFM
/// width of `x` whatever the character (a glyph named `nbspace` has no AFM
/// width, and a reader-specific fallback would move the gap).
/// `kp24_t3_<name>_*`: a Type 3 font with explicit widths whose glyph name
/// gives the character.
fn kp24() -> Vec<Fixture> {
    let mut out = Vec::new();
    for cp in [
        0x20u32, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x85, 0xA0, 0x1680, 0x2000, 0x2002, 0x2003, 0x2007,
        0x2009, 0x200A, 0x200B, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF,
    ] {
        let ch = char::from_u32(cp).expect("scalar value").to_string();
        let cmap = to_unicode_cmap(1, &[(0x41, "A"), (0x42, "B"), (0x78, &ch)]);
        let font = FontDict::Type1(Type1Font {
            base_font: "Helvetica".to_owned(),
            encoding: SimpleEncoding::Named(BaseEncoding::WinAnsi),
            to_unicode: Some(cmap),
        });
        for (side, shows) in SIDES {
            let mut b = PdfBuilder::new();
            b.add_page_a4()
                .add_font("W", font.clone())
                .raw_content(&format!("BT /W 11 Tf 100 700 Td {shows} ET"));
            out.push(Fixture::exact(
                format!("kp24_w_{cp:04X}_{side}.pdf"),
                &["KP24"],
                b.to_bytes(),
            ));
        }
    }
    for name in [
        "space", "nbspace", "uni0009", "uni0085", "uni00A0", "uni2003", "uni2028", "uni3000",
    ] {
        for (side, shows) in SIDES {
            out.push(Fixture::exact(
                format!("kp24_t3_{name}_{side}.pdf"),
                &["KP24"],
                type3_ab(name, shows),
            ));
        }
    }
    out
}

/// The probed character before and after a 0.3 em gap: 0.3 em at 11 pt is
/// 3.3 pt, above SPACE_GAP (0.149 em) and far below GAP_BREAK (0.798 em).
/// Code 0x78 carries the probed character.
const SIDES: [(&str, &str); 2] = [
    ("before", "[<4178> -300 <42>] TJ"),
    ("after", "[<41> -300 <7842>] TJ"),
];

/// A page showing `shows` in a Type 3 font with glyphs `A`, `B` and `name`
/// (at code 0x78), all 600 units wide.
fn type3_ab(name: &str, shows: &str) -> Vec<u8> {
    let glyph = |code: u8, name: &str| Type3Glyph {
        code,
        name: name.to_owned(),
        width: 600.0,
        procedure: "600 0 d0 0 0 400 600 re f".to_owned(),
    };
    let t3 = FontDict::Type3(Type3Font {
        font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        font_bbox: [0.0, 0.0, 600.0, 600.0],
        glyphs: vec![glyph(0x41, "A"), glyph(0x42, "B"), glyph(0x78, name)],
        to_unicode: None,
    });
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font("T", t3)
        .raw_content(&format!("BT /T 11 Tf 100 700 Td {shows} ET"));
    b.to_bytes()
}
