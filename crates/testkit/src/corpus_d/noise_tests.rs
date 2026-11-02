//! Unit tests of the corpus-D noise builder (spec 0002 §4.12, "Noise
//! variants"): the pinned deviates, the field's moments, the structure of a
//! built file, determinism, and the inputs it must refuse.

use hayro_syntax::Pdf;
use hayro_syntax::object::{Name, Stream};

use super::*;
use crate::pdfwriter::{Objects, stream_body};

fn clean(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/corpus-d")
        .join(format!("{name}.pdf"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_first_eight_deviates_are_pinned() {
    assert_eq!(deviates(NOISE_SEED, 8), [5, -21, 10, -16, -18, -7, -17, 6]);
}

#[test]
fn a_field_of_128s_has_mean_zero_and_sigma_twelve() {
    let mut field = vec![128u8; 2480 * 3509];
    add_noise(&mut field, NOISE_SEED);
    #[allow(clippy::cast_precision_loss)]
    let n = field.len() as f64;
    let mean = field.iter().map(|&p| f64::from(p) - 128.0).sum::<f64>() / n;
    let var = field
        .iter()
        .map(|&p| (f64::from(p) - 128.0 - mean).powi(2))
        .sum::<f64>()
        / n;
    assert!(mean.abs() < 0.05, "mean {mean}");
    assert!((var.sqrt() - 12.0).abs() < 0.05, "sigma {}", var.sqrt());
    // No clamping at 128 ± 72: every deviate is visible.
    assert!(field.iter().all(|&p| (56..=200).contains(&p)));
}

#[test]
fn the_same_seed_gives_the_same_field_and_another_seed_does_not() {
    let (mut a, mut b, mut c) = (vec![200u8; 4096], vec![200u8; 4096], vec![200u8; 4096]);
    add_noise(&mut a, NOISE_SEED);
    add_noise(&mut b, NOISE_SEED);
    add_noise(&mut c, NOISE_SEED + 1);
    assert_eq!(a, b);
    assert_ne!(a, c);
    // Clamping at the top: 255 never wraps.
    let mut white = vec![255u8; 4096];
    add_noise(&mut white, NOISE_SEED);
    assert!(white.contains(&255) && white.iter().any(|&p| p < 255));
}

/// The one image of `pdf`'s single page: (stream bytes as decoded, its
/// dictionary's width, height, has-filter flag).
fn image_facts(pdf: &Pdf) -> (u32, u32, bool, Vec<u8>) {
    let page = &pdf.pages()[0];
    let xobjects = &page.resources().x_objects;
    assert_eq!(xobjects.len(), 1);
    let key = xobjects.keys().next().expect("one");
    let image: Stream<'_> = xobjects.get(key.as_ref()).expect("stream");
    let dict = image.dict();
    assert!(is_gray(dict));
    assert_eq!(dict.get::<u32>("BitsPerComponent"), Some(8));
    (
        dict.get::<u32>("Width").expect("w"),
        dict.get::<u32>("Height").expect("h"),
        dict.get::<Name<'_>>("Filter").is_some(),
        image.decoded().expect("decodes").into_owned(),
    )
}

#[test]
fn a_variant_keeps_the_clean_page_geometry_and_is_unfiltered() {
    for name in ["clean_body_11pt", "clean_rotate90", "clean_cropbox"] {
        let src = clean(name);
        let out = noise_variant(&src).expect("builds");
        assert!(out.starts_with(b"%PDF-1.4\n"), "{name}");
        let text = String::from_utf8_lossy(&out);
        assert!(
            text.contains("/ColorSpace /DeviceGray /BitsPerComponent 8"),
            "{name}"
        );
        for absent in ["/ID", "/Info", "/CreationDate", "/ModDate", "/Filter"] {
            assert!(!text.contains(absent), "{name}: {absent}");
        }
        let (a, b) = (Pdf::new(src).expect("clean"), Pdf::new(out).expect("built"));
        assert_eq!(b.pages().len(), 1);
        let (pa, pb) = (&a.pages()[0], &b.pages()[0]);
        assert_eq!(
            spell(rect(pa.media_box())),
            spell(rect(pb.media_box())),
            "{name}"
        );
        assert_eq!(
            spell(rect(pa.crop_box())),
            spell(rect(pb.crop_box())),
            "{name}"
        );
        assert_eq!(degrees(pa.rotation()), degrees(pb.rotation()), "{name}");
        let (wa, ha, _, pixels_a) = image_facts(&a);
        let (wb, hb, filtered, pixels_b) = image_facts(&b);
        assert_eq!((wa, ha), (wb, hb), "{name}");
        assert!(!filtered, "{name}");
        let mut expected = pixels_a;
        add_noise(&mut expected, NOISE_SEED);
        assert_eq!(
            expected, pixels_b,
            "{name}: pixels are the clean ones plus noise"
        );
    }
    // The two pages with special geometry really have it.
    let rot = Pdf::new(noise_variant(&clean("clean_rotate90")).expect("rot")).expect("pdf");
    assert_eq!(degrees(rot.pages()[0].rotation()), 90);
    let crop = Pdf::new(noise_variant(&clean("clean_cropbox")).expect("crop")).expect("pdf");
    let c = crop.pages()[0].crop_box();
    assert_eq!((c.y0, c.y1), (25.0, 817.0));
}

#[test]
fn building_twice_gives_identical_bytes() {
    let src = clean("sparse_below_min_chars");
    assert_eq!(
        noise_variant(&src).expect("a"),
        noise_variant(&src).expect("b")
    );
}

/// A one-page file with the given image dictionaries (entries without
/// `/Length`) and samples, drawn with `content`.
fn image_page(images: &[(&str, Vec<u8>)], content: &str, fonts: &str) -> Vec<u8> {
    let mut objs = Objects::new();
    let (catalog, pages, page) = (objs.alloc(), objs.alloc(), objs.alloc());
    let mut xobjects = String::new();
    for (i, (dict, data)) in images.iter().enumerate() {
        let id = objs.add(stream_body(dict, data));
        xobjects.push_str(&format!("/Im{i} {id} 0 R "));
    }
    let contents = objs.add(stream_body("", content.as_bytes()));
    objs.set(catalog, format!("<< /Type /Catalog /Pages {pages} 0 R >>"));
    objs.set(
        pages,
        format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"),
    );
    objs.set(
        page,
        format!(
            "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 100 100] /Resources << \
             /XObject << {xobjects}>> {fonts} >> /Contents {contents} 0 R >>"
        ),
    );
    objs.to_bytes(catalog, "")
}

const GRAY8: &str =
    "/Type /XObject /Subtype /Image /Width 4 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8";

#[test]
fn a_well_formed_hand_made_scan_is_accepted() {
    let pdf = image_page(
        &[(GRAY8, vec![255; 8])],
        "q 100 0 0 100 0 0 cm /Im0 Do Q",
        "",
    );
    noise_variant(&pdf).expect("a one-image gray page is a scan");
}

#[test]
fn pages_that_are_not_scans_are_refused() {
    let draw = "q 100 0 0 100 0 0 cm /Im0 Do Q";
    let text = image_page(
        &[(GRAY8, vec![255; 8])],
        "BT /F1 12 Tf (x) Tj ET",
        "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>",
    );
    let two = image_page(&[(GRAY8, vec![255; 8]), (GRAY8, vec![0; 8])], draw, "");
    let one_bit = image_page(
        &[(
            "/Type /XObject /Subtype /Image /Width 8 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 1",
            vec![0xFF],
        )],
        draw,
        "",
    );
    let rgb = image_page(
        &[(
            "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8",
            vec![0, 0, 0],
        )],
        draw,
        "",
    );
    let short = image_page(&[(GRAY8, vec![255; 7])], draw, "");
    let mut builder = crate::PdfBuilder::new();
    builder
        .add_page_a4()
        .insert_text((72.0, 72.0), "text", crate::TextStyle::default())
        .expect("text");
    for (what, pdf) in [
        ("text layer", text),
        ("two images", two),
        ("1-bit image", one_bit),
        ("RGB image", rgb),
        ("short samples", short),
        ("builder text page", builder.to_bytes()),
        ("not a PDF", b"hello".to_vec()),
    ] {
        let err = noise_variant(&pdf).expect_err(what);
        assert!(matches!(err, BuildError::CorpusD(_)), "{what}: {err:?}");
        assert!(err.to_string().starts_with("corpus D: "), "{what}: {err}");
    }
}

#[test]
fn names_follow_the_catalogue() {
    let names = noise_names();
    assert_eq!(names.len(), 12);
    assert_eq!(names[0], "clean_body_09pt_noise");
    assert_eq!(names[11], "sparse_below_min_chars_noise");
}

#[test]
fn the_build_dir_is_under_target_and_never_under_fixtures() {
    let dir = default_build_dir();
    assert!(dir.ends_with("corpus-d"));
    assert!(!dir.to_string_lossy().contains("tests/fixtures"));
}

#[test]
fn a_missing_clean_file_is_an_error_not_a_panic() {
    let tmp = std::env::temp_dir().join(format!("pxt-noise-missing-{}", std::process::id()));
    let err = build_noise(&tmp.join("no-fixtures"), &tmp.join("out"), &[]).expect_err("missing");
    assert!(matches!(err, BuildError::CorpusD(_)), "{err:?}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn the_pure_noise_page_is_the_field_on_a4() {
    let page = pure_noise_page();
    assert_eq!((page.width, page.height), (2480, 3509));
    assert_eq!(page.pixels, noise_field(2480, 3509));
    let mut white = vec![255u8; 2480 * 3509];
    add_noise(&mut white, NOISE_SEED);
    assert_eq!(page.pixels, white);
    let pdf = Pdf::new(scan_pdf(&[page])).expect("parses");
    assert_eq!(pdf.pages().len(), 1);
    let (w, h, filtered, pixels) = image_facts(&pdf);
    assert_eq!((w, h), (2480, 3509));
    assert!(!filtered);
    assert_eq!(pixels, white);
    let media = pdf.pages()[0].media_box();
    assert_eq!((media.x1, media.y1), (595.0, 842.0));
}

#[test]
fn scan_pdf_assembles_pages_in_order_and_a_variant_page_round_trips() {
    let small = |value: u8, rotate: u32, crop: Option<[f32; 4]>| ScanPage {
        media_box: [0.0, 0.0, 100.0, 50.0],
        crop_box: crop,
        rotate,
        width: 4,
        height: 2,
        pixels: vec![value; 8],
    };
    let variant = noise_variant_page(&clean("clean_cropbox")).expect("variant page");
    let pages = vec![
        small(0, 0, None),
        small(128, 90, Some([0.0, 10.0, 100.0, 40.0])),
        variant.clone(),
    ];
    let bytes = scan_pdf(&pages);
    assert_eq!(bytes, scan_pdf(&pages), "deterministic");
    let pdf = Pdf::new(bytes).expect("parses");
    let parsed = pdf.pages();
    assert_eq!(parsed.len(), 3);
    assert_eq!(degrees(parsed[1].rotation()), 90);
    assert_eq!(parsed[1].crop_box().y0, 10.0);
    assert_eq!(degrees(parsed[0].rotation()), 0);
    // The one-page document of a variant page is the variant itself.
    assert_eq!(
        scan_pdf(&[variant]),
        noise_variant(&clean("clean_cropbox")).expect("variant")
    );
}
