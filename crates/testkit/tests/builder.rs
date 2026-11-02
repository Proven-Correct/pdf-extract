//! Builder conventions (spec §4.3, T49, T50): coordinate system, WinAnsi
//! rejection, determinism, and that each page-level feature lands in the
//! dictionary hayro reads back.

use hayro_syntax::Pdf;
use hayro_syntax::object::{Dict, Name, Object};
use hayro_syntax::page::Rotation;
use pdf_extract_testkit::{
    Annotation, BuildError, FontDict, PdfBuilder, SimpleEncoding, TextStyle, text_width,
};

/// Every `(operator, numeric operands)` of page `index`'s content stream, as
/// hayro's content lexer sees it.
fn ops(pdf: &Pdf, index: usize) -> Vec<(String, Vec<f32>)> {
    let page = &pdf.pages()[index];
    // hayro's untyped iterator lends each instruction, so it is not an
    // `Iterator`; drain it by hand.
    let mut iter = page.operations();
    let mut out = Vec::new();
    while let Some(instr) = iter.next() {
        let nums = instr
            .operands()
            .filter_map(|o| match o {
                Object::Number(n) => Some(n.as_f32()),
                _ => None,
            })
            .collect();
        out.push((String::from_utf8_lossy(instr.operator).into_owned(), nums));
    }
    out
}

#[test]
fn insert_text_origin_is_top_left_with_y_down() {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .insert_text((100.0, 100.0), "A", TextStyle::default())
        .expect("ASCII");
    let pdf = Pdf::new(b.to_bytes()).expect("opens");
    let tm: Vec<Vec<f32>> = ops(&pdf, 0)
        .into_iter()
        .filter(|(op, _)| op == "Tm")
        .map(|(_, n)| n)
        .collect();
    assert_eq!(tm, vec![vec![1.0, 0.0, 0.0, 1.0, 100.0, 742.0]]);
    let tf: Vec<Vec<f32>> = ops(&pdf, 0)
        .into_iter()
        .filter(|(op, _)| op == "Tf")
        .map(|(_, n)| n)
        .collect();
    assert_eq!(tf, vec![vec![11.0]], "default size 11");
}

#[test]
fn insert_text_follows_page_height_and_style() {
    let mut b = PdfBuilder::new();
    let p = b.add_page(300.0, 200.0);
    let style = TextStyle {
        fontsize: 22.5,
        render_mode: 3,
        font: "Helv",
    };
    p.insert_text((10.25, 50.0), "Hi", style).expect("ASCII");
    let pdf = Pdf::new(b.to_bytes()).expect("opens");
    let all = ops(&pdf, 0);
    let names: Vec<&str> = all.iter().map(|(o, _)| o.as_str()).collect();
    assert_eq!(names, ["q", "BT", "Tf", "Tr", "Tm", "Tj", "ET", "Q"]);
    assert_eq!(all[2].1, [22.5]);
    assert_eq!(all[3].1, [3.0]);
    assert_eq!(all[4].1, [1.0, 0.0, 0.0, 1.0, 10.25, 150.0]);
}

#[test]
fn non_win_ansi_text_is_rejected_and_appends_nothing() {
    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    let before = p.content().to_owned();
    assert_eq!(
        p.insert_text((0.0, 0.0), "日本", TextStyle::default()),
        Err(BuildError::NotWinAnsi { ch: '日' })
    );
    assert_eq!(
        p.insert_text((0.0, 0.0), "A\tB", TextStyle::default()),
        Err(BuildError::NotWinAnsi { ch: '\t' })
    );
    assert_eq!(p.content(), before);
    assert_eq!(
        text_width("日本", 11.0),
        Err(BuildError::NotWinAnsi { ch: '日' })
    );
    let msg = BuildError::NotWinAnsi { ch: '日' }.to_string();
    assert!(msg.contains("U+65E5"), "{msg}");
}

#[test]
fn render_mode_and_non_finite_inputs_are_rejected() {
    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    let bad_mode = TextStyle {
        render_mode: 8,
        ..TextStyle::default()
    };
    assert_eq!(
        p.insert_text((0.0, 0.0), "A", bad_mode),
        Err(BuildError::RenderMode(8))
    );
    assert_eq!(
        p.insert_text((f32::NAN, 0.0), "A", TextStyle::default()),
        Err(BuildError::NonFinite)
    );
    assert_eq!(
        p.insert_text((0.0, 0.0), "A", TextStyle::size(f32::INFINITY)),
        Err(BuildError::NonFinite)
    );
    assert!(p.content().is_empty());
}

fn feature_doc() -> PdfBuilder {
    let mut b = PdfBuilder::new();
    let p = b.add_page(400.0, 300.0);
    p.insert_text((10.0, 20.0), "Feature", TextStyle::default())
        .expect("ASCII");
    let ocg = p.add_ocg("Layer", false);
    p.set_crop_box([10.0, 20.0, 390.0, 280.0])
        .set_rotation(90)
        .add_font(
            "F2",
            FontDict::standard14("Courier", SimpleEncoding::Builtin),
        )
        .add_form_xobject("Fm", [0.0, 0.0, 100.0, 100.0], "BT /F2 9 Tf (x) Tj ET")
        .set_form_ocg("Fm", &ocg)
        .set_form_ocg("Missing", &ocg)
        .add_annotation(Annotation::new("Stamp", [0.0, 0.0, 50.0, 20.0]).with_appearance("0 g"))
        .raw_content("/Fm Do");
    b.add_page_a4();
    b
}

#[test]
fn output_is_deterministic() {
    let b = feature_doc();
    assert_eq!(b.to_bytes(), b.to_bytes());
    assert_eq!(b.to_bytes(), feature_doc().to_bytes());
    let dir = std::env::temp_dir().join(format!("testkit-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("doc.pdf");
    b.save(&path).expect("save");
    assert_eq!(std::fs::read(&path).expect("read back"), b.to_bytes());
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn page_mut_reaches_an_earlier_page() {
    // The Python kit edits pages by index; that must equal editing through
    // the `&mut PageBuilder` that `add_page` returned.
    let mut direct = PdfBuilder::new();
    direct
        .add_page_a4()
        .insert_text((72.0, 72.0), "first", TextStyle::default())
        .expect("ASCII");
    direct
        .add_page(612.0, 792.0)
        .insert_text((72.0, 72.0), "second", TextStyle::default())
        .expect("ASCII");

    let mut indexed = PdfBuilder::new();
    indexed.add_page_a4();
    indexed.add_page(612.0, 792.0);
    for (i, text) in [(1, "second"), (0, "first")] {
        indexed
            .page_mut(i)
            .expect("page exists")
            .insert_text((72.0, 72.0), text, TextStyle::default())
            .expect("ASCII");
    }
    assert_eq!(indexed.to_bytes(), direct.to_bytes());
    assert!(indexed.page_mut(2).is_none());
}

#[test]
fn page_features_reach_the_page_dictionary() {
    let b = feature_doc();
    assert_eq!(b.page_count(), 2);
    let pdf = Pdf::new(b.to_bytes()).expect("opens");
    let pages = pdf.pages();
    assert_eq!(pages.len(), 2);
    let p = &pages[0];
    let mb = p.media_box();
    assert_eq!((mb.x0, mb.y0, mb.x1, mb.y1), (0.0, 0.0, 400.0, 300.0));
    let cb = p.crop_box();
    assert_eq!((cb.x0, cb.y0, cb.x1, cb.y1), (10.0, 20.0, 390.0, 280.0));
    assert!(matches!(p.rotation(), Rotation::Horizontal));
    let res = p.resources();
    assert!(
        res.get_font(&Name::new_unescaped(b"Helv")).is_some(),
        "default font"
    );
    assert!(res.get_font(&Name::new_unescaped(b"F2")).is_some());
    let form = res
        .get_x_object(&Name::new_unescaped(b"Fm"))
        .expect("form resource");
    assert!(
        form.dict().get::<Dict<'_>>(b"OC".as_slice()).is_some(),
        "/OC on form"
    );
    let annots = p
        .raw()
        .get::<hayro_syntax::object::Array<'_>>(b"Annots".as_slice());
    assert_eq!(annots.map(|a| a.iter::<Dict<'_>>().count()), Some(1));
    // A4 default on the second page; no crop box means crop = media.
    let mb2 = pages[1].media_box();
    assert_eq!((mb2.x1, mb2.y1), (595.0, 842.0));
}

#[test]
fn ocgs_are_registered_in_the_catalog_default_configuration() {
    let mut b = PdfBuilder::new();
    let p = b.add_page_a4();
    let on = p.add_ocg("On", true);
    let off = p.add_ocg("Off", false);
    assert_eq!((on.name(), off.name()), ("On", "Off"));
    let bytes = b.to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/OCProperties << /OCGs ["), "{text}");
    let on_id = text
        .split(" 0 obj\n<< /Type /OCG /Name (On) >>")
        .next()
        .and_then(|s| s.rsplit('\n').next())
        .expect("On OCG object");
    let off_id = text
        .split(" 0 obj\n<< /Type /OCG /Name (Off) >>")
        .next()
        .and_then(|s| s.rsplit('\n').next())
        .expect("Off OCG object");
    assert!(text.contains(&format!("/ON [{on_id} 0 R] /OFF [{off_id} 0 R]")));
    assert!(text.contains(&format!(
        "/Properties << /On {on_id} 0 R /Off {off_id} 0 R >>"
    )));
}

#[test]
fn adding_default_font_name_replaces_helvetica() {
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_font(
            "Helv",
            FontDict::standard14("Times-Roman", SimpleEncoding::Builtin),
        )
        .add_font(
            "Helv",
            FontDict::standard14("Courier", SimpleEncoding::Builtin),
        );
    let bytes = b.to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("/BaseFont /Helvetica"));
    assert!(
        !text.contains("/BaseFont /Times-Roman"),
        "later add_font wins"
    );
    assert!(text.contains("/BaseFont /Courier"));
}

#[test]
fn annotations_write_flags_states_matrix_and_contents() {
    use pdf_extract_testkit::{Appearance, AppearanceStream};
    let mut b = PdfBuilder::new();
    b.add_page_a4()
        .add_annotation(
            Annotation::new("FreeText", [0.0, 0.0, 10.0, 10.0])
                .with_flags(Annotation::FLAG_HIDDEN | Annotation::FLAG_NO_VIEW)
                .with_contents("Hé", "/Helv 11 Tf 0 g"),
        )
        .add_annotation(
            Annotation::new("Widget", [0.0, 0.0, 10.0, 20.0]).with_appearance_spec(
                Appearance::States {
                    states: vec![
                        ("On".into(), AppearanceStream::new("0 g")),
                        ("Off".into(), AppearanceStream::new("")),
                    ],
                    current: "On".into(),
                },
            ),
        )
        .add_annotation(
            Annotation::new("Stamp", [0.0, 0.0, 10.0, 10.0]).with_appearance_spec(
                Appearance::Stream(AppearanceStream {
                    content: "0 g".into(),
                    bbox: Some([0.0, 0.0, 20.0, 20.0]),
                    matrix: Some([0.5, 0.0, 0.0, 0.5, 0.0, 0.0]),
                }),
            ),
        );
    let bytes = b.to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("/F 34 /Contents <FEFF004800E9> /DA (/Helv 11 Tf 0 g) >>"),
        "{text}"
    );
    assert!(text.contains("/AS /On"));
    assert!(
        text.contains("/BBox [0 0 10 20]"),
        "default bbox is the rect size"
    );
    assert!(text.contains("/BBox [0 0 20 20] /Resources"));
    assert!(text.contains("/Matrix [0.5 0 0 0.5 0 0]"));
    assert!(Pdf::new(bytes.clone()).is_ok());
}

#[test]
fn overflowing_or_non_finite_numbers_never_reach_the_file() {
    let mut b = PdfBuilder::new();
    let p = b.add_page(595.0, f32::MAX);
    assert_eq!(
        p.insert_text((0.0, -f32::MAX), "A", TextStyle::default()),
        Err(BuildError::NonFinite),
        "computed baseline overflows"
    );
    assert_eq!(text_width("A", f32::NAN), Err(BuildError::NonFinite));
    let mut nan_page = PdfBuilder::new();
    nan_page.add_page(f32::NAN, 842.0);
    let panicked = std::panic::catch_unwind(|| nan_page.to_bytes()).is_err();
    assert!(
        panicked,
        "a NaN MediaBox is a test-authoring error, not a file"
    );
}

#[test]
#[should_panic(expected = "does not fit")]
fn to_unicode_rejects_codes_wider_than_the_codespace() {
    let _ = pdf_extract_testkit::to_unicode_cmap(1, &[(0x1234, "x")]);
}

#[test]
#[should_panic(expected = "given twice")]
fn type3_duplicate_codes_are_rejected() {
    use pdf_extract_testkit::{Type3Font, Type3Glyph};
    let g = |name: &str| Type3Glyph {
        code: 65,
        name: name.into(),
        width: 500.0,
        procedure: "500 0 d0".into(),
    };
    let mut b = PdfBuilder::new();
    b.add_page_a4().add_font(
        "T3",
        FontDict::Type3(Type3Font {
            font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
            font_bbox: [0.0, 0.0, 500.0, 500.0],
            glyphs: vec![g("a"), g("b")],
            to_unicode: None,
        }),
    );
    let _ = b.to_bytes();
}
