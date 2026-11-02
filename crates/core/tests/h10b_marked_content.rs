//! Tests of fork change H10b: `interpret()` closes the marked-content
//! sequences a content stream leaves open, before it returns (spec §3.2,
//! oracle §4.9b).
//!
//! Upstream let an unbalanced `BDC` in a Form XObject stay open after the
//! form: the device saw the sequence continue over the rest of the page, and
//! a hidden optional-content span hid everything after the form.

mod common;

use common::{assemble_pdf, run_device, stream};
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use hayro_syntax::object::Dict;
use kurbo::{Affine, BezPath};

/// What the device saw, in order: `<` and `>` for marked content, the text
/// of each glyph otherwise.
#[derive(Default)]
struct Events(String);

impl<'a> Device<'a> for Events {
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
        if let Some(BfString::Char(c)) = glyph.as_unicode() {
            self.0.push(c);
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn begin_marked_content(&mut self, _: &[u8], _: Option<i32>, _: Option<&Dict<'_>>) {
        self.0.push('<');
    }
    fn end_marked_content(&mut self) {
        self.0.push('>');
    }
}

/// A page that draws form `/Fm` (content `form`) inside `page`, with
/// Helvetica as `/F` and the optional-content group `/G` (object 8), which
/// the catalog sets OFF.
fn form_page(page: &str, form: &str) -> Vec<u8> {
    assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R \
         /OCProperties << /OCGs [8 0 R] /D << /OFF [8 0 R] >> >> >>"
            .to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 4 0 R \
         /Contents 5 0 R >>"
            .to_owned(),
        "<< /Font << /F 6 0 R >> /XObject << /Fm 7 0 R >> /Properties << /G 8 0 R >> >>".to_owned(),
        stream("", page),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] \
             /Resources << /Font << /F 6 0 R >> /Properties << /G 8 0 R >> >>",
            form,
        ),
        "<< /Type /OCG /Name (G) >>".to_owned(),
    ])
}

#[test]
fn sequences_left_open_by_a_form_end_with_the_form() {
    // Two unclosed levels inside the form, then text after the form at the
    // page's own level.
    let bytes = form_page(
        "/P BMC /Fm Do BT /F 10 Tf 10 10 Td (B) Tj ET EMC",
        "/Span << /ActualText (x) >> BDC /Q BMC BT /F 10 Tf 10 50 Td (A) Tj ET",
    );
    assert_eq!(run_device(&bytes, Events::default()).0, "<<<A>>B>");
}

#[test]
fn a_hidden_span_left_open_by_a_form_hides_only_the_form() {
    let bytes = form_page(
        "/Fm Do BT /F 10 Tf 10 10 Td (After) Tj ET",
        "/OC /G BDC BT /F 10 Tf 10 50 Td (Hidden) Tj ET",
    );
    assert_eq!(run_device(&bytes, Events::default()).0, "<>After");
}

#[test]
fn the_page_content_is_closed_too() {
    // The page's own stream is one invocation as well.
    let bytes = form_page("/P BMC /Q BMC BT /F 10 Tf 10 10 Td (A) Tj ET", "");
    assert_eq!(run_device(&bytes, Events::default()).0, "<<A>>");
}

#[test]
fn balanced_and_stray_emc_are_unchanged() {
    // Balanced sequences see exactly their own EMCs; a stray EMC in a form
    // still closes the enclosing sequence, as upstream does, and is not
    // closed a second time.
    let bytes = form_page(
        "/P BMC /Fm Do EMC BT /F 10 Tf 10 10 Td (B) Tj ET",
        "/Q BMC BT /F 10 Tf 10 50 Td (A) Tj ET EMC EMC",
    );
    assert_eq!(run_device(&bytes, Events::default()).0, "<<A>>>B");
}
