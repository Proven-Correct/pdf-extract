//! §4.9 visible-box and clip-path filtering (T22-T24, KP7, KP19), and the
//! default ascent/descent for unusable descriptors.

use pdf_extract_testkit::{PdfBuilder, TextStyle};

use super::common::raw_pdf;
use super::{extract, joined, texts};

/// The labels of the records, grouped by baseline: records with the same
/// origin y belong to one label run.
fn runs(bytes: Vec<u8>) -> Vec<String> {
    let p = extract(bytes);
    let mut out: Vec<(f32, String)> = Vec::new();
    for c in &p.chars {
        match out.last_mut() {
            Some((y, s)) if *y == c.origin.1 => s.push_str(c.text.as_str()),
            _ => out.push((c.origin.1, c.text.as_str().to_owned())),
        }
    }
    out.into_iter().map(|(_, s)| s).collect()
}

#[test]
fn kp7_mediabox_edges() {
    // Spec A.2, coordinates as given to `insert_text` (top-left origin, y
    // down, 842 pt page): `y845` has its baseline 3 pt below the page and
    // its ascent reaches back in; `y850` (8 pt below) does not; `x590` keeps
    // only the `x` that straddles the right edge.
    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    for (x, y, label) in [
        (100.0, 840.0, "y840"),
        (100.0, 845.0, "y845"),
        (100.0, 850.0, "y850"),
        (590.0, 400.0, "x590"),
        (-5.0, 450.0, "xneg5"),
        (100.0, 2.0, "y2"),
        (100.0, -3.0, "yneg3"),
        (100.0, 500.0, "inside"),
    ] {
        page.insert_text((x, y), label, TextStyle::size(11.0))
            .expect("WinAnsi");
    }
    assert_eq!(
        runs(b.to_bytes()),
        ["y840", "y845", "x", "xneg5", "y2", "inside"]
    );
}

#[test]
fn kp7_cropbox_clips_and_is_intersected_with_the_mediabox() {
    // Spec A.2, PDF coordinates.
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .set_crop_box([100.0, 100.0, 400.0, 700.0])
        .raw_content(
            "BT /Helv 11 Tf 150 50 Td (BelowCrop) Tj ET \
         BT /Helv 11 Tf 450 300 Td (RightOfCrop) Tj ET \
         BT /Helv 11 Tf 150 750 Td (AboveCrop) Tj ET \
         BT /Helv 11 Tf 150 300 Td (InsideCrop) Tj ET \
         BT /Helv 11 Tf 150 95 Td (Straddle95) Tj ET \
         BT /Helv 11 Tf 150 105 Td (Straddle105) Tj ET",
        );
    assert_eq!(
        runs(b.to_bytes()),
        ["InsideCrop", "Straddle95", "Straddle105"]
    );

    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .set_crop_box([100.0, -100.0, 400.0, 1000.0])
        .raw_content(
            "BT /Helv 11 Tf 150 50 Td (Kept) Tj ET BT /Helv 11 Tf 150 850 Td (AboveMedia) Tj ET",
        );
    assert_eq!(runs(b.to_bytes()), ["Kept"]);
}

#[test]
fn kp19_rectangular_clip() {
    // Spec A.2: `0 0 150 842 re W n`; `Straddle` at 140 keeps `St`; `ab` at
    // 140 is kept and `cd` at 160 is not.
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(
        "0 0 150 842 re W n \
         BT /Helv 11 Tf 140 700 Td (Straddle) Tj ET \
         BT /Helv 11 Tf 140 600 Td (ab) Tj ET BT /Helv 11 Tf 160 600 Td (cd) Tj ET",
    );
    assert_eq!(runs(b.to_bytes()), ["St", "ab"]);
}

#[test]
fn kp19_non_rectangular_clips_use_their_bounding_box() {
    // A diamond with bounding box 100..200: `Corner` lies outside the path
    // but inside the box, and is kept, as the reference keeps it.
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(
        "150 100 m 200 150 l 150 200 l 100 150 l h W n \
         BT /Helv 11 Tf 102 102 Td (Corner) Tj ET \
         BT /Helv 11 Tf 140 150 Td (Mid) Tj ET \
         BT /Helv 11 Tf 300 300 Td (Away) Tj ET",
    );
    assert_eq!(runs(b.to_bytes()), ["Corner", "Mid"]);
}

#[test]
fn kp19_text_clip_mode_then_inside_and_outside() {
    // Spec A.2: 40 pt `CLIP` in mode 7 (kept once, invisible), then `inside`
    // within its outline's box and `outside` far away.
    let mut b = PdfBuilder::new();
    b.add_page_a4().raw_content(
        "BT /Helv 40 Tf 7 Tr 100 700 Td (CLIP) Tj ET \
         BT /Helv 11 Tf 0 Tr 100 705 Td (inside) Tj ET \
         BT /Helv 11 Tf 0 Tr 300 300 Td (outside) Tj ET",
    );
    let p = extract(b.to_bytes());
    assert_eq!(joined(&p), "CLIPinside");
    assert!(p.chars[..4].iter().all(|c| c.invisible));
    assert!(p.chars[4..].iter().all(|c| !c.invisible));
}

#[test]
fn kp19_form_bbox_clips() {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_form_xobject(
            "Fm",
            [0.0, 0.0, 50.0, 50.0],
            "BT /Helv 11 Tf 100 600 Td (OutsideBBox) Tj ET BT /Helv 11 Tf 10 10 Td (InBBox) Tj ET",
        )
        .raw_content("/Fm Do BT /Helv 11 Tf 100 600 Td (Page) Tj ET");
    assert_eq!(runs(b.to_bytes()), ["InBBox", "Page"]);
}

/// A 200 × 200 page showing `A` at baselines y = 201 (1 pt above the top)
/// and y = -3 (3 pt below the bottom) in Helvetica with `descriptor` (`None`:
/// no descriptor, so the AFM metrics apply).
fn edge_glyphs(descriptor: Option<&str>) -> Vec<u8> {
    let (desc_ref, desc_obj) = match descriptor {
        Some(d) => (
            " /FontDescriptor 7 0 R",
            format!("<< /Type /FontDescriptor /FontName /Helvetica /Flags 32 {d} >>"),
        ),
        None => ("", "<< >>".to_owned()),
    };
    raw_pdf(&[
        "<< /Font << /F 6 0 R >> >>".to_owned(),
        super::common::stream(
            "",
            "BT /F 11 Tf 50 201 Td (A) Tj ET BT /F 11 Tf 50 -3 Td (B) Tj ET",
        ),
        format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica \
             /Encoding /WinAnsiEncoding{desc_ref} >>"
        ),
        desc_obj,
    ])
}

#[test]
fn unusable_descriptor_metrics_fall_back_to_the_defaults() {
    // Spec §4.9 (5): a positive /Descent or a non-positive /Ascent would
    // float the box off the baseline; the 0.72 / -0.22 defaults apply and
    // both edge glyphs are kept, exactly as with the AFM metrics.
    assert_eq!(texts(&extract(edge_glyphs(None))), ["A", "B"]);
    for bad in ["/Ascent 718 /Descent 200", "/Ascent 0 /Descent -207"] {
        assert_eq!(texts(&extract(edge_glyphs(Some(bad)))), ["A", "B"], "{bad}");
    }
    // Valid metrics are used as given: a tiny ascent and descent (0.55 pt
    // at 11 pt) keep the box near the baseline, so both glyphs fall outside.
    let tight = extract(edge_glyphs(Some("/Ascent 50 /Descent -50")));
    assert!(tight.chars.is_empty(), "{:?}", texts(&tight));
}
