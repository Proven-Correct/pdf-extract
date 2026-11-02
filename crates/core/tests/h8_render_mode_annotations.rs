//! Tests of fork change H8 and of hayro's optional-content handling (spec
//! §3.2 H8, T19-T21, oracle §4.7).
//!
//! - H8b: text in render mode 7 (clip only) reached no device upstream, so a
//!   text extractor lost it; the oracle keeps it once (T20). It is now drawn
//!   as `GlyphDrawMode::Invisible` after its outline joins the clip.
//! - H8c: annotation appearances are drawn after page content (T19), except
//!   Hidden ones (upstream already) and, now, NoView ones and Popup and Link
//!   annotations, which the oracle also leaves out (spec A.3).
//! - KP11 (no fork change): hidden optional content is excluded, as the
//!   oracle does for files read from disk; an OCG used without any
//!   `/OCProperties` is visible. Asserted here so a hayro update that changes
//!   either is noticed.

mod common;

use common::{assemble_pdf, push_unicode, run_device, stream};
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use kurbo::{Affine, BezPath};
use pdf_extract_testkit::{Annotation, Appearance, AppearanceStream, PdfBuilder, TextStyle};

/// Glyph text, plus how each glyph was drawn and how many clips were pushed.
#[derive(Default)]
struct Glyphs {
    text: String,
    modes: Vec<&'static str>,
    clips: usize,
}

impl<'a> Device<'a> for Glyphs {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {
        self.clips += 1;
    }
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(
        &mut self,
        g: &Glyph<'a>,
        _: Affine,
        _: Affine,
        _: &Paint<'a>,
        mode: &GlyphDrawMode,
    ) {
        push_unicode(&mut self.text, g);
        self.modes.push(match mode {
            GlyphDrawMode::Fill => "fill",
            GlyphDrawMode::Stroke(_) => "stroke",
            GlyphDrawMode::Invisible => "invisible",
        });
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

fn glyphs(pdf: &[u8]) -> Glyphs {
    run_device(pdf, Glyphs::default())
}

fn one_run(text: &str, render_mode: u8) -> Glyphs {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .insert_text(
            (72.0, 72.0),
            text,
            TextStyle {
                render_mode,
                ..TextStyle::default()
            },
        )
        .expect("WinAnsi text");
    glyphs(&b.to_bytes())
}

#[test]
fn clip_mode_text_is_drawn_once_as_invisible() {
    // H8b: one Invisible draw per glyph; the glyphs still become the clip.
    let out = one_run("Clip", 7);
    assert_eq!(out.text, "Clip");
    assert_eq!(out.modes, ["invisible"; 4]);
    assert!(out.clips >= 1, "the text clip is still pushed at ET");
}

#[test]
fn other_render_modes_are_unchanged() {
    // Tr 3 was already Invisible; Tr 0 fills; Tr 4 fills once and clips.
    assert_eq!(one_run("ab", 3).modes, ["invisible"; 2]);
    assert_eq!(one_run("ab", 0).modes, ["fill"; 2]);
    let clip_fill = one_run("ab", 4);
    assert_eq!(clip_fill.modes, ["fill"; 2]);
    assert!(clip_fill.clips >= 1);
    assert_eq!(one_run("ab", 0).clips, 0);
}

#[test]
fn annotations_hidden_noview_popup_and_link_are_skipped() {
    // Spec A.3: FreeText with /AP is included after the body; Hidden,
    // NoView, Popup and Link are not, and neither is a widget whose /N is a
    // state dictionary. The Print flag alone does not hide anything.
    let ap = |text: &str| format!("BT /Helv 11 Tf 2 5 Td ({text}) Tj ET");
    let rect = |i: f32| [72.0, 600.0 - 30.0 * i, 272.0, 620.0 - 30.0 * i];
    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    p.insert_text((72.0, 72.0), "Body", TextStyle::default())
        .expect("ASCII");
    p.add_annotation(Annotation::new("FreeText", rect(0.0)).with_appearance(&ap("Shown")))
        .add_annotation(
            Annotation::new("FreeText", rect(1.0))
                .with_flags(Annotation::FLAG_HIDDEN)
                .with_appearance(&ap("Hidden")),
        )
        .add_annotation(
            Annotation::new("FreeText", rect(2.0))
                .with_flags(Annotation::FLAG_NO_VIEW | Annotation::FLAG_PRINT)
                .with_appearance(&ap("NoView")),
        )
        .add_annotation(Annotation::new("Popup", rect(3.0)).with_appearance(&ap("Popup")))
        .add_annotation(Annotation::new("Link", rect(4.0)).with_appearance(&ap("Link")))
        .add_annotation(Annotation::new("Widget", rect(5.0)).with_appearance_spec(
            Appearance::States {
                states: vec![("On".into(), AppearanceStream::new(&ap("Widget")))],
                current: "On".into(),
            },
        ))
        .add_annotation(
            Annotation::new("Stamp", rect(6.0))
                .with_flags(Annotation::FLAG_PRINT)
                .with_appearance(&ap("Printed")),
        );
    assert_eq!(glyphs(&b.to_bytes()).text, "BodyShownPrinted");
}

// ---------------------------------------------------------------------------
// KP11: optional content (unpatched hayro behaviour)

#[test]
fn kp11_hidden_ocg_via_bdc_and_form() {
    // The builder's OCG fixtures: `/OC /name BDC` content and a form whose
    // XObject carries `/OC`, each with a group that /D turns OFF.
    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    let on = p.add_ocg("VisibleOCG", true);
    let off = p.add_ocg("HiddenOCG", false);
    let form_off = p.add_ocg("HiddenForm", false);
    p.add_form_xobject(
        "Oc",
        [0.0, 0.0, 595.0, 842.0],
        "BT /Helv 11 Tf 72 500 Td (HiddenOCG) Tj ET",
    )
    .set_form_ocg("Oc", &form_off)
    .raw_content(&format!(
        "/OC /{} BDC BT /Helv 11 Tf 72 700 Td (Visible) Tj ET EMC \
         /OC /{} BDC BT /Helv 11 Tf 72 680 Td (HiddenOCG) Tj ET EMC \
         /Oc Do BT /Helv 11 Tf 72 660 Td (Plain) Tj ET",
        on.name(),
        off.name()
    ));
    assert_eq!(glyphs(&b.to_bytes()).text, "VisiblePlain");
}

/// A page whose `/Properties` has `/G` (an OCG, object 6) and `/M` (an OCMD
/// over it, object 7), under the given catalog `/OCProperties` entry.
fn ocg_page(oc_properties: &str, content: &str) -> Vec<u8> {
    assemble_pdf(&[
        format!("<< /Type /Catalog /Pages 2 0 R {oc_properties} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 4 0 R \
         /Contents 5 0 R >>"
            .to_owned(),
        "<< /Font << /Helv 8 0 R >> /Properties << /G 6 0 R /M 7 0 R >> >>".to_owned(),
        stream("", content),
        "<< /Type /OCG /Name (G) >>".to_owned(),
        "<< /Type /OCMD /OCGs [6 0 R] /P /AnyOn >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_owned(),
    ])
}

const BOTH: &str = "/OC /G BDC BT /Helv 11 Tf 10 100 Td (HiddenOCG) Tj ET EMC \
                    /OC /M BDC BT /Helv 11 Tf 10 80 Td (HiddenOCG) Tj ET EMC \
                    BT /Helv 11 Tf 10 60 Td (Plain) Tj ET";

#[test]
fn kp11_hidden_ocg_via_ocmd_and_off_list() {
    let pdf = ocg_page(
        "/OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >>",
        BOTH,
    );
    assert_eq!(glyphs(&pdf).text, "Plain");
}

#[test]
fn kp11_hidden_ocg_via_base_state_off() {
    let pdf = ocg_page(
        "/OCProperties << /OCGs [6 0 R] /D << /BaseState /OFF >> >>",
        BOTH,
    );
    assert_eq!(glyphs(&pdf).text, "Plain");
}

#[test]
fn kp11_ocg_without_ocproperties_is_visible() {
    // Spec A.3: an OCG referenced in /Properties with no /OCProperties in
    // the catalog is treated as visible.
    let content = "/OC /G BDC BT /Helv 11 Tf 10 100 Td (Shown) Tj ET EMC";
    assert_eq!(glyphs(&ocg_page("", content)).text, "Shown");
}
