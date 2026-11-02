//! A panicking image codec skips its image (internal #76; error-model "An image codec
//! panic skips the image"): with the fork's test-only hook armed (H18,
//! feature `codec-panic-hook`, on through this crate's dev-dependency), the
//! next guarded codec call panics inside the codec boundary. The render
//! returns, the image draws nothing, the page carries exactly one
//! `image: decode failed` warning, and the rest of the page is drawn. A panic
//! on the image path outside the codec call is not caught.
//!
//! Each page draws the same codec image twice, side by side: the hook fires
//! on the first decode only, so the second image shows that the page went on
//! (and that the hook cleared).

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};

use common::codecs::{
    BILEVEL_8, J2K_64, JPEG_64, MMR_BLACK_8X8, SMALL_GRAY, jbig2_black_8x8, unhex,
};
use common::{dark_in, images_pdf, kinds, options, render_with};
use hayro_syntax::{arm_codec_panic_for_test, arm_unguarded_image_panic_for_test};
use pdf_extract_core::{Limits, WarningKind};

/// The four codecs' minimal images, as `(name, dict, data)`.
fn codec_images() -> Vec<(&'static str, String, Vec<u8>)> {
    vec![
        (
            "DCT",
            format!("{SMALL_GRAY} /Filter /DCTDecode"),
            unhex(&JPEG_64),
        ),
        (
            "JPX",
            format!("{SMALL_GRAY} /Filter /JPXDecode"),
            unhex(&J2K_64),
        ),
        (
            "JBIG2",
            format!("{BILEVEL_8} /Filter /JBIG2Decode"),
            jbig2_black_8x8(),
        ),
        (
            "CCITT",
            format!(
                "{BILEVEL_8} /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8 /Rows 8 >>"
            ),
            MMR_BLACK_8X8.to_vec(),
        ),
    ]
}

/// A page drawing `dict`/`data` twice: image 0 in the top-left 100 px
/// square at 72 dpi, image 1 in the top-right one.
fn twice(dict: &str, data: &[u8]) -> Vec<u8> {
    images_pdf(&[(dict, data.to_vec()), (dict, data.to_vec())])
}

#[test]
fn a_codec_panic_skips_the_image_with_one_decode_failure() {
    for (name, dict, data) in codec_images() {
        arm_codec_panic_for_test();
        let r = render_with(twice(&dict, &data), Limits::default(), &options(72))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(kinds(&r), [WarningKind::ImageDecodeFailure], "{name}");
        assert_eq!(r.warnings[0].count, 1, "{name}");
        assert!(
            r.warnings[0]
                .to_string()
                .starts_with("image: decode failed"),
            "{name}: {}",
            r.warnings[0]
        );
        assert_eq!(dark_in(&r.image, 2, 2, 98, 98), 0, "{name}: skipped");
        assert!(
            dark_in(&r.image, 102, 2, 198, 98) > 0,
            "{name}: the rest of the page is drawn"
        );
    }
}

#[test]
fn without_the_hook_both_images_draw() {
    // The control: the fixtures decode, so the failure above is the hook's.
    for (name, dict, data) in codec_images() {
        let r = render_with(twice(&dict, &data), Limits::default(), &options(72))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(r.warnings.is_empty(), "{name}: {:?}", r.warnings);
        assert!(dark_in(&r.image, 2, 2, 98, 98) > 0, "{name}");
        assert!(dark_in(&r.image, 102, 2, 198, 98) > 0, "{name}");
    }
}

#[test]
fn a_panic_on_the_image_path_outside_the_codec_propagates() {
    for (name, dict, data) in codec_images() {
        arm_unguarded_image_panic_for_test();
        let bytes = twice(&dict, &data);
        let result = catch_unwind(AssertUnwindSafe(|| {
            render_with(bytes, Limits::default(), &options(72))
        }));
        let payload = result.expect_err(name);
        assert_eq!(
            payload.downcast_ref::<&str>(),
            Some(&"image-path panic injected for testing"),
            "{name}"
        );
    }
}
