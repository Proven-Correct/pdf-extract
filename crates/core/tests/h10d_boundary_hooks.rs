//! Tests of fork change H10d: the `Device` hooks `begin_form`/`end_form` and
//! `begin_annotation`/`end_annotation` (spec §3.2, oracle §4.9b).
//!
//! The text device turns them into block boundaries (T32). They must come in
//! matched pairs around every form that is drawn (nested forms and
//! annotation appearance streams included) and around every annotation that
//! is drawn, and never for content that is skipped. Devices that do not
//! implement them (every other device in these tests) are unaffected: the
//! defaults do nothing.

mod common;

use std::fmt::Write as _;

use common::{assemble_pdf, run_device, stream};
use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use hayro_syntax::object::{Dict, ObjectIdentifier};
use kurbo::{Affine, BezPath};
use pdf_extract_testkit::hostile;

/// Everything the device saw, as one string: glyph text; `{n:` and `}` for
/// a form (object `n`, `?` without one); `[Subtype:` and `]` for an
/// annotation; `<` and `>` for marked content.
#[derive(Default)]
struct Trace(String);

impl<'a> Device<'a> for Trace {
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
    fn begin_form(&mut self, id: Option<ObjectIdentifier>) {
        match id {
            Some(id) => {
                let _ = write!(self.0, "{{{}:", id.obj_number);
            }
            None => self.0.push_str("{?:"),
        }
    }
    fn end_form(&mut self) {
        self.0.push('}');
    }
    fn begin_annotation(&mut self, subtype: &[u8]) {
        let _ = write!(self.0, "[{}:", String::from_utf8_lossy(subtype));
    }
    fn end_annotation(&mut self) {
        self.0.push(']');
    }
}

const FONT: &str = "/Font << /F 6 0 R >>";

/// A form stream object with Helvetica as `/F`, extra `resources`, and
/// `content`.
fn form(extra: &str, resources: &str, content: &str) -> String {
    stream(
        &format!(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] {extra} \
             /Resources << {FONT} {resources} >>"
        ),
        content,
    )
}

fn text(y: u32, s: &str) -> String {
    format!("BT /F 10 Tf 10 {y} Td ({s}) Tj ET")
}

/// Page content `P`, form 7 twice (each drawing form 8, which leaves a
/// marked-content sequence open), a hidden form 11, then three
/// annotations: a FreeText whose appearance (object 12) also draws form 8, a
/// hidden one, and one without an appearance stream.
fn fixture() -> Vec<u8> {
    assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R \
         /OCProperties << /OCGs [10 0 R] /D << /OFF [10 0 R] >> >> >>"
            .to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 4 0 R \
         /Contents 5 0 R /Annots [9 0 R 13 0 R 14 0 R] >>"
            .to_owned(),
        format!("<< {FONT} /XObject << /Fm1 7 0 R /Hid 11 0 R >> >>"),
        stream("", &format!("{} /Fm1 Do /Fm1 Do /Hid Do", text(10, "P"))),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        form(
            "",
            "/XObject << /Fm2 8 0 R >>",
            &format!("{} /Fm2 Do", text(20, "1")),
        ),
        form("", "", &format!("/Q BMC {}", text(30, "2"))),
        "<< /Type /Annot /Subtype /FreeText /Rect [0 0 100 100] /AP << /N 12 0 R >> >>".to_owned(),
        "<< /Type /OCG /Name (G) >>".to_owned(),
        form("/OC 10 0 R", "", &text(40, "H")),
        form(
            "",
            "/XObject << /Fm2 8 0 R >>",
            &format!("{} /Fm2 Do", text(50, "A")),
        ),
        "<< /Type /Annot /Subtype /Square /F 2 /Rect [0 0 100 100] /AP << /N 12 0 R >> >>"
            .to_owned(),
        "<< /Type /Annot /Subtype /Text /Rect [0 0 10 10] >>".to_owned(),
    ])
}

#[test]
fn forms_and_annotations_are_bracketed() {
    // Form 8's open sequence is closed (H10b) before its `end_form`; the
    // hidden form and the two annotations that are not drawn leave no trace.
    assert_eq!(
        run_device(&fixture(), Trace::default()).0,
        "P{7:1{8:<2>}}{7:1{8:<2>}}[FreeText:{12:A{8:<2>}}]"
    );
}

#[test]
fn a_form_that_draws_itself_is_bracketed_once() {
    // The repeat is skipped (H3b), so it gets no pair either.
    let out = run_device(&hostile::form_self_recursion(), Trace::default()).0;
    assert_eq!(out.matches('{').count(), 1, "{out}");
    assert_eq!(out.matches('}').count(), 1, "{out}");
}

#[test]
fn forms_past_the_nesting_cap_are_not_bracketed() {
    let out = common::on_small_stack(|| run_device(&hostile::nested_forms(60), Trace::default()).0);
    assert_eq!(out.matches('{').count(), 50);
    assert_eq!(out.matches('}').count(), 50);
    assert!(out.ends_with(&"}".repeat(50)), "{out}");
}
