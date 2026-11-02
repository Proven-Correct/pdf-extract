//! Drivers shared by the fuzz targets and the small-stack replay binary.
//!
//! Four of the five drivers are core's (`pdf_extract_core::fuzz_api`), so the
//! stable regression replay in core's test suite runs exactly what the
//! targets run. The render driver lives here because core cannot depend on
//! the render crate; `crates/render/tests/fuzz_regressions.rs` keeps an
//! identical copy, and the two must change together.

pub use pdf_extract_core::fuzz_api::{
    open_and_count, parse_cmap, text_sampled_pages, unicode_fallbacks,
};

use pdf_extract_core::Document;
use pdf_extract_core::fuzz_api::{FUZZ_MAX_RENDER_PIXELS, FUZZ_RENDER_DPI, fuzz_open_options};
use pdf_extract_render::{RenderOptions, render_gray};

/// The `render` target: page 0 at 36 dpi under the fuzzing limits, with
/// annotations drawn (the renderer's widest path).
pub fn render_page0(bytes: Vec<u8>) {
    let Ok(doc) = Document::from_bytes(bytes, &fuzz_open_options()) else {
        return;
    };
    let Ok(page) = doc.page(0) else {
        return;
    };
    let opts = RenderOptions {
        dpi: FUZZ_RENDER_DPI,
        max_render_pixels: FUZZ_MAX_RENDER_PIXELS,
        render_annotations: true,
    };
    let _ = render_gray(&page, &opts);
}
