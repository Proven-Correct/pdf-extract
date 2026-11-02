//! Tests of fork change H10h: `Glyph::to_unicode()`, the ToUnicode lookup
//! alone (spec §3.2, T26, D-39, oracle §4.9b).
//!
//! `as_unicode()` falls back to hayro's own glyph-name table when the
//! ToUnicode map lacks a code, skips map entries for U+0000, and for CID
//! fonts answers from a substitute UCS2 CMap. The text device owns those
//! steps itself, so it needs the map's own answer.

mod common;

use common::{raw_pdf, run_device, stream};
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use kurbo::{Affine, BezPath};
use pdf_extract_testkit::to_unicode_cmap;

/// `(to_unicode(), as_unicode())` for each glyph.
#[derive(Default)]
struct Both(Vec<(Option<BfString>, Option<BfString>)>);

impl<'a> Device<'a> for Both {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        _: Affine,
        _: Affine,
        _: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
        self.0.push((glyph.to_unicode(), glyph.as_unicode()));
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn ch(c: char) -> Option<BfString> {
    Some(BfString::Char(c))
}

/// Shows `shown` in font `/F` (object 6) with `cmap` as its ToUnicode
/// (object 7) and `extra` objects from 8.
fn page(font: &str, shown: &str, cmap: Option<String>, extra: &[String]) -> Vec<u8> {
    let mut objects = vec![
        "<< /Font << /F 6 0 R >> >>".to_owned(),
        stream("", &format!("BT /F 11 Tf 10 100 Td {shown} Tj ET")),
        font.to_owned(),
        stream("", &cmap.unwrap_or_default()),
    ];
    objects.extend_from_slice(extra);
    raw_pdf(&objects)
}

#[test]
fn simple_fonts_answer_from_the_map_only() {
    // A: mapped; B: mapped to U+0000; C: not in the map, but named `C` by
    // the encoding.
    let cmap = to_unicode_cmap(1, &[(65, "Z"), (66, "\0")]);
    for subtype in ["Type1", "TrueType"] {
        let font = format!(
            "<< /Type /Font /Subtype /{subtype} /BaseFont /Helvetica \
             /Encoding /WinAnsiEncoding /ToUnicode 7 0 R >>"
        );
        let got = run_device(
            &page(&font, "(ABC)", Some(cmap.clone()), &[]),
            Both::default(),
        )
        .0;
        assert_eq!(got[0], (ch('Z'), ch('Z')), "{subtype}");
        assert_eq!(got[1].0, ch('\0'), "{subtype}");
        assert_eq!(got[2], (None, ch('C')), "{subtype}");
    }
    // Upstream's Type 1 lookup skips the U+0000 entry in favour of the name.
    let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
                /Encoding /WinAnsiEncoding /ToUnicode 7 0 R >>";
    let got = run_device(&page(font, "(B)", Some(cmap), &[]), Both::default()).0;
    assert_eq!(got[0], (ch('\0'), ch('B')));
}

#[test]
fn no_map_means_none() {
    let font = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";
    let got = run_device(&page(font, "(A)", None, &[]), Both::default()).0;
    assert_eq!(got, [(None, ch('A'))]);
}

#[test]
fn a_substitute_ucs2_map_is_not_tounicode() {
    // A non-embedded Adobe-Japan1 font without ToUnicode: hayro answers
    // `as_unicode()` from the collection's UCS2 CMap; `to_unicode()` does
    // not, since the document has no ToUnicode map.
    let font = "<< /Type /Font /Subtype /Type0 /BaseFont /KozMin /Encoding /Identity-H \
                /DescendantFonts [8 0 R] >>";
    let descendant = "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /KozMin /CIDSystemInfo \
                      << /Registry (Adobe) /Ordering (Japan1) /Supplement 6 >> \
                      /FontDescriptor 9 0 R >>"
        .to_owned();
    let descriptor = "<< /Type /FontDescriptor /FontName /KozMin /Flags 4 >>".to_owned();
    let got = run_device(
        &page(
            font,
            "<0022>",
            None,
            &[descendant.clone(), descriptor.clone()],
        ),
        Both::default(),
    )
    .0;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, None);
    assert!(got[0].1.is_some());
    // With a ToUnicode map, its entries are returned as is.
    let with_map = font.replace(">>", "/ToUnicode 7 0 R >>");
    let cmap = to_unicode_cmap(2, &[(0x22, "Q")]);
    let got = run_device(
        &page(&with_map, "<0022>", Some(cmap), &[descendant, descriptor]),
        Both::default(),
    )
    .0;
    assert_eq!(got, [(ch('Q'), ch('Q'))]);
}

#[test]
fn type3_fonts_use_their_map() {
    let font = "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] \
                /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 8 0 R >> \
                /Encoding << /Differences [97 /a 98 /a] >> /FirstChar 97 /LastChar 98 \
                /Widths [1000 1000] /ToUnicode 7 0 R >>";
    let cmap = to_unicode_cmap(1, &[(97, "x")]);
    let got = run_device(
        &page(font, "(ab)", Some(cmap), &[stream("", "1000 0 d0")]),
        Both::default(),
    )
    .0;
    assert_eq!(got, [(ch('x'), ch('x')), (None, None)]);
}
