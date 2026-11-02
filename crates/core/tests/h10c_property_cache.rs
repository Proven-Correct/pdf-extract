//! Tests of fork change H10c: a named property list (`/Name BDC`) is
//! resolved once per interpreter context (spec §3.2, oracle §4.9b).
//!
//! Upstream parsed the referenced object twice for every `BDC` that named
//! it, so a large property list (a long `/ActualText`) referenced from many
//! `BDC`s cost its full size per operator.

mod common;

use std::time::{Duration, Instant};

use common::{Collector, assemble_pdf, interpret, run_device, stream};
use hayro_interpret::font::Glyph;
use hayro_interpret::property_hooks::resolutions;
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use hayro_syntax::Pdf;
use hayro_syntax::object::dict::keys::ACTUAL_TEXT;
use hayro_syntax::object::{self, Dict};
use kurbo::{Affine, BezPath};

/// A page whose content refers to `/P0` (object 7, an `/ActualText` of
/// `text_len` bytes) and `/P1` (object 8, an optional-content group) `refs`
/// times each, then shows `z`.
fn named(text_len: usize, refs: usize) -> Vec<u8> {
    let mut content = String::from("BT /F 11 Tf 10 100 Td ");
    content.push_str(&"/Span /P0 BDC EMC /OC /P1 BDC EMC ".repeat(refs));
    content.push_str("(z) Tj ET");
    assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 4 0 R \
         /Contents 5 0 R >>"
            .to_owned(),
        "<< /Font << /F 6 0 R >> /Properties << /P0 7 0 R /P1 8 0 R >> >>".to_owned(),
        stream("", &content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /ActualText ({}) >>", "x".repeat(text_len)),
        "<< /Type /OCG /Name (G) >>".to_owned(),
    ])
}

#[test]
fn a_named_property_is_resolved_once_per_context() {
    let pdf = Pdf::new(named(16, 2000)).expect("opens");
    let page = &pdf.pages()[0];
    let before = resolutions();
    let out: Collector = interpret(&pdf, page);
    assert_eq!(out.text, "z");
    // `/P0` and `/P1`, once each, for 4,000 `BDC`s.
    assert_eq!(resolutions() - before, 2);
    // A new context resolves them again: the cache does not outlive it.
    let before = resolutions();
    interpret(&pdf, page);
    assert_eq!(resolutions() - before, 2);
}

#[test]
fn a_4mb_property_referenced_2000_times_is_fast() {
    let pdf = Pdf::new(named(4 << 20, 2000)).expect("opens");
    let page = &pdf.pages()[0];
    let _ = page.page_stream();
    let start = Instant::now();
    let out = interpret(&pdf, page);
    let elapsed = start.elapsed();
    assert_eq!(out.text, "z");
    assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
}

/// The `/ActualText` of every property list the device receives.
#[derive(Default)]
struct ActualTexts(Vec<String>);

impl<'a> Device<'a> for ActualTexts {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(
        &mut self,
        _: &Glyph<'a>,
        _: Affine,
        _: Affine,
        _: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn begin_marked_content(&mut self, _: &[u8], _: Option<i32>, props: Option<&Dict<'_>>) {
        let text = props
            .and_then(|d| d.get::<object::String<'_>>(ACTUAL_TEXT))
            .map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned());
        self.0.push(text.unwrap_or_default());
    }
}

#[test]
fn names_in_different_property_dictionaries_are_kept_apart() {
    // The page and a form both name `/P`, each in its own `/Properties`
    // dictionary and with a different `/ActualText`; the page's entry is
    // resolved (and cached) first, and used again after the form.
    let bytes = assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 4 0 R \
         /Contents 5 0 R >>"
            .to_owned(),
        "<< /XObject << /Fm 6 0 R >> /Properties << /P 7 0 R >> >>".to_owned(),
        stream("", "/Span /P BDC EMC /Fm Do /Span /P BDC EMC"),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] \
             /Resources << /Properties << /P 8 0 R >> >>",
            "/Span /P BDC EMC",
        ),
        "<< /ActualText (Page) >>".to_owned(),
        "<< /ActualText (Form) >>".to_owned(),
    ]);
    assert_eq!(
        run_device(&bytes, ActualTexts::default()).0,
        ["Page", "Form", "Page"]
    );
}
