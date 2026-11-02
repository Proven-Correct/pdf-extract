//! Unit tests of the gray conversion (spec 0002 O1).

use hayro::vello_cpu::color::PremulRgba8;

use super::{luma, to_gray};

#[test]
fn primaries_and_extremes() {
    assert_eq!(luma(255, 255, 255, 255), 255);
    assert_eq!(luma(0, 0, 0, 255), 0);
    // round(0.2126 × 255) = round(54.21) = 54; 0.7152 × 255 = 182.38; 0.0722 × 255 = 18.41.
    assert_eq!(luma(255, 0, 0, 255), 54);
    assert_eq!(luma(0, 255, 0, 255), 182);
    assert_eq!(luma(0, 0, 255, 255), 18);
}

#[test]
fn matches_the_floating_point_formula_on_opaque_pixels() {
    // Exhaustive over gray levels and a lattice of colours: the integer form
    // equals round(0.2126 R + 0.7152 G + 0.0722 B) wherever the float is not
    // within rounding noise of a .5 tie.
    for r in (0..=255u8).step_by(5) {
        for g in (0..=255u8).step_by(5) {
            for b in (0..=255u8).step_by(5) {
                let f = 0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b);
                if (f.fract() - 0.5).abs() < 1e-9 {
                    continue;
                }
                assert_eq!(f64::from(luma(r, g, b, 255)), f.round(), "{r} {g} {b}");
            }
        }
    }
}

#[test]
fn transparent_pixels_read_as_white() {
    assert_eq!(luma(0, 0, 0, 0), 255);
    // Half-transparent black (premultiplied 0, alpha 128) over white.
    assert_eq!(luma(0, 0, 0, 128), 127);
    // A malformed premultiplied pixel (channel above alpha) is clamped.
    assert_eq!(luma(255, 255, 255, 0), 255);
}

#[test]
fn buffers_convert_in_order() {
    let px = |r, g, b, a| PremulRgba8 { r, g, b, a };
    let out = to_gray(&[px(255, 255, 255, 255), px(0, 0, 0, 255), px(0, 0, 0, 0)]);
    assert_eq!(out, [255, 0, 255]);
}
