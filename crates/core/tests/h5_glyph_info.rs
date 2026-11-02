//! Tests of fork change H5: per-glyph font facts on `Glyph` (spec §3.2,
//! oracle §4.7).
//!
//! The text device (unit 09) needs more than `as_unicode()`: the raw code and
//! CID for the unmapped-glyph policy (KP6) and the standard-Macintosh-order
//! fallback (KP16), the glyph name for the nbspace/sfthyphen rule (KP15), the
//! advance and ascent/descent for the glyph box (T23), and whether the font is
//! vertical or embedded. Upstream keeps all of this private. The fixtures
//! mirror the KP6 and KP16 cases of spec Appendix A.4, built with the test
//! kit's `FontDict` builders (or by hand where a dictionary entry the builders
//! do not write is the point).
//!
//! Embedded simple fonts (Type 1, CFF, TrueType; internal issue #27) are the tail of
//! this file: they take different branches of patch 0007 than the standard and
//! substitute fonts above, and need font programs, which the test kit cannot
//! write (see `assets/PROVENANCE.md`).

mod common;

use common::{assemble_pdf, push_unicode, raw_pdf, run_device, stream};
use hayro_interpret::font::{FontKind, Glyph};
use hayro_interpret::hayro_cmap::{CharacterCollection, CidFamily};
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use kurbo::{Affine, BezPath};
use pdf_extract_testkit::{
    BaseEncoding, CidFontSubtype, FontDescriptor, FontDict, PdfBuilder, SimpleEncoding, Type0Font,
    Type3Font, Type3Glyph, helvetica_ascent_descent, to_unicode_cmap,
};

/// Everything H5 exposes about one drawn glyph.
#[derive(Clone, Debug, PartialEq)]
struct Facts {
    code: u32,
    cid: Option<u32>,
    kind: FontKind,
    name: Option<String>,
    embedded: bool,
    to_unicode: bool,
    collection: Option<CharacterCollection>,
    advance: (f64, f64),
    vertical: bool,
    ascent_descent: Option<(f32, f32)>,
}

#[derive(Default)]
struct Recorder(Vec<Facts>);

impl<'a> Device<'a> for Recorder {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(
        &mut self,
        g: &Glyph<'a>,
        _: Affine,
        _: Affine,
        _: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
        // The per-variant methods must agree with the `Glyph` ones.
        match g {
            Glyph::Outline(o) => {
                assert_eq!(o.char_code(), g.char_code());
                assert_eq!(o.font_kind(), g.font_kind());
            }
            Glyph::Type3(t) => {
                assert_eq!(t.char_code(), g.char_code());
                assert_eq!(t.font_kind(), g.font_kind());
            }
        }
        let advance = g.advance();
        self.0.push(Facts {
            code: g.char_code(),
            cid: g.cid(),
            kind: g.font_kind(),
            name: g.glyph_name().map(str::to_owned),
            embedded: g.is_embedded(),
            to_unicode: g.has_to_unicode(),
            collection: g.character_collection().cloned(),
            advance: (advance.x, advance.y),
            vertical: g.is_vertical(),
            ascent_descent: g.ascent_descent(),
        });
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// Records one glyph per `draw_glyph` (the fixtures use fill mode only).
fn facts(pdf: &[u8]) -> Vec<Facts> {
    run_device(pdf, Recorder::default()).0
}

/// Approximate comparison for metrics computed in floating point.
fn assert_metrics(got: Option<(f32, f32)>, want: Option<(f32, f32)>) {
    match (got, want) {
        (Some(g), Some(w)) => assert!(
            (g.0 - w.0).abs() < 1e-5 && (g.1 - w.1).abs() < 1e-5,
            "ascent/descent {g:?}, want {w:?}"
        ),
        _ => assert_eq!(got, want),
    }
}

/// The non-metric part of a `Facts`, for table rows.
fn simple(code: u32, kind: FontKind, name: Option<&str>, embedded: bool, advance: f64) -> Facts {
    Facts {
        code,
        cid: None,
        kind,
        name: name.map(str::to_owned),
        embedded,
        to_unicode: false,
        collection: None,
        advance: (advance, 0.0),
        vertical: false,
        ascent_descent: None,
    }
}

/// Compares everything but the metrics exactly, the metrics approximately.
fn assert_rows(got: &[Facts], want: &[(Facts, Option<(f32, f32)>)]) {
    assert_eq!(got.len(), want.len(), "glyph count: {got:#?}");
    for (g, (w, metrics)) in got.iter().zip(want) {
        assert_metrics(g.ascent_descent, *metrics);
        let g = Facts {
            ascent_descent: None,
            ..g.clone()
        };
        assert_eq!(&g, w);
    }
}

fn type3_glyph(code: u8, name: &str, width: f32) -> Type3Glyph {
    Type3Glyph {
        code,
        name: name.into(),
        width,
        procedure: format!("{width} 0 d0 0 0 100 100 re f"),
    }
}

#[test]
fn type3_kp6_codes() {
    // Spec A.4: Type 3 without ToUnicode, codes 65 (/g65), 66 (/square), 0,
    // 1, 9, 10, 32, 200; listed in the order the content shows them.
    let codes = [
        (65, "g65"),
        (66, "square"),
        (0, "g0"),
        (1, "g1"),
        (9, "g9"),
        (32, "space"),
        (10, "g10"),
        (200, "g200"),
    ];
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font(
            "T3",
            FontDict::Type3(Type3Font {
                font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
                font_bbox: [0.0, 0.0, 600.0, 600.0],
                glyphs: codes
                    .iter()
                    .map(|&(c, n)| type3_glyph(c, n, 600.0))
                    .collect(),
                to_unicode: None,
            }),
        )
        .raw_content("BT /T3 20 Tf 72 700 Td <4142000109200AC8> Tj ET");
    let want: Vec<_> = codes
        .iter()
        .map(|&(c, n)| {
            (
                simple(u32::from(c), FontKind::Type3, Some(n), true, 600.0),
                Some((0.6, 0.0)),
            )
        })
        .collect();
    assert_rows(&facts(&b.to_bytes()), &want);
}

#[test]
fn type3_advance_and_metrics_follow_the_font_matrix() {
    // Widths are in glyph space: 300 units at 0.002 per unit are 0.6 em, i.e.
    // an advance of 600 in thousandths. The bbox maps to (0.8, -0.2) em. A
    // ToUnicode map is reported; an all-zero bbox gives no metrics.
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font(
            "T3",
            FontDict::Type3(Type3Font {
                font_matrix: [0.002, 0.0, 0.0, 0.002, 0.0, 0.0],
                font_bbox: [0.0, -100.0, 400.0, 400.0],
                glyphs: vec![type3_glyph(65, "A", 300.0)],
                to_unicode: Some(to_unicode_cmap(1, &[(65, "A")])),
            }),
        )
        .add_font(
            "T0",
            FontDict::Type3(Type3Font {
                font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
                font_bbox: [0.0, 0.0, 0.0, 0.0],
                glyphs: vec![type3_glyph(66, "B", 500.0)],
                to_unicode: None,
            }),
        )
        .raw_content("BT /T3 10 Tf 72 700 Td (A) Tj /T0 10 Tf (B) Tj ET");
    let mut with_cmap = simple(65, FontKind::Type3, Some("A"), true, 600.0);
    with_cmap.to_unicode = true;
    assert_rows(
        &facts(&b.to_bytes()),
        &[
            (with_cmap, Some((0.8, -0.2))),
            (simple(66, FontKind::Type3, Some("B"), true, 500.0), None),
        ],
    );
}

#[test]
fn type1_standard_fonts() {
    // Spec A.4: Helvetica with /Differences [65 /zzunknown 66 /uni2603
    // 128 /g128]. None of these names has a Helvetica width, so the advance
    // is 0 (what hayro advances the pen by). WinAnsi `A` takes the AFM width
    // 667. Metrics are Helvetica's AFM values; Symbol's AFM has none.
    let helv = helvetica_ascent_descent();
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font(
            "FDiff",
            FontDict::standard14(
                "Helvetica",
                SimpleEncoding::Differences {
                    base: None,
                    differences: vec![
                        (65, "zzunknown".into()),
                        (66, "uni2603".into()),
                        (128, "g128".into()),
                    ],
                },
            ),
        )
        .add_font(
            "FMac",
            FontDict::standard14("Helvetica", SimpleEncoding::Named(BaseEncoding::MacRoman)),
        )
        .add_font(
            "FStd",
            FontDict::standard14("Helvetica", SimpleEncoding::Builtin),
        )
        .add_font(
            "FSym",
            FontDict::standard14("Symbol", SimpleEncoding::Builtin),
        )
        .raw_content(
            "BT /FDiff 11 Tf 72 700 Td <414280> Tj /Helv 11 Tf (A) Tj \
             /FMac 11 Tf <CA> Tj /FStd 11 Tf <A0> Tj /FSym 11 Tf (a) Tj ET",
        );
    let t1 = |code, name, advance| simple(code, FontKind::Type1, name, false, advance);
    assert_rows(
        &facts(&b.to_bytes()),
        &[
            (t1(65, Some("zzunknown"), 0.0), Some(helv)),
            (t1(66, Some("uni2603"), 0.0), Some(helv)),
            (t1(128, Some("g128"), 0.0), Some(helv)),
            (t1(65, Some("A"), 667.0), Some(helv)),
            // MacRoman 0xCA is /nbspace, which the text device normalises
            // to a space (KP15); StandardEncoding has no name for 0xA0.
            (t1(0xCA, Some("nbspace"), 278.0), Some(helv)),
            (t1(0xA0, None, 0.0), Some(helv)),
            (t1(97, Some("alpha"), 631.0), None),
        ],
    );
}

#[test]
fn type1_widths_tounicode_and_descriptor_metrics() {
    // /Widths wins over the AFM for a standard font, /ToUnicode is reported,
    // and descriptor metrics win over the AFM. A descriptor with zero metrics
    // is ignored in favour of the AFM.
    let cmap = to_unicode_cmap(1, &[(65, "A")]);
    let pdf = raw_pdf(&[
        "<< /Font << /W 6 0 R /D 8 0 R /Z 10 0 R >> >>".to_owned(),
        stream(
            "",
            "BT /W 11 Tf 72 700 Td (AB) Tj /D 11 Tf (A) Tj /Z 11 Tf (A) Tj ET",
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /LastChar 66 \
         /Widths [500 250] /ToUnicode 7 0 R >>"
            .to_owned(),
        stream("", &cmap),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FontDescriptor 9 0 R >>".to_owned(),
        "<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 /Ascent 800 /Descent -100 >>"
            .to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FontDescriptor 11 0 R >>".to_owned(),
        "<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 /Ascent 0 /Descent 0 >>"
            .to_owned(),
    ]);
    let helv = helvetica_ascent_descent();
    let mut a = simple(65, FontKind::Type1, Some("A"), false, 500.0);
    a.to_unicode = true;
    let mut b = simple(66, FontKind::Type1, Some("B"), false, 250.0);
    b.to_unicode = true;
    assert_rows(
        &facts(&pdf),
        &[
            (a, Some(helv)),
            (b, Some(helv)),
            (
                simple(65, FontKind::Type1, Some("A"), false, 667.0),
                Some((0.8, -0.1)),
            ),
            (
                simple(65, FontKind::Type1, Some("A"), false, 667.0),
                Some(helv),
            ),
        ],
    );
}

#[test]
fn text_without_a_font_uses_the_helvetica_substitute() {
    // No Tf at all: hayro draws ASCII through its Helvetica fallback (T37).
    let pdf = raw_pdf(&["<< >>".to_owned(), stream("", "BT 72 700 Td (A) Tj ET")]);
    assert_rows(
        &facts(&pdf),
        &[(
            simple(65, FontKind::Type1, Some("A"), false, 667.0),
            Some(helvetica_ascent_descent()),
        )],
    );
}

#[test]
fn truetype_substitute() {
    // A non-embedded TrueType font is drawn with a standard substitute; it is
    // not embedded and takes the substitute's AFM metrics.
    let pdf = raw_pdf(&[
        "<< /Font << /TT 6 0 R >> >>".to_owned(),
        stream("", "BT /TT 11 Tf 72 700 Td (a) Tj ET"),
        "<< /Type /Font /Subtype /TrueType /BaseFont /Arial /Encoding /WinAnsiEncoding >>"
            .to_owned(),
    ]);
    assert_rows(
        &facts(&pdf),
        &[(
            simple(97, FontKind::TrueType, Some("a"), false, 556.0),
            Some(helvetica_ascent_descent()),
        )],
    );
}

fn identity() -> CharacterCollection {
    CharacterCollection {
        family: CidFamily::AdobeIdentity,
        supplement: 0,
    }
}

fn cid_row(code: u32, cid: u32, advance: (f64, f64), vertical: bool) -> Facts {
    Facts {
        code,
        cid: Some(cid),
        kind: FontKind::Type0,
        name: None,
        embedded: false,
        to_unicode: false,
        collection: Some(identity()),
        advance,
        vertical,
        ascent_descent: None,
    }
}

#[test]
fn type0_identity_kp16() {
    // Spec A.4 (KP16): non-embedded Identity-H `Arial`, no ToUnicode, CIDs
    // 0x24 0x25 0x26 0x03 0x44, plus 0x45 which /W does not list (DW). The
    // descriptor is the test kit's default (Ascent 900, Descent -200); the
    // same font without a descriptor has no metrics.
    let mut arial = Type0Font::identity_h("Arial");
    arial.descriptor = Some(FontDescriptor::default());
    arial.default_width = Some(1000.0);
    arial.widths = Some("3 [278] 36 [667 667 722] 68 [556]".into());
    let mut bare = Type0Font::identity_h("Arial");
    bare.widths = arial.widths.clone();
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font("CIdH", FontDict::Type0(arial))
        .add_font("Bare", FontDict::Type0(bare))
        .raw_content(
            "BT /CIdH 11 Tf 72 700 Td <002400250026000300440045> Tj /Bare 11 Tf <0024> Tj ET",
        );
    let m = Some((0.9, -0.2));
    assert_rows(
        &facts(&b.to_bytes()),
        &[
            (cid_row(0x24, 36, (667.0, 0.0), false), m),
            (cid_row(0x25, 37, (667.0, 0.0), false), m),
            (cid_row(0x26, 38, (722.0, 0.0), false), m),
            (cid_row(0x03, 3, (278.0, 0.0), false), m),
            (cid_row(0x44, 68, (556.0, 0.0), false), m),
            (cid_row(0x45, 69, (1000.0, 0.0), false), m),
            (cid_row(0x24, 36, (667.0, 0.0), false), None),
        ],
    );
}

#[test]
fn type0_vertical_and_predefined_cmap() {
    // Identity-V: vertical, advance from the default DW2 (-1000). The Japan1
    // ordering comes from /CIDSystemInfo (Identity-V names no collection).
    // 90ms-RKSJ-H reads one- and two-byte codes and maps them to Japan1 CIDs.
    let mut japan1 = Type0Font::identity_h("MS-Mincho");
    japan1.encoding = "90ms-RKSJ-H".into();
    japan1.ordering = "Japan1".into();
    japan1.supplement = 2;
    japan1.cid_subtype = CidFontSubtype::Type0;
    japan1.descriptor = Some(FontDescriptor::default());
    let mut vertical = japan1.clone();
    vertical.encoding = "Identity-V".into();
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font("CJp", FontDict::Type0(japan1))
        .add_font("CJv", FontDict::Type0(vertical))
        .raw_content("BT /CJv 11 Tf 300 600 Td <0022> Tj /CJp 11 Tf 72 680 Td <93FA41> Tj ET");
    let got = facts(&b.to_bytes());
    assert_eq!(got.len(), 3, "{got:#?}");

    let v = &got[0];
    assert_eq!((v.code, v.cid, v.vertical), (0x22, Some(0x22), true));
    assert_eq!(v.advance, (0.0, -1000.0));
    assert_eq!(
        v.collection.as_ref().map(|c| &c.family),
        Some(&CidFamily::AdobeJapan1)
    );

    // 0x93FA is a two-byte code for U+65E5; `A` (0x41) is one byte and maps
    // into 90ms-RKSJ-H's proportional Roman range (<20>..<7E> from CID 231).
    let (kanji, a) = (&got[1], &got[2]);
    assert_eq!(
        (kanji.code, a.code, a.cid),
        (0x93FA, 0x41, Some(231 + 0x41 - 0x20))
    );
    assert!(kanji.cid.is_some_and(|c| c > 255), "{kanji:?}");
    assert!(!kanji.vertical && !kanji.embedded && !kanji.to_unicode);
    assert_eq!(
        kanji.collection.as_ref().map(|c| &c.family),
        Some(&CidFamily::AdobeJapan1)
    );
    assert_eq!(kanji.advance, (1000.0, 0.0));
}

#[test]
fn type0_tounicode_is_reported() {
    let mut f = Type0Font::identity_h("Arial");
    f.to_unicode = Some(to_unicode_cmap(2, &[(0x24, "A")]));
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font("C", FontDict::Type0(f))
        .raw_content("BT /C 11 Tf 72 700 Td <0024> Tj ET");
    let got = facts(&b.to_bytes());
    assert_eq!(got.len(), 1);
    assert!(got[0].to_unicode);
}

/// The Unicode the fixtures above produce is unchanged by H5 (the facts are
/// read-only views): the KP6 Type 3 font has no ToUnicode, so no glyph maps.
#[test]
fn type3_without_tounicode_has_no_unicode() {
    struct Text(String);
    impl<'a> Device<'a> for Text {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
        fn draw_glyph(
            &mut self,
            g: &Glyph<'a>,
            _: Affine,
            _: Affine,
            _: &Paint<'a>,
            _: &GlyphDrawMode,
        ) {
            push_unicode(&mut self.0, g);
        }
        fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font(
            "T3",
            FontDict::Type3(Type3Font {
                font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
                font_bbox: [0.0, 0.0, 600.0, 600.0],
                glyphs: vec![type3_glyph(65, "g65", 600.0)],
                to_unicode: None,
            }),
        )
        .raw_content("BT /T3 20 Tf 72 700 Td (A) Tj ET");
    assert_eq!(run_device(&b.to_bytes(), Text(String::new())).0, "\u{FFFD}");
}

// ---------------------------------------------------------------------------
// Embedded simple fonts (internal issue #27).
//
// Patch 0007 reports AFM ascent/descent only for a font that is drawn with a
// standard font, never for an embedded program ("an embedded program's metrics
// need not match the standard font it imitates"), and TrueType keeps its own
// `ToUnicode` and glyph-name paths for embedded programs. Every fixture here
// is named `/Helvetica` and omits `/Ascent`/`/Descent` on purpose: that is the
// one case where a wrongly applied AFM fallback would produce a value (the
// Helvetica one) instead of `None`.

const TINOS: &[u8] = include_bytes!("assets/tinos_subset.ttf");
const FOXIT_SANS_CFF: &[u8] = include_bytes!("assets/foxit_sans.cff");

/// The embedded program kinds a simple font can carry.
#[derive(Clone, Copy, Debug)]
enum Program {
    /// `/FontFile` (Type 1), built by [`type1_program`].
    Type1,
    /// `/FontFile3` `/Type1C` (bare CFF): Foxit Sans.
    Cff,
    /// `/FontFile2` (TrueType): the Tinos subset.
    TrueType,
}

impl Program {
    fn kind(self) -> FontKind {
        match self {
            Self::Type1 | Self::Cff => FontKind::Type1,
            Self::TrueType => FontKind::TrueType,
        }
    }
}

/// Eexec/charstring cipher (Adobe Type 1 Font Format §7): the stream is
/// encrypted byte by byte with a 16-bit running key.
fn type1_encrypt(plain: &[u8], mut key: u16) -> Vec<u8> {
    plain
        .iter()
        .map(|&p| {
            let c = p ^ (key >> 8) as u8;
            key = (u16::from(c).wrapping_add(key))
                .wrapping_mul(52845)
                .wrapping_add(22719);
            c
        })
        .collect()
}

/// A minimal but complete Type 1 program (PFA with a binary eexec section)
/// with glyphs `.notdef`, `A` and `B` under `StandardEncoding`, written here
/// rather than embedded as a file: no third-party Type 1 font with a
/// permissive license is already in the tree, and the parser only needs the
/// structure. Charstrings are stored unencrypted (`/lenIV -1`) to keep the
/// bytes legible; the eexec layer, which the parser always decrypts, is real.
/// Returns the program and its `/Length1` (clear text) and `/Length2`
/// (encrypted part).
fn type1_program() -> (Vec<u8>, usize, usize) {
    let clear = b"%!PS-AdobeFont-1.0: TestType1 1.0\n\
        11 dict begin\n\
        /FontName /TestType1 def\n\
        /FontType 1 def\n\
        /PaintType 0 def\n\
        /FontMatrix [0.001 0 0 0.001 0 0] readonly def\n\
        /FontBBox {0 0 600 100} readonly def\n\
        /Encoding StandardEncoding def\n\
        currentdict end\n\
        currentfile eexec\n";
    // `0 500 hsbw endchar`.
    let notdef = [139u8, 248, 136, 13, 14];
    // `0 600 hsbw 0 0 rmoveto 100 0 rlineto 0 100 rlineto closepath endchar`.
    let square = [
        139u8, 248, 236, 13, 139, 139, 21, 239, 139, 5, 139, 239, 5, 9, 14,
    ];
    let mut private = b"abcd".to_vec(); // the four random lead bytes eexec skips
    private.extend_from_slice(
        b"dup /Private 5 dict dup begin\n\
          /RD {string currentfile exch readstring pop} executeonly def\n\
          /ND {noaccess def} executeonly def\n\
          /NP {noaccess put} executeonly def\n\
          /lenIV -1 def\n\
          2 index /CharStrings 3 dict dup begin\n",
    );
    for (name, code) in [(".notdef", &notdef[..]), ("A", &square), ("B", &square)] {
        private.extend_from_slice(format!("/{name} {} RD ", code.len()).as_bytes());
        private.extend_from_slice(code);
        private.extend_from_slice(b" ND\n");
    }
    private.extend_from_slice(
        b"end end readonly put noaccess put\n\
          dup /FontName get exch definefont pop\n\
          mark currentfile closefile\n",
    );
    let encrypted = type1_encrypt(&private, 55665);
    let (len1, len2) = (clear.len(), encrypted.len());
    let mut program = clear.to_vec();
    program.extend_from_slice(&encrypted);
    program.extend_from_slice(&[b'0'; 64]);
    program.extend_from_slice(b"\ncleartomark\n");
    (program, len1, len2)
}

fn stream_obj(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A one-page PDF drawing `(AB)` in a simple font named `/Helvetica` with
/// `program` embedded. `descriptor_metrics` is the descriptor's extra
/// `/Ascent`/`/Descent` text (empty for none); `to_unicode` is a ToUnicode
/// CMap body. Objects: 1-3 catalog, pages, page; 4 content; 5 font;
/// 6 descriptor; 7 font file; 8 ToUnicode.
fn embedded_pdf(program: Program, descriptor_metrics: &str, to_unicode: Option<&str>) -> Vec<u8> {
    let (subtype, file_key, data, lengths) = match program {
        Program::Type1 => {
            let (data, len1, len2) = type1_program();
            let lengths = format!("/Length1 {len1} /Length2 {len2} /Length3 77");
            ("Type1", "FontFile", data, lengths)
        }
        Program::Cff => (
            "Type1",
            "FontFile3",
            FOXIT_SANS_CFF.to_vec(),
            "/Subtype /Type1C".to_owned(),
        ),
        Program::TrueType => (
            "TrueType",
            "FontFile2",
            TINOS.to_vec(),
            format!("/Length1 {}", TINOS.len()),
        ),
    };
    let to_unicode_ref = if to_unicode.is_some() {
        " /ToUnicode 8 0 R"
    } else {
        ""
    };
    assemble_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] \
          /Resources << /Font << /F 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        stream_obj("", b"BT /F 11 Tf 72 100 Td (AB) Tj ET"),
        format!(
            "<< /Type /Font /Subtype /{subtype} /BaseFont /Helvetica /FirstChar 65 \
             /LastChar 66 /Widths [600 650] /FontDescriptor 6 0 R{to_unicode_ref} >>"
        )
        .into_bytes(),
        format!(
            "<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 \
             /FontBBox [0 0 1000 1000] /ItalicAngle 0 /CapHeight 700 /StemV 80 \
             {descriptor_metrics} /{file_key} 7 0 R >>"
        )
        .into_bytes(),
        stream_obj(&lengths, &data),
        stream_obj("", to_unicode.unwrap_or_default().as_bytes()),
    ])
}

const ALL_PROGRAMS: [Program; 3] = [Program::Type1, Program::Cff, Program::TrueType];

#[test]
fn embedded_simple_fonts_report_their_program_and_names() {
    // The fixtures must really load their programs: if one fell back to a
    // substitute, `embedded` would be false and every metrics assertion below
    // would be vacuous.
    for program in ALL_PROGRAMS {
        let got = facts(&embedded_pdf(program, "", None));
        assert_eq!(got.len(), 2, "{program:?}: {got:#?}");
        for (g, (code, name, width)) in got.iter().zip([(65, "A", 600.0), (66, "B", 650.0)]) {
            assert!(g.embedded, "{program:?} must load: {g:?}");
            assert_eq!(g.kind, program.kind(), "{program:?}");
            assert_eq!(g.code, code);
            // The embedded TrueType path reaches the name through its own
            // `code_to_name` (no /Encoding, so the standard-encoding fallback);
            // Type 1 and CFF through `code_to_ps_name`.
            assert_eq!(g.name.as_deref(), Some(name), "{program:?} glyph name");
            assert_eq!(g.advance, (width, 0.0), "{program:?}");
            assert!(!g.vertical);
        }
    }
}

#[test]
fn embedded_simple_fonts_do_not_borrow_afm_metrics() {
    // Helvetica's AFM ascent/descent must not leak onto an embedded program
    // whose descriptor gives none: the embedded font's own extent is unknown,
    // so the answer is `None`.
    assert!(helvetica_ascent_descent().0 > 0.0, "the AFM value is real");
    for program in ALL_PROGRAMS {
        for g in facts(&embedded_pdf(program, "", None)) {
            assert!(g.embedded, "{program:?} must load");
            assert_eq!(g.ascent_descent, None, "{program:?}");
        }
    }
}

#[test]
fn embedded_simple_fonts_use_descriptor_metrics() {
    // The positive control for the test above: with `/Ascent`/`/Descent` the
    // descriptor's values (in thousandths) are reported, not the AFM's.
    for program in ALL_PROGRAMS {
        for g in facts(&embedded_pdf(program, "/Ascent 800 /Descent -200", None)) {
            assert!(g.embedded, "{program:?} must load");
            assert_metrics(g.ascent_descent, Some((0.8, -0.2)));
        }
    }
}

#[test]
fn embedded_simple_fonts_report_to_unicode_presence() {
    // `has_to_unicode` follows the font's own ToUnicode map and nothing else,
    // for every embedded program kind (TrueType has a separate field for it).
    let cmap = to_unicode_cmap(1, &[(65, "Z"), (66, "Y")]);
    for program in ALL_PROGRAMS {
        let with = facts(&embedded_pdf(program, "", Some(&cmap)));
        let without = facts(&embedded_pdf(program, "", None));
        assert_eq!(with.len(), 2);
        assert!(
            with.iter().all(|g| g.embedded && g.to_unicode),
            "{program:?} with ToUnicode: {with:#?}"
        );
        assert!(
            without.iter().all(|g| g.embedded && !g.to_unicode),
            "{program:?} without ToUnicode: {without:#?}"
        );
    }
}
