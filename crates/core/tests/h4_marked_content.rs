//! Tests of fork change H4: `Device::begin_marked_content` receives the
//! marked-content property list (spec §3.2, oracle §4.7).
//!
//! Upstream passes only the tag and an inline `/MCID`, so a device cannot see
//! `/ActualText` (KP9, T28), which the text device needs. H4 adds the property
//! dictionary: the inline dictionary of a `BDC`, or the `/Properties` entry a
//! name operand refers to; `None` for `BMC` and for names that do not resolve.

mod common;

use common::{raw_pdf, run_device, stream};
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use hayro_syntax::object::dict::keys::ACTUAL_TEXT;
use hayro_syntax::object::{self, Dict};
use kurbo::{Affine, BezPath};

/// One `begin_marked_content` call, or a glyph drawn between them.
#[derive(Debug, PartialEq)]
enum Event {
    Begin {
        tag: String,
        mcid: Option<i32>,
        /// `None`: no property dictionary was passed. `Some(None)`: a
        /// dictionary without `/ActualText`.
        actual_text: Option<Option<Vec<u8>>>,
    },
    End,
    Glyph,
}

fn begin(tag: &str, mcid: Option<i32>, actual_text: Option<Option<&[u8]>>) -> Event {
    Event::Begin {
        tag: tag.to_owned(),
        mcid,
        actual_text: actual_text.map(|t| t.map(<[u8]>::to_vec)),
    }
}

#[derive(Default)]
struct Marks(Vec<Event>);

impl<'a> Device<'a> for Marks {
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
        self.0.push(Event::Glyph);
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}

    fn begin_marked_content(&mut self, tag: &[u8], mcid: Option<i32>, props: Option<&Dict<'_>>) {
        let actual_text = props.map(|d| {
            d.get::<object::String<'_>>(ACTUAL_TEXT)
                .map(|s| s.as_bytes().to_vec())
        });
        self.0.push(Event::Begin {
            tag: String::from_utf8_lossy(tag).into_owned(),
            mcid,
            actual_text,
        });
    }

    fn end_marked_content(&mut self) {
        self.0.push(Event::End);
    }
}

const HELV: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

/// A page whose resources carry `/Properties << /P1 6 0 R /P2 << ... >> >>`
/// (one indirect, one direct entry) and the form `/Fm`.
fn page(content: &str, form_content: &str) -> Vec<u8> {
    raw_pdf(&[
        "<< /Font << /Helv 7 0 R >> /XObject << /Fm 8 0 R >> \
         /Properties << /P1 6 0 R /P2 << /ActualText <FEFF00660069> >> /Bad 7 0 R >> >>"
            .to_owned(),
        stream("", content),
        "<< /ActualText (Named) /MCID 9 >>".to_owned(),
        HELV.to_owned(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] /Resources 4 0 R",
            form_content,
        ),
    ])
}

fn events(content: &str) -> Vec<Event> {
    run_device(&page(content, ""), Marks::default()).0
}

#[test]
fn inline_properties_are_passed() {
    let ev = events("/Span << /ActualText (Hello) /MCID 3 >> BDC BT /Helv 11 Tf (H) Tj ET EMC");
    assert_eq!(
        ev,
        [
            begin("Span", Some(3), Some(Some(b"Hello"))),
            Event::Glyph,
            Event::End
        ]
    );
}

#[test]
fn named_properties_are_resolved() {
    // P1 is an indirect object, P2 a direct dictionary; both resolve. The MCID
    // inside a named property list is not reported (unchanged upstream
    // behaviour: only an inline /MCID is).
    let ev = events("/Span /P1 BDC EMC /Span /P2 BDC EMC");
    assert_eq!(
        ev,
        [
            begin("Span", None, Some(Some(b"Named"))),
            Event::End,
            begin("Span", None, Some(Some(b"\xFE\xFF\x00f\x00i"))),
            Event::End,
        ]
    );
}

#[test]
fn bmc_and_unresolvable_names_have_no_properties() {
    // /Missing is not in /Properties; /Bad resolves to a font dictionary,
    // which is still a dictionary and is passed (it has no /ActualText).
    let ev = events("/P BMC EMC /Span /Missing BDC EMC /Span /Bad BDC EMC /Span 42 BDC EMC");
    assert_eq!(
        ev,
        [
            begin("P", None, None),
            Event::End,
            begin("Span", None, None),
            Event::End,
            begin("Span", None, Some(None)),
            Event::End,
            begin("Span", None, None),
            Event::End,
        ]
    );
}

#[test]
fn inline_properties_inside_a_form_are_passed() {
    // A form's content is decoded into a buffer that lives only while the
    // form is interpreted; its inline dictionaries still reach the device.
    let pdf = page(
        "/Fm Do",
        "/Span << /ActualText (InForm) >> BDC BT /Helv 11 Tf (x) Tj ET EMC",
    );
    let ev = run_device(&pdf, Marks::default()).0;
    assert_eq!(
        ev,
        [
            begin("Span", None, Some(Some(b"InForm"))),
            Event::Glyph,
            Event::End
        ]
    );
}

#[test]
fn optional_content_marks_still_pass_their_group() {
    // `/OC /name BDC` drives hayro's OCG state and is reported like any other
    // BDC; the property dictionary is the OCG itself.
    let pdf = raw_pdf(&[
        "<< /Font << /Helv 7 0 R >> /Properties << /L 6 0 R >> >>".to_owned(),
        stream("", "/OC /L BDC BT /Helv 11 Tf (x) Tj ET EMC"),
        "<< /Type /OCG /Name (Layer) >>".to_owned(),
        HELV.to_owned(),
    ]);
    let ev = run_device(&pdf, Marks::default()).0;
    assert_eq!(
        ev,
        [begin("OC", None, Some(None)), Event::Glyph, Event::End]
    );
}
