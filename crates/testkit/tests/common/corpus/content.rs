//! Content-stream fixtures: clipping (KP7, KP19), annotations (KP8),
//! ActualText (KP9, KP17), optional content (KP11, and forms that inherit
//! the page's named properties, spec 0002 O37), render modes (KP13,
//! KP14), missing fonts (KP18), forms and damaged content (KP20) and deep
//! form nesting (KP21). Placements and labels follow spec Appendix A.2-A.4.

use pdf_extract_testkit::hostile;
use pdf_extract_testkit::{Annotation, Appearance, AppearanceStream, PdfBuilder, TextStyle};

use super::{Fixture, HELV_DICT, assemble, helv, raw, stream};

/// One page (595 x 842) whose `/Resources` is object 4 and `/Contents`
/// object 5; `extra` bodies are numbered from 6. `catalog_extra` is added to
/// the catalog dictionary.
fn page_with_objects(
    catalog_extra: &str,
    resources: &str,
    content: &str,
    extra: &[String],
) -> Vec<u8> {
    let mut bodies = vec![
        format!("<< /Type /Catalog /Pages 2 0 R{catalog_extra} >>"),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources 4 0 R \
         /Contents 5 0 R >>"
            .to_owned(),
        resources.to_owned(),
        stream("", content),
    ];
    bodies.extend_from_slice(extra);
    assemble(&bodies)
}

fn kp7() -> Vec<Fixture> {
    let mut out = Vec::new();
    // MediaBox edges, coordinates as given to `insert_text` (top-left
    // origin, y down) on an 842 pt page (A.2).
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
    ] {
        page.insert_text((x, y), label, helv(11.0))
            .expect("WinAnsi");
    }
    out.push(Fixture::exact(
        "kp07_mediabox_edges.pdf",
        &["KP7"],
        b.to_bytes(),
    ));

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
    out.push(Fixture::exact(
        "kp07_cropbox_clips.pdf",
        &["KP7"],
        b.to_bytes(),
    ));

    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .set_crop_box([100.0, -100.0, 400.0, 1000.0])
        .raw_content(
            "BT /Helv 11 Tf 150 50 Td (Kept) Tj ET BT /Helv 11 Tf 150 850 Td (AboveMedia) Tj ET",
        );
    out.push(Fixture::exact(
        "kp07_crop_beyond_media.pdf",
        &["KP7"],
        b.to_bytes(),
    ));
    out
}

/// A page showing `Body`, plus one annotation.
fn annotated(annot: Annotation) -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    page.insert_text((72.0, 72.0), "Body", helv(11.0))
        .expect("WinAnsi");
    page.add_annotation(annot);
    b.to_bytes()
}

fn kp8() -> Vec<Fixture> {
    let ap = "BT /Helv 11 Tf 2 5 Td (AnnotText) Tj ET";
    let rect = [72.0, 600.0, 272.0, 620.0];
    let mut out = vec![Fixture::exact(
        "kp08_freetext_ap.pdf",
        &["KP8"],
        annotated(Annotation::new("FreeText", rect).with_appearance(ap)),
    )];
    for (file, subtype, flags) in [
        ("kp08_hidden.pdf", "FreeText", Annotation::FLAG_HIDDEN),
        ("kp08_noview.pdf", "FreeText", Annotation::FLAG_NO_VIEW),
        ("kp08_popup.pdf", "Popup", 0),
        ("kp08_link.pdf", "Link", 0),
    ] {
        out.push(Fixture::exact(
            file,
            &["KP8"],
            annotated(
                Annotation::new(subtype, rect)
                    .with_flags(flags)
                    .with_appearance(ap),
            ),
        ));
    }
    out.push(Fixture::exact(
        "kp08_widget_states.pdf",
        &["KP8"],
        annotated(
            Annotation::new("Widget", rect).with_appearance_spec(Appearance::States {
                states: vec![
                    ("On".into(), AppearanceStream::new(ap)),
                    ("Off".into(), AppearanceStream::new("")),
                ],
                current: "On".into(),
            }),
        ),
    ));
    out.push(Fixture::exact(
        "kp08_stamp_matrix.pdf",
        &["KP8"],
        annotated(
            Annotation::new("Stamp", [72.0, 400.0, 172.0, 440.0]).with_appearance_spec(
                Appearance::Stream(AppearanceStream {
                    content: "BT /Helv 11 Tf 2 5 Td (Scaled) Tj ET".into(),
                    bbox: Some([0.0, 0.0, 200.0, 80.0]),
                    matrix: Some([0.5, 0.0, 0.0, 0.5, 0.0, 0.0]),
                }),
            ),
        ),
    ));
    out.push(
        Fixture::exact(
            "kp08_freetext_no_ap.pdf",
            &["KP8"],
            annotated(
                Annotation::new("FreeText", [72.0, 360.0, 272.0, 380.0])
                    .with_contents("NoAppearance", "/Helv 11 Tf 0 g"),
            ),
        )
        .diverges(
            "§2.5: a FreeText annotation without an appearance stream; the oracle \
             synthesises one from /Contents and /DA, we extract nothing",
        ),
    );
    out
}

fn kp9() -> Vec<Fixture> {
    let kp = &["KP9"];
    let mut out = vec![
        Fixture::exact(
            "kp09_inline.pdf",
            kp,
            raw(
                "BT /Helv 11 Tf 100 700 Td /Span << /ActualText (Hello) >> BDC (Hello) Tj EMC \
                 ( world) Tj ET",
            ),
        ),
        Fixture::exact(
            "kp09_ligature.pdf",
            kp,
            raw(
                "BT /Helv 11 Tf 100 700 Td /Span << /ActualText <FEFF00660069> >> BDC (X) Tj \
                 EMC (ne) Tj ET",
            ),
        ),
        Fixture::exact(
            "kp09_named.pdf",
            kp,
            page_with_objects(
                "",
                "<< /Font << /Helv 6 0 R >> /Properties << /P1 << /ActualText (ABC) >> >> >>",
                "BT /Helv 11 Tf 100 700 Td /Span /P1 BDC (xyz) Tj EMC ET",
                &[HELV_DICT.to_owned()],
            ),
        ),
        Fixture::exact(
            "kp09_empty.pdf",
            kp,
            raw(
                "BT /Helv 11 Tf 100 700 Td /Span << /ActualText () >> BDC (dropped) Tj EMC \
                 (kept) Tj ET",
            ),
        ),
    ];
    let kp17 = &["KP9", "KP17"];
    let why = "KP17: an ActualText span across lines, gaps or nesting; the oracle's \
               distribution rule is not reproducible black-box, we emit the whole string \
               at the span's first glyph (T28)";
    out.push(
        Fixture::exact(
            "kp09_cross_line.pdf",
            kp17,
            raw(
                "BT /Helv 11 Tf 100 700 Td /Span << /ActualText (joined) >> BDC (ab) Tj \
                 0 -20 Td (cd) Tj EMC ET",
            ),
        )
        .diverges(why),
    );
    // The glyph *starts* are 40 pt apart (a 34.5 pt gap); the oracle gives
    // `a\n b` here, not A.4's `a \nb` (spec revision 10).
    out.push(
        Fixture::exact(
            "kp09_gap.pdf",
            kp17,
            raw(
                "BT /Helv 11 Tf 100 700 Td /Span << /ActualText (a b) >> BDC (x) Tj \
                 40 0 Td (y) Tj EMC ET",
            ),
        )
        .diverges(
            "KP17: an ActualText span over two glyphs whose starts are 40 pt apart; the \
             oracle gives `a\\n b` (A.4 recorded `a \\nb` for its own recipe), we emit \
             `a b` at the first glyph (T28)",
        ),
    );
    out.push(
        Fixture::exact(
            "kp09_nested.pdf",
            kp17,
            raw(
                "BT /Helv 11 Tf 100 700 Td /Span << /ActualText (OUTER) >> BDC (a) Tj \
                 /Span << /ActualText (IN) >> BDC (b) Tj EMC (c) Tj EMC ET",
            ),
        )
        .diverges(why),
    );
    out
}

/// A page whose resources name OCG `/G` (object 6) and an OCMD `/M` over it
/// (object 7); `oc_properties` goes into the catalog.
fn ocg_page(oc_properties: &str, content: &str) -> Vec<u8> {
    page_with_objects(
        oc_properties,
        "<< /Font << /Helv 8 0 R >> /Properties << /G 6 0 R /M 7 0 R >> >>",
        content,
        &[
            "<< /Type /OCG /Name (G) >>".to_owned(),
            "<< /Type /OCMD /OCGs [6 0 R] /P /AnyOn >>".to_owned(),
            HELV_DICT.to_owned(),
        ],
    )
}

fn kp11() -> Vec<Fixture> {
    let kp = &["KP11"];
    let mut out = Vec::new();

    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    let on = page.add_ocg("VisibleOCG", true);
    let off = page.add_ocg("HiddenOCG", false);
    page.raw_content(&format!(
        "/OC /{} BDC BT /Helv 11 Tf 72 700 Td (VisibleOCG) Tj ET EMC \
         /OC /{} BDC BT /Helv 11 Tf 72 680 Td (HiddenOCG) Tj ET EMC \
         BT /Helv 11 Tf 72 660 Td (Plain) Tj ET",
        on.name(),
        off.name()
    ));
    out.push(Fixture::exact("kp11_hidden_ocg_bdc.pdf", kp, b.to_bytes()));

    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    let off = page.add_ocg("HiddenForm", false);
    page.add_form_xobject(
        "Fm",
        [0.0, 0.0, 595.0, 842.0],
        "BT /Helv 11 Tf 72 680 Td (FormOCG) Tj ET",
    )
    .set_form_ocg("Fm", &off)
    .raw_content(
        "BT /Helv 11 Tf 72 700 Td (Before) Tj ET /Fm Do BT /Helv 11 Tf 72 660 Td (After) Tj ET",
    );
    out.push(Fixture::exact("kp11_hidden_ocg_form.pdf", kp, b.to_bytes()));

    let both = "/OC /M BDC BT /Helv 11 Tf 72 700 Td (HiddenOCMD) Tj ET EMC \
                BT /Helv 11 Tf 72 680 Td (Plain) Tj ET";
    out.push(Fixture::exact(
        "kp11_hidden_ocg_ocmd.pdf",
        kp,
        ocg_page(
            " /OCProperties << /OCGs [6 0 R] /D << /OFF [6 0 R] >> >>",
            both,
        ),
    ));
    out.push(Fixture::exact(
        "kp11_hidden_ocg_basestate.pdf",
        kp,
        ocg_page(
            " /OCProperties << /OCGs [6 0 R] /D << /BaseState /OFF >> >>",
            "/OC /G BDC BT /Helv 11 Tf 72 700 Td (HiddenOCG) Tj ET EMC \
             BT /Helv 11 Tf 72 680 Td (Plain) Tj ET",
        ),
    ));
    out.push(Fixture::exact(
        "kp11_ocg_no_ocprops.pdf",
        kp,
        ocg_page(
            "",
            "/OC /G BDC BT /Helv 11 Tf 72 700 Td (ShownOCG) Tj ET EMC",
        ),
    ));
    out
}

/// A form under a page whose resources object (4) names `P1`, an OFF
/// optional-content group (object 9), in its `/Properties`; page and form
/// each show one visible and one `/OC /P1`-marked line. `page_resources` is
/// the page's `/Resources` value and `form_resources` the form's
/// `/Resources` entry (with its key, or empty). Objects: 4 the shared
/// resources, 5 page content, 6 the form `/Fx`, 7 Helvetica, 8 a full copy
/// of 4, 9 the OCG, 10 a resources dictionary holding only `/Properties`.
fn ocg_form_page(page_resources: &str, form_resources: &str) -> Vec<u8> {
    assemble(&[
        "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [9 0 R] /D << /OFF [9 0 R] >> >> >>"
            .to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources {page_resources} \
             /Contents 5 0 R >>"
        ),
        "<< /Font << /Helv 7 0 R >> /XObject << /Fx 6 0 R >> /Properties << /P1 9 0 R >> >>"
            .to_owned(),
        stream(
            "",
            "BT /Helv 11 Tf 72 700 Td (PAGEVIS) Tj ET \
             /OC /P1 BDC BT /Helv 11 Tf 72 680 Td (HIDDENPAGE) Tj ET EMC /Fx Do",
        ),
        stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 595 842] {form_resources}"),
            "BT /Helv 11 Tf 72 660 Td (FORMVIS) Tj ET \
             /OC /P1 BDC BT /Helv 11 Tf 72 640 Td (HIDDENFORM) Tj ET EMC",
        ),
        HELV_DICT.to_owned(),
        "<< /Font << /Helv 7 0 R >> /Properties << /P1 9 0 R >> >>".to_owned(),
        "<< /Type /OCG /Name (P1) >>".to_owned(),
        "<< /Properties << /P1 9 0 R >> >>".to_owned(),
    ])
}

/// Spec 0002 O37 (0001 Q8): a form must hide content under an OFF group that
/// only the page's `/Properties` names, whether it reaches them through the
/// page's own resources object or by inheritance. The first three leaked
/// before fork change H12; the other four never did and pin that.
fn ocg_forms() -> Vec<Fixture> {
    let kp = &["KP11"];
    [
        ("ocg_form_shared_resources.pdf", "4 0 R", "/Resources 4 0 R"),
        ("ocg_form_no_resources.pdf", "4 0 R", ""),
        (
            "ocg_form_direct_resources_no_properties.pdf",
            "4 0 R",
            "/Resources << /Font << /Helv 7 0 R >> >>",
        ),
        (
            "ocg_form_direct_resources.pdf",
            "4 0 R",
            "/Resources << /Font << /Helv 7 0 R >> /Properties << /P1 9 0 R >> >>",
        ),
        (
            "ocg_form_indirect_resources_copy.pdf",
            "4 0 R",
            "/Resources 8 0 R",
        ),
        (
            "ocg_form_indirect_resources_properties_only.pdf",
            "4 0 R",
            "/Resources 10 0 R",
        ),
        (
            "ocg_form_shared_resources_page_direct.pdf",
            "<< /Font << /Helv 7 0 R >> /XObject << /Fx 6 0 R >> /Properties << /P1 9 0 R >> >>",
            "/Resources 4 0 R",
        ),
    ]
    .into_iter()
    .map(|(file, page, form)| Fixture::exact(file, kp, ocg_form_page(page, form)))
    .collect()
}

/// Precedence shape (internal issue #55 B23): the page's `/Properties` and the form's
/// own `/Resources` name the *same* `P1` for different groups, so the answer
/// depends on which dictionary wins (the form's own, per fork change H12:
/// the innermost form's `/Resources` wins; spec 0002 H12/O37, ADR 0074).
/// Objects: 4 page resources (`P1` = `page_ocg`), 6 the form `/Fx`, 7
/// Helvetica, 8 a form resources object (`P1` = OCG 10), 9 an ON group, 10 an
/// OFF group. Page and form each show one plain and one `/OC /P1` line.
fn ocg_form_precedence_page(page_ocg: &str, form_resources: &str) -> Vec<u8> {
    assemble(&[
        "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [9 0 R 10 0 R] \
         /D << /OFF [10 0 R] >> >> >>"
            .to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources 4 0 R \
         /Contents 5 0 R >>"
            .to_owned(),
        format!(
            "<< /Font << /Helv 7 0 R >> /XObject << /Fx 6 0 R >> \
             /Properties << /P1 {page_ocg} >> >>"
        ),
        stream(
            "",
            "BT /Helv 11 Tf 72 700 Td (PAGEVIS) Tj ET \
             /OC /P1 BDC BT /Helv 11 Tf 72 680 Td (PAGEMARK) Tj ET EMC /Fx Do",
        ),
        stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 595 842] {form_resources}"),
            "BT /Helv 11 Tf 72 660 Td (FORMVIS) Tj ET \
             /OC /P1 BDC BT /Helv 11 Tf 72 640 Td (FORMMARK) Tj ET EMC",
        ),
        HELV_DICT.to_owned(),
        "<< /Font << /Helv 7 0 R >> /Properties << /P1 10 0 R >> >>".to_owned(),
        "<< /Type /OCG /Name (On) >>".to_owned(),
        "<< /Type /OCG /Name (Off) >>".to_owned(),
    ])
}

/// Named `/Resources` beat inherited ones: the page's `P1` and the form's
/// disagree, and the form's own wins whether it is a direct dictionary or an
/// indirect object. The reverse case (form ON under an OFF page group) rules
/// out an implementation that lets any OFF group in the chain hide content.
fn ocg_form_precedence() -> Vec<Fixture> {
    let kp = &["KP11"];
    [
        (
            "ocg_form_precedence_direct_off.pdf",
            "9 0 R",
            "/Resources << /Font << /Helv 7 0 R >> /Properties << /P1 10 0 R >> >>",
        ),
        (
            "ocg_form_precedence_indirect_off.pdf",
            "9 0 R",
            "/Resources 8 0 R",
        ),
        (
            "ocg_form_precedence_direct_on.pdf",
            "10 0 R",
            "/Resources << /Font << /Helv 7 0 R >> /Properties << /P1 9 0 R >> >>",
        ),
    ]
    .into_iter()
    .map(|(file, page, form)| Fixture::exact(file, kp, ocg_form_precedence_page(page, form)))
    .collect()
}

fn kp13_14() -> Vec<Fixture> {
    (0..=7u8)
        .map(|mode| {
            let mut b = PdfBuilder::new();
            b.add_page_a4()
                .insert_text(
                    (100.0, 100.0),
                    &format!("Mode{mode}"),
                    TextStyle {
                        render_mode: mode,
                        ..helv(11.0)
                    },
                )
                .expect("WinAnsi");
            let file = format!("kp13_tr{mode}.pdf");
            if (4..=6).contains(&mode) {
                Fixture::exact(file, &["KP13", "KP14"], b.to_bytes()).diverges(
                    "KP14 (Q4): the oracle emits text in render modes 4-6 twice; we emit it once",
                )
            } else {
                Fixture::exact(file, &["KP13"], b.to_bytes())
            }
        })
        .collect()
}

fn kp18() -> Vec<Fixture> {
    vec![
        Fixture::exact(
            "kp18_no_tf.pdf",
            &["KP18"],
            raw("BT 100 700 Td (NoTf) Tj ET"),
        )
        .diverges(
            "KP18: text shown before any Tf; we extract it with hayro's Helvetica \
             fallback, the oracle drops it",
        ),
        Fixture::exact(
            "kp18_missing_font_resource.pdf",
            &["KP18"],
            raw("BT /Missing 11 Tf 100 700 Td (NoFontRes) Tj ET"),
        ),
    ]
}

fn kp19() -> Vec<Fixture> {
    let kp = &["KP19"];
    let mut out = vec![
        Fixture::exact(
            "kp19_rect_clip.pdf",
            kp,
            raw("0 0 150 842 re W n \
                 BT /Helv 11 Tf 140 700 Td (Straddle) Tj ET \
                 BT /Helv 11 Tf 140 600 Td (ab) Tj ET BT /Helv 11 Tf 160 600 Td (cd) Tj ET"),
        ),
        Fixture::exact(
            "kp19_diamond_clip.pdf",
            kp,
            raw("150 100 m 200 150 l 150 200 l 100 150 l h W n \
                 BT /Helv 11 Tf 102 102 Td (Corner) Tj ET \
                 BT /Helv 11 Tf 140 150 Td (Mid) Tj ET \
                 BT /Helv 11 Tf 300 300 Td (Away) Tj ET"),
        ),
        Fixture::exact(
            "kp19_tr7_clip.pdf",
            kp,
            raw("BT /Helv 40 Tf 7 Tr 100 700 Td (CLIP) Tj ET \
                 BT /Helv 11 Tf 0 Tr 100 705 Td (inside) Tj ET \
                 BT /Helv 11 Tf 0 Tr 300 300 Td (outside) Tj ET"),
        ),
    ];
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_form_xobject(
            "Fm",
            [0.0, 0.0, 50.0, 50.0],
            "BT /Helv 11 Tf 100 600 Td (OutsideBBox) Tj ET BT /Helv 11 Tf 5 20 Td (InBBox) Tj ET",
        )
        .raw_content("/Fm Do BT /Helv 11 Tf 100 600 Td (Page) Tj ET");
    out.push(Fixture::exact("kp19_form_bbox.pdf", kp, b.to_bytes()));
    out
}

fn kp20() -> Vec<Fixture> {
    let kp = &["KP20"];
    let mut out = Vec::new();
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_form_xobject(
            "Fm0",
            [0.0, 0.0, 595.0, 842.0],
            "BT /Helv 11 Tf 72 680 Td (InForm) Tj ET",
        )
        .raw_content(
            "BT /Helv 11 Tf 72 700 Td (Before) Tj ET /Fm0 Do BT /Helv 11 Tf 72 660 Td (After) Tj ET",
        );
    out.push(Fixture::exact("kp20_form_order.pdf", kp, b.to_bytes()));

    // The form names itself through the page's shared resources.
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_form_xobject(
            "Self",
            [0.0, 0.0, 595.0, 842.0],
            "BT /Helv 11 Tf 72 700 Td (Rec) Tj ET /Self Do",
        )
        .raw_content("/Self Do");
    out.push(Fixture::exact(
        "kp20_form_self_recursion.pdf",
        kp,
        b.to_bytes(),
    ));

    out.push(Fixture::exact(
        "kp20_contents_array.pdf",
        kp,
        assemble(&[
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
             /Resources << /Font << /Helv 6 0 R >> >> /Contents [4 0 R 5 0 R] >>"
                .to_owned(),
            stream("", "BT /Helv 11 Tf 72 700 Td (A) Tj ET"),
            stream("", "BT /Helv 11 Tf 72 680 Td (B) Tj ET"),
            HELV_DICT.to_owned(),
        ]),
    ));

    out.push(Fixture::exact(
        "kp20_damaged_content.pdf",
        kp,
        raw(
            "BT /Helv 11 Tf 72 700 Td (GoodOp) Tj ET\nBT /Helv 11 Tf 72 680 Td (Partial) Tj ET\n\
             ) ) ] >> garbage\nBT /Helv 11 Tf 72 660 Td (AfterGarbage) Tj ET",
        ),
    ));
    out
}

pub fn fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    out.extend(kp7());
    out.extend(kp8());
    out.extend(kp9());
    out.extend(kp11());
    out.extend(ocg_forms());
    out.extend(ocg_form_precedence());
    out.extend(kp13_14());
    out.extend(kp18());
    out.extend(kp19());
    out.extend(kp20());
    // KP21 is also the committed hostile `nested_forms_60` of Appendix C:
    // one file (the bytes are identical, and expectations and snapshots are
    // keyed by hash), gated as the KP21 divergence with the hostile row's
    // warning kind.
    out.push(
        Fixture::exact(
            "kp21_nested_forms_60.pdf",
            &["KP21"],
            hostile::nested_forms(60),
        )
        .diverges(
            "KP21: Form XObjects nested 60 deep; the oracle extracts every level, \
                 hayro stops at its nesting cap of 50",
        )
        .warns(&["InterpreterNesting"]),
    );
    out
}
