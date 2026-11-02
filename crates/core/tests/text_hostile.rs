//! Hostile-input bullets of spec §4.10 (moved there from §4.4 and §4.6 by
//! revision 2), asserted through the public text API: the extracted text,
//! the page's warning kinds, and the `PageError::Limit` outcomes. Every case
//! runs on a 512 KiB thread stack (spec §2.3).
//!
//! The fork-level counterparts (hayro's own observables for the same
//! fixtures) live in `h2a_object_depth.rs`, `h2b_decode_limits.rs`,
//! `hayro_budget.rs`, `hayro_form_cycles.rs` and `hayro_nesting.rs`.

mod common;

use std::time::{Duration, Instant};

use common::on_small_stack;
use pdf_extract_core::{
    Document, LimitKind, Limits, OpenOptions, PageError, TextOptions, TextPage, WarningKind,
};
use pdf_extract_testkit::hostile;

/// Extracts page 0 of `bytes` opened with `limits`.
fn extract(bytes: Vec<u8>, limits: Limits) -> Result<TextPage, PageError> {
    let opts = OpenOptions {
        limits,
        ..OpenOptions::default()
    };
    let doc = Document::from_bytes(bytes, &opts).expect("fixture opens");
    doc.page(0)?.text_page(&TextOptions::default())
}

/// `(text, warning kinds)` of page 0, which must extract.
fn outcome(bytes: Vec<u8>, limits: Limits) -> (String, Vec<WarningKind>) {
    let tp = extract(bytes, limits).expect("page extracts");
    let kinds = tp.warnings().iter().map(|w| w.kind).collect();
    (tp.to_text(Default::default()), kinds)
}

#[test]
fn deep_nesting_in_content_keeps_the_text_before_it() {
    // The over-deep operand ends the stream: `After` is lost, as in hayro
    // and the reference.
    let (text, kinds) =
        on_small_stack(|| outcome(hostile::deep_nesting_in_content(200_000), Limits::default()));
    assert_eq!(text, "Before\n");
    assert_eq!(kinds, [WarningKind::ObjectDepth]);
}

#[test]
fn deep_nesting_in_resources_falls_back_to_helvetica() {
    let (text, kinds) = on_small_stack(|| {
        outcome(
            hostile::deep_nesting_in_resources(200_000),
            Limits::default(),
        )
    });
    assert_eq!(text, "Res\n");
    assert_eq!(kinds, [WarningKind::ObjectDepth]);
}

#[test]
fn deep_nesting_in_resources_also_repairs_the_xref() {
    // Spec §4.10 (revision 10): the unreadable resources object sends hayro
    // into a lazy xref repair. The document reports `XrefRepaired` (T8);
    // the page's own warnings never carry it.
    let (doc_kinds, page_kinds) = on_small_stack(|| {
        let doc = Document::from_bytes(
            hostile::deep_nesting_in_resources(200_000),
            &OpenOptions::default(),
        )
        .expect("opens");
        let tp = doc
            .page(0)
            .expect("page 0")
            .text_page(&TextOptions::default())
            .expect("extracts");
        let doc_kinds: Vec<WarningKind> = doc.warnings().iter().map(|w| w.kind).collect();
        let page_kinds: Vec<WarningKind> = tp.warnings().iter().map(|w| w.kind).collect();
        (doc_kinds, page_kinds)
    });
    assert!(
        doc_kinds.contains(&WarningKind::XrefRepaired),
        "{doc_kinds:?}"
    );
    assert_eq!(page_kinds, [WarningKind::ObjectDepth]);
}

#[test]
fn deep_nesting_in_a_type4_function_keeps_the_text() {
    let (text, kinds) = on_small_stack(|| {
        outcome(
            hostile::deep_nesting_in_type4_function(200_000),
            Limits::default(),
        )
    });
    assert_eq!(text, "Fn\n");
    assert_eq!(kinds, [WarningKind::ObjectDepth]);
}

#[test]
fn flate_bomb_over_the_decode_limit_is_empty() {
    let (text, kinds) = on_small_stack(|| {
        outcome(
            hostile::flate_bomb(50 << 20),
            Limits {
                max_stream_decoded_bytes: 16 << 20,
                ..Limits::default()
            },
        )
    });
    assert_eq!(text, "");
    assert_eq!(kinds, [WarningKind::StreamDecodeLimit]);
}

#[test]
fn form_self_recursion_is_drawn_once() {
    let (text, kinds) =
        on_small_stack(|| outcome(hostile::form_self_recursion(), Limits::default()));
    assert_eq!(text, "Rec\n");
    assert_eq!(kinds, [WarningKind::FormXObjectCycle]);
}

/// `L1\n` … `Ln\n`.
fn levels(n: u32) -> String {
    (1..=n).map(|i| format!("L{i}\n")).collect()
}

#[test]
fn nested_forms_stop_at_fifty_levels() {
    // KP21: the reference extracts all 60 levels; hayro stops at 50.
    let (text, kinds) = on_small_stack(|| outcome(hostile::nested_forms(60), Limits::default()));
    assert_eq!(text, levels(50));
    assert_eq!(kinds, [WarningKind::InterpreterNesting]);
    let (text, kinds) = on_small_stack(|| outcome(hostile::nested_forms(49), Limits::default()));
    assert_eq!(text, levels(49));
    assert!(kinds.is_empty(), "{kinds:?}");
}

#[test]
fn operator_flood_exceeds_ops_per_page() {
    let err = on_small_stack(|| {
        extract(
            hostile::operator_flood(400_000),
            Limits {
                max_ops_per_page: 200_000,
                ..Limits::default()
            },
        )
        .expect_err("over the operator cap")
    });
    assert_eq!(
        err,
        PageError::Limit {
            kind: LimitKind::OpsPerPage,
            value: 200_000
        }
    );
    // `text()` fails the same way.
    let err = on_small_stack(|| {
        let opts = OpenOptions {
            limits: Limits {
                max_ops_per_page: 1_000,
                ..Limits::default()
            },
            ..OpenOptions::default()
        };
        let doc = Document::from_bytes(hostile::operator_flood(10_000), &opts).expect("opens");
        doc.page(0)
            .expect("page")
            .text(&TextOptions::default())
            .expect_err("over the cap")
    });
    assert!(matches!(
        err,
        PageError::Limit {
            kind: LimitKind::OpsPerPage,
            ..
        }
    ));
}

#[test]
fn form_fanout_is_cut_by_the_page_time_budget() {
    // The one wall-clock gate (spec D-32): 500 ms budget, 2 s of slack for
    // the 4,096 operators between deadline checks and CI noise.
    let bytes = hostile::form_fanout(22);
    let (err, elapsed) = on_small_stack(move || {
        let opts = OpenOptions {
            limits: Limits {
                page_time_budget: Duration::from_millis(500),
                ..Limits::default()
            },
            ..OpenOptions::default()
        };
        let doc = Document::from_bytes(bytes, &opts).expect("opens");
        let page = doc.page(0).expect("page");
        let start = Instant::now();
        let err = page
            .text(&TextOptions::default())
            .expect_err("over the time budget");
        (err, start.elapsed())
    });
    assert_eq!(
        err,
        PageError::Limit {
            kind: LimitKind::PageTimeBudget,
            value: 500
        }
    );
    assert!(
        elapsed < Duration::from_millis(2_500),
        "took {elapsed:?} for a 500 ms budget"
    );
    eprintln!("form_fanout(22) stopped after {elapsed:?} (budget 500 ms)");
}
