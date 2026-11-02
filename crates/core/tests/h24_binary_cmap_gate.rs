//! H24: a CMap stream from the document in hayro-cmap's binary format is
//! refused (internal #121).
//!
//! hayro-cmap sends any data that starts with `bcmap` to the parser of the
//! binary format its embedded predefined CMaps are stored in. That parser
//! trusts its input: it rebuilds each range as `previous end + 1 + delta`
//! from Huffman-decoded values, which overflows on hostile deltas (a panic
//! with overflow checks, as in test and fuzz builds; a wrong range in
//! release). hayro-cmap is not vendored, so the fork refuses such a stream
//! where a document's CMap is parsed, the `/ToUnicode` and Type 0
//! `/Encoding` streams, exactly as it treats a CMap that fails to parse. The
//! embedded CMaps, reached by name through the resolver, still use the
//! binary parser.
//!
//! The overflowing input is the `cmap` fuzz target's reproducer, also
//! replayed by `fuzz_regressions.rs` through the fuzz driver, which mirrors
//! the gate.

mod common;

use common::{assemble_pdf, run_device};
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_cmap::{BfString, CMap, load_embedded};
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use kurbo::{Affine, BezPath};
use pdf_extract_core::{BlockSeparator, Document, OpenOptions, TextOptions, WarningKind};

/// The `cmap` fuzz reproducer: a binary CMap whose range deltas overflow
/// `hayro-cmap`'s `bcmap/mod.rs` reconstruction (~:396).
const OVERFLOWING: &[u8] = include_bytes!("../../../fuzz/regressions/cmap/b1ba8b8b53308b06");

/// A text CMap that `CMap::parse` rejects (a CID that is not a number): the
/// existing failure path the gate is measured against.
const UNPARSEABLE: &[u8] = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
    1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
    1 begincidrange <0000> <00FF> (not a cid) endcidrange\n\
    endcmap end end";

/// `(to_unicode(), as_unicode())` for each glyph drawn.
#[derive(Default, Debug, PartialEq)]
struct Glyphs(Vec<(Option<BfString>, Option<BfString>)>);

impl<'a> Device<'a> for Glyphs {
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

/// A one-page PDF showing `shown` in font `/F` (object 6), with `cmap` as
/// the raw data of stream object 7 (binary, so the bodies are bytes) and
/// `extra` objects from 8.
fn page(font: &str, shown: &str, cmap: &[u8], extra: &[&str]) -> Vec<u8> {
    let content = format!("BT /F 11 Tf 10 100 Td {shown} Tj ET");
    let mut cmap_stream = format!("<< /Length {} >>\nstream\n", cmap.len()).into_bytes();
    cmap_stream.extend_from_slice(cmap);
    cmap_stream.extend_from_slice(b"\nendstream");
    let mut bodies: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 4 0 R \
           /Contents 5 0 R >>"
            .to_vec(),
        b"<< /Font << /F 6 0 R >> >>".to_vec(),
        format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        )
        .into_bytes(),
        font.as_bytes().to_vec(),
        cmap_stream,
    ];
    bodies.extend(extra.iter().map(|e| e.as_bytes().to_vec()));
    assemble_pdf(&bodies)
}

/// What text extraction makes of `pdf`: page 0's text and its warning kinds.
fn extract(pdf: Vec<u8>) -> (String, Vec<WarningKind>) {
    let doc = Document::from_bytes(pdf, &OpenOptions::default()).expect("opens");
    let page = doc.page(0).expect("page");
    let text_page = page.text_page(&TextOptions::default()).expect("text");
    let text = text_page
        .to_text(BlockSeparator::BlankLine)
        .trim_end()
        .to_owned();
    let kinds = text_page.warnings().iter().map(|w| w.kind).collect();
    (text, kinds)
}

const SIMPLE_FONT: &str = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
                           /Encoding /WinAnsiEncoding /ToUnicode 7 0 R >>";

const TYPE0_DESCENDANT: &str = "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /KozMin \
                                /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) \
                                /Supplement 6 >> /FontDescriptor 9 0 R >>";
const TYPE0_DESCRIPTOR: &str = "<< /Type /FontDescriptor /FontName /KozMin /Flags 4 >>";

#[test]
fn the_reproducer_is_a_binary_cmap_the_unguarded_parser_overflows_on() {
    assert!(OVERFLOWING.starts_with(b"bcmap"));
    assert!(CMap::parse(UNPARSEABLE, load_embedded).is_none());
    // Overflow checks follow debug assertions in the test profile. Without
    // them the arithmetic wraps and the parse returns a wrong map instead.
    if cfg!(debug_assertions) {
        let unguarded = std::panic::catch_unwind(|| CMap::parse(OVERFLOWING, load_embedded));
        assert!(
            unguarded.is_err(),
            "hayro-cmap no longer overflows on the reproducer; the H24 gate may be revisited"
        );
    }
}

#[test]
fn a_binary_to_unicode_stream_is_ignored_like_an_unparseable_one() {
    // Glyph A has no ToUnicode entry either way, so `as_unicode()` falls back
    // to the encoding's glyph name.
    let none = || vec![(None, Some(BfString::Char('A')))];
    for cmap in [OVERFLOWING, b"bcmap".as_slice(), UNPARSEABLE] {
        let got = run_device(&page(SIMPLE_FONT, "(A)", cmap, &[]), Glyphs::default()).0;
        assert_eq!(got, none(), "{:?}", &cmap[..5]);
    }
    let baseline = extract(page(SIMPLE_FONT, "(AB)", UNPARSEABLE, &[]));
    assert_eq!(baseline.0, "AB");
    assert_eq!(
        extract(page(SIMPLE_FONT, "(AB)", OVERFLOWING, &[])),
        baseline
    );
}

#[test]
fn a_binary_encoding_stream_fails_the_font_like_an_unparseable_one() {
    let font = "<< /Type /Font /Subtype /Type0 /BaseFont /KozMin /Encoding 7 0 R \
                /DescendantFonts [8 0 R] >>";
    let extra = [TYPE0_DESCENDANT, TYPE0_DESCRIPTOR];
    let baseline = run_device(
        &page(font, "<0022>", UNPARSEABLE, &extra),
        Glyphs::default(),
    );
    let got = run_device(
        &page(font, "<0022>", OVERFLOWING, &extra),
        Glyphs::default(),
    );
    assert_eq!(got, baseline);
    assert_eq!(
        extract(page(font, "<0022>", OVERFLOWING, &extra)),
        extract(page(font, "<0022>", UNPARSEABLE, &extra))
    );
}

#[test]
fn a_predefined_cmap_named_as_to_unicode_still_parses_from_the_bundle() {
    // The bundle the resolver serves is itself in the binary format: the gate
    // is on document streams, not on the format, so a CMap named in place of
    // a ToUnicode stream (PDFJS-11915) still maps.
    let bundled = load_embedded(hayro_interpret::hayro_cmap::CMapName::from_bytes(
        b"Adobe-Japan1-UCS2",
    ))
    .expect("bundled UCS2 CMap");
    assert!(bundled.starts_with(b"bcmap"));
    // An Identity ordering has no UCS2 CMap to fall back on, so a mapping can
    // only come from the named ToUnicode.
    let descendant = TYPE0_DESCENDANT.replace("(Japan1)", "(Identity)");
    let run = |to_unicode: &str| {
        let font = format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /KozMin /Encoding /Identity-H \
             {to_unicode} /DescendantFonts [8 0 R] >>"
        );
        run_device(
            &page(&font, "<0022>", b"", &[&descendant, TYPE0_DESCRIPTOR]),
            Glyphs::default(),
        )
        .0
    };
    let unnamed = run("");
    assert_eq!(unnamed.len(), 1);
    assert_eq!(unnamed[0].0, None, "{unnamed:?}");
    let named = run("/ToUnicode /Adobe-Japan1-UCS2");
    assert_eq!(named.len(), 1);
    assert!(named[0].0.is_some(), "{named:?}");
}

/// A text CMap equivalent to Identity-H: the success path of the gate in
/// `read_encoding`.
const IDENTITY_TEXT: &[u8] = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
    /CMapName /Text-Identity-H def\n\
    1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
    1 begincidrange <0000> <FFFF> 0 endcidrange\n\
    endcmap end end";

/// A 1-byte text CMap sending code 0x41 to CID 34, so the glyph can only
/// come from the stream's own ranges, not from an identity mapping.
const ONE_BYTE_TEXT: &[u8] = b"begincmap\n\
    1 begincodespacerange <00> <FF> endcodespacerange\n\
    1 begincidchar <41> 34 endcidchar\n\
    endcmap";

#[test]
fn a_text_encoding_stream_still_parses_through_the_gate() {
    // The binary tests above compare a refused stream with an unparseable
    // one, so they would also pass if the gate refused every stream. This
    // pins the other side: a text CMap stream is the font's encoding.
    let extra = [TYPE0_DESCENDANT, TYPE0_DESCRIPTOR];
    let run = |encoding: &str, shown: &str, cmap: &[u8]| {
        let font = format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /KozMin /Encoding {encoding} \
             /DescendantFonts [8 0 R] >>"
        );
        let pdf = page(&font, shown, cmap, &extra);
        (run_device(&pdf, Glyphs::default()), extract(pdf))
    };
    let named = run("/Identity-H", "<0022>", b"");
    // CID 34 is `A` in Adobe-Japan1, which `as_unicode()` reaches through the
    // ordering's UCS2 CMap.
    assert_eq!(named.0.0, vec![(None, Some(BfString::Char('A')))]);
    assert_eq!(named.1.0, "A");
    let failed = run("7 0 R", "<0022>", UNPARSEABLE);
    assert_ne!(
        failed, named,
        "a failed font must differ from the identity one"
    );
    assert_eq!(run("7 0 R", "<0022>", IDENTITY_TEXT), named);
    assert_eq!(run("7 0 R", "<41>", ONE_BYTE_TEXT), named);
}

#[test]
fn a_text_to_unicode_stream_still_maps() {
    let to_unicode = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
        1 begincodespacerange <00> <FF> endcodespacerange\n\
        1 beginbfchar <41> <0058> endbfchar\n\
        endcmap end end";
    let pdf = page(SIMPLE_FONT, "(A)", to_unicode, &[]);
    let got = run_device(&pdf, Glyphs::default()).0;
    assert_eq!(
        got,
        vec![(Some(BfString::Char('X')), Some(BfString::Char('X')))]
    );
    assert_eq!(extract(pdf).0, "X");
}
