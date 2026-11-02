//! Hostile input through the renderer (spec 0002 §4.7 "Hostile", §2.3 items
//! 1 and 2). Every render runs on a 512 KiB thread stack at 36 dpi and must
//! either succeed or end in `RenderError::Limit`: no panic, no stack
//! overflow.
//!
//! The renderer, unlike the text device, runs Type 3 glyph procedures,
//! tiling patterns and soft masks, so the fork's 50-level nesting cap on
//! those three self-recursive kinds (ADR 29) is exercised here by the device
//! it was written for: each completes with an `InterpreterNesting` warning.

mod common;

use common::{kinds, manifest_fixtures, on_small_stack, options, raw_pdf, render_with};
use pdf_extract_core::{Document, Limits, OpenOptions, WarningKind};
use pdf_extract_render::{RenderError, Rendered, render_gray};
use pdf_extract_testkit::hostile::{self, TreeShape};
use pdf_extract_testkit::{FontDict, PdfBuilder, Type3Font, Type3Glyph, jbig2, jpx};

/// Renders every page of `bytes` at 36 dpi on a small stack. `None` means the
/// document did not open (opening is the text path's contract, so that is
/// not a render failure); `Some` carries the outcome of each page, and is
/// empty for a document that opens with no pages.
fn render_all(bytes: Vec<u8>, max_pages: usize) -> Option<Vec<Result<Rendered, RenderError>>> {
    on_small_stack(move || {
        let doc = Document::from_bytes(bytes, &OpenOptions::default()).ok()?;
        Some(
            doc.pages()
                .take(max_pages)
                .map(|page| render_gray(&page, &options(36)))
                .collect(),
        )
    })
}

fn assert_contained(name: &str, outcomes: &[Result<Rendered, RenderError>]) {
    for (i, outcome) in outcomes.iter().enumerate() {
        match outcome {
            Ok(r) => assert!(r.image.width >= 1 && r.image.height >= 1, "{name} p{i}"),
            Err(RenderError::Limit { .. }) => {}
            Err(e) => panic!("{name} page {i}: {e:?}"),
        }
    }
}

#[test]
fn committed_hostile_fixtures_render_or_hit_a_limit() {
    let paths = manifest_fixtures("hostile");
    // Run 0001's hostile set (Appendix C); a truncated manifest must not
    // pass vacuously.
    assert!(paths.len() >= 9, "{} hostile fixtures", paths.len());
    for path in paths {
        let bytes = std::fs::read(&path).expect("fixture reads");
        let outcomes = render_all(bytes, usize::MAX).unwrap_or_default();
        assert_contained(&path.display().to_string(), &outcomes);
    }
}

#[test]
fn runtime_hostile_fixtures_render_or_hit_a_limit() {
    // The "runtime" rows of run 0001 Appendix C, generated rather than
    // committed. Page trees render their first page only.
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("deep_obj_200k", hostile::deep_nesting_in_page_dict(200_000)),
        (
            "deep_content_200k",
            hostile::deep_nesting_in_content(200_000),
        ),
        (
            "deep_resources_200k",
            hostile::deep_nesting_in_resources(200_000),
        ),
        (
            "deep_type4_200k",
            hostile::deep_nesting_in_type4_function(200_000),
        ),
        ("deep_tree_3000", hostile::deep_page_tree(3000)),
        ("flate_bomb_50mib", hostile::flate_bomb(50 << 20)),
        ("many_ops_2m", hostile::operator_flood(2_000_000)),
        ("form_self_recursion", hostile::form_self_recursion()),
        ("nested_forms_60", hostile::nested_forms(60)),
        // Run 0003 H14: a JBIG2 image whose decoder would copy 512 GiB.
        ("jbig2_symbol_export", jbig2::symbol_export_bomb_pdf()),
        // Run 0003 H15: a JPEG 2000 image with 3.6 billion tile records.
        ("jpx_tile_bomb", jpx::tile_bomb_pdf()),
        // internal #63: a JPEG 2000 palette image of two components, which the
        // decoder wrote past its one-channel output.
        (
            "jpx_palette_2_components",
            jpx::image_pdf(&jpx::jp2_palette(2, 8), 8, 8),
        ),
        (
            "tree_100k",
            hostile::page_tree(TreeShape::Fanout {
                leaves_per_node: 47,
                levels: 3,
            }),
        ),
        ("tree_cycle", hostile::page_tree(TreeShape::Cycle)),
        (
            "tree_dup_leaf",
            hostile::page_tree(TreeShape::DuplicateLeaf),
        ),
        (
            "tree_lying_count",
            hostile::page_tree(TreeShape::LyingCount {
                claimed: usize::MAX >> 1,
            }),
        ),
        (
            "tree_typeless_leaf",
            hostile::page_tree(TreeShape::TypelessLeaf),
        ),
        (
            "tree_direct_kid",
            hostile::page_tree(TreeShape::DirectDictKid),
        ),
    ];
    let mut unopened = Vec::new();
    let mut no_pages = Vec::new();
    for (name, bytes) in cases {
        match render_all(bytes, 2) {
            Some(outcomes) => {
                if outcomes.is_empty() {
                    no_pages.push(name);
                }
                assert_contained(name, &outcomes);
            }
            None => unopened.push(name),
        }
    }
    // The measured outcome of the 18 cases, pinned exactly so one more case
    // dropping out of the renderer's reach fails here instead of thinning
    // the test (a bare count threshold left four cases of slack). Opening
    // and the page-tree walk are parsing, not rasterisation, so the result
    // does not depend on the OS or the SIMD kernels `render-cross-os` is
    // concerned with. All 18 open.
    assert!(
        unopened.is_empty(),
        "documents that did not open: {unopened:?}"
    );
    // Two open but yield no page to render (`deep_obj_200k` loses its only
    // page dictionary; `deep_tree_3000` is deeper than the page-tree depth
    // limit); every other case renders at least one page.
    assert_eq!(
        no_pages,
        ["deep_obj_200k", "deep_tree_3000"],
        "documents that opened with no pages"
    );
}

#[test]
fn form_fanout_22_is_stopped_by_the_operator_budget() {
    // 2^22 leaf draws, about 25M operators: over the 10M default operator
    // budget. Each leaf draws a glyph, so in a debug build the renderer can
    // reach the default 30 s deadline first; a 3 s deadline keeps the test
    // short, and whichever budget ends it is a `Limit`.
    let limits = Limits {
        page_time_budget: std::time::Duration::from_secs(3),
        ..Limits::default()
    };
    let outcome =
        on_small_stack(move || render_with(hostile::form_fanout(22), limits, &options(36)));
    assert!(
        matches!(outcome, Err(RenderError::Limit { .. })),
        "{outcome:?}"
    );
}

#[test]
fn hostile_warnings_reach_the_render() {
    let r = on_small_stack(|| {
        render_with(
            hostile::form_self_recursion(),
            Limits::default(),
            &options(36),
        )
    })
    .expect("renders");
    assert!(
        kinds(&r).contains(&WarningKind::FormXObjectCycle),
        "{:?}",
        r.warnings
    );
    let r =
        on_small_stack(|| render_with(hostile::nested_forms(60), Limits::default(), &options(36)))
            .expect("renders");
    assert!(
        kinds(&r).contains(&WarningKind::InterpreterNesting),
        "{:?}",
        r.warnings
    );
    let r = on_small_stack(|| {
        render_with(
            hostile::deep_nesting_in_content(200_000),
            Limits::default(),
            &options(36),
        )
    })
    .expect("renders");
    assert!(
        kinds(&r).contains(&WarningKind::ObjectDepth),
        "{:?}",
        r.warnings
    );
}

#[test]
fn a_jpx_palette_image_of_two_components_is_skipped_as_a_decode_failure() {
    // internal #63: the codec refuses the image instead of panicking; the page
    // renders without it and says why.
    let r = on_small_stack(|| {
        render_with(
            jpx::image_pdf(&jpx::jp2_palette(2, 8), 8, 8),
            Limits::default(),
            &options(36),
        )
    })
    .expect("renders");
    assert_eq!(
        kinds(&r),
        [WarningKind::ImageDecodeFailure],
        "{:?}",
        r.warnings
    );
}

/// Renders `bytes` at 36 dpi on a small stack and asserts it completes with
/// an `InterpreterNesting` warning (the 50-level cap was reached).
fn assert_capped(name: &str, bytes: Vec<u8>) -> Rendered {
    let r = on_small_stack(move || render_with(bytes, Limits::default(), &options(36)))
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    assert!(
        kinds(&r).contains(&WarningKind::InterpreterNesting),
        "{name}: {:?}",
        r.warnings
    );
    r
}

#[test]
fn a_self_referential_type3_glyph_stops_at_fifty_levels() {
    // Glyph `a` fills a small square and then shows `a` again.
    let font = FontDict::Type3(Type3Font {
        font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        font_bbox: [0.0, 0.0, 1000.0, 1000.0],
        glyphs: vec![Type3Glyph {
            code: b'a',
            name: "a".to_owned(),
            width: 1000.0,
            procedure: "1000 0 d0 0 0 1000 1000 re f BT /T3 1 Tf (a) Tj ET".to_owned(),
        }],
        to_unicode: None,
    });
    let mut doc = PdfBuilder::new();
    doc.add_page_a4()
        .add_font("T3", font)
        .raw_content("BT /T3 100 Tf 100 100 Td (a) Tj ET");
    let r = assert_capped("type3", doc.to_bytes());
    // The outermost glyph ran: its square (100 pt at 36 dpi = 50 px) is dark.
    assert!(common::dark_in(&r.image, 52, 345, 98, 370) > 0);
}

#[test]
fn a_self_filling_tiling_pattern_stops_at_fifty_levels() {
    // Pattern P0 fills its cell with P0 again.
    let paint = "/Pattern cs /P0 scn 0 0 10 10 re f";
    let pdf = raw_pdf(&[
        "<< /Pattern << /P0 6 0 R >> >>".to_owned(),
        stream("", paint),
        stream(
            "/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 \
             /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources 4 0 R",
            paint,
        ),
    ]);
    assert_capped("tiling pattern", pdf);
}

#[test]
fn a_self_referencing_soft_mask_stops_at_fifty_levels() {
    // ExtGState GS0 installs a luminosity mask whose group applies GS0
    // again before filling.
    let content = "/GS0 gs 0 0 10 10 re f";
    let pdf = raw_pdf(&[
        "<< /ExtGState << /GS0 6 0 R >> >>".to_owned(),
        stream("", content),
        "<< /Type /ExtGState /SMask << /S /Luminosity /G 7 0 R >> >>".to_owned(),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 200 200] \
             /Group << /S /Transparency /CS /DeviceGray >> /Resources 4 0 R",
            content,
        ),
    ]);
    assert_capped("soft mask", pdf);
}

/// A stream object body: `<< dict /Length n >> stream … endstream`.
fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len() + 1
    )
}
