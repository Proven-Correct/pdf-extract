//! §4.9 Unicode chain: one fixture per step of T26, and the unmapped policy
//! on the KP6 Type 3 fixture.

use pdf_extract_testkit::{
    BaseEncoding, FontDescriptor, FontDict, PdfBuilder, SimpleEncoding, Type0Font, Type1Font,
    Type3Font, Type3Glyph, to_unicode_cmap,
};

use super::common::assemble_pdf;
use super::{Unmapped, extract, extract_with, joined, texts};
use crate::Limits;

/// One page showing `content` with font `font` as `/F`.
fn with_font(font: FontDict, content: &str) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4().add_font("F", font).raw_content(content);
    b.to_bytes()
}

fn helvetica(encoding: SimpleEncoding, to_unicode: Option<String>) -> FontDict {
    FontDict::Type1(Type1Font {
        base_font: "Helvetica".to_owned(),
        encoding,
        to_unicode,
    })
}

#[test]
fn step1_tounicode_wins_and_may_map_to_several_code_points() {
    let cmap = to_unicode_cmap(1, &[(65, "ffi"), (66, "Z"), (0xA0, "\u{A0}"), (0xAD, "x")]);
    let font = helvetica(SimpleEncoding::Named(BaseEncoding::WinAnsi), Some(cmap));
    let p = extract(with_font(font, "BT /F 11 Tf 72 700 Td <414243A0AD> Tj ET"));
    // C is not in the map: step 2 maps its glyph name. Explicit entries for
    // the KP15 positions are honoured as written (re-probed).
    assert_eq!(texts(&p), ["ffi", "Z", "C", "\u{A0}", "x"]);
}

#[test]
fn step1_a_code_missing_from_the_map_never_reaches_hayros_name_table() {
    // D-39 via H10h: `parenleftbig` is in hayro's glyph-name table (which
    // `as_unicode()` falls back to) but not in the AGL, so with the map
    // silent on code 65 the glyph is unmapped and its raw code emitted, as
    // without a map.
    let cmap = to_unicode_cmap(1, &[(66, "Z")]);
    let differences = SimpleEncoding::Differences {
        base: Some(BaseEncoding::WinAnsi),
        differences: vec![(65, "parenleftbig".into())],
    };
    let p = extract(with_font(
        helvetica(differences.clone(), Some(cmap)),
        "BT /F 11 Tf 72 700 Td <4142> Tj ET",
    ));
    assert_eq!(texts(&p), ["A", "Z"]);
    let p = extract(with_font(
        helvetica(differences, None),
        "BT /F 11 Tf 72 700 Td <41> Tj ET",
    ));
    assert_eq!(texts(&p), ["A"]);
}

#[test]
fn a_simple_glyph_without_a_name_is_unmapped() {
    // T26 has no step for a simple font without ToUnicode whose encoding
    // names nothing at the code (StandardEncoding 0x80): the raw code.
    let font = helvetica(SimpleEncoding::Builtin, None);
    let p = extract(with_font(font, "BT /F 11 Tf 72 700 Td <8041> Tj ET"));
    assert_eq!(texts(&p), ["\u{80}", "A"]);
}

#[test]
fn step2_differences_names() {
    let font = helvetica(
        SimpleEncoding::Differences {
            base: None,
            differences: vec![
                (65, "zzunknown".into()),
                (66, "uni2603".into()),
                (67, "Eacute".into()),
                (128, "g128".into()),
            ],
        },
        None,
    );
    let p = extract(with_font(font, "BT /F 11 Tf 72 700 Td <41424380> Tj ET"));
    // Spec A.4: unknown names fall through to the raw code (KP6).
    assert_eq!(texts(&p), ["A", "\u{2603}", "\u{C9}", "\u{80}"]);
    // KP15 is per encoding position: `nbspace` and `sfthyphen` named by
    // /Differences elsewhere keep U+00A0 and U+00AD (re-probed).
    let font = helvetica(
        SimpleEncoding::Differences {
            base: Some(BaseEncoding::WinAnsi),
            differences: vec![(1, "nbspace".into()), (2, "sfthyphen".into())],
        },
        None,
    );
    let p = extract(with_font(font, "BT /F 11 Tf 72 700 Td <61016202A0> Tj ET"));
    assert_eq!(texts(&p), ["a", "\u{A0}", "b", "\u{AD}", " "]);
}

#[test]
fn step2_kp15_normalisation_per_encoding() {
    // Spec A.4: WinAnsi a\xa0b\xadc → `a b-c`; MacRoman a\xcab → `a b`;
    // StandardEncoding has no glyph at 0xA0 (raw code) and guilsinglright at
    // 0xAD; Symbol's built-in encoding gives Greek.
    let winansi = helvetica(SimpleEncoding::Named(BaseEncoding::WinAnsi), None);
    let p = extract(with_font(
        winansi,
        "BT /F 11 Tf 72 700 Td <61A062AD63> Tj ET",
    ));
    assert_eq!(joined(&p), "a b-c");
    let macroman = helvetica(SimpleEncoding::Named(BaseEncoding::MacRoman), None);
    let p = extract(with_font(macroman, "BT /F 11 Tf 72 700 Td <61CA62> Tj ET"));
    assert_eq!(joined(&p), "a b");
    let standard = helvetica(SimpleEncoding::Builtin, None);
    let p = extract(with_font(
        standard,
        "BT /F 11 Tf 72 700 Td <61A062AD63> Tj ET",
    ));
    assert_eq!(joined(&p), "a\u{A0}b\u{203A}c");
    let symbol = FontDict::standard14("Symbol", SimpleEncoding::Builtin);
    let p = extract(with_font(symbol, "BT /F 11 Tf 72 700 Td (abg) Tj ET"));
    assert_eq!(joined(&p), "\u{3B1}\u{3B2}\u{3B3}");
}

fn type3(glyphs: &[(u8, &str)], to_unicode: Option<String>) -> FontDict {
    FontDict::Type3(Type3Font {
        font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        font_bbox: [0.0, 0.0, 600.0, 600.0],
        glyphs: glyphs
            .iter()
            .map(|&(code, name)| Type3Glyph {
                code,
                name: name.to_owned(),
                width: 600.0,
                procedure: "600 0 d0 0 0 100 100 re f".to_owned(),
            })
            .collect(),
        to_unicode,
    })
}

#[test]
fn step2_type3_names_go_through_the_glyph_list() {
    // hayro maps Type 3 glyphs through ToUnicode only; the device adds the
    // name step for them. A Type 3 `nbspace` stays U+00A0, as in the
    // reference (re-probed).
    let font = type3(
        &[
            (1, "whitesquare"),
            (2, "nbspace"),
            (3, "uni00E9"),
            (4, "g4"),
        ],
        Some(to_unicode_cmap(1, &[(4, "T")])),
    );
    let p = extract(with_font(font, "BT /F 20 Tf 72 700 Td <01020304> Tj ET"));
    assert_eq!(texts(&p), ["\u{25A1}", "\u{A0}", "\u{E9}", "T"]);
}

fn cid_font(encoding: &str, ordering: &str) -> Type0Font {
    Type0Font {
        encoding: encoding.to_owned(),
        ordering: ordering.to_owned(),
        supplement: 6,
        descriptor: Some(FontDescriptor::default()),
        ..Type0Font::identity_h("MS-Mincho")
    }
}

#[test]
fn step3_japan1_ucs2() {
    // Spec A.4: 90ms-RKSJ-H, Japan1, non-embedded → 日本語AB; Identity-H
    // with the Japan1 ordering, CIDs 34-36 → ABC.
    let font = FontDict::Type0(cid_font("90ms-RKSJ-H", "Japan1"));
    let p = extract(with_font(
        font,
        "BT /F 11 Tf 72 700 Td <93FA967B8CEA4142> Tj ET",
    ));
    assert_eq!(joined(&p), "\u{65E5}\u{672C}\u{8A9E}AB");
    let font = FontDict::Type0(cid_font("Identity-H", "Japan1"));
    let p = extract(with_font(
        font,
        "BT /F 11 Tf 72 700 Td <002200230024> Tj ET",
    ));
    assert_eq!(joined(&p), "ABC");
}

/// A page showing two-byte `codes` in an Identity-H CIDFontType2 font
/// `Tinos` with `ordering`, its program embedded (`tests/assets`) or not.
fn tinos_page(embedded: bool, codes: &str) -> Vec<u8> {
    const TINOS: &[u8] = include_bytes!("../../../tests/assets/tinos_subset.ttf");
    let font_file = if embedded { " /FontFile2 8 0 R" } else { "" };
    let mut program = format!("<< /Length {} /Length1 {0} >>\nstream\n", TINOS.len()).into_bytes();
    program.extend_from_slice(TINOS);
    program.extend_from_slice(b"\nendstream");
    let content = format!("BT /C 11 Tf 72 700 Td <{codes}> Tj ET");
    assemble_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
           /Resources << /Font << /C 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len() + 1
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /Tinos /Encoding /Identity-H \
           /DescendantFonts [6 0 R] >>"
            .to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Tinos /CIDSystemInfo \
           << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 7 0 R \
           /CIDToGIDMap /Identity /DW 1000 >>"
            .to_vec(),
        format!(
            "<< /Type /FontDescriptor /FontName /Tinos /Flags 32 /FontBBox [0 -216 1000 891] \
             /ItalicAngle 0 /Ascent 891 /Descent -216 /CapHeight 662 /StemV 80{font_file} >>"
        )
        .into_bytes(),
        program,
    ])
}

#[test]
fn step4_embedded_identity_font_uses_its_reversed_cmap() {
    // Glyphs 3, 4, 1, 5, 6 of the subset are A, B, space, a, b.
    let p = extract(tinos_page(true, "00030004000100050006"));
    assert_eq!(joined(&p), "AB ab");
}

#[test]
fn step4b_standard_mac_order_for_non_embedded_identity_fonts() {
    // KP16 (spec A.4): CIDs 36, 37, 38, 3, 68 → `ABC a`. CIDs 0 and 1
    // (.notdef, .null) have no Unicode and fall to the unmapped policy;
    // CID 259 is past the 258 names and stays its raw value (re-probed).
    let mut arial = Type0Font::identity_h("Arial");
    arial.descriptor = Some(FontDescriptor::default());
    let p = extract(with_font(
        FontDict::Type0(arial.clone()),
        "BT /F 11 Tf 72 700 Td <002400250026000300440000000101030103> Tj ET",
    ));
    assert_eq!(
        texts(&p),
        [
            "A", "B", "C", " ", "a", "\u{FFFD}", "\u{FFFD}", "\u{103}", "\u{103}"
        ]
    );
    // The same order applies to the substituted (non-embedded) Tinos.
    assert_eq!(joined(&extract(tinos_page(false, "002400250026"))), "ABC");
    // But never to an embedded program, whose own glyph order is
    // authoritative: the subset has no glyphs 36-38 in its `cmap`, so the
    // CIDs are unmapped and emitted raw, not read as Mac order.
    assert_eq!(joined(&extract(tinos_page(true, "002400250026"))), "$%&");
    // A ToUnicode map is authoritative even where it is silent: CID 0x24 is
    // unmapped, so the raw CID (`$`) is emitted.
    arial.to_unicode = Some(to_unicode_cmap(2, &[(0x50, "P")]));
    let p = extract(with_font(
        FontDict::Type0(arial),
        "BT /F 11 Tf 72 700 Td <00240050> Tj ET",
    ));
    assert_eq!(texts(&p), ["$", "P"]);
}

/// The KP6 Type 3 fixture (spec A.4), codes in the order of §4.9. Code 66
/// is named `whitesquare`, the AGL name for U+25A1: the probe's `/square`
/// is a TeX name, unmapped here since D-39 (see
/// `tex_names_are_unmapped_residual`).
fn kp6() -> Vec<u8> {
    let font = type3(
        &[
            (65, "g65"),
            (66, "whitesquare"),
            (0, "g0"),
            (1, "g1"),
            (9, "g9"),
            (10, "g10"),
            (32, "g32"),
            (200, "g200"),
        ],
        None,
    );
    with_font(font, "BT /F 20 Tf 72 700 Td <41420001090A20C8> Tj ET")
}

#[test]
fn step5_unmapped_policies_on_kp6() {
    let run = |policy| extract_with(kp6(), Limits::default(), policy).expect("extracts");
    let cid = run(Unmapped::Cid);
    assert_eq!(
        texts(&cid),
        [
            "A", "\u{25A1}", "\u{FFFD}", "\u{FFFD}", "\t", "\n", " ", "\u{C8}"
        ]
    );
    let skip = run(Unmapped::Skip);
    assert_eq!(texts(&skip), ["\u{25A1}"]);
    // Skipping a glyph does not move the others.
    assert_eq!(skip.chars[0].origin, cid.chars[1].origin);
    let replacement = run(Unmapped::Replacement);
    assert_eq!(
        joined(&replacement),
        "\u{FFFD}\u{25A1}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}"
    );
    for (r, c) in replacement.chars.iter().zip(&cid.chars) {
        assert_eq!(r.origin, c.origin);
    }
}

#[test]
fn kp15_never_applies_to_type3_fonts() {
    // A Type 3 encoding is all /Differences, which the reference never
    // normalises (review finding; re-probed rule).
    let font = type3(
        &[(0xA0, "nbspace"), (0xAD, "sfthyphen"), (0xCA, "nbspace")],
        None,
    );
    let p = extract(with_font(font, "BT /F 20 Tf 72 700 Td <A0ADCA> Tj ET"));
    assert_eq!(texts(&p), ["\u{A0}", "\u{AD}", "\u{A0}"]);
}

#[test]
fn tex_names_are_unmapped_residual() {
    // D-39 / §2.5: TeX names are not in the AGL. The reference maps
    // `/square` to U+25A1 and leaves `/parenleftbig` raw; we leave both raw,
    // including on a Type 1 font where hayro's own table would map them.
    let p = extract(with_font(
        type3(&[(66, "square")], None),
        "BT /F 20 Tf 72 700 Td <42> Tj ET",
    ));
    assert_eq!(texts(&p), ["B"]);
    let font = helvetica(
        SimpleEncoding::Differences {
            base: None,
            differences: vec![(65, "parenleftbig".into()), (66, "dalethatafpatah".into())],
        },
        None,
    );
    let p = extract(with_font(font, "BT /F 11 Tf 72 700 Td <4142> Tj ET"));
    assert_eq!(texts(&p), ["A", "B"]);
}

#[test]
fn kp15_differences_nbspace_divergence() {
    // KP15 divergence fixture `kp15_differences_nbspace`: a /Differences
    // entry restating 0xA0 as `nbspace` under WinAnsi. The reference keeps
    // U+00A0; hayro cannot say the name came from /Differences, so we read a
    // space (§2.5).
    let font = helvetica(
        SimpleEncoding::Differences {
            base: Some(BaseEncoding::WinAnsi),
            differences: vec![(0xA0, "nbspace".into())],
        },
        None,
    );
    let p = extract(with_font(font, "BT /F 11 Tf 72 700 Td <61A062> Tj ET"));
    assert_eq!(joined(&p), "a b");
}

fn arial(subtype: pdf_extract_testkit::CidFontSubtype, descriptor: bool) -> FontDict {
    let mut f = Type0Font::identity_h("Arial");
    f.cid_subtype = subtype;
    if descriptor {
        f.descriptor = Some(FontDescriptor::default());
    }
    FontDict::Type0(f)
}

#[test]
fn kp22_mac_order_for_both_subtypes_with_or_without_descriptor() {
    use pdf_extract_testkit::CidFontSubtype::{Type0, Type2};
    let show = "BT /F 11 Tf 72 700 Td <00240025002600030044> Tj ET";
    // `kp22_cidfonttype0_macorder` (exact against the reference).
    assert_eq!(
        joined(&extract(with_font(arial(Type0, true), show))),
        "ABC a"
    );
    // `kp22_cid_no_descriptor` (divergence: the reference emits raw bytes).
    assert_eq!(
        joined(&extract(with_font(arial(Type2, false), show))),
        "ABC a"
    );
}

#[test]
fn kp22_cid_ge_258_raw() {
    // `kp22_cid_ge_258_raw`: CIDs past the 258 Mac names are unmapped and
    // emitted raw (the reference gives U+0103..U+0106 and U+012C).
    let p = extract(with_font(
        arial(pdf_extract_testkit::CidFontSubtype::Type2, true),
        "BT /F 11 Tf 72 700 Td <01010102010301040106012C> Tj ET",
    ));
    // CID 257 is the last Mac name (`dcroat`).
    assert_eq!(joined(&p), "\u{111}\u{102}\u{103}\u{104}\u{106}\u{12C}");
}

#[test]
fn sanitising_covers_every_source() {
    // §4.9 (6), revision 10b: sanitising reaches ToUnicode only through
    // entries of several code points (`<0041 0000>` → `A` + U+FFFD; LF in
    // `<0041 000A 0042>` is kept). Single control code points follow T26
    // step 1 first: NUL is ignored (the name `g65` is unmapped: raw `A`),
    // LF reads as a space. A `uni0000` name and (in the ActualText tests)
    // `<FEFF0001>` give U+FFFD.
    let cmap = to_unicode_cmap(
        1,
        &[
            (65, "\0"),
            (66, "\n"),
            (67, "x\u{1}y"),
            (68, "A\0"),
            (69, "A\nB"),
        ],
    );
    let font = type3(
        &[
            (65, "g65"),
            (66, "g66"),
            (67, "g67"),
            (68, "g68"),
            (69, "g69"),
        ],
        Some(cmap),
    );
    let p = extract(with_font(font, "BT /F 20 Tf 72 700 Td <4142434445> Tj ET"));
    assert_eq!(texts(&p), ["A", " ", "x\u{FFFD}y", "A\u{FFFD}", "A\nB"]);
    let font = type3(
        &[(1, "uni0000"), (2, "u0001"), (3, "uniFFFE"), (4, "uni0085")],
        None,
    );
    let p = extract(with_font(font, "BT /F 20 Tf 72 700 Td <01020304> Tj ET"));
    assert_eq!(texts(&p), ["\u{FFFD}", "\u{FFFD}", "\u{FFFD}", "\u{85}"]);
}

fn type1_tounicode_nul() -> Vec<u8> {
    let cmap = to_unicode_cmap(1, &[(65, "\0")]);
    let font = helvetica(SimpleEncoding::Named(BaseEncoding::WinAnsi), Some(cmap));
    with_font(font, "BT /F 11 Tf 72 700 Td <41> Tj ET")
}

#[test]
fn type1_tounicode_nul_is_ignored() {
    // T26 step 1 (revision 10b): a single-code-point entry for U+0000 is
    // ignored, so the glyph name (`A`) decides.
    assert_eq!(texts(&extract(type1_tounicode_nul())), ["A"]);
}

/// The single-code-point targets of the revision 10b probe table and what
/// the glyph between `A` and `B` (code `x`, CID 91 for the Type 0 font)
/// must read as.
const PROBE_TABLE: [(char, &str); 13] = [
    ('\0', "x"),
    ('\u{1}', "x"),
    ('\u{8}', " "),
    ('\t', " "),
    ('\n', " "),
    ('\u{B}', " "),
    ('\r', " "),
    ('\u{1F}', "x"),
    ('\u{7F}', "x"),
    ('\u{80}', "x"),
    ('\u{85}', "x"),
    ('\u{A0}', "\u{A0}"),
    ('\u{2028}', "\u{2028}"),
];

#[test]
fn tounicode_control_targets_in_a_type1_font() {
    for (target, want) in PROBE_TABLE {
        let cmap = to_unicode_cmap(1, &[(0x78, &target.to_string())]);
        let font = helvetica(SimpleEncoding::Named(BaseEncoding::WinAnsi), Some(cmap));
        let p = extract(with_font(font, "BT /F 11 Tf 72 700 Td <417842> Tj ET"));
        assert_eq!(
            joined(&p),
            format!("A{want}B"),
            "U+{:04X}",
            u32::from(target)
        );
    }
}

#[test]
fn tounicode_control_targets_in_a_non_embedded_truetype_font() {
    for (target, want) in PROBE_TABLE {
        let cmap = to_unicode_cmap(1, &[(0x78, &target.to_string())]);
        let content = "BT /F 11 Tf 72 700 Td <417842> Tj ET";
        let pdf = assemble_pdf(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /F 5 0 R >> >> /Contents 4 0 R >>"
                .to_owned(),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len() + 1
            ),
            "<< /Type /Font /Subtype /TrueType /BaseFont /Arial \
             /Encoding /WinAnsiEncoding /ToUnicode 6 0 R >>"
                .to_owned(),
            format!(
                "<< /Length {} >>\nstream\n{cmap}\nendstream",
                cmap.len() + 1
            ),
        ]);
        assert_eq!(
            joined(&extract(pdf)),
            format!("A{want}B"),
            "U+{:04X}",
            u32::from(target)
        );
    }
}

#[test]
fn tounicode_control_targets_in_a_type0_font() {
    // Identity-H Arial with a descriptor: CID 91 is `x` in the standard
    // Macintosh order, but step 4b is only for fonts without a ToUnicode
    // map, so an ignored entry leaves the glyph unmapped and the raw CID
    // (U+005B) is emitted, as the reference does (`tounicode_ctl_t0_*`).
    for (target, want) in PROBE_TABLE {
        let want = if want == "x" { "[" } else { want };
        let mut f = Type0Font::identity_h("Arial");
        f.descriptor = Some(FontDescriptor::default());
        f.to_unicode = Some(to_unicode_cmap(
            2,
            &[(36, "A"), (37, "B"), (91, &target.to_string())],
        ));
        let p = extract(with_font(
            FontDict::Type0(f),
            "BT /F 11 Tf 72 700 Td <0024005B0025> Tj ET",
        ));
        assert_eq!(
            joined(&p),
            format!("A{want}B"),
            "U+{:04X}",
            u32::from(target)
        );
    }
}
