//! Page rasterisation for OCR (run 0002 spec §1.1, §3.3 `pdf-extract-render`).
//!
//! [`render_gray`] draws one page of a [`pdf_extract_core::Document`] into an
//! 8-bit gray image. The drawing is hayro's own renderer (`hayro::render`,
//! crates.io 0.7.1, unpatched; spec D-1) driven by the same vendored
//! interpreter as the text path, so everything the text path bounds is
//! bounded here too: the operator and time budget (`max_ops_per_page`,
//! `page_time_budget`), the per-stream decode cap, the 50-level nesting cap
//! and the image pixel cap (`max_image_pixels`, H13), all taken from the
//! document's [`Limits`].
//!
//! What this crate adds on top of hayro:
//!
//! - **The resolution clamp** (`geometry`): `max_render_pixels` lowers the
//!   resolution (silently, except for [`Rendered::dpi`]), and hayro's `u16`
//!   raster sides are never exceeded; a degenerate page is one white pixel.
//! - **Gray conversion** (`gray`): hayro has no gray output.
//! - **The page's warnings** (`signals`), and the mapping of an exhausted
//!   budget to [`RenderError::Limit`]: a partial raster would be OCRed into
//!   garbage, and a caller that set a budget asked for a whole page or an
//!   error (O3).
//!
//! **One thread, no state.** A render runs entirely on the calling thread
//! (hayro's renderer is built single-threaded) and keeps nothing between
//! calls: hayro's `RenderCache` holds `Rc`s, is `!Send`, and is created per
//! call like the text path's `InterpreterCache`. That is also what makes the
//! thread-local signal sampling of `signals` sound.
//!
//! Panics in hayro or its dependencies propagate to Rust callers (run 0001
//! D-16); the Python binding catches them.

#![forbid(unsafe_code)]

mod geometry;
mod gray;
mod signals;

use std::sync::Arc;
use std::time::Instant;

use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings};
use hayro_interpret::{BudgetExhaustion, InterpretBudget, InterpreterSettings};
use pdf_extract_core::{LimitKind, Limits, Page, PageError, RenderInput, Warning, WarningKind};

use crate::signals::{Counters, SinkCounts, Warnings};

pub use crate::geometry::RASTER_PIXEL_SLACK;

/// An 8-bit gray raster: row-major, `width` bytes per row, 0 black and 255
/// white.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrayImage {
    /// Width in pixels (at least 1).
    pub width: u32,
    /// Height in pixels (at least 1).
    pub height: u32,
    /// `width × height` luminance bytes.
    pub data: Vec<u8>,
}

impl GrayImage {
    /// An all-white image (used for degenerate and unresolvable pages).
    fn white(width: u32, height: u32) -> Self {
        let len = usize::try_from(u64::from(width) * u64::from(height)).unwrap_or(0);
        Self {
            width,
            height,
            data: vec![u8::MAX; len],
        }
    }
}

/// How to render a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    /// Requested resolution in dots per inch; values below 1 are treated
    /// as 1. Lowered when the raster would exceed `max_render_pixels`.
    pub dpi: u32,
    /// The most pixels the raster may hold; the resolution is lowered to fit
    /// (spec O2). Bounds the gray image and hayro's RGBA working pixmap
    /// (4 bytes a pixel).
    pub max_render_pixels: u64,
    /// Whether annotation appearance streams are drawn (under the same
    /// visibility filter as the text path: not hidden, not `NoView`, not
    /// `Popup` or `Link`).
    pub render_annotations: bool,
}

impl Default for RenderOptions {
    /// 300 dpi, 40,000,000 pixels, annotations drawn: the OCR engine's
    /// defaults (spec O8).
    fn default() -> Self {
        Self {
            dpi: 300,
            max_render_pixels: 40_000_000,
            render_annotations: true,
        }
    }
}

/// A rendered page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    /// The raster.
    pub image: GrayImage,
    /// The resolution actually used (`dpi'` of spec O2), which is below the
    /// requested one when `max_render_pixels` lowered it.
    pub dpi: u32,
    /// What the renderer worked around on this page (skipped images,
    /// nesting caps, damaged content, ...), one warning per kind.
    pub warnings: Vec<Warning>,
}

/// Why a page could not be rendered.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// The document was closed before the render started.
    #[error("the document is closed")]
    Closed,
    /// The operator or time budget ran out while drawing; the partial raster
    /// is discarded.
    #[error("limit {kind} exceeded ({value})")]
    Limit {
        /// [`LimitKind::OpsPerPage`] or [`LimitKind::PageTimeBudget`].
        kind: LimitKind,
        /// The configured limit (operators, or milliseconds).
        value: u64,
    },
}

impl From<PageError> for RenderError {
    fn from(err: PageError) -> Self {
        match err {
            PageError::Limit { kind, value } => Self::Limit { kind, value },
            // `with_hayro_page` fails only with `Closed`: a `Page` handle's
            // index is in range by construction. Mapped rather than
            // asserted, so no input can turn it into a panic.
            PageError::Closed | PageError::IndexOutOfRange { .. } => Self::Closed,
        }
    }
}

/// Renders `page` into an 8-bit gray image (spec O1–O6).
///
/// The raster covers the page's visible box (CropBox ∩ MediaBox) with
/// `/Rotate` applied, on opaque white, at `ceil(w_pt × dpi' / 72) ×
/// ceil(h_pt × dpi' / 72)` pixels. Images over `max_image_pixels` or that
/// fail to decode are skipped with a warning, not an error.
///
/// # Errors
/// [`RenderError::Closed`] if the document was closed;
/// [`RenderError::Limit`] when the page's operator or time budget ran out.
pub fn render_gray(page: &Page, opts: &RenderOptions) -> Result<Rendered, RenderError> {
    page.with_hayro_page(|input| render_input(input, opts))?
}

fn render_input(input: RenderInput<'_>, opts: &RenderOptions) -> Result<Rendered, RenderError> {
    let start = Instant::now();
    let limits = input.limits;
    let mut warnings = Warnings::new(input.index);
    let requested_dpi = opts.dpi.max(1);

    let Some(page) = input.page else {
        // Only after a lazy xref repair changed what the leaf points to; the
        // text path gives an empty page with the same warning.
        warnings.push_n(WarningKind::PageTreeNodeUnreadable, 1, || {
            "page dictionary no longer resolves; page is empty".into()
        });
        return Ok(Rendered {
            image: GrayImage::white(1, 1),
            dpi: requested_dpi,
            warnings: warnings.finish(),
        });
    };

    let visible = page.intersected_crop_box();
    let (w_pt, h_pt) = match page.rotation() {
        hayro_syntax::page::Rotation::Horizontal
        | hayro_syntax::page::Rotation::FlippedHorizontal => (visible.height(), visible.width()),
        hayro_syntax::page::Rotation::None | hayro_syntax::page::Rotation::Flipped => {
            (visible.width(), visible.height())
        }
    };
    let Some(plan) = geometry::plan(w_pt, h_pt, requested_dpi, opts.max_render_pixels) else {
        return Ok(Rendered {
            image: GrayImage::white(1, 1),
            dpi: requested_dpi,
            warnings: warnings.finish(),
        });
    };

    // --- signal window: synchronous, this thread only (`signals`) ---
    let before = Counters::sample();
    let sink = Arc::new(SinkCounts::default());
    let budget = InterpretBudget::new(
        Some(limits.max_ops_per_page),
        // A budget too large to represent is no deadline at all.
        start.checked_add(limits.page_time_budget),
    );
    // Clones share the budget's counters: hayro clones the settings into its
    // own `Context`, and this handle reads the outcome afterwards.
    let outcome = budget.clone();
    let settings = InterpreterSettings {
        budget,
        warning_sink: {
            let sink = sink.clone();
            Arc::new(move |w| sink.note(w))
        },
        render_annotations: opts.render_annotations,
        ..InterpreterSettings::default()
    };
    // Rounding dpi' to f32 only perturbs where content lands, never the
    // raster size, which `plan` fixed.
    #[allow(clippy::cast_precision_loss)]
    let scale = plan.dpi as f32 / 72.0;
    let render_settings = RenderSettings {
        x_scale: scale,
        y_scale: scale,
        // `plan` keeps both sides within u16 (geometry::MAX_SIDE_PX).
        width: Some(u16::try_from(plan.width).unwrap_or(u16::MAX)),
        height: Some(u16::try_from(plan.height).unwrap_or(u16::MAX)),
        bg_color: WHITE,
    };
    let cache = RenderCache::new();
    let pixmap = hayro::render(page, &cache, &settings, &render_settings);
    let signals = Counters::sample().since(&before);
    // --- end of signal window ---

    let stats = outcome.stats();
    match stats.exhausted {
        Some(BudgetExhaustion::Ops) => {
            return Err(RenderError::Limit {
                kind: LimitKind::OpsPerPage,
                value: limits.max_ops_per_page,
            });
        }
        Some(BudgetExhaustion::Deadline) => return Err(time_limit(limits)),
        // Nothing here sets the cancel flag.
        Some(BudgetExhaustion::Cancelled) | None => {}
    }
    warnings.add_render(limits, &signals, &stats, &sink);

    Ok(Rendered {
        image: GrayImage {
            width: plan.width,
            height: plan.height,
            data: gray::to_gray(pixmap.data()),
        },
        dpi: plan.dpi,
        warnings: warnings.finish(),
    })
}

fn time_limit(limits: &Limits) -> RenderError {
    RenderError::Limit {
        kind: LimitKind::PageTimeBudget,
        value: u64::try_from(limits.page_time_budget.as_millis()).unwrap_or(u64::MAX),
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
