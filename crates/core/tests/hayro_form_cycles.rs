//! Tests of fork change H3b: Form XObject cycle detection (spec §3.2, oracle
//! §4.6).
//!
//! Upstream hayro stops a form that draws itself only at its nesting cap, so
//! the form's content appears 50 times; the oracle shows it once. H3b keeps the
//! forms being drawn on a stack in the `Context`, skips a form that is already
//! on it, and counts the skip in `InterpretStats::form_cycle_hits`. A form drawn
//! several times side by side is not a cycle and must still be drawn each time.
//!
//! Every case runs on a 512 KiB stack.

mod common;

use common::{Run, raw_pdf, run_on_small_stack, stream};
use pdf_extract_testkit::hostile;

const HELV: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

#[test]
fn self_drawing_form_is_drawn_once() {
    let out = run_on_small_stack(hostile::form_self_recursion(), Run::default());
    assert_eq!(out.text.matches("Rec").count(), 1, "text: {:?}", out.text);
    assert_eq!(out.stats.form_cycle_hits, 1);
    assert_eq!(out.stats.nesting_cap_hits, 0);
}

#[test]
fn mutually_recursive_forms_are_each_drawn_once() {
    // A shows `A` and draws B one line down; B shows `B` and draws A.
    let form = |text: &str, next: &str| {
        stream(
            "/Type /XObject /Subtype /Form /BBox [-1000 -1000 1000 1000] /Resources 4 0 R",
            &format!("BT /Helv 11 Tf 0 0 Td ({text}) Tj ET q 1 0 0 1 0 -12 cm /{next} Do Q"),
        )
    };
    let pdf = raw_pdf(&[
        "<< /Font << /Helv 8 0 R >> /XObject << /A 6 0 R /B 7 0 R >> >>".to_owned(),
        stream("", "q 1 0 0 1 20 150 cm /A Do Q"),
        form("A", "B"),
        form("B", "A"),
        HELV.to_owned(),
    ]);
    let out = run_on_small_stack(pdf, Run::default());
    assert_eq!(out.lines(), ["A", "B"]);
    assert_eq!(out.stats.form_cycle_hits, 1);
    assert_eq!(out.stats.nesting_cap_hits, 0);
}

#[test]
fn repeated_forms_are_not_cycles() {
    // form_fanout(3): each level draws the next form twice, so the leaf form
    // is drawn 8 times; none of those draws is nested in itself.
    let out = run_on_small_stack(hostile::form_fanout(3), Run::default());
    assert_eq!(out.text.matches("Leaf").count(), 8);
    assert_eq!(out.stats.form_cycle_hits, 0);
}

#[test]
fn form_drawn_again_after_it_finished_is_not_a_cycle() {
    // The page draws F twice in sequence; the stack must be popped in between.
    let pdf = raw_pdf(&[
        "<< /Font << /Helv 7 0 R >> /XObject << /F 6 0 R >> >>".to_owned(),
        stream(
            "",
            "q 1 0 0 1 20 150 cm /F Do Q q 1 0 0 1 20 100 cm /F Do Q",
        ),
        stream(
            "/Type /XObject /Subtype /Form /BBox [-1000 -1000 1000 1000] /Resources 4 0 R",
            "BT /Helv 11 Tf 0 0 Td (F) Tj ET",
        ),
        HELV.to_owned(),
    ]);
    let out = run_on_small_stack(pdf, Run::default());
    assert_eq!(out.lines(), ["F", "F"]);
    assert_eq!(out.stats.form_cycle_hits, 0);
}
