//! Tests of fork change H3, part one: the interpreter work budget (spec §3.2,
//! oracle §4.6).
//!
//! hayro-interpret has no bound on the work a page can cause: a small file can
//! hold millions of operators, or Form XObjects that draw each other 2^n
//! times. H3 counts every operator, at any nesting level, against the
//! `InterpretBudget` in `InterpreterSettings` (an operator limit plus a
//! deadline checked every `check_interval` operators) and stops interpretation
//! when it runs out. `Context::stats()` reports the outcome.
//!
//! Operator-limit tests are functional only and use small limits: hayro runs
//! about 0.9M operators/s in release and far fewer unoptimised, so a
//! seconds-per-operator claim would be platform-dependent (D-32). The only
//! wall-clock assertions are on the deadline, which is what bounds time; their
//! margins are wide for shared CI runners, and the measured times are printed
//! (`--nocapture`).

mod common;

use std::time::{Duration, Instant};

use common::{Run, run_on_small_stack, run_page, run_page_timed};
use hayro_interpret::{BudgetExhaustion, InterpretBudget, InterpreterSettings};
use hayro_syntax::Pdf;
use pdf_extract_testkit::hostile;
use pdf_extract_testkit::{FontDict, PdfBuilder, Type3Font, Type3Glyph};

fn open(bytes: Vec<u8>) -> Pdf {
    Pdf::new(bytes).expect("fixture opens")
}

fn max_ops(n: u64) -> Run {
    Run {
        budget: InterpretBudget::new(Some(n), None),
        ..Run::default()
    }
}

#[test]
fn default_settings_have_no_limits_and_still_count() {
    // `InterpreterSettings::default()` must keep hayro's behaviour (nothing is
    // cut), while `stats()` works because the budget state always exists.
    let budget = InterpreterSettings::default().budget;
    assert_eq!((budget.max_ops, budget.deadline), (None, None));
    assert_eq!(budget.stats(), Default::default());

    let pdf = open(hostile::operator_flood(1_000));
    let out = run_page(&pdf, &Run::default());
    assert_eq!(out.lines(), ["Start", "End"]);
    assert_eq!(out.stats.exhausted, None);
    assert_eq!(out.stats.ops, 1_010);
    assert_eq!(out.stats.nesting_cap_hits, 0);
}

#[test]
fn operator_flood_is_cut_at_max_ops() {
    // 400,010 operators against a limit of 200,000.
    let out = run_on_small_stack(hostile::operator_flood(400_000), max_ops(200_000));
    assert_eq!(out.stats.exhausted, Some(BudgetExhaustion::Ops));
    // The counter stops at the operator that was refused, well within
    // `check_interval` of the limit.
    assert_eq!(out.stats.ops, 200_001);
    // Glyphs emitted before the cut are kept; nothing after it is drawn.
    assert_eq!(out.lines(), ["Start"]);
}

#[test]
fn operator_flood_completes_without_limits() {
    let out = run_on_small_stack(hostile::operator_flood(400_000), Run::default());
    assert_eq!(out.stats.exhausted, None);
    assert_eq!(out.stats.ops, 400_010);
    assert_eq!(out.lines(), ["Start", "End"]);
}

#[test]
fn limit_equal_to_the_operator_count_lets_every_operator_run() {
    // operator_flood(1000) is `BT Tf Td Tj ET`, 1,000 × `n`, `BT Tf Td Tj ET`:
    // 1,010 operators, the closing `Tj` being number 1,009.
    let pdf = open(hostile::operator_flood(1_000));
    let out = run_page(&pdf, &max_ops(1_010));
    assert_eq!(out.stats.exhausted, None);
    assert_eq!(out.lines(), ["Start", "End"]);

    // One short of the closing `ET` still shows `End`...
    let out = run_page(&pdf, &max_ops(1_009));
    assert_eq!(out.stats.exhausted, Some(BudgetExhaustion::Ops));
    assert_eq!(out.lines(), ["Start", "End"]);

    // ...one short of the closing `Tj` does not.
    let out = run_page(&pdf, &max_ops(1_008));
    assert_eq!(out.stats.exhausted, Some(BudgetExhaustion::Ops));
    assert_eq!(out.lines(), ["Start"]);
}

#[test]
fn form_fanout_is_cut_at_max_ops() {
    // About 25.2M operators reached through 2^22 form draws; the limit must
    // stop a fan-out across nested forms, not just a flat flood.
    let out = run_on_small_stack(hostile::form_fanout(22), max_ops(200_000));
    assert_eq!(out.stats.exhausted, Some(BudgetExhaustion::Ops));
    assert_eq!(out.stats.ops, 200_001);
    let leaves = out.text.matches("Leaf").count();
    assert!(leaves > 0 && leaves < 1 << 22, "{leaves} leaves drawn");
}

#[test]
fn form_fanout_is_stopped_by_the_deadline() {
    // The gating wall-clock test: no operator limit, a 500 ms deadline. The
    // 2 s slack covers the 4,096 operators between deadline checks at
    // unoptimised speed plus CI noise.
    let pdf = open(hostile::form_fanout(22));
    let (out, elapsed) = run_page_timed(&pdf, &Run::default(), |b| {
        b.deadline = Some(Instant::now() + Duration::from_millis(500));
    });
    println!(
        "form_fanout(22), 500 ms deadline: stopped after {elapsed:?}, {} ops",
        out.stats.ops
    );
    assert_eq!(out.stats.exhausted, Some(BudgetExhaustion::Deadline));
    assert!(
        elapsed < Duration::from_millis(2_500),
        "took {elapsed:?}, bound is 500 ms + 2 s"
    );
}

#[test]
fn deadline_stops_an_operator_flood() {
    // 50M operators (100 MB decoded), no operator limit, a 50 ms deadline set
    // when interpretation starts.
    let pdf = open(hostile::operator_flood(50_000_000));
    let (out, elapsed) = run_page_timed(&pdf, &Run::default(), |b| {
        b.deadline = Some(Instant::now() + Duration::from_millis(50));
    });
    println!(
        "operator_flood(50M), 50 ms deadline: stopped after {elapsed:?}, {} ops",
        out.stats.ops
    );
    assert_eq!(out.stats.exhausted, Some(BudgetExhaustion::Deadline));
    assert_eq!(out.lines(), ["Start"]);
    assert!(
        elapsed < Duration::from_millis(300),
        "took {elapsed:?}, bound is 300 ms"
    );
}

#[test]
fn deadline_is_checked_every_check_interval_operators() {
    // A deadline already in the past is only noticed at the first check, so
    // exactly `check_interval` operators are counted (the refused one
    // included).
    let pdf = open(hostile::operator_flood(10_000));
    let mut budget = InterpretBudget::new(None, Some(Instant::now()));
    budget.check_interval = 100;
    let out = run_page(
        &pdf,
        &Run {
            budget,
            ..Run::default()
        },
    );
    assert_eq!(out.stats.exhausted, Some(BudgetExhaustion::Deadline));
    assert_eq!(out.stats.ops, 100);

    // An interval of 0 means "check every operator" rather than dividing by 0.
    let mut budget = InterpretBudget::new(None, Some(Instant::now()));
    budget.check_interval = 0;
    let out = run_page(
        &pdf,
        &Run {
            budget,
            ..Run::default()
        },
    );
    assert_eq!(out.stats.exhausted, Some(BudgetExhaustion::Deadline));
    assert_eq!(out.stats.ops, 1);
    assert_eq!(out.text, "");
}

#[test]
fn clones_share_one_budget_state() {
    // The caller keeps a clone; the interpreter's copy reports into it.
    let budget = InterpretBudget::new(Some(5), None);
    let pdf = open(hostile::operator_flood(100));
    let out = run_page(
        &pdf,
        &Run {
            budget: budget.clone(),
            ..Run::default()
        },
    );
    assert_eq!(budget.stats(), out.stats);
    assert_eq!(budget.stats().exhausted, Some(BudgetExhaustion::Ops));
}

/// A page showing `count` glyphs of a Type 3 font whose one glyph procedure
/// is exactly `proc_ops` operators long: `d0`, a filled rectangle (so each
/// procedure run is visible as one `draw_path`), then `n` padding.
fn type3_page(proc_ops: usize, count: usize) -> Vec<u8> {
    let mut procedure = String::from("1000 0 d0 0 0 500 500 re f\n");
    procedure.push_str(&"n\n".repeat(proc_ops - 3));
    let font = FontDict::Type3(Type3Font {
        font_matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
        font_bbox: [0.0, 0.0, 1000.0, 1000.0],
        glyphs: vec![Type3Glyph {
            code: b'a',
            name: "a".to_owned(),
            width: 1000.0,
            procedure,
        }],
        to_unicode: None,
    });
    let mut doc = PdfBuilder::new();
    doc.add_page_a4()
        .add_font("T3", font)
        .raw_content(&format!("BT /T3 1 Tf ({}) Tj ET", "a".repeat(count)));
    doc.to_bytes()
}

#[test]
fn type3_glyph_procedures_share_the_page_budget() {
    // 200 glyphs × 10,000 operators, run in nested contexts that hayro builds
    // from a clone of the settings. The page itself has 4 operators, so only
    // state shared with those nested contexts can reach 100,000.
    let out = run_on_small_stack(
        type3_page(10_000, 200),
        Run {
            budget: InterpretBudget::new(Some(100_000), None),
            run_nested: true,
        },
    );
    assert_eq!(out.stats.exhausted, Some(BudgetExhaustion::Ops));
    assert_eq!(out.stats.ops, 100_001);
    // `BT` and `Tf` are operators 1–2 and `Tj` 3, so procedure k covers
    // operators 3 + 10,000(k − 1) + 1 onwards: the 10th draws its rectangle
    // and is cut in its padding; procedures 11–200 return before running.
    assert_eq!(out.paths, 10);
    assert_eq!(
        out.text.chars().count(),
        200,
        "every glyph reaches the device"
    );
}

#[test]
fn type3_glyph_procedures_run_to_completion_without_limits() {
    let out = run_on_small_stack(
        type3_page(10_000, 20),
        Run {
            run_nested: true,
            ..Run::default()
        },
    );
    assert_eq!(out.stats.exhausted, None);
    assert_eq!(out.stats.ops, 3 + 20 * 10_000 + 1);
    assert_eq!(out.paths, 20);
}
