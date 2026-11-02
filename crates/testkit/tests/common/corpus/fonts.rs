//! Unicode-chain fixtures: unmapped codes (KP6), vertical text (KP12),
//! simple-font encodings (KP15), non-embedded Identity CID fonts in the
//! standard Macintosh glyph order (KP16, KP22), and ToUnicode entries that
//! map to control characters (T26, revision 10b) in Type 1, TrueType, Type 0
//! and Type 3 fonts. Fonts are never embedded (spec T50); every CIDFont
//! except the one KP22 is about carries a `/FontDescriptor`, without which
//! the oracle emits raw bytes (§4.13).

use pdf_extract_testkit::{
    BaseEncoding, CidFontSubtype, FontDescriptor, FontDict, PdfBuilder, SimpleEncoding, Type0Font,
    Type1Font, Type3Font, Type3Glyph, to_unicode_cmap,
};

use super::{Fixture, assemble, stream};

/// One A4 page showing `content` with `font` as `/F`.
fn with_font(font: FontDict, content: &str) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4().add_font("F", font).raw_content(content);
    b.to_bytes()
}

fn helvetica(encoding: SimpleEncoding) -> FontDict {
    FontDict::Type1(Type1Font {
        base_font: "Helvetica".to_owned(),
        encoding,
        to_unicode: None,
    })
}

/// A Type 3 font, 1000 units per em, every glyph 600 units wide.
fn type3(glyphs: &[(u8, &str)]) -> FontDict {
    FontDict::Type3(Type3Font {
        font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        font_bbox: [0.0, 0.0, 600.0, 600.0],
        glyphs: glyphs
            .iter()
            .map(|&(code, name)| Type3Glyph {
                code,
                name: name.to_owned(),
                width: 600.0,
                procedure: "600 0 d0 0 0 400 600 re f".to_owned(),
            })
            .collect(),
        to_unicode: None,
    })
}

/// Non-embedded Identity-H `Arial` of the given CIDFont subtype, with or
/// without a descriptor; 500-unit default width.
fn arial(subtype: CidFontSubtype, descriptor: bool) -> FontDict {
    let mut f = Type0Font::identity_h("Arial");
    f.cid_subtype = subtype;
    f.default_width = Some(500.0);
    if descriptor {
        f.descriptor = Some(FontDescriptor::default());
    }
    FontDict::Type0(f)
}

/// Two-byte hex codes of `cids`.
fn cid_hex(cids: &[u16]) -> String {
    cids.iter().map(|c| format!("{c:04X}")).collect()
}

/// The CID of a printable ASCII character in the standard Macintosh glyph
/// order: `space` is glyph 3 and the order then runs through `asciitilde`
/// in ASCII order, so the offset is constant.
fn mac_cid(c: char) -> u16 {
    assert!((' '..='~').contains(&c), "printable ASCII only");
    u16::try_from(u32::from(c) - 29).expect("small")
}

fn kp6() -> Vec<Fixture> {
    vec![
        // Codes 65 (`g65`), 66 (`whitesquare`, U+25A1), 0, 1, 9, 10, 32
        // (`g32`), 200: A.4.
        Fixture::exact(
            "kp06_type3_unmapped.pdf",
            &["KP6"],
            with_font(
                type3(&[
                    (65, "g65"),
                    (66, "whitesquare"),
                    (0, "g0"),
                    (1, "g1"),
                    (9, "g9"),
                    (10, "g10"),
                    (32, "g32"),
                    (200, "g200"),
                ]),
                "BT /F 20 Tf 72 700 Td <41420001090A20C8> Tj ET",
            ),
        )
        .diverges(
            "KP6 (D-10): unmapped codes 0 and 1 are emitted as U+FFFD; the oracle emits \
             the raw NUL and U+0001",
        ),
        Fixture::exact(
            "kp06_type3_square.pdf",
            &["KP6"],
            with_font(
                type3(&[(65, "g65"), (66, "square")]),
                "BT /F 20 Tf 72 700 Td <4142> Tj ET",
            ),
        )
        .diverges(
            "§2.5 TeX glyph names (D-39): /square is not in the AGL, so we emit the raw \
             code; the oracle maps it to U+25A1",
        ),
        Fixture::exact(
            "kp06_type1_differences.pdf",
            &["KP6"],
            with_font(
                helvetica(SimpleEncoding::Differences {
                    base: None,
                    differences: vec![
                        (65, "zzunknown".into()),
                        (66, "uni2603".into()),
                        (128, "g128".into()),
                    ],
                }),
                "BT /F 11 Tf 72 700 Td <414280> Tj ET",
            ),
        ),
        Fixture::exact(
            "kp06_type0_identity_nonembedded.pdf",
            &["KP6", "KP16"],
            with_font(
                arial(CidFontSubtype::Type2, true),
                "BT /F 11 Tf 72 700 Td <00240025002600030044> Tj ET",
            ),
        ),
    ]
}

fn kp12() -> Vec<Fixture> {
    let mut vertical = Type0Font::identity_h("MS-Mincho");
    vertical.encoding = "Identity-V".into();
    vertical.ordering = "Japan1".into();
    vertical.supplement = 2;
    vertical.cid_subtype = CidFontSubtype::Type0;
    vertical.descriptor = Some(FontDescriptor::default());
    let font = FontDict::Type0(vertical);
    let show = "BT /F 11 Tf 300 600 Td <002200230024> Tj ET";
    vec![
        Fixture::exact(
            "kp12_identity_v.pdf",
            &["KP12"],
            with_font(font.clone(), show),
        ),
        Fixture::exact(
            "kp12_vertical_then_horizontal.pdf",
            &["KP12"],
            with_font(
                font,
                &format!("{show}\nBT /Helv 11 Tf 100 300 Td (Horizontal) Tj ET"),
            ),
        ),
    ]
}

fn kp15() -> Vec<Fixture> {
    let kp = &["KP15"];
    vec![
        Fixture::exact(
            "kp15_winansi.pdf",
            kp,
            with_font(
                helvetica(SimpleEncoding::Named(BaseEncoding::WinAnsi)),
                "BT /F 11 Tf 72 700 Td <61A062AD63> Tj ET",
            ),
        ),
        Fixture::exact(
            "kp15_macroman.pdf",
            kp,
            with_font(
                helvetica(SimpleEncoding::Named(BaseEncoding::MacRoman)),
                "BT /F 11 Tf 72 700 Td <61CA62> Tj ET",
            ),
        ),
        Fixture::exact(
            "kp15_standard.pdf",
            kp,
            with_font(
                helvetica(SimpleEncoding::Builtin),
                "BT /F 11 Tf 72 700 Td <61A062AD63> Tj ET",
            ),
        ),
        Fixture::exact(
            "kp15_symbol.pdf",
            kp,
            with_font(
                FontDict::standard14("Symbol", SimpleEncoding::Builtin),
                "BT /F 11 Tf 72 700 Td (abg) Tj ET",
            ),
        ),
        Fixture::exact(
            "kp15_differences_nbspace.pdf",
            kp,
            with_font(
                helvetica(SimpleEncoding::Differences {
                    base: Some(BaseEncoding::WinAnsi),
                    differences: vec![(0xA0, "nbspace".into())],
                }),
                "BT /F 11 Tf 72 700 Td <61A062> Tj ET",
            ),
        )
        .diverges(
            "KP15 (§2.5): /Differences restating nbspace at 0xA0 under WinAnsi; we read \
             U+0020 (hayro cannot tell where the name came from), the oracle keeps U+00A0",
        ),
        // Appendix C lists this row as a divergence, but both readers keep
        // U+00A0 and U+00AD for Type 3 names (T26, re-probed), and the
        // oracle agrees with us on this file; per §4.13 a divergence that
        // disappears is made exact (reported to the spec owner).
        Fixture::exact(
            "kp15_type3_nbspace.pdf",
            kp,
            with_font(
                type3(&[
                    (0x61, "a"),
                    (0x62, "b"),
                    (0xA0, "nbspace"),
                    (0xAD, "sfthyphen"),
                    (0xCA, "nbspace"),
                ]),
                "BT /F 11 Tf 72 700 Td <61A062AD61CA62> Tj ET",
            ),
        ),
    ]
}

fn kp16_22() -> Vec<Fixture> {
    let text = "Hello, World 2026.";
    let cids: Vec<u16> = text.chars().map(mac_cid).collect();
    vec![
        Fixture::exact(
            "kp16_identity_nonembedded_macorder.pdf",
            &["KP16"],
            with_font(
                arial(CidFontSubtype::Type2, true),
                &format!("BT /F 11 Tf 72 700 Td <{}> Tj ET", cid_hex(&cids)),
            ),
        ),
        Fixture::exact(
            "kp22_cid_no_descriptor.pdf",
            &["KP22"],
            with_font(
                arial(CidFontSubtype::Type2, false),
                "BT /F 11 Tf 72 700 Td <00240025002600030044> Tj ET",
            ),
        )
        .diverges(
            "KP22: a non-embedded CID font without /FontDescriptor; the oracle emits the \
             raw string bytes, we apply the standard Macintosh order",
        ),
        Fixture::exact(
            "kp22_cidfonttype0_macorder.pdf",
            &["KP16", "KP22"],
            with_font(
                arial(CidFontSubtype::Type0, true),
                "BT /F 11 Tf 72 700 Td <00240025002600030044> Tj ET",
            ),
        ),
        // CID 257 is the last Mac name (`dcroat`); 258 and beyond are raw.
        Fixture::exact(
            "kp22_cid_ge_258_raw.pdf",
            &["KP16", "KP22"],
            with_font(
                arial(CidFontSubtype::Type2, true),
                "BT /F 11 Tf 72 700 Td <01010102010301040106012C> Tj ET",
            ),
        ),
    ]
}

/// ToUnicode entries that map to control characters (T26 step 1, spec
/// revision 10b, Appendix A.4): `A`, the code of `x` mapped to the target,
/// `B`, in WinAnsi Helvetica, no gap. A single code point in U+0008-U+000D
/// reads as a space, any other single control is ignored (the glyph name
/// `x` decides), U+00A0 and U+2028 are taken as written; entries of several
/// code points are verbatim and then sanitised (D-10), which is where we
/// part from the oracle for NUL.
fn tounicode_controls() -> Vec<Fixture> {
    let page = |target: &str| {
        let cmap = to_unicode_cmap(1, &[(0x41, "A"), (0x42, "B"), (0x78, target)]);
        let font = FontDict::Type1(Type1Font {
            base_font: "Helvetica".to_owned(),
            encoding: SimpleEncoding::Named(BaseEncoding::WinAnsi),
            to_unicode: Some(cmap),
        });
        with_font(font, "BT /F 11 Tf 72 700 Td <417842> Tj ET")
    };
    let mut out = Vec::new();
    for cp in [
        0x0000u32, 0x0009, 0x000A, 0x000D, 0x001F, 0x007F, 0x0080, 0x0085, 0x009F, 0x00A0, 0x2028,
    ] {
        let target = char::from_u32(cp).expect("scalar value").to_string();
        out.push(Fixture::exact(
            format!("tounicode_ctl_{cp:04X}.pdf"),
            &[],
            page(&target),
        ));
    }
    out.push(Fixture::exact(
        "tounicode_multi_0041_0009.pdf",
        &[],
        page("A\t"),
    ));
    out.push(
        Fixture::exact("tounicode_multi_0041_0000.pdf", &[], page("A\0")).diverges(
            "D-10: a multi-code-point ToUnicode entry is verbatim, then sanitised; its NUL \
             becomes U+FFFD where the oracle emits it raw",
        ),
    );
    out.push(Fixture::exact(
        "tounicode_multi_0041_000A_0042.pdf",
        &[],
        page("A\nB"),
    ));
    out.extend(tounicode_controls_other_fonts());
    out
}

/// The single-code-point targets of the sweep in the other font types: each
/// rule of T26 step 1 at least once (space, ignored C0, ignored DEL and C1,
/// honoured U+00A0 and U+2028). Revision 10b probed Type 1, TrueType and
/// Type 0 and found no font-type dependence, but only the Type 1 rows were
/// committed; the Type 3 rows are the one ToUnicode case left open (spec
/// §4.14, expected identical).
const CONTROL_SWEEP: [u32; 10] = [
    0x0000, 0x0009, 0x000A, 0x000D, 0x001F, 0x007F, 0x0085, 0x009F, 0x00A0, 0x2028,
];

/// `tounicode_ctl_{tt,t0,t3}_<cp>`: `A`, the glyph for `x` mapped to the
/// target, `B`, no gap, in a non-embedded TrueType font, a non-embedded
/// Identity-H CIDFontType2 in the standard Macintosh order (KP16) and a
/// Type 3 font. Where the entry is ignored, the font's own reading of the
/// code decides: the WinAnsi or Macintosh glyph name, or the Type 3 glyph
/// name `x`.
fn tounicode_controls_other_fonts() -> Vec<Fixture> {
    let mut out = Vec::new();
    for cp in CONTROL_SWEEP {
        let target = char::from_u32(cp).expect("scalar value").to_string();
        out.push(Fixture::exact(
            format!("tounicode_ctl_tt_{cp:04X}.pdf"),
            &[],
            truetype_page(&target),
        ));
        let cids = [mac_cid('A'), mac_cid('x'), mac_cid('B')];
        let cmap = to_unicode_cmap(
            2,
            &[
                (u32::from(cids[0]), "A"),
                (u32::from(cids[2]), "B"),
                (u32::from(cids[1]), &target),
            ],
        );
        let mut font = Type0Font::identity_h("Arial");
        font.default_width = Some(500.0);
        font.descriptor = Some(FontDescriptor::default());
        font.to_unicode = Some(cmap);
        out.push(Fixture::exact(
            format!("tounicode_ctl_t0_{cp:04X}.pdf"),
            &[],
            with_font(
                FontDict::Type0(font),
                &format!("BT /F 11 Tf 72 700 Td <{}> Tj ET", cid_hex(&cids)),
            ),
        ));
        let FontDict::Type3(mut t3) = type3(&[(0x41, "A"), (0x42, "B"), (0x78, "x")]) else {
            unreachable!("type3() builds a Type 3 font");
        };
        t3.to_unicode = Some(to_unicode_cmap(
            1,
            &[(0x41, "A"), (0x42, "B"), (0x78, &target)],
        ));
        out.push(Fixture::exact(
            format!("tounicode_ctl_t3_{cp:04X}.pdf"),
            &[],
            with_font(FontDict::Type3(t3), "BT /F 11 Tf 72 700 Td <417842> Tj ET"),
        ));
    }
    out
}

/// One A4 page showing `A x B` in a non-embedded WinAnsi TrueType `Arial`
/// whose ToUnicode maps `x` to `target`. Assembled by hand: the test kit's
/// `FontDict` offers simple fonts only as `/Type1` (spec §3.4).
fn truetype_page(target: &str) -> Vec<u8> {
    let cmap = to_unicode_cmap(1, &[(0x41, "A"), (0x42, "B"), (0x78, target)]);
    assemble(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R \
         /Resources << /Font << /F 5 0 R >> >> >>"
            .to_owned(),
        stream("", "BT /F 11 Tf 72 700 Td <417842> Tj ET"),
        "<< /Type /Font /Subtype /TrueType /BaseFont /Arial \
         /Encoding /WinAnsiEncoding /ToUnicode 6 0 R >>"
            .to_owned(),
        stream("", &cmap),
    ])
}

pub fn fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    out.extend(kp6());
    out.extend(kp12());
    out.extend(kp15());
    out.extend(kp16_22());
    out.extend(tounicode_controls());
    out
}
