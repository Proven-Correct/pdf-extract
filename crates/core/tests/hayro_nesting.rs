//! Tests of fork change H3, part two: the 50-level nested-interpretation cap
//! and its counter (spec §3.2, §2.3 item 2, D-33; oracle §4.6).
//!
//! Upstream hayro checks its cap of 50 only when drawing a Form XObject. Type 3
//! glyphs, tiling patterns and soft masks record a nesting depth but never
//! check it, so a Type 3 font whose glyph procedure shows its own glyph
//! recurses until the stack overflows under any device that runs glyph
//! procedures (hayro's renderer does). H3 applies the cap to all three and
//! counts every refusal in `InterpretStats::nesting_cap_hits`.
//!
//! Every case runs on a 512 KiB stack. The device here runs nested content
//! (`run_nested`), unlike the text device.

mod common;

use std::process::Command;

use common::{Run, raw_pdf, run_on_small_stack, stream};
use pdf_extract_testkit::hostile;
use pdf_extract_testkit::{FontDict, PdfBuilder, Type3Font, Type3Glyph};

fn nested() -> Run {
    Run {
        run_nested: true,
        ..Run::default()
    }
}

#[test]
fn nested_forms_below_the_cap_are_all_drawn() {
    let out = run_on_small_stack(hostile::nested_forms(49), Run::default());
    let expected: Vec<String> = (1..=49).map(|i| format!("L{i}")).collect();
    assert_eq!(out.lines(), expected);
    assert_eq!(out.stats.nesting_cap_hits, 0);
}

#[test]
fn nested_forms_stop_at_fifty_levels() {
    // KP21 divergence: the oracle extracts all 60 levels; hayro's cap keeps
    // L1..L50 and refuses F51 once (its descendants are never reached).
    let out = run_on_small_stack(hostile::nested_forms(60), Run::default());
    let expected: Vec<String> = (1..=50).map(|i| format!("L{i}")).collect();
    assert_eq!(out.lines(), expected);
    assert_eq!(out.stats.nesting_cap_hits, 1);
    assert_eq!(out.stats.exhausted, None);
}

/// A page showing glyph `a` of a Type 3 font whose procedure fills a small
/// rectangle (one `draw_path` per procedure run) and then shows `a` in the
/// same font again.
fn type3_self_recursion() -> Vec<u8> {
    let font = FontDict::Type3(Type3Font {
        font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        font_bbox: [0.0, 0.0, 1000.0, 1000.0],
        glyphs: vec![Type3Glyph {
            code: b'a',
            name: "a".to_owned(),
            width: 1000.0,
            procedure: "1000 0 d0 0 0 1 1 re f BT /T3 1 Tf (a) Tj ET".to_owned(),
        }],
        to_unicode: None,
    });
    let mut doc = PdfBuilder::new();
    doc.add_page_a4()
        .add_font("T3", font)
        .raw_content("BT /T3 1 Tf (a) Tj ET");
    doc.to_bytes()
}

#[test]
fn self_referential_type3_glyph_stops_at_the_cap() {
    let out = run_on_small_stack(type3_self_recursion(), nested());
    // Glyph procedures run at nesting depths 1..=50, as forms do; the glyph
    // shown at depth 50 reaches the device but its procedure is refused.
    assert_eq!(out.paths, 50);
    assert_eq!(out.text.chars().count(), 51);
    assert_eq!(out.stats.nesting_cap_hits, 1);
    assert_eq!(out.stats.exhausted, None);
}

/// Set by [`type3_recursion_does_not_abort`] for its child process.
const CHILD_ENV: &str = "PDF_EXTRACT_TEST_TYPE3_CHILD";

/// Runs only inside the child process of [`type3_recursion_does_not_abort`].
#[test]
fn type3_recursion_child() {
    if std::env::var_os(CHILD_ENV).is_some() {
        run_on_small_stack(type3_self_recursion(), nested());
    }
}

#[test]
fn type3_recursion_does_not_abort() {
    // Regression test for the stack overflow H3 fixes: before H3 the child
    // dies of SIGABRT ("has overflowed its stack"), which in-process would
    // take the whole test binary down rather than fail one test.
    let status = Command::new(std::env::current_exe().expect("test binary path"))
        .args(["--exact", "type3_recursion_child", "--test-threads=1"])
        .env(CHILD_ENV, "1")
        .status()
        .expect("run child");
    assert!(status.success(), "child exited with {status}");
}

#[test]
fn self_filling_tiling_pattern_stops_at_the_cap() {
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
    let out = run_on_small_stack(pdf, nested());
    // The page's fill plus one per pattern level that ran (depths 1..=50).
    assert_eq!(out.paths, 51);
    assert!(out.stats.nesting_cap_hits >= 1);
    assert_eq!(out.stats.exhausted, None);
}

#[test]
fn self_referencing_soft_mask_stops_at_the_cap() {
    // ExtGState GS0 installs a luminosity mask whose group form applies GS0
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
    let out = run_on_small_stack(pdf, nested());
    // Each mask level runs its group as a form, so the chain uses two levels
    // of depth per mask and ends at whichever check it reaches first.
    assert!(out.paths > 1, "the masks ran ({} paths)", out.paths);
    assert!(out.stats.nesting_cap_hits >= 1);
    assert_eq!(out.stats.exhausted, None);
}

#[test]
fn nested_content_is_not_run_without_a_device_that_asks() {
    // The cap is only reached through a device that runs glyph procedures:
    // with the text-only device behaviour the same font is harmless.
    let out = run_on_small_stack(type3_self_recursion(), Run::default());
    assert_eq!(out.paths, 0);
    assert_eq!(out.text, "\u{FFFD}");
    assert_eq!(out.stats.nesting_cap_hits, 0);
}
