//! Unit tests of the raster geometry (spec 0002 O1, O2, §4.7 "Clamp").

use super::{MAX_SIDE_PT, MAX_SIDE_PX, Plan, RASTER_PIXEL_SLACK, plan};

const DEFAULT_MAX: u64 = 40_000_000;

#[test]
fn a4_and_letter_at_72_dpi_are_one_pixel_per_point() {
    assert_eq!(
        plan(595.0, 842.0, 72, DEFAULT_MAX),
        Some(Plan {
            dpi: 72,
            width: 595,
            height: 842
        })
    );
    assert_eq!(
        plan(612.0, 792.0, 72, DEFAULT_MAX),
        Some(Plan {
            dpi: 72,
            width: 612,
            height: 792
        })
    );
}

#[test]
fn sides_are_rounded_up() {
    // 595 × 300 / 72 = 2479.17 and 842 × 300 / 72 = 3508.33 (O1's ceil).
    let p = plan(595.0, 842.0, 300, DEFAULT_MAX).expect("plans");
    assert_eq!((p.dpi, p.width, p.height), (300, 2480, 3509));
}

#[test]
fn max_render_pixels_lowers_the_resolution() {
    // floor(sqrt(1e6 × 72² / (595 × 842))) = floor(101.72) = 101.
    let p = plan(595.0, 842.0, 300, 1_000_000).expect("plans");
    assert_eq!(p.dpi, 101);
    assert_eq!((p.width, p.height), (835, 1182));
    // A request under the clamp is untouched.
    assert_eq!(plan(595.0, 842.0, 100, 1_000_000).expect("plans").dpi, 100);
}

#[test]
fn the_largest_page_is_clamped_by_pixels_before_u16() {
    // 14,400 pt at 300 dpi would be 60,000 px a side; the 40 MP default
    // lowers it to 31 dpi (6,200 px), far from u16.
    let p = plan(MAX_SIDE_PT, MAX_SIDE_PT, 300, DEFAULT_MAX).expect("plans");
    assert_eq!((p.dpi, p.width, p.height), (31, 6200, 6200));
}

#[test]
fn u16_bounds_the_side_when_pixels_do_not() {
    // With an unlimited pixel budget only the u16 side limit applies:
    // floor(65,535 × 72 / 14,400) = 327 dpi -> 65,400 px.
    let p = plan(MAX_SIDE_PT, 10.0, 2400, u64::MAX).expect("plans");
    assert_eq!(p.dpi, 327);
    assert!(p.width <= MAX_SIDE_PX);
    assert_eq!(p.width, 65_400);
}

#[test]
fn the_resolution_never_drops_below_one() {
    let p = plan(MAX_SIDE_PT, MAX_SIDE_PT, 300, 0).expect("plans");
    assert_eq!((p.dpi, p.width, p.height), (1, 200, 200));
    assert_eq!(plan(100.0, 100.0, 0, DEFAULT_MAX).expect("plans").dpi, 1);
}

#[test]
fn a_tiny_box_is_still_one_pixel() {
    let p = plan(0.01, 0.01, 72, DEFAULT_MAX).expect("plans");
    assert_eq!((p.width, p.height), (1, 1));
}

#[test]
fn degenerate_boxes_are_not_planned() {
    for (w, h) in [
        (0.0, 842.0),
        (595.0, 0.0),
        (-1.0, 842.0),
        (595.0, -5.0),
        (MAX_SIDE_PT + 1.0, 10.0),
        (10.0, MAX_SIDE_PT + 0.5),
        (f64::NAN, 10.0),
        (10.0, f64::INFINITY),
    ] {
        assert_eq!(plan(w, h, 300, DEFAULT_MAX), None, "{w} × {h}");
    }
}

#[test]
fn the_clamped_raster_respects_the_pixel_budget_up_to_rounding() {
    // Property over a grid of shapes and budgets: the raster never exceeds
    // the budget by more than the one extra row and column `ceil` can add
    // (except at the 1 dpi floor), and never exceeds the request.
    for &(w, h) in &[
        (595.0, 842.0),
        (612.0, 792.0),
        (100.0, 5000.0),
        (14_400.0, 1.0),
    ] {
        for &max in &[1_000u64, 250_000, 1_000_000, 8_700_000, 40_000_000] {
            for &dpi in &[1u32, 72, 150, 300, 600, 2400] {
                let p = plan(w, h, dpi, max).expect("plans");
                assert!(p.dpi >= 1 && p.dpi <= dpi.max(1));
                let area = u64::from(p.width) * u64::from(p.height);
                let slack = u64::from(p.width) + u64::from(p.height) + 1;
                assert!(
                    p.dpi == 1 || area <= max + slack,
                    "{w}×{h} at {dpi} under {max}: {p:?}"
                );
            }
        }
    }
}

#[test]
fn no_raster_exceeds_the_budget_by_more_than_the_slack() {
    // The unconditional form of the property above, which process isolation
    // relies on to size its frame (RASTER_PIXEL_SLACK): extreme shapes (one
    // side at the u16 limit), the 1 dpi floor, and budgets around the 64 MiB
    // frame.
    for &(w, h) in &[
        (595.0, 842.0),
        (100.0, 5000.0),
        (MAX_SIDE_PT, 1.0),
        (MAX_SIDE_PT, 73.3),
        (MAX_SIDE_PT, MAX_SIDE_PT),
        (8000.0, 9000.0),
        (0.37, 0.37),
    ] {
        for &max in &[
            1u64,
            1_000,
            40_000_000,
            66_977_776,
            67_108_847,
            u64::MAX / 2,
        ] {
            for &dpi in &[1u32, 72, 300, 2400] {
                let p = plan(w, h, dpi, max).expect("plans");
                let area = u64::from(p.width) * u64::from(p.height);
                assert!(
                    area <= max.saturating_add(RASTER_PIXEL_SLACK),
                    "{w}×{h} at {dpi} under {max}: {p:?}"
                );
            }
        }
    }
}
