//! Premultiplied RGBA8 to 8-bit luminance (spec 0002 O1, §3.2 "What we add").
//!
//! hayro's pixmap is premultiplied RGBA8 with no gray output. Each pixel is
//! composited over opaque white and converted with the Rec. 709 weights,
//! `round(0.2126 R + 0.7152 G + 0.0722 B)`.
//!
//! **Integer arithmetic, on purpose.** The weights sum to 10,000, so the
//! conversion is `(2126 R + 7152 G + 722 B + 5000) / 10000` in `u32`: exact
//! round-half-up with no floating point, hence identical on every host (O5).
//! The renderer paints an opaque white background first, so alpha is 255
//! everywhere in practice and premultiplied equals straight; compositing is
//! still applied so a translucent pixel (were hayro ever to leave one) reads
//! as it would over paper, not as darker than it is.

/// Rec. 709 luma weights scaled by 10,000 (R, G, B).
const WEIGHTS: [u32; 3] = [2126, 7152, 722];

/// The gray value of one premultiplied pixel composited over white.
///
/// Over white, a premultiplied channel `c` with alpha `a` becomes
/// `c + (255 − a)`; a well-formed premultiplied pixel has `c <= a`, and the
/// sum is clamped for one that is not.
pub(crate) fn luma(r: u8, g: u8, b: u8, a: u8) -> u8 {
    let under = 255 - u32::from(a);
    let over_white = |c: u8| (u32::from(c) + under).min(255);
    let weighted =
        WEIGHTS[0] * over_white(r) + WEIGHTS[1] * over_white(g) + WEIGHTS[2] * over_white(b);
    // At most 255 × 10,000 + 5,000, so the quotient is at most 255.
    u8::try_from((weighted + 5_000) / 10_000).unwrap_or(u8::MAX)
}

/// Converts a row-major premultiplied RGBA8 buffer to gray, one byte per
/// pixel in the same order.
pub(crate) fn to_gray(pixels: &[hayro::vello_cpu::color::PremulRgba8]) -> Vec<u8> {
    pixels.iter().map(|p| luma(p.r, p.g, p.b, p.a)).collect()
}

#[cfg(test)]
#[path = "gray_tests.rs"]
mod tests;
