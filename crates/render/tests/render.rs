//! `render_gray` on builder pages (spec 0002 §4.7, first three bullets):
//! raster dimensions, rotation, crop, background and fill, the resolution
//! clamp, degenerate boxes, budgets, the closed document, and pages that a
//! lazy xref repair after open changed.

mod common;

use std::time::{Duration, Instant};

use common::{at, dark_in, kinds, open, options, render_at, render_with};
use pdf_extract_core::{LimitKind, Limits, WarningKind};
use pdf_extract_render::{RenderError, RenderOptions, render_gray};
use pdf_extract_testkit::contract::{self, Redefined};
use pdf_extract_testkit::{Annotation, PdfBuilder, TextStyle, hostile};

fn page(width: f32, height: f32, content: &str) -> Vec<u8> {
    let mut doc = PdfBuilder::new();
    doc.add_page(width, height).raw_content(content);
    doc.to_bytes()
}

fn a4(content: &str) -> Vec<u8> {
    page(595.0, 842.0, content)
}

#[test]
fn a4_and_letter_at_72_dpi() {
    let r = render_at(a4(""), 72);
    assert_eq!((r.image.width, r.image.height, r.dpi), (595, 842, 72));
    assert_eq!(r.image.data.len(), 595 * 842);
    let r = render_at(page(612.0, 792.0, ""), 72);
    assert_eq!((r.image.width, r.image.height), (612, 792));
}

#[test]
fn a4_at_300_dpi_rounds_each_side_up() {
    // O1: ceil(595 × 300 / 72) × ceil(842 × 300 / 72).
    let r = render_at(a4(""), 300);
    assert_eq!((r.image.width, r.image.height, r.dpi), (2480, 3509, 300));
}

#[test]
fn a_rotated_page_renders_landscape() {
    for degrees in [90, 270, -90] {
        let mut doc = PdfBuilder::new();
        doc.add_page_a4().set_rotation(degrees);
        let r = render_at(doc.to_bytes(), 72);
        assert_eq!(
            (r.image.width, r.image.height),
            (842, 595),
            "/Rotate {degrees}"
        );
    }
    let mut doc = PdfBuilder::new();
    doc.add_page_a4().set_rotation(180);
    let r = render_at(doc.to_bytes(), 72);
    assert_eq!((r.image.width, r.image.height), (595, 842));
}

#[test]
fn rotation_turns_the_content_too() {
    // A black square at the bottom-left corner of the unrotated page; /Rotate
    // 90 turns the page clockwise, so the corner moves to the top left.
    let mut doc = PdfBuilder::new();
    doc.add_page_a4()
        .raw_content("0 g 0 0 50 50 re f")
        .set_rotation(90);
    let r = render_at(doc.to_bytes(), 72);
    assert_eq!(at(&r.image, 10, 10), 0);
    assert_eq!(at(&r.image, 830, 585), 255);
}

#[test]
fn a_crop_box_page_has_the_crop_dimensions() {
    let mut doc = PdfBuilder::new();
    doc.add_page_a4()
        .raw_content("0 g 100 100 50 50 re f")
        .set_crop_box([100.0, 100.0, 400.0, 300.0]);
    let r = render_at(doc.to_bytes(), 72);
    assert_eq!((r.image.width, r.image.height), (300, 200));
    // The crop box's origin is the raster's bottom-left corner.
    assert_eq!(at(&r.image, 10, 190), 0);
    assert_eq!(at(&r.image, 100, 100), 255);
}

#[test]
fn a_crop_box_beyond_the_media_box_is_intersected() {
    let mut doc = PdfBuilder::new();
    doc.add_page_a4().set_crop_box([500.0, 800.0, 700.0, 900.0]);
    let r = render_at(doc.to_bytes(), 72);
    assert_eq!((r.image.width, r.image.height), (95, 42));
}

#[test]
fn a_blank_page_is_all_white() {
    let r = render_at(a4(""), 72);
    assert!(r.image.data.iter().all(|&v| v == 255));
    assert!(r.warnings.is_empty());
}

#[test]
fn a_black_rectangle_is_black_inside_and_white_outside() {
    // PDF y-up: x 100..300, y 100..300 is rows 542..742 at 72 dpi.
    let r = render_at(a4("0 0 0 rg 100 100 200 200 re f"), 72);
    let img = &r.image;
    for y in 0..842 {
        for x in 0..595 {
            let v = at(img, x, y);
            let inside = (100..300).contains(&x) && (542..742).contains(&y);
            let edge = [99, 100, 299, 300].contains(&x) || [541, 542, 741, 742].contains(&y);
            if edge {
                continue;
            }
            if inside {
                assert!(v <= 2, "({x}, {y}) = {v}");
            } else {
                assert!(v >= 253, "({x}, {y}) = {v}");
            }
        }
    }
}

#[test]
fn text_is_drawn() {
    let mut doc = PdfBuilder::new();
    doc.add_page_a4()
        .insert_text((72.0, 100.0), "Hello", TextStyle::size(24.0))
        .expect("text");
    let r = render_at(doc.to_bytes(), 72);
    assert!(dark_in(&r.image, 60, 60, 200, 110) > 50);
}

#[test]
fn max_render_pixels_lowers_the_resolution() {
    let opts = RenderOptions {
        dpi: 300,
        max_render_pixels: 1_000_000,
        ..RenderOptions::default()
    };
    let r = render_with(a4(""), Limits::default(), &opts).expect("renders");
    // O2: floor(sqrt(1e6 × 72² / (595 × 842))) = 101.
    assert_eq!(r.dpi, 101);
    assert_eq!((r.image.width, r.image.height), (835, 1182));
    assert!(r.warnings.is_empty(), "lowering is silent");
}

#[test]
fn the_largest_page_is_clamped_before_u16() {
    let r = render_at(page(14_400.0, 14_400.0, "0 g 0 0 14400 14400 re f"), 300);
    assert_eq!((r.dpi, r.image.width, r.image.height), (31, 6200, 6200));
    assert!(r.image.data.iter().all(|&v| v == 0));
}

#[test]
fn degenerate_boxes_render_one_white_pixel() {
    // Empty intersection of crop and media box.
    let mut doc = PdfBuilder::new();
    doc.add_page_a4()
        .set_crop_box([1000.0, 1000.0, 1100.0, 1100.0]);
    // Zero-area and oversized media boxes.
    for bytes in [
        doc.to_bytes(),
        page(0.0, 0.0, "0 g 0 0 10 10 re f"),
        page(595.0, 0.0, ""),
        page(14_401.0, 100.0, "0 g 0 0 10 10 re f"),
    ] {
        let r = render_at(bytes, 300);
        assert_eq!((r.image.width, r.image.height), (1, 1));
        assert_eq!(r.image.data, [255]);
        assert_eq!(r.dpi, 300);
    }
}

/// Internal issue #56 B20, render twin of the text path's empty page: once a lazy
/// xref repair (forced here by rendering the first page, whose content
/// stream's xref offset is wrong) has replaced the second leaf with a
/// non-dictionary, that page renders as one white pixel at the requested
/// resolution with a `PageTreeNodeUnreadable` warning, not an error.
/// Rendered before the repair, the same page is normal. Pins current, not
/// desired, behaviour (see the module doc of
/// `crates/core/tests/repair_after_open.rs`).
#[test]
fn a_leaf_replaced_by_a_repair_after_open_renders_one_white_pixel() {
    let bytes = contract::redefined_after_open(Redefined::SecondPage);
    let doc = open(bytes.clone(), Limits::default());
    let second = render_gray(&doc.page(1).expect("page"), &options(72)).expect("renders");
    assert_eq!((second.image.width, second.image.height), (300, 400));

    let doc = open(bytes, Limits::default());
    let first = render_gray(&doc.page(0).expect("page"), &options(72)).expect("renders");
    assert_eq!((first.image.width, first.image.height), (300, 400));
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    assert!(
        doc.warnings()
            .iter()
            .any(|w| w.kind == WarningKind::XrefRepaired)
    );

    let second = render_gray(&doc.page(1).expect("page"), &options(72)).expect("renders");
    assert_eq!((second.image.width, second.image.height), (1, 1));
    assert_eq!(second.image.data, [255]);
    assert_eq!(second.dpi, 72);
    assert_eq!(kinds(&second), [WarningKind::PageTreeNodeUnreadable]);
    assert_eq!(second.warnings[0].page, Some(1));
}

/// Internal issue #56 B21, characterization: once the repair has replaced the
/// catalog, the inline page-tree root's inherited `/MediaBox` is silently
/// gone, so the second page renders at the default page size (A4 here) and
/// draws the text that the inherited box keeps off the first page. Rendered
/// before the repair, the same page has the inherited box. Pins current, not
/// desired, behaviour (see the module doc of
/// `crates/core/tests/repair_after_open.rs`).
#[test]
fn a_catalog_replaced_by_a_repair_after_open_drops_the_inherited_media_box() {
    let bytes = contract::redefined_after_open(Redefined::Catalog);
    let doc = open(bytes.clone(), Limits::default());
    let second = render_gray(&doc.page(1).expect("page"), &options(72)).expect("renders");
    assert_eq!((second.image.width, second.image.height), (300, 400));

    let doc = open(bytes, Limits::default());
    let first = render_gray(&doc.page(0).expect("page"), &options(72)).expect("renders");
    assert_eq!((first.image.width, first.image.height), (300, 400));
    let second = render_gray(&doc.page(1).expect("page"), &options(72)).expect("renders");
    assert_eq!((second.image.width, second.image.height), (596, 842));
    assert!(second.warnings.is_empty(), "{:?}", second.warnings);
    // REDEFINED_BEYOND_TEXT's baseline is at y = 600, so 242 rows down.
    assert!(dark_in(&second.image, 15, 225, 80, 250) > 0);
}

#[test]
fn dpi_zero_is_one() {
    let r = render_at(a4(""), 0);
    assert_eq!((r.dpi, r.image.width, r.image.height), (1, 9, 12));
}

#[test]
fn the_operator_budget_is_a_limit_error() {
    let limits = Limits {
        max_ops_per_page: 10_000,
        ..Limits::default()
    };
    let err = render_with(hostile::operator_flood(100_000), limits, &options(72))
        .expect_err("over budget");
    assert!(
        matches!(
            err,
            RenderError::Limit {
                kind: LimitKind::OpsPerPage,
                value: 10_000
            }
        ),
        "{err:?}"
    );
    assert_eq!(err.to_string(), "limit ops_per_page exceeded (10000)");
}

#[test]
fn the_time_budget_is_a_limit_error_within_a_second() {
    let limits = Limits {
        page_time_budget: Duration::from_millis(200),
        ..Limits::default()
    };
    let doc = open(hostile::form_fanout(22), limits);
    let page = doc.page(0).expect("page 0");
    let start = Instant::now();
    let err = render_gray(&page, &options(72)).expect_err("over budget");
    let elapsed = start.elapsed();
    assert!(
        matches!(
            err,
            RenderError::Limit {
                kind: LimitKind::PageTimeBudget,
                value: 200
            }
        ),
        "{err:?}"
    );
    assert!(elapsed < Duration::from_secs(1), "took {elapsed:?}");
}

#[test]
fn within_budget_the_page_renders() {
    let r = render_at(hostile::operator_flood(1_000), 72);
    assert!(
        dark_in(&r.image, 60, 100, 200, 130) > 0,
        "Start and End drawn"
    );
}

#[test]
fn a_closed_document_is_an_error() {
    let doc = open(a4(""), Limits::default());
    let page = doc.page(0).expect("page 0");
    doc.close();
    let err = render_gray(&page, &options(72)).expect_err("closed");
    assert!(matches!(err, RenderError::Closed));
    assert_eq!(err.to_string(), "the document is closed");
}

#[test]
fn annotations_follow_the_visibility_filter() {
    let square = "0 g 0 0 100 100 re f";
    let mut doc = PdfBuilder::new();
    doc.add_page_a4()
        .add_annotation(
            Annotation::new("Stamp", [100.0, 600.0, 200.0, 700.0]).with_appearance(square),
        )
        // Hidden (flag 2): never drawn.
        .add_annotation(
            Annotation::new("Stamp", [300.0, 600.0, 400.0, 700.0])
                .with_appearance(square)
                .with_flags(2),
        );
    let bytes = doc.to_bytes();
    let r = render_at(bytes.clone(), 72);
    // PDF y 600..700 is rows 142..242.
    assert!(
        dark_in(&r.image, 110, 150, 190, 230) > 6000,
        "visible stamp drawn"
    );
    assert_eq!(
        dark_in(&r.image, 300, 142, 400, 242),
        0,
        "hidden stamp skipped"
    );
    let off = RenderOptions {
        render_annotations: false,
        ..options(72)
    };
    let r = render_with(bytes, Limits::default(), &off).expect("renders");
    assert!(
        r.image.data.iter().all(|&v| v == 255),
        "no annotations drawn"
    );
}

#[test]
fn rendering_twice_gives_identical_bytes() {
    let mut doc = PdfBuilder::new();
    let p = doc.add_page_a4();
    p.insert_text((72.0, 100.0), "Determinism", TextStyle::size(18.0))
        .expect("text");
    p.raw_content("0.3 g 50 50 400 300 re f");
    let bytes = doc.to_bytes();
    let a = render_at(bytes.clone(), 150);
    let b = render_at(bytes, 150);
    assert_eq!(a, b);
    assert!(kinds(&a).is_empty());
}
