//! Image decoding through the renderer (spec 0002 §4.7 "Images", O4): one
//! image per codec hayro supports (DCT, JPEG 2000, JBIG2, CCITT), each a
//! minimal stream embedded here, renders without panic and darkens the area
//! it covers; an image over `max_image_pixels` is skipped with one
//! `image: pixel limit` warning and never also `image: decode failed`; a
//! broken image is one `image: decode failed`; a JBIG2 or JPEG 2000 image
//! whose decoder would exceed its memory budget (run 0003 H14, H15) is one
//! `image: memory limit`.

mod common;

use common::codecs::{SMALL_GRAY, codec_images};
use common::{dark_in, images_pdf, kinds, options, render_with};
use pdf_extract_core::{Limits, WarningKind};
use pdf_extract_render::Rendered;
use pdf_extract_testkit::{jbig2, jpx};

fn render(bytes: Vec<u8>, limits: Limits) -> Rendered {
    render_with(bytes, limits, &options(72)).expect("renders")
}

#[test]
fn every_codec_draws_its_image() {
    let bytes = codec_images();
    let r = render(bytes, Limits::default());
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    // 200 × 200 pt at 72 dpi; image i fills the 100 px square at column
    // i % 2, row i / 2.
    for (i, name) in ["DCT", "JPX", "JBIG2", "CCITT"].into_iter().enumerate() {
        let (x, y) = (
            u32::try_from(i % 2).unwrap() * 100,
            u32::try_from(i / 2).unwrap() * 100,
        );
        let dark = dark_in(&r.image, x + 2, y + 2, x + 98, y + 98);
        assert!(dark > 0, "{name}: no image pixels");
        if i >= 2 {
            // The bilevel images are solid black.
            assert_eq!(dark, 96 * 96, "{name}: solid black");
        }
    }
}

/// A 64 × 64 flate image declaring 20000 × 20000 (H13's declared case).
fn declared_huge() -> (String, Vec<u8>) {
    use std::io::Write as _;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&[0u8; 64 * 64]).expect("compress");
    (
        "/Width 20000 /Height 20000 /ColorSpace /DeviceGray /BitsPerComponent 8 \
         /Filter /FlateDecode"
            .to_owned(),
        e.finish().expect("compress"),
    )
}

#[test]
fn an_image_over_the_pixel_cap_is_one_pixel_limit_warning() {
    let (dict, data) = declared_huge();
    let limits = Limits {
        max_image_pixels: 1_000_000,
        ..Limits::default()
    };
    let r = render(images_pdf(&[(&dict, data)]), limits);
    assert_eq!(kinds(&r), [WarningKind::ImagePixelLimit]);
    assert_eq!(r.warnings[0].count, 1);
    assert!(r.warnings[0].to_string().starts_with("image: pixel limit"));
    // Skipped, not drawn.
    assert_eq!(dark_in(&r.image, 0, 0, 100, 100), 0);
}

#[test]
fn a_codec_header_over_the_cap_is_one_pixel_limit_warning() {
    // CCITT whose /DecodeParms say 20000 × 20000 under a 64 × 64 dictionary
    // (H13's codec-header case).
    let dict = "/Width 64 /Height 64 /ColorSpace /DeviceGray /BitsPerComponent 1 \
                /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 20000 /Rows 20000 >>";
    let limits = Limits {
        max_image_pixels: 1_000_000,
        ..Limits::default()
    };
    let r = render(images_pdf(&[(dict, vec![0; 16])]), limits);
    assert_eq!(kinds(&r), [WarningKind::ImagePixelLimit]);
}

#[test]
fn a_broken_image_is_one_decode_failure() {
    let r = render(
        images_pdf(&[(&format!("{SMALL_GRAY} /Filter /DCTDecode"), vec![0; 32])]),
        Limits::default(),
    );
    assert_eq!(kinds(&r), [WarningKind::ImageDecodeFailure]);
    assert!(
        r.warnings[0]
            .to_string()
            .starts_with("image: decode failed")
    );
}

#[test]
fn an_inline_image_over_the_cap_is_one_pixel_limit_warning() {
    // H13 holds inline images to the document's cap too.
    let content = "q 100 0 0 100 0 0 cm BI /W 20000 /H 20000 /CS /G /BPC 8 ID \0\0\0\0 EI Q";
    let bytes = common::raw_pdf(&["<< >>".to_owned(), stream(content)]);
    let limits = Limits {
        max_image_pixels: 1_000_000,
        ..Limits::default()
    };
    let r = render(bytes, limits);
    assert_eq!(kinds(&r), [WarningKind::ImagePixelLimit]);
}

#[test]
fn a_jbig2_image_over_its_memory_budget_is_one_memory_limit_warning() {
    // An 8192 × 8192 symbol exported 65,535 times, drawn as a 64 × 64 image
    // over the whole 200 pt page: skipped, the page otherwise rendered.
    let limits = Limits {
        max_image_pixels: 1_000_000,
        ..Limits::default()
    };
    let r = render(jbig2::symbol_export_bomb_pdf(), limits);
    assert_eq!(kinds(&r), [WarningKind::ImageMemoryLimit]);
    assert_eq!(r.warnings[0].count, 1);
    assert_eq!(r.warnings[0].page, Some(0));
    assert!(
        r.warnings[0].to_string().starts_with("image: memory limit"),
        "{}",
        r.warnings[0]
    );
    assert_eq!(dark_in(&r.image, 0, 0, 200, 200), 0);
    // The default cap (256 MP, a 512 MB budget) refuses it too.
    let r = render(jbig2::symbol_export_bomb_pdf(), Limits::default());
    assert_eq!(kinds(&r), [WarningKind::ImageMemoryLimit]);
}

#[test]
fn a_jbig2_image_within_its_memory_budget_draws() {
    // The exported-copies control: a 192 × 192 image whose one black pixel
    // covers about one rendered pixel of the 200 pt page at 72 dpi.
    let limits = Limits {
        max_image_pixels: 1_000_000,
        ..Limits::default()
    };
    let control = jbig2::image_pdf(&jbig2::symbol_export(64, 4, 192, true), 192, 192);
    let r = render(control, limits);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert!(dark_in(&r.image, 0, 0, 200, 200) > 0, "the image was drawn");
}

#[test]
fn a_jpx_image_over_its_memory_budget_is_one_memory_limit_warning() {
    // A 236 × 236 image on a 1 × 1-tiled reference grid of 3.6 billion
    // tiles, drawn over the whole 200 pt page: skipped from its header, the
    // page otherwise rendered, under a small cap and the default alike.
    for limits in [
        Limits {
            max_image_pixels: 1_000_000,
            ..Limits::default()
        },
        Limits::default(),
    ] {
        let r = render(jpx::tile_bomb_pdf(), limits);
        assert_eq!(kinds(&r), [WarningKind::ImageMemoryLimit]);
        assert_eq!(r.warnings[0].count, 1);
        assert_eq!(r.warnings[0].page, Some(0));
        assert!(
            r.warnings[0].to_string().starts_with("image: memory limit"),
            "{}",
            r.warnings[0]
        );
        assert_eq!(dark_in(&r.image, 0, 0, 200, 200), 0);
    }
}

#[test]
fn a_jpx_image_within_its_memory_budget_draws() {
    // The 64 × 64 gray codestream over the whole page, under a cap whose
    // budget (64 MB) it needs a thousandth of.
    let limits = Limits {
        max_image_pixels: 1_000_000,
        ..Limits::default()
    };
    let r = render(jpx::image_pdf(&jpx::gray_64(), 64, 64), limits);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert!(dark_in(&r.image, 0, 0, 200, 200) > 0, "the image was drawn");
}

fn stream(data: &str) -> String {
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len())
}
