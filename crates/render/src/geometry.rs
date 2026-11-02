//! Raster geometry of one page (spec 0002 O1, O2): the resolution actually
//! used and the pixel dimensions that follow from it.
//!
//! Pure arithmetic over the visible box in points, kept apart from hayro so
//! the clamp rules can be tested exhaustively without rendering.
//!
//! The three bounds on the resolution, in the order they bite:
//!
//! 1. the caller's `dpi` (never below 1);
//! 2. `max_render_pixels`: `floor(sqrt(max × 72² / (w × h)))`, so the raster
//!    holds about `max` pixels at most (O2; the `ceil` of each side can add
//!    less than one row and one column);
//! 3. hayro's `RenderSettings` carries the raster size as `u16`, so neither
//!    side may exceed 65,535 pixels whatever `max_render_pixels` allows.
//!
//! A box with no area, or with a side over 14,400 pt (PDF's user-space
//! limit), is degenerate: the page renders as one white pixel (O2) rather
//! than failing or allocating by an attacker's numbers.

/// The largest side, in points, of a page that is rendered (PDF 1.7 Annex C:
/// user space is at most 14,400 units a side at the default `UserUnit`).
pub(crate) const MAX_SIDE_PT: f64 = 14_400.0;

/// The largest raster side hayro can be asked for (`RenderSettings` stores
/// width and height as `u16`).
pub(crate) const MAX_SIDE_PX: u32 = u16::MAX as u32;

/// How many pixels a planned raster may hold beyond `max_render_pixels`.
/// With `a × b ≤ max` the exact sides at the clamped resolution, `ceil`
/// gives fewer than `(a + 1)(b + 1) = a·b + a + b + 1` pixels, and each side
/// is at most `MAX_SIDE_PX`; so `2 × MAX_SIDE_PX + 1` covers the rounding
/// with room for float error. The 1 dpi floor (at most 200 × 200 = 40,000
/// pixels for the largest page) is within the same slack, so
/// `width × height ≤ max_render_pixels + RASTER_PIXEL_SLACK` holds for every
/// plan. A caller with a hard ceiling on raster bytes (the process-isolation
/// frame) budgets for this.
pub const RASTER_PIXEL_SLACK: u64 = 2 * MAX_SIDE_PX as u64 + 1;

/// How one page is rasterised: the resolution after clamping and the raster
/// dimensions at that resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    /// `dpi'` of spec O2.
    pub(crate) dpi: u32,
    /// Raster width in pixels, `ceil(w_pt × dpi / 72)`, at least 1.
    pub(crate) width: u32,
    /// Raster height in pixels, `ceil(h_pt × dpi / 72)`, at least 1.
    pub(crate) height: u32,
}

/// Plans the raster of a visible box `w_pt × h_pt` (already rotated) at the
/// requested `dpi` under `max_render_pixels`. `None` when the box is
/// degenerate (module docs).
pub(crate) fn plan(w_pt: f64, h_pt: f64, dpi: u32, max_render_pixels: u64) -> Option<Plan> {
    let usable = |side: f64| side.is_finite() && side > 0.0 && side <= MAX_SIDE_PT;
    if !usable(w_pt) || !usable(h_pt) {
        return None;
    }
    // Float-to-int `as` saturates (and maps NaN to 0), so every bound below
    // is a valid u32 whatever the inputs; `max(1)` then applies O2's floor.
    // u64 -> f64 rounds above 2^53, far beyond any pixel budget that matters.
    #[allow(clippy::cast_precision_loss)]
    let by_pixels = (max_render_pixels as f64 * 72.0 * 72.0 / (w_pt * h_pt))
        .sqrt()
        .floor() as u32;
    // The largest d with ceil(side × d / 72) <= 65,535 for the longer side.
    let by_side = (f64::from(MAX_SIDE_PX) * 72.0 / w_pt.max(h_pt)).floor() as u32;
    let dpi = dpi.min(by_pixels).min(by_side).max(1);
    Some(Plan {
        dpi,
        width: pixels(w_pt, dpi),
        height: pixels(h_pt, dpi),
    })
}

/// `ceil(side_pt × dpi / 72)`, at least 1 and at most [`MAX_SIDE_PX`] (the
/// caller's clamp already guarantees the upper bound; it is re-applied so the
/// `u16` conversion downstream cannot fail).
fn pixels(side_pt: f64, dpi: u32) -> u32 {
    let px = (side_pt * f64::from(dpi) / 72.0).ceil() as u32;
    px.clamp(1, MAX_SIDE_PX)
}

#[cfg(test)]
#[path = "geometry_tests.rs"]
mod tests;
