//! §4.9 render modes (T20, KP4, KP13, KP14), annotations (KP8), optional
//! content (KP11) and damaged content (T35).

use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use hayro_syntax::object::Dict;
use kurbo::{Affine, BezPath, Rect};
use pdf_extract_testkit::{Annotation, Appearance, AppearanceStream, PdfBuilder, TextStyle};

use super::common::{assemble_pdf, run_device, stream};
use super::{extract, joined, texts};
use crate::text::device::TextDevice;
use crate::text::options::Unmapped;
use crate::text::record::{Boundary, Marker};

/// Forwards to a [`TextDevice`] and counts the `draw_glyph` calls it gets,
/// by mode, to show what hayro sends and what the device keeps.
struct Counting {
    inner: TextDevice,
    fills: usize,
    strokes: usize,
    invisibles: usize,
}

impl Counting {
    fn new() -> Self {
        Self {
            inner: TextDevice::new(Rect::new(-1e5, -1e5, 1e5, 1e5), Unmapped::Cid, 1000),
            fills: 0,
            strokes: 0,
            invisibles: 0,
        }
    }
}

impl<'a> Device<'a> for Counting {
    fn set_soft_mask(&mut self, m: Option<SoftMask<'a>>) {
        self.inner.set_soft_mask(m);
    }
    fn set_blend_mode(&mut self, b: BlendMode) {
        self.inner.set_blend_mode(b);
    }
    fn draw_path(&mut self, p: &BezPath, t: Affine, paint: &Paint<'a>, m: &PathDrawMode) {
        self.inner.draw_path(p, t, paint, m);
    }
    fn push_clip_path(&mut self, c: &ClipPath) {
        self.inner.push_clip_path(c);
    }
    fn push_transparency_group(&mut self, o: f32, m: Option<SoftMask<'a>>, b: BlendMode) {
        self.inner.push_transparency_group(o, m, b);
    }
    fn draw_glyph(
        &mut self,
        g: &Glyph<'a>,
        t: Affine,
        gt: Affine,
        paint: &Paint<'a>,
        mode: &GlyphDrawMode,
    ) {
        match mode {
            GlyphDrawMode::Fill => self.fills += 1,
            GlyphDrawMode::Stroke(_) => self.strokes += 1,
            GlyphDrawMode::Invisible => self.invisibles += 1,
        }
        self.inner.draw_glyph(g, t, gt, paint, mode);
    }
    fn draw_image(&mut self, i: Image<'a, '_>, t: Affine) {
        self.inner.draw_image(i, t);
    }
    fn pop_clip_path(&mut self) {
        self.inner.pop_clip_path();
    }
    fn pop_transparency_group(&mut self) {
        self.inner.pop_transparency_group();
    }
    fn begin_marked_content(&mut self, tag: &[u8], mcid: Option<i32>, p: Option<&Dict<'_>>) {
        self.inner.begin_marked_content(tag, mcid, p);
    }
    fn end_marked_content(&mut self) {
        self.inner.end_marked_content();
    }
    fn begin_form(&mut self, id: Option<hayro_syntax::object::ObjectIdentifier>) {
        self.inner.begin_form(id);
    }
    fn end_form(&mut self) {
        self.inner.end_form();
    }
    fn begin_annotation(&mut self, subtype: &[u8]) {
        self.inner.begin_annotation(subtype);
    }
    fn end_annotation(&mut self) {
        self.inner.end_annotation();
    }
}

fn one_glyph_in_mode(mode: u8) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .insert_text(
            (100.0, 100.0),
            "M",
            TextStyle {
                render_mode: mode,
                ..TextStyle::default()
            },
        )
        .expect("WinAnsi");
    b.to_bytes()
}

#[test]
fn every_render_mode_gives_one_record() {
    // T20: once each for 0-7 (the reference doubles 4-6, KP14); invisible
    // for 3 (KP13) and 7.
    for mode in 0..=7 {
        let p = extract(one_glyph_in_mode(mode));
        assert_eq!(texts(&p), ["M"], "mode {mode}");
        assert_eq!(p.chars[0].invisible, matches!(mode, 3 | 7), "mode {mode}");
    }
}

#[test]
fn fill_and_stroke_modes_are_drawn_twice_and_kept_once() {
    // Spec §4.9 (5): for modes 2 and 6 hayro sends Fill then Stroke; a
    // Stroke-only glyph (mode 1) is kept.
    for (mode, fills, strokes) in [(1, 0, 1), (2, 1, 1), (6, 1, 1), (5, 0, 1)] {
        let d = run_device(&one_glyph_in_mode(mode), Counting::new());
        assert_eq!((d.fills, d.strokes), (fills, strokes), "mode {mode}");
        let (chars, _, _) = d.inner.finish();
        assert_eq!(chars.len(), 1, "mode {mode}");
    }
}

#[test]
fn a_stroke_after_a_fill_from_another_show_is_kept() {
    // The same glyph at the same origin, filled by one show (Tr 0) and
    // stroked by the next (Tr 1): two shows, two records. hayro sets the
    // soft mask and blend mode before each show, which is what clears the
    // device's memory of the last Fill.
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(
        "BT /Helv 11 Tf 100 700 Td 0 Tr (B) Tj ET BT /Helv 11 Tf 100 700 Td 1 Tr (B) Tj ET",
    );
    let p = extract(b.to_bytes());
    assert_eq!(texts(&p), ["B", "B"]);
    assert_eq!(p.chars[0].origin, p.chars[1].origin);
}

#[test]
fn fake_bold_repeats_are_kept() {
    // KP4: the same word shown twice at the same origin is two shows, so
    // eight records, also when each show fills and strokes.
    for mode in [0, 2] {
        let content = format!(
            "BT /Helv 11 Tf {mode} Tr 100 700 Td (Bold) Tj ET BT /Helv 11 Tf {mode} Tr 100 700 Td \
             (Bold) Tj ET"
        );
        let mut b = PdfBuilder::new();
        b.add_page_a4().raw_content(&content);
        let p = extract(b.to_bytes());
        assert_eq!(joined(&p), "BoldBold", "mode {mode}");
    }
}

#[test]
fn annotations_follow_the_body_and_hidden_kinds_are_skipped() {
    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    page.insert_text((100.0, 100.0), "Body", TextStyle::default())
        .expect("WinAnsi");
    let shows = |label: &str| format!("BT /Helv 11 Tf 2 5 Td ({label}) Tj ET");
    let rect = [100.0, 600.0, 300.0, 650.0];
    page.add_annotation(Annotation::new("FreeText", rect).with_appearance(&shows("AnnotVisible")));
    for (subtype, flags, label) in [
        ("FreeText", Annotation::FLAG_HIDDEN, "Hidden"),
        ("FreeText", Annotation::FLAG_NO_VIEW, "NoView"),
        ("Popup", 0, "Popup"),
        ("Link", 0, "Link"),
    ] {
        page.add_annotation(
            Annotation::new(subtype, rect)
                .with_flags(flags)
                .with_appearance(&shows(label)),
        );
    }
    // A stamp whose 200 × 100 appearance box is scaled into a 100 × 50 rect.
    page.add_annotation(
        Annotation::new("Stamp", [300.0, 300.0, 400.0, 350.0]).with_appearance_spec(
            Appearance::Stream(AppearanceStream {
                content: "BT /Helv 20 Tf 10 10 Td (Scaled) Tj ET".to_owned(),
                bbox: Some([0.0, 0.0, 200.0, 100.0]),
                matrix: Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            }),
        ),
    );
    let p = extract(b.to_bytes());
    assert_eq!(joined(&p), "BodyAnnotVisibleScaled");
    let scaled = &p.chars[p.chars.len() - 6];
    assert_eq!(scaled.text.as_str(), "S");
    assert_eq!(scaled.origin, (305.0, 305.0));
    assert!((scaled.size - 10.0).abs() < 1e-3);
}

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

#[test]
fn kp11_hidden_optional_content_is_absent() {
    // BDC and form /OC through the builder.
    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    let off = page.add_ocg("Off", false);
    page.add_form_xobject(
        "Fm",
        [0.0, 0.0, 595.0, 842.0],
        "BT /Helv 11 Tf 10 300 Td (HiddenOCG) Tj ET",
    )
    .set_form_ocg("Fm", &off)
    .raw_content(&format!(
        "/OC /{} BDC BT /Helv 11 Tf 10 100 Td (HiddenOCG) Tj ET EMC /Fm Do \
         BT /Helv 11 Tf 10 60 Td (Plain) Tj ET",
        off.name()
    ));
    assert_eq!(joined(&extract(b.to_bytes())), "Plain");
    // An OCMD over an OFF group, and /BaseState /OFF.
    let both = "/OC /G BDC BT /Helv 11 Tf 10 100 Td (HiddenOCG) Tj ET EMC \
                /OC /M BDC BT /Helv 11 Tf 10 80 Td (HiddenOCG) Tj ET EMC \
                BT /Helv 11 Tf 10 60 Td (Plain) Tj ET";
    for props in [
        "/OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >>",
        "/OCProperties << /OCGs [6 0 R] /D << /BaseState /OFF >> >>",
    ] {
        assert_eq!(joined(&extract(ocg_page(props, both))), "Plain", "{props}");
    }
    // Referenced, but no /OCProperties in the catalog: visible.
    let shown = "/OC /G BDC BT /Helv 11 Tf 10 100 Td (HiddenOCG) Tj ET EMC";
    assert_eq!(joined(&extract(ocg_page("", shown))), "HiddenOCG");
}

fn damaged(garbage: &str) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(&format!(
        "BT /Helv 11 Tf 72 700 Td (GoodOp) Tj ET\nBT /Helv 11 Tf 72 680 Td (Partial) Tj ET\n\
         {garbage}\nBT /Helv 11 Tf 72 660 Td (AfterGarbage) Tj ET"
    ));
    b.to_bytes()
}

#[test]
fn unknown_operators_are_skipped() {
    // hayro skips an unknown operator and carries on, as the reference does.
    assert_eq!(
        joined(&extract(damaged("garbage"))),
        "GoodOpPartialAfterGarbage"
    );
}

#[test]
fn damaged_content_keeps_the_text_after_the_garbage() {
    // §4.9 (6), H10a: the reference gives `GoodOp\nPartial\nAfterGarbage\n`
    // (re-probed); the lexer skips the five bytes `) ) ] > >` it cannot
    // read (`garbage` is an unknown operator, skipped without a skip count)
    // and core reports one `ContentDamaged` warning counting them.
    let p = extract(damaged(") ) ] >> garbage"));
    assert_eq!(joined(&p), "GoodOpPartialAfterGarbage");
    assert_eq!(super::kinds(&p), [crate::WarningKind::ContentDamaged]);
    assert_eq!(p.warnings[0].count, 5);
    assert!(p.warnings[0].to_string().starts_with("content: damaged"));
    // The shared detail the renderer also raises, not a copy of it.
    assert_eq!(
        p.warnings[0].detail,
        crate::text::details::CONTENT_DAMAGED_DETAIL
    );
    // A plain unknown operator is not damage.
    assert!(extract(damaged("garbage")).warnings.is_empty());
}

#[test]
fn form_and_annotation_boundaries_are_marked() {
    // §4.9 (6), H10d: form and annotation boundaries appear as markers in
    // the record stream, for the line assembler's blocks (T32).
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_form_xobject(
            "Fm",
            [0.0, 0.0, 595.0, 842.0],
            "BT /Helv 11 Tf 100 600 Td (InForm) Tj ET",
        )
        .raw_content("BT /Helv 11 Tf 100 700 Td (Before) Tj ET /Fm Do")
        .add_annotation(
            Annotation::new("FreeText", [100.0, 300.0, 300.0, 350.0])
                .with_appearance("BT /Helv 11 Tf 2 5 Td (Annot) Tj ET"),
        );
    let p = extract(b.to_bytes());
    assert_eq!(joined(&p), "BeforeInFormAnnot");
    // Records 0-5 `Before`, 6-11 `InForm`, 12-16 `Annot`. The appearance
    // stream is itself a form, inside the annotation's pair.
    let marker = |at, boundary| Marker { at, boundary };
    assert_eq!(
        p.markers,
        [
            marker(6, Boundary::FormStart),
            marker(12, Boundary::FormEnd),
            marker(12, Boundary::AnnotationStart),
            marker(12, Boundary::FormStart),
            marker(17, Boundary::FormEnd),
            marker(17, Boundary::AnnotationEnd),
        ]
    );
}

#[test]
fn boundaries_without_text_leave_no_marker() {
    // A form that draws only a path, nested forms around nothing, and an
    // annotation whose appearance has no text: nothing to separate.
    let pdf = assemble_pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 4 0 R \
         /Contents 5 0 R /Annots [9 0 R] >>"
            .to_owned(),
        "<< /Font << /F 6 0 R >> /XObject << /A 7 0 R /B 8 0 R >> >>".to_owned(),
        stream(
            "",
            "BT /F 10 Tf 10 10 Td (x) Tj ET /A Do /B Do BT /F 10 Tf 10 30 Td (y) Tj ET",
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200]",
            "0 0 10 10 re f",
        ),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] \
             /Resources << /XObject << /A 7 0 R >> >>",
            "/A Do /A Do",
        ),
        "<< /Type /Annot /Subtype /Square /Rect [0 0 50 50] /AP << /N 7 0 R >> >>".to_owned(),
    ]);
    let p = extract(pdf);
    assert_eq!(joined(&p), "xy");
    assert!(p.markers.is_empty(), "{:?}", p.markers);
}

#[test]
fn boundary_markers_are_bounded() {
    // 45 nested forms around each of 40 glyphs give 3,600 markers; a page
    // with a 10-character limit keeps at most 2 × 10 + 256.
    let mut content = String::new();
    for i in 0..40 {
        content.push_str(&format!("/L0 Do % {i}\n"));
    }
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] \
         /Resources << /XObject << /L0 5 0 R >> >> /Contents 4 0 R >>"
            .to_owned(),
        stream("", &content),
    ];
    let depth = 45;
    for level in 0..depth {
        let obj = 5 + level;
        let (resources, body) = if level + 1 < depth {
            (
                format!("/XObject << /L0 {} 0 R >>", obj + 1),
                "/L0 Do".to_owned(),
            )
        } else {
            (
                format!("/Font << /F {} 0 R >>", obj + 1),
                "BT /F 10 Tf 10 10 Td (x) Tj ET".to_owned(),
            )
        };
        objects.push(stream(
            &format!(
                "/Type /XObject /Subtype /Form /BBox [0 0 200 200] /Resources << {resources} >>"
            ),
            &body,
        ));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned());
    let pdf = assemble_pdf(&objects);
    let full = extract(pdf.clone());
    assert_eq!(full.chars.len(), 40);
    assert_eq!(full.markers.len(), 40 * 2 * depth);
    let limited = super::extract_with(
        pdf,
        crate::Limits {
            max_chars_per_page: 10,
            ..crate::Limits::default()
        },
        Unmapped::Cid,
    )
    .expect("partial page");
    assert_eq!(limited.chars.len(), 10);
    assert_eq!(limited.markers.len(), 2 * 10 + 256);
}

#[test]
fn paths_images_and_groups_are_ignored() {
    // A filled path, an inline image and a transparency-group form around
    // the text: none of them produces records or disturbs the glyphs.
    let pdf = super::common::raw_pdf(&[
        "<< /Font << /F 6 0 R >> /XObject << /G 7 0 R >> >>".to_owned(),
        stream(
            "",
            "0 0 10 10 re f BI /W 1 /H 1 /CS /G /BPC 8 ID x EI /G Do \
             BT /F 11 Tf 10 100 Td (Text) Tj ET",
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] \
             /Group << /S /Transparency >> /Resources 4 0 R",
            "BT /F 11 Tf 10 150 Td (InGroup) Tj ET",
        ),
    ]);
    let d = run_device(&pdf, Counting::new());
    let (chars, _, _) = d.inner.finish();
    let text: String = chars.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(text, "InGroupText");
}
