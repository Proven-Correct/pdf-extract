//! Unit tests of the crate's small pure pieces: the error mapping and the
//! default options.

use std::time::Duration;

use pdf_extract_core::{LimitKind, Limits, PageError};

use super::{GrayImage, RenderError, RenderOptions, time_limit};

#[test]
fn page_errors_map_to_render_errors() {
    assert!(matches!(
        RenderError::from(PageError::Closed),
        RenderError::Closed
    ));
    assert!(matches!(
        RenderError::from(PageError::IndexOutOfRange { index: 3, count: 1 }),
        RenderError::Closed
    ));
    assert!(matches!(
        RenderError::from(PageError::Limit {
            kind: LimitKind::OpsPerPage,
            value: 7
        }),
        RenderError::Limit {
            kind: LimitKind::OpsPerPage,
            value: 7
        }
    ));
}

#[test]
fn the_time_limit_reports_milliseconds() {
    let limits = Limits {
        page_time_budget: Duration::from_millis(1500),
        ..Limits::default()
    };
    assert_eq!(
        time_limit(&limits).to_string(),
        "limit page_time_budget exceeded (1500)"
    );
    let limits = Limits {
        page_time_budget: Duration::MAX,
        ..Limits::default()
    };
    assert!(matches!(
        time_limit(&limits),
        RenderError::Limit {
            value: u64::MAX,
            ..
        }
    ));
}

#[test]
fn defaults_are_the_ocr_engine_defaults() {
    let d = RenderOptions::default();
    assert_eq!(
        (d.dpi, d.max_render_pixels, d.render_annotations),
        (300, 40_000_000, true)
    );
}

#[test]
fn white_images_are_white() {
    let w = GrayImage::white(3, 2);
    assert_eq!((w.width, w.height, w.data), (3, 2, vec![255; 6]));
}
