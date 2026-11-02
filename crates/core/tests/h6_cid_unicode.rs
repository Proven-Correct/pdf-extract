//! Tests of fork change H6: Unicode fallbacks for CID fonts without a
//! `ToUnicode` map (spec §3.2 H6, T26 steps 3 and 4, oracle §4.7).
//!
//! - H6a: a CID font of a known character collection (Japan1 here) maps CIDs
//!   through the collection's UCS2 CMap. Upstream did so only when the font
//!   program was *not* embedded; the oracle does it either way (spec A.4:
//!   Identity-H with Japan1 ordering, CIDs 34-36 → `ABC`).
//! - H6b: an embedded TrueType program with `Identity` ordering maps the glyph
//!   the code selects back through the program's own `cmap`.
//!
//! The embedded program is `assets/tinos_subset.ttf` (see
//! `assets/PROVENANCE.md`): glyphs 1-6 are space, quotedbl, A, B, a, b, and
//! its `cmap` maps both U+0020 and U+00A0 to glyph 1.

mod common;

use std::fmt::Write as _;

use common::{assemble_pdf, push_unicode, run_device};
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use kurbo::{Affine, BezPath};

const TINOS: &[u8] = include_bytes!("assets/tinos_subset.ttf");

/// Per glyph: its Unicode (U+FFFD when unmapped), whether the program is
/// embedded, and whether the font has its own ToUnicode map.
#[derive(Default)]
struct Text {
    text: String,
    embedded: Vec<bool>,
    to_unicode: Vec<bool>,
}

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
        push_unicode(&mut self.text, g);
        self.embedded.push(g.is_embedded());
        self.to_unicode.push(g.has_to_unicode());
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// How the test CID font is set up.
#[derive(Clone, Copy)]
struct CidFont<'s> {
    /// `/CIDSystemInfo /Ordering`.
    ordering: &'s str,
    /// Embed `tinos_subset.ttf` as `/FontFile2`.
    embedded: bool,
    /// `/CIDToGIDMap` stream contents (big-endian glyph IDs indexed by CID);
    /// `None` writes `/Identity`.
    cid_to_gid: Option<&'s [u16]>,
    /// A ToUnicode CMap body for the Type 0 font.
    to_unicode: Option<&'s str>,
}

impl CidFont<'static> {
    fn new(ordering: &'static str, embedded: bool) -> Self {
        Self {
            ordering,
            embedded,
            cid_to_gid: None,
            to_unicode: None,
        }
    }
}

fn stream_obj(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A one-page PDF showing `codes` (two-byte, Identity-H) in a CIDFontType2
/// font. Objects: 1-3 catalog, pages, page; 4 content; 5 Type 0 font; 6
/// CIDFont; 7 descriptor; 8 font file; 9 CIDToGIDMap; 10 ToUnicode.
fn cid_pdf(font: CidFont<'_>, codes: &[u16]) -> Vec<u8> {
    let mut hex = String::new();
    for c in codes {
        let _ = write!(hex, "{c:04X}");
    }
    let content = format!("BT /C 11 Tf 72 700 Td <{hex}> Tj ET");
    let to_unicode = if font.to_unicode.is_some() {
        " /ToUnicode 10 0 R"
    } else {
        ""
    };
    let cid_to_gid = if font.cid_to_gid.is_some() {
        "9 0 R"
    } else {
        "/Identity"
    };
    let font_file = if font.embedded {
        " /FontFile2 8 0 R"
    } else {
        ""
    };

    let gid_bytes: Vec<u8> = font
        .cid_to_gid
        .unwrap_or_default()
        .iter()
        .flat_map(|g| g.to_be_bytes())
        .collect();
    let bodies: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
           /Resources << /Font << /C 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        stream_obj("", content.as_bytes()),
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /Tinos /Encoding /Identity-H \
             /DescendantFonts [6 0 R]{to_unicode} >>"
        )
        .into_bytes(),
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Tinos /CIDSystemInfo \
             << /Registry (Adobe) /Ordering ({}) /Supplement 0 >> /FontDescriptor 7 0 R \
             /CIDToGIDMap {cid_to_gid} /DW 1000 >>",
            font.ordering
        )
        .into_bytes(),
        format!(
            "<< /Type /FontDescriptor /FontName /Tinos /Flags 32 /FontBBox [0 -216 1000 891] \
             /ItalicAngle 0 /Ascent 891 /Descent -216 /CapHeight 662 /StemV 80{font_file} >>"
        )
        .into_bytes(),
        stream_obj(&format!("/Length1 {}", TINOS.len()), TINOS),
        stream_obj("", &gid_bytes),
        stream_obj("", font.to_unicode.unwrap_or_default().as_bytes()),
    ];
    assemble_pdf(&bodies)
}

fn run(font: CidFont<'_>, codes: &[u16]) -> Text {
    run_device(&cid_pdf(font, codes), Text::default())
}

#[test]
fn japan1_ucs2_without_embedded_program() {
    // Spec A.4: Identity-H with Japan1 ordering, CIDs 34-36 → ABC. Upstream
    // already handled the non-embedded case; kept as the reference.
    let out = run(CidFont::new("Japan1", false), &[34, 35, 36]);
    assert_eq!(out.text, "ABC");
    assert_eq!(out.embedded, [false; 3]);
    // The substituted UCS2 CMap is not the font's own ToUnicode.
    assert_eq!(out.to_unicode, [false; 3]);
}

#[test]
fn japan1_ucs2_with_embedded_program() {
    // H6a: the same CIDs with the program embedded (before H6a: no Unicode).
    let out = run(CidFont::new("Japan1", true), &[34, 35, 36]);
    assert_eq!(out.embedded, [true; 3], "the program must load");
    assert_eq!(out.text, "ABC");
    assert_eq!(out.to_unicode, [false; 3]);
}

#[test]
fn identity_embedded_truetype_uses_reversed_cmap() {
    // H6b: CID = glyph ID (Identity encoding and CIDToGIDMap). Glyph 1 is
    // reached from U+0020 and U+00A0; the lower code point wins. Glyph 0
    // (.notdef) and glyph 7 (beyond the font) have no character.
    let out = run(CidFont::new("Identity", true), &[3, 4, 1, 5, 6, 2, 0, 7]);
    assert_eq!(out.embedded, [true; 8]);
    assert_eq!(out.text, "AB ab\"\u{FFFD}\u{FFFD}");
}

#[test]
fn reversed_cmap_follows_the_cid_to_gid_map() {
    // CIDs 0, 1, 2 map to glyphs 0, 5, 6 (`a`, `b`); CID 3 is beyond the
    // map, so it selects glyph 0.
    let font = CidFont {
        cid_to_gid: Some(&[0, 5, 6]),
        ..CidFont::new("Identity", true)
    };
    let out = run(font, &[1, 2, 3]);
    assert_eq!(out.text, "ab\u{FFFD}");
}

#[test]
fn tounicode_wins_over_the_reversed_cmap() {
    // A ToUnicode map is authoritative: codes it maps use it, codes it does
    // not map stay unmapped (the reversed cmap is only for fonts without one).
    let cmap = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                /CMapName /T def 1 begincodespacerange <0000> <FFFF> endcodespacerange \
                1 beginbfchar <0003> <005A> endbfchar endcmap \
                CMapName currentdict /CMap defineresource pop end end";
    let font = CidFont {
        to_unicode: Some(cmap),
        ..CidFont::new("Identity", true)
    };
    let out = run(font, &[3, 4]);
    assert_eq!(out.text, "Z\u{FFFD}");
    assert_eq!(out.to_unicode, [true; 2]);
}

#[test]
fn identity_without_embedded_program_is_left_unmapped() {
    // A substitute font's cmap says nothing about the PDF's glyph IDs, so H6b
    // does not apply; the standard-Macintosh-order fallback for this case
    // (KP16) belongs to the text device.
    let out = run(CidFont::new("Identity", false), &[36, 37, 38]);
    assert_eq!(out.embedded, [false; 3]);
    assert_eq!(out.text, "\u{FFFD}\u{FFFD}\u{FFFD}");
}
