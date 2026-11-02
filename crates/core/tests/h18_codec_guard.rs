//! H18: the image-codec panic boundary in the vendored hayro fork (internal #76).
//!
//! `hayro-syntax` runs each call into an image codec crate inside
//! `catch_unwind` and turns a panic into `DecodeFailure::ImageDecode`; only the
//! codec call is guarded, so a panic elsewhere on the image path propagates.
//! Fuzzing builds (`--cfg fuzzing`) compile the catch out, so there a codec
//! panic propagates too. The panics come from the fork's test-only hook
//! (feature `codec-panic-hook`, on through this crate's dev-dependency): no
//! known input panics a codec reliably.
//!
//! The image is the 8 × 8 all-black CCITT Group 4 bitmap of the render tests;
//! `crates/render/tests/codec_panic.rs` covers all four codecs end to end.

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};

use common::assemble_pdf;
use hayro_syntax::object::stream::{DecodeFailure, ImageDecodeParams};
use hayro_syntax::object::{ObjectIdentifier, Stream};
use hayro_syntax::{
    CODEC_PANICS_CAUGHT, DecodeLimits, Pdf, arm_codec_panic_for_test,
    arm_unguarded_image_panic_for_test,
};

/// An 8 × 8 all-black bitmap in T.6 (Group 4) coding; see
/// `crates/render/tests/common/codecs.rs`.
const MMR_BLACK_8X8: [u8; 4] = [0x26, 0xA2, 0xFF, 0xFE];

/// A PDF whose object 4 is the CCITT image (no page needs to draw it: the
/// tests decode the stream directly).
fn ccitt_pdf() -> Pdf {
    let mut image = b"<< /Type /XObject /Subtype /Image /Width 8 /Height 8 \
        /ColorSpace /DeviceGray /BitsPerComponent 1 /Filter /CCITTFaxDecode \
        /DecodeParms << /K -1 /Columns 8 /Rows 8 >> /Length 4 >>\nstream\n"
        .to_vec();
    image.extend_from_slice(&MMR_BLACK_8X8);
    image.extend_from_slice(b"\nendstream");
    let bytes = assemble_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 8 8] >>".to_vec(),
        image,
    ]);
    Pdf::new_with_options(bytes, "", DecodeLimits::default()).expect("opens")
}

/// Decodes the image, returning its decoded length.
fn decode(pdf: &Pdf) -> Result<usize, DecodeFailure> {
    let stream = pdf
        .xref()
        .get::<Stream<'_>>(ObjectIdentifier::new(4, 0))
        .expect("image stream");
    let params = ImageDecodeParams {
        bpc: Some(1),
        num_components: Some(1),
        width: 8,
        height: 8,
        ..ImageDecodeParams::default()
    };
    stream.decoded_image(&params).map(|r| r.data.len())
}

#[test]
fn the_guard_is_compiled_in_except_for_fuzzing() {
    // The cfg assertion behind "fuzzing sees codec panics": run with
    // `RUSTFLAGS="--cfg fuzzing"` it checks the guard is compiled out.
    assert_eq!(CODEC_PANICS_CAUGHT, !cfg!(fuzzing));
}

#[test]
fn an_armed_codec_panic_fails_one_decode_and_clears() {
    let pdf = ccitt_pdf();
    assert!(decode(&pdf).is_ok(), "the fixture decodes unarmed");
    arm_codec_panic_for_test();
    let armed = catch_unwind(AssertUnwindSafe(|| decode(&pdf)));
    if cfg!(fuzzing) {
        assert!(armed.is_err(), "a fuzzing build lets the codec panic out");
    } else {
        let result = armed.expect("the codec guard catches the panic");
        assert!(
            matches!(result, Err(DecodeFailure::ImageDecode)),
            "{result:?}"
        );
    }
    // The flag cleared when it fired: the next decode is unaffected.
    assert!(matches!(decode(&pdf), Ok(64)));
}

#[test]
fn the_hook_is_armed_per_thread() {
    let pdf = ccitt_pdf();
    arm_codec_panic_for_test();
    let other = std::thread::scope(|s| s.spawn(|| decode(&pdf)).join().expect("joins"));
    assert!(
        matches!(other, Ok(64)),
        "another thread's decode is not armed"
    );
    let here = catch_unwind(AssertUnwindSafe(|| decode(&pdf)));
    assert!(!matches!(here, Ok(Ok(_))), "this thread's decode still is");
}

#[test]
fn a_panic_outside_the_codec_call_propagates() {
    // Armed in the codec module after the parameters are resolved, just
    // before the guarded call: on the image path, but outside the guard, so
    // the guard must not catch it (nor would a guard widened to the module).
    let pdf = ccitt_pdf();
    arm_unguarded_image_panic_for_test();
    let result = catch_unwind(AssertUnwindSafe(|| decode(&pdf)));
    let payload = result.expect_err("the panic propagates");
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"image-path panic injected for testing")
    );
    assert!(matches!(decode(&pdf), Ok(64)));
}
