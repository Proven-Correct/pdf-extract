//! Unit tests of the noise pre-check on synthetic rasters (spec 0003 §4.2,
//! the `stats` and `downsample2` items). The corpus-D sweep of `classify`
//! renders real pages and lives in `tests/precheck.rs`.

use pdf_extract_testkit::corpus_d::{NOISE_SEED, add_noise};

use super::*;

/// The A4 raster at 300 dpi.
const A4_300: (u32, u32) = (2480, 3509);

fn raster(width: u32, height: u32, value: u8) -> GrayImage {
    GrayImage {
        width,
        height,
        data: vec![value; width as usize * height as usize],
    }
}

/// The white A4 page under corpus D's noise field (`add_noise`, seed
/// 20261003): spec 0003 Appendix A.1's "pure noise σ 12 on white".
fn noise_field() -> GrayImage {
    let mut img = raster(A4_300.0, A4_300.1, 255);
    add_noise(&mut img.data, NOISE_SEED);
    img
}

/// A `w × h` white raster with a `200 × 20` rectangle of `value` at
/// `(10, 10)`.
fn with_rectangle(w: u32, h: u32, value: u8) -> GrayImage {
    let mut img = raster(w, h, 255);
    for y in 10..30 {
        for x in 10..210 {
            img.data[y * w as usize + x] = value;
        }
    }
    img
}

/// A direct port of Tesseract 5.5.3's `OtsuStats`
/// (`src/ccstruct/otsuthr.cpp`), `int` arithmetic and all, kept beside the
/// production version so the two are compared rather than one assumed.
#[allow(
    clippy::needless_range_loop,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "a line-by-line port of the C++ loops"
)]
fn tesseract_otsu_stats(histogram: &[i32; 256]) -> (i32, i32, i32) {
    let mut h: i32 = 0;
    let mut mu_total = 0.0f64;
    for i in 0..256 {
        h += histogram[i];
        mu_total += i as f64 * f64::from(histogram[i]);
    }
    let mut best_t: i32 = -1;
    let mut best_omega_0: i32 = 0;
    let mut best_sig_sq_b = 0.0f64;
    let mut omega_0: i32 = 0;
    let mut mu_t = 0.0f64;
    for t in 0..255 {
        omega_0 += histogram[t];
        mu_t += t as f64 * f64::from(histogram[t]);
        if omega_0 == 0 {
            continue;
        }
        let omega_1 = h - omega_0;
        if omega_1 == 0 {
            break;
        }
        let mu_0 = mu_t / f64::from(omega_0);
        let mu_1 = (mu_total - mu_t) / f64::from(omega_1);
        let mut sig_sq_b = mu_1 - mu_0;
        sig_sq_b *= sig_sq_b * f64::from(omega_0) * f64::from(omega_1);
        if best_t < 0 || sig_sq_b > best_sig_sq_b {
            best_sig_sq_b = sig_sq_b;
            best_t = t as i32;
            best_omega_0 = omega_0;
        }
    }
    (best_t, h, best_omega_0)
}

/// SplitMix64, for seeded random histograms.
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[test]
fn a_white_page_is_empty() {
    let s = stats(&raster(A4_300.0, A4_300.1, 255));
    assert_eq!(s.class, OtsuClass::Empty);
    assert_eq!(s.threshold, 0);
    assert_eq!(s.foreground, 0.0);
    assert_eq!(s.isolation, None);
    assert_eq!(s.verdict(), Verdict::Clear);
}

#[test]
fn a_one_pixel_raster_is_empty() {
    for v in [0, 128, 255] {
        let s = stats(&raster(1, 1, v));
        assert_eq!(s.class, OtsuClass::Empty, "{v}");
        assert_eq!(classify(&raster(1, 1, v)), Verdict::Clear);
    }
}

#[test]
fn a_degenerate_raster_is_read_as_its_complete_rows() {
    // No pixels at all, and fewer bytes than width × height: no panic, the
    // complete rows only.
    for img in [
        raster(0, 5, 0),
        GrayImage {
            width: 4,
            height: 4,
            data: vec![0, 255, 0, 255, 0, 255],
        },
    ] {
        let s = stats(&img);
        assert_eq!(
            s.class,
            if img.width == 0 {
                OtsuClass::Empty
            } else {
                OtsuClass::None
            }
        );
        assert!(downsample2(&img).is_none());
    }
}

#[test]
fn the_noise_field_matches_the_probe() {
    let img = noise_field();
    let s = stats(&img);
    assert_eq!(s.class, OtsuClass::ForegroundDark);
    assert!(s.foreground_dark);
    assert_eq!(s.threshold, 246);
    assert!((s.foreground - 0.2417).abs() <= 0.005, "{s:?}");
    let random = (1.0 - s.foreground).powi(4);
    let iso = s.isolation.expect("above MIN_FOREGROUND");
    assert!((iso - random).abs() <= 0.02, "{iso} vs {random}");
    assert_eq!(s.verdict(), Verdict::Speckle);

    let half = downsample2(&img).expect("large enough");
    let s = stats(&half);
    assert_eq!(s.class, OtsuClass::None);
    assert!((s.foreground - 0.36).abs() <= 0.01, "{s:?}");
    assert!(
        (s.isolation.expect("computed") - 0.165).abs() <= 0.01,
        "{s:?}"
    );
    assert_eq!(classify(&half), Verdict::Speckle);
}

#[test]
fn the_inverted_noise_field_is_speckle_on_its_light_side() {
    // Spec 0003 Appendix A.6: Tesseract's ink is the light side here.
    let mut img = noise_field();
    for p in &mut img.data {
        *p = 255 - *p;
    }
    let s = stats(&img);
    assert_eq!(s.class, OtsuClass::BackgroundDark);
    assert!(!s.foreground_dark);
    assert_eq!(s.threshold, 8);
    assert!((s.foreground - 0.242).abs() <= 0.005, "{s:?}");
    assert!(
        (s.isolation.expect("computed") - 0.331).abs() <= 0.01,
        "{s:?}"
    );
    // The dark side, which the rule does not look at, is connected.
    let (w, h) = (img.width as usize, img.height as usize);
    assert!(super::isolation(&img.data, w, h, s.threshold, true) < 0.01);
    assert_eq!(s.verdict(), Verdict::Speckle);
    let half = downsample2(&img).expect("large enough");
    assert_eq!(classify(&half), Verdict::Speckle);
}

#[test]
fn a_large_dark_region_is_connected_and_clear() {
    // Upper 45 % black, lower part white with a few black "words": Otsu's
    // `None` class, which the signed-off rule skipped (spec 0003 revision
    // (3)); its dark side is connected.
    let (w, h) = (400usize, 400usize);
    let mut img = raster(w as u32, h as u32, 255);
    img.data[..w * (h * 45 / 100)].fill(0);
    for word in 0..5 {
        for y in 250..262 {
            for x in 20 + 70 * word..80 + 70 * word {
                img.data[y * w + x] = 0;
            }
        }
    }
    let s = stats(&img);
    assert_eq!(s.class, OtsuClass::None);
    assert!(s.foreground_dark);
    assert_eq!(s.isolation, Some(0.0));
    assert_eq!(s.verdict(), Verdict::Clear);
    assert_eq!(
        classify(&downsample2(&img).expect("halves")),
        Verdict::Clear
    );
}

#[test]
fn text_like_strokes_are_connected() {
    // 400 × 200: the 200 × 20 rectangle is 5 % of the raster.
    for value in [0, 200] {
        let img = with_rectangle(400, 200, value);
        let s = stats(&img);
        assert_eq!(s.class, OtsuClass::ForegroundDark, "{value}");
        assert_eq!(s.isolation, Some(0.0), "{value}");
        assert_eq!(s.verdict(), Verdict::Clear);
    }
    let mut noisy = with_rectangle(400, 200, 0);
    add_noise(&mut noisy.data, NOISE_SEED);
    let s = stats(&noisy);
    assert_eq!(s.class, OtsuClass::ForegroundDark);
    assert!(s.isolation.expect("computed") < 0.1, "{s:?}");
    assert_eq!(s.verdict(), Verdict::Clear);
}

#[test]
fn little_foreground_skips_the_isolation_rule() {
    // The rectangle is 0.4 % of a 1000 × 1000 raster: under MIN_FOREGROUND.
    let s = stats(&with_rectangle(1000, 1000, 0));
    assert_eq!(s.class, OtsuClass::ForegroundDark);
    assert!(s.foreground < MIN_FOREGROUND);
    assert_eq!(s.isolation, None);
    assert_eq!(s.verdict(), Verdict::Clear);
}

#[test]
fn an_inverted_page_is_background_dark_and_clear() {
    // White text on black: Tesseract's BackgroundDark, its ink the light
    // side, which is connected.
    let mut img = raster(400, 200, 0);
    for y in 10..30 {
        for x in 10..210 {
            img.data[y * 400 + x] = 255;
        }
    }
    let s = stats(&img);
    assert_eq!(s.class, OtsuClass::BackgroundDark);
    assert!(!s.foreground_dark);
    assert!((s.foreground - 0.05).abs() < 1e-12);
    assert_eq!(s.isolation, Some(0.0));
    assert_eq!(classify(&img), Verdict::Clear);
}

#[test]
fn isolated_pixels_and_the_edges() {
    // A checkerboard: every pixel is isolated, including those on the
    // border (outside counts as the other side); class None (half dark),
    // so Tesseract's ink is the light side.
    let mut img = raster(6, 6, 255);
    for y in 0..6 {
        for x in 0..6 {
            if (x + y) % 2 == 0 {
                img.data[y * 6 + x] = 0;
            }
        }
    }
    let s = stats(&img);
    assert_eq!((s.class, s.foreground_dark), (OtsuClass::None, false));
    assert_eq!(s.isolation, Some(1.0));
    assert_eq!(classify(&img), Verdict::Speckle);
    // Sparse isolated dots at 2 % density, fg-dark: isolation 1.0, far
    // above 0.5 × 0.98⁴.
    let mut img = raster(100, 100, 255);
    // 200 dots on a 10 x 5 px grid: no two adjacent.
    for i in 0..200 {
        let (x, y) = ((i % 10) * 10, (i / 10) * 5);
        img.data[y * 100 + x] = 0;
    }
    let s = stats(&img);
    assert_eq!(s.class, OtsuClass::ForegroundDark);
    assert_eq!(s.isolation, Some(1.0));
    assert_eq!(s.verdict(), Verdict::Speckle);
    // A dark pixel whose only neighbour lies across a row boundary is still
    // isolated: neighbours are 4-connected within the raster, no wrap.
    let mut img = raster(10, 10, 255);
    img.data[9] = 0; // (9, 0)
    img.data[10] = 0; // (0, 1)
    assert_eq!(super::isolation(&img.data, 10, 10, 0, true), 1.0);
    img.data[19] = 0; // (9, 1), below (9, 0)
    assert!((super::isolation(&img.data, 10, 10, 0, true) - 1.0 / 3.0).abs() < 1e-12);
    assert_eq!(
        super::isolation(&raster(3, 3, 255).data, 3, 3, 0, true),
        0.0
    );
    // The light side of the same raster: the white pixels, all connected.
    assert_eq!(super::isolation(&img.data, 10, 10, 0, false), 0.0);
}

#[test]
fn the_verdict_line_is_inclusive() {
    let at = |foreground: f64, iso: f64| Stats {
        threshold: 200,
        class: OtsuClass::ForegroundDark,
        foreground_dark: true,
        foreground,
        isolation: Some(iso),
    };
    let line = ISOLATION_FACTOR * (1.0f64 - 0.2).powi(4);
    assert_eq!(at(0.2, line).verdict(), Verdict::Speckle);
    assert_eq!(at(0.2, line - 1e-9).verdict(), Verdict::Clear);
    // The class never decides by itself (revision (3)).
    let mut s = at(0.2, 1.0);
    for class in [
        OtsuClass::ForegroundDark,
        OtsuClass::BackgroundDark,
        OtsuClass::None,
    ] {
        s.class = class;
        s.isolation = Some(1.0);
        assert_eq!(s.verdict(), Verdict::Speckle, "{class:?}");
        s.isolation = Some(0.0);
        assert_eq!(s.verdict(), Verdict::Clear, "{class:?}");
        s.isolation = None;
        assert_eq!(s.verdict(), Verdict::Clear, "{class:?}");
    }
    s.class = OtsuClass::Empty;
    s.isolation = Some(1.0);
    assert_eq!(s.verdict(), Verdict::Clear);
}

#[test]
fn otsu_agrees_with_tesseract_on_random_histograms() {
    let mut state = 20_261_005u64;
    for case in 0..20 {
        let mut ours = [0u64; 256];
        let mut theirs = [0i32; 256];
        // Mixtures of sparse and dense bins, some histograms degenerate.
        let bins = 1 + (splitmix(&mut state) % 256) as usize;
        for _ in 0..bins {
            let bin = (splitmix(&mut state) % 256) as usize;
            let n = (splitmix(&mut state) % 100_000) as i32;
            theirs[bin] += n;
        }
        for (o, t) in ours.iter_mut().zip(&theirs) {
            *o = u64::try_from(*t).expect("non-negative");
        }
        let (t, h, o0) = otsu(&ours);
        let (bt, bh, bo0) = tesseract_otsu_stats(&theirs);
        assert_eq!(t.map_or(-1, i32::from), bt, "case {case}");
        assert_eq!(h, u64::try_from(bh).expect("h"), "case {case}");
        assert_eq!(o0, u64::try_from(bo0).expect("o0"), "case {case}");
    }
    // Two equal peaks: the first maximum wins in both.
    let mut ours = [0u64; 256];
    let mut theirs = [0i32; 256];
    for (bin, n) in [(10, 50), (100, 50), (250, 50)] {
        ours[bin] = n;
        theirs[bin] = i32::try_from(n).expect("small");
    }
    let (t, _, _) = otsu(&ours);
    assert_eq!(t.map_or(-1, i32::from), tesseract_otsu_stats(&theirs).0);
    // Everything in one bin: Tesseract's −1.
    let mut one = [0u64; 256];
    one[77] = 9;
    assert_eq!(otsu(&one), (None, 9, 0));
    assert_eq!(otsu(&[0; 256]), (None, 0, 0));
}

#[test]
fn downsample_dimensions_and_rounding() {
    let img = GrayImage {
        width: 2,
        height: 2,
        data: vec![0, 255, 255, 255],
    };
    let d = downsample2(&img).expect("2 x 2");
    assert_eq!((d.width, d.height, d.data.as_slice()), (1, 1, &[191][..]));
    // Odd sides: the trailing row and column are dropped.
    let img = GrayImage {
        width: 5,
        height: 3,
        data: (0..15).map(|v| v * 10).collect(),
    };
    let d = downsample2(&img).expect("5 x 3");
    assert_eq!((d.width, d.height), (2, 1));
    // (0 + 10 + 50 + 60 + 2) / 4 = 30; (20 + 30 + 70 + 80 + 2) / 4 = 50.
    assert_eq!(d.data, [30, 50]);
    // Halves round up; white stays white.
    let img = GrayImage {
        width: 2,
        height: 2,
        data: vec![1, 2, 2, 1],
    };
    assert_eq!(downsample2(&img).expect("2 x 2").data, [2]);
    assert_eq!(
        downsample2(&raster(4, 4, 255)).expect("4 x 4").data,
        [255; 4]
    );
    for (w, h) in [(1, 9), (9, 1), (1, 1)] {
        assert!(downsample2(&raster(w, h, 0)).is_none(), "{w} x {h}");
    }
    let (w, h) = A4_300;
    let d = downsample2(&raster(w, h, 255)).expect("A4");
    assert_eq!((d.width, d.height), (1240, 1754));
}

#[test]
fn the_histogram_counts_every_pixel_once() {
    // Lengths around the 4-wide unrolling, values in every lane.
    for len in [0usize, 1, 3, 4, 5, 7, 8, 1027] {
        let pixels: Vec<u8> = (0..len).map(|i| (i * 37 % 256) as u8).collect();
        let mut naive = [0u64; 256];
        for &p in &pixels {
            naive[usize::from(p)] += 1;
        }
        assert_eq!(histogram(&pixels), naive, "{len}");
    }
}
