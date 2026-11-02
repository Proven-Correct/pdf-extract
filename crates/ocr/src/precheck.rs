//! The noise pre-check (run 0003 F11–F13): a cheap look at the rendered
//! raster, before Tesseract sees it, for the one input class known to stall
//! Tesseract's layout analysis — a page holding little real ink under
//! pixel-level noise.
//!
//! **Why it works.** Tesseract binarises the whole raster with one global
//! Otsu threshold (`thresholding_method = 0`; `OtsuThreshold` and
//! `OtsuStats` in the vendored `src/ccstruct/otsuthr.cpp`). When a page
//! holds a few thousand dark pixels or more, Otsu's between-class variance is
//! won by the ink/paper split and the noise lands on the paper side. On a
//! near-blank page it is won by splitting the *noise* itself: a fifth of the
//! page becomes foreground, a million connected components of one to three
//! pixels, and layout analysis spends tens of seconds producing garbage
//! (spec 0003 Appendix A.1). [`stats`] replicates Tesseract's own threshold
//! and class verdict exactly, finds the side of the split Tesseract will
//! binarise as ink (the dark side when it holds under half the pixels, the
//! light side otherwise), then measures that side's **isolation fraction**
//! — the share of its pixels with no 4-neighbour on the same side.
//! Connected ink (text, banners, figures, inverted regions) gives ≈ 0;
//! independent noise at density `p` is isolated with probability
//! `(1 − p)⁴`. [`classify`] calls a raster [`Verdict::Speckle`] when the
//! foreground side is at least [`MIN_FOREGROUND`] of the raster and at least
//! [`ISOLATION_FACTOR`] times as isolated as random noise would be.
//!
//! **The class never decides by itself** (spec 0003 revision (3)).
//! Tesseract's "no convincing foreground" class is the normal class of a
//! page with a large dark region, which it reads fine; its "dark
//! background" class is the normal class of an inverted page, which stalls
//! exactly like the upright one when its light side is noise. Testing the
//! side Tesseract treats as ink covers both polarities with one rule.
//!
//! **The remedy** (applied by the engine, F13): a 2 × 2 box filter
//! ([`downsample2`]) quarters the pixel count and halves the noise
//! amplitude; real strokes survive. If the half-resolution raster is
//! [`Verdict::Clear`] it is recognised at half the resolution; otherwise
//! recognition is skipped.
//!
//! Everything here is pure, deterministic and allocation-free apart from
//! the output raster of [`downsample2`]: the isolation pass reads the gray
//! values directly instead of building a binary image. The constants are
//! documented, not configurable (spec 0003 Q4); the measurements behind
//! them and their margins are in spec 0003 Appendix A.2.

use pdf_extract_render::GrayImage;

/// The lowest rendered resolution the engine runs the pre-check at. Below
/// it a 2 × 2 downsample would destroy text, and no 72-dpi render of
/// corpus D is speckle-dominated (spec 0003 F12, Appendix A.2).
pub const MIN_DPI: u32 = 144;

/// The smallest foreground-side share for which the isolation rule
/// applies. A page with less foreground than this — a few specks of dust —
/// goes to Tesseract unchanged; it is cheap there.
pub const MIN_FOREGROUND: f64 = 0.01;

/// A raster is speckle when its foreground side's isolation fraction is at
/// least this factor times `(1 − p)⁴`, the isolation fraction of
/// independent random pixels at foreground density `p`.
pub const ISOLATION_FACTOR: f64 = 0.5;

/// The pre-check's decision for one raster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Recognise as is.
    Clear,
    /// Dominated by speckle: Tesseract would segment noise.
    Speckle,
}

/// Tesseract's verdict on its own Otsu split (`OtsuThreshold`), by the
/// share `ω₀ / H` of pixels at or below the threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtsuClass {
    /// Every pixel is on one side (`ω₀ ∈ {0, H}`): nothing to threshold.
    Empty,
    /// `ω₀ < H / 4`: a convincing dark foreground on a light background.
    ForegroundDark,
    /// `ω₀ > 3H / 4`: a convincing light foreground on a dark background.
    BackgroundDark,
    /// In between: Tesseract finds no convincing foreground and uses the
    /// split anyway ("the best of the bad lot").
    None,
}

/// What the pre-check measured on one raster (report and diagnosis; the
/// decision is [`Stats::verdict`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    /// Tesseract's Otsu threshold `t`. `0` when the class is
    /// [`OtsuClass::Empty`] (Tesseract has none then).
    pub threshold: u8,
    /// Tesseract's class verdict.
    pub class: OtsuClass,
    /// Which side Tesseract binarises as ink: `true` for the pixels `≤ t`
    /// (`ω₀ < H / 2`, its `hi_value = 1`), `false` for the pixels `> t`.
    /// `true` for an empty raster.
    pub foreground_dark: bool,
    /// That side's share of the raster: `ω₀ / H` or `1 − ω₀ / H` (`0` for
    /// an [`OtsuClass::Empty`] raster).
    pub foreground: f64,
    /// The share of foreground-side pixels with no 4-neighbour on the same
    /// side (pixels outside the raster count as the other side); computed
    /// for every class but [`OtsuClass::Empty`] when `foreground ≥`
    /// [`MIN_FOREGROUND`], the only case the rule reads it.
    pub isolation: Option<f64>,
}

impl Stats {
    /// The rule of spec 0003 F11 over these statistics: `Speckle` iff the
    /// class is not [`OtsuClass::Empty`], the foreground side is at least
    /// [`MIN_FOREGROUND`] of the raster and its isolation fraction is at
    /// least `ISOLATION_FACTOR × (1 − p)⁴`; otherwise `Clear`.
    pub fn verdict(&self) -> Verdict {
        match (self.class, self.isolation) {
            (OtsuClass::Empty, _) | (_, None) => Verdict::Clear,
            (_, Some(iso)) if iso >= ISOLATION_FACTOR * (1.0 - self.foreground).powi(4) => {
                Verdict::Speckle
            }
            _ => Verdict::Clear,
        }
    }
}

/// The complete rows of `img`: a `GrayImage` with fewer bytes than
/// `width × height` (its fields are public) is read as the rows it holds,
/// never indexed past its end.
fn rows(img: &GrayImage) -> (usize, usize) {
    let w = img.width as usize;
    if w == 0 {
        return (0, 0);
    }
    (w, (img.height as usize).min(img.data.len() / w))
}

/// Tesseract's `OtsuStats` over a 256-bin histogram: `(best_t, H,
/// best_omega_0)`, `best_t` being `None` where Tesseract returns −1.
///
/// The arithmetic follows the C++ step by step, including the order of the
/// floating-point products, so a tie resolves the same way: maximise
/// `ω₀ · ω₁ · (μ₁ − μ₀)²` over `t ∈ 0..=254`, `ω₀` accumulating
/// `histogram[t]` (pixels `≤ t`), the first maximum winning (strict `>`),
/// and the first valid `t` taken whatever its score.
pub(crate) fn otsu(histogram: &[u64; 256]) -> (Option<u8>, u64, u64) {
    let mut h: u64 = 0;
    let mut mu_total = 0.0f64;
    for (i, &n) in histogram.iter().enumerate() {
        h += n;
        #[allow(clippy::cast_precision_loss)]
        let term = i as f64 * n as f64;
        mu_total += term;
    }
    let mut best_t: Option<u8> = None;
    let mut best_omega_0 = 0u64;
    let mut best_sig = 0.0f64;
    let mut omega_0 = 0u64;
    let mut mu_t = 0.0f64;
    for t in 0u8..=254 {
        let n = histogram[usize::from(t)];
        omega_0 += n;
        #[allow(clippy::cast_precision_loss)]
        let term = f64::from(t) * n as f64;
        mu_t += term;
        if omega_0 == 0 {
            continue;
        }
        let omega_1 = h - omega_0;
        if omega_1 == 0 {
            break;
        }
        #[allow(clippy::cast_precision_loss)]
        let (o0, o1) = (omega_0 as f64, omega_1 as f64);
        let mu_0 = mu_t / o0;
        let mu_1 = (mu_total - mu_t) / o1;
        let d = mu_1 - mu_0;
        let sig = d * (d * o0 * o1);
        if best_t.is_none() || sig > best_sig {
            best_sig = sig;
            best_t = Some(t);
            best_omega_0 = omega_0;
        }
    }
    (best_t, h, best_omega_0)
}

/// Tesseract's class verdict for `ω₀` of `H` pixels (`OtsuThreshold`).
fn class_of(omega_0: u64, h: u64) -> OtsuClass {
    if omega_0 == 0 || omega_0 == h {
        return OtsuClass::Empty;
    }
    #[allow(clippy::cast_precision_loss)]
    let (o0, h) = (omega_0 as f64, h as f64);
    if o0 > h * 0.75 {
        OtsuClass::BackgroundDark
    } else if o0 < h * 0.25 {
        OtsuClass::ForegroundDark
    } else {
        OtsuClass::None
    }
}

/// The isolation fraction of the foreground side (`value ≤ t` when `dark`,
/// `value > t` otherwise) of the first `rows` rows of `data` (row-major, `w`
/// wide), or `0` with no foreground.
fn isolation(data: &[u8], w: usize, rows: usize, t: u8, dark: bool) -> f64 {
    if dark {
        isolation_of(data, w, rows, |v| v <= t)
    } else {
        isolation_of(data, w, rows, |v| v > t)
    }
}

/// [`isolation`] for one side; generic so each side's test is inlined into
/// the pixel loop.
fn isolation_of(data: &[u8], w: usize, rows: usize, side: impl Fn(u8) -> bool) -> f64 {
    let fg = |i: usize| side(data[i]);
    let (mut foreground, mut isolated) = (0u64, 0u64);
    for y in 0..rows {
        let row = y * w;
        for x in 0..w {
            let i = row + x;
            if !fg(i) {
                continue;
            }
            foreground += 1;
            let connected = (x > 0 && fg(i - 1))
                || (x + 1 < w && fg(i + 1))
                || (y > 0 && fg(i - w))
                || (y + 1 < rows && fg(i + w));
            if !connected {
                isolated += 1;
            }
        }
    }
    if foreground == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let share = isolated as f64 / foreground as f64;
    share
}

/// The 256-bin histogram of `pixels`. Four interleaved partial histograms
/// keep runs of equal pixels (most of a page) from serialising on one
/// counter; this halves the cost of the whole check (spec 0003 F15).
fn histogram(pixels: &[u8]) -> [u64; 256] {
    let mut lanes = [[0u32; 256]; 4];
    // u32 lanes cannot overflow: a block holds fewer than 2³² pixels, and
    // the lanes are flushed into the u64 total after each block.
    let mut total = [0u64; 256];
    for block in pixels.chunks(4 * (u32::MAX as usize / 4)) {
        let (quads, rest) = block.as_chunks::<4>();
        for &[a, b, c, d] in quads {
            lanes[0][usize::from(a)] += 1;
            lanes[1][usize::from(b)] += 1;
            lanes[2][usize::from(c)] += 1;
            lanes[3][usize::from(d)] += 1;
        }
        for &p in rest {
            lanes[0][usize::from(p)] += 1;
        }
        for lane in &mut lanes {
            for (t, n) in total.iter_mut().zip(lane.iter_mut()) {
                *t += u64::from(*n);
                *n = 0;
            }
        }
    }
    total
}

/// The pre-check's statistics of `img` (spec 0003 F11): the histogram,
/// Tesseract's Otsu threshold, class verdict and foreground side, that
/// side's share and, where the rule needs it, its isolation fraction.
pub fn stats(img: &GrayImage) -> Stats {
    let (w, rows) = rows(img);
    let pixels = &img.data[..w * rows];
    let (t, h, omega_0) = otsu(&histogram(pixels));
    let class = class_of(omega_0, h);
    let threshold = match class {
        OtsuClass::Empty => 0,
        _ => t.unwrap_or(0),
    };
    #[allow(clippy::cast_precision_loss)]
    let (o0, hf) = (omega_0 as f64, h as f64);
    // Tesseract's `hi_value = best_omega_0 < H * 0.5`, in `OtsuThreshold`.
    let foreground_dark = class == OtsuClass::Empty || o0 < hf * 0.5;
    let foreground = match class {
        OtsuClass::Empty => 0.0,
        _ if foreground_dark => o0 / hf,
        _ => 1.0 - o0 / hf,
    };
    let isolation = (class != OtsuClass::Empty && foreground >= MIN_FOREGROUND)
        .then(|| isolation(pixels, w, rows, threshold, foreground_dark));
    Stats {
        threshold,
        class,
        foreground_dark,
        foreground,
        isolation,
    }
}

/// Whether `img` is dominated by speckle (spec 0003 F11): `stats(img)`'s
/// [`Stats::verdict`].
pub fn classify(img: &GrayImage) -> Verdict {
    stats(img).verdict()
}

/// `img` box-filtered 2 × 2 to `⌊w/2⌋ × ⌊h/2⌋`: each output pixel the
/// rounded mean of its 2 × 2 block (halves rounding up), a trailing odd row
/// or column dropped. `None` when either side is under 2 pixels.
pub fn downsample2(img: &GrayImage) -> Option<GrayImage> {
    let (w, rows) = rows(img);
    if w < 2 || rows < 2 {
        return None;
    }
    let (ow, oh) = (w / 2, rows / 2);
    let mut data = Vec::with_capacity(ow * oh);
    for y in 0..oh {
        let top = &img.data[2 * y * w..(2 * y + 1) * w];
        let bottom = &img.data[(2 * y + 1) * w..(2 * y + 2) * w];
        for x in 0..ow {
            let sum = u16::from(top[2 * x])
                + u16::from(top[2 * x + 1])
                + u16::from(bottom[2 * x])
                + u16::from(bottom[2 * x + 1]);
            // At most (4 × 255 + 2) / 4 = 255.
            data.push(u8::try_from((sum + 2) / 4).unwrap_or(u8::MAX));
        }
    }
    Some(GrayImage {
        width: u32::try_from(ow).unwrap_or(u32::MAX),
        height: u32::try_from(oh).unwrap_or(u32::MAX),
        data,
    })
}

#[cfg(test)]
#[path = "precheck_tests.rs"]
mod tests;
