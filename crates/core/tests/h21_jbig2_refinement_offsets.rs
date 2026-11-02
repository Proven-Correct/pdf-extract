//! H21: a JBIG2 refinement whose reference sits right of the new bitmap
//! (positive GRREFERENCEDX) decodes exactly (internal #115).
//!
//! The refinement decoder buffers 32-pixel words of the three reference rows
//! around the current pixel. While the reference column under the pixel was
//! still left of the reference (x − dx < 0), it zeroed those words, which
//! `start_row` had loaded at column 0, and its reload test then stayed false
//! until the column reached 31, so reference columns 0–30 (and the template's
//! right-hand pixel one column earlier) read as white. A positive offset
//! therefore decoded to other pixels whatever the encoder wrote; a negative
//! offset, either vertical offset and the right edge were unaffected. The
//! fork now keeps the column-0 words while the column is negative.
//!
//! The streams come from `pdf_extract_testkit::jbig2::refined_from`, whose
//! refinement data is written by the test kit's MQ encoder (template 1, no
//! typical prediction), so a decode that differs from the target is the
//! decoder's fault; the MQ round trip in `h14_jbig2_budget.rs` checks the
//! encoder at the offsets that were already decoded correctly.

use hayro_jbig2::{Decoder, DecoderContext};
use pdf_extract_testkit::jbig2;

/// The page as the decoder emits it: one byte (0 white, 1 black) per pixel,
/// row by row.
#[derive(Default)]
struct Pixels {
    pixels: Vec<u8>,
    rows: usize,
}

impl Decoder for Pixels {
    fn push_pixel(&mut self, black: bool) {
        self.pixels.push(u8::from(black));
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        let n = chunk_count as usize * 8;
        self.pixels.resize(self.pixels.len() + n, u8::from(black));
    }

    fn next_line(&mut self) {
        self.rows += 1;
    }
}

/// `count` pseudo-random rows of `width` pixels (`#` black), about half
/// black, from a fixed seed, so every reference pixel of the template
/// varies. The same generator as `h14_jbig2_budget.rs`.
fn noise(width: usize, count: usize, seed: u64) -> Vec<String> {
    let mut state = seed;
    (0..count)
        .map(|_| {
            (0..width)
                .map(|_| {
                    // Knuth's MMIX LCG; the top bit is the pixel.
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    if state >> 63 == 1 { '#' } else { '.' }
                })
                .collect()
        })
        .collect()
}

/// Decodes `target` refined from `reference` at `offset` and returns the
/// pixels that differ from `target` as drawn at (3, 5), where `refined_from`
/// places the new symbol, as (x, y) page coordinates.
fn mismatches(reference: &[String], target: &[String], offset: (i32, i32)) -> Vec<(usize, usize)> {
    let reference: Vec<&str> = reference.iter().map(String::as_str).collect();
    let target: Vec<&str> = target.iter().map(String::as_str).collect();
    let stream = jbig2::refined_from(&reference, &target, offset);
    let image = hayro_jbig2::Image::new_embedded(&stream, None).expect("parses");
    let mut pixels = Pixels::default();
    image
        .decode_with(&mut pixels, &mut DecoderContext::default())
        .expect("decodes");
    let width = pixels.pixels.len() / pixels.rows;
    let want = |x: usize, y: usize| {
        let (Some(tx), Some(ty)) = (x.checked_sub(3), y.checked_sub(5)) else {
            return false;
        };
        target
            .get(ty)
            .and_then(|row| row.as_bytes().get(tx))
            .is_some_and(|&c| c == b'#')
    };
    (0..pixels.pixels.len())
        .map(|i| (i % width, i / width))
        .filter(|&(x, y)| (pixels.pixels[y * width + x] == 1) != want(x, y))
        .collect()
}

/// Asserts that each offset decodes exactly, for a 40 × 40 target against a
/// 37 × 35 reference (a width that pads its stored rows) and for a 70 × 6
/// target against a 66 × 5 reference, whose rows cross two word boundaries
/// so the reload after column 31 is reached too.
fn assert_exact(offsets: &[(i32, i32)]) {
    let shapes = [
        (noise(37, 35, 8), noise(40, 40, 7)),
        (noise(66, 5, 21), noise(70, 6, 22)),
    ];
    let mut failures = Vec::new();
    for (reference, target) in &shapes {
        let size = (target[0].len(), target.len());
        for &offset in offsets {
            let wrong = mismatches(reference, target, offset);
            if !wrong.is_empty() {
                failures.push(format!(
                    "{size:?} at {offset:?}: {} pixels differ, first {:?}",
                    wrong.len(),
                    &wrong[..wrong.len().min(4)]
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn a_reference_one_column_right_decodes_exactly() {
    assert_exact(&[(1, 0)]);
}

#[test]
fn a_reference_one_column_right_and_one_row_down_decodes_exactly() {
    assert_exact(&[(1, 1)]);
}

#[test]
fn a_reference_one_column_right_and_one_row_up_decodes_exactly() {
    assert_exact(&[(1, -1)]);
}

#[test]
fn references_at_the_offsets_that_were_already_right_still_decode_exactly() {
    // The control: offsets the bug never touched, so a failure here points
    // at the test kit or the fix rather than at the positive-RDX path.
    assert_exact(&[(0, 0), (-1, 0), (0, 1), (0, -1), (-1, 1), (-1, -1)]);
}
