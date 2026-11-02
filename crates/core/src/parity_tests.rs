//! Unit tests of `parity` (the `migration-parity` spec's scenarios).

use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-12
}

#[test]
fn identical_pages() {
    let c = compare_page("Hello\nworld\n", "  Hello\nworld  ");
    assert!(c.routing_agrees);
    assert!(close(c.similarity, 1.0));
    assert!(c.exact);
    assert_eq!(c.line_delta, 0);
    assert_eq!((c.reference_chars, c.candidate_chars), (11, 11));
}

#[test]
fn similarity_is_the_parity_definition() {
    assert!(close(similarity("abcd", "abce"), 0.75));
    assert!(close(compare_page("abcd", "abce").similarity, 0.75));
    // Characters, not bytes.
    assert!(similarity("é", "e").abs() < f64::EPSILON);
    assert!(similarity("abc", "").abs() < f64::EPSILON);
    assert!(close(similarity("", ""), 1.0));
}

#[test]
fn python_strip_rules() {
    // U+001C-U+001F and U+0085 are whitespace to Python; U+200B is not.
    assert_eq!(py_strip("\u{1c}\u{85} a b\u{3000}\n"), "a b");
    assert_eq!(py_strip("\u{200b}a"), "\u{200b}a");
}

#[test]
fn routing_disagreement() {
    let c = compare_page(&"x".repeat(60), &"x".repeat(10));
    assert!(!c.routing_agrees);
    assert!(!c.exact);
    // At the threshold both sides route as text.
    assert!(compare_page(&"y".repeat(50), &"z".repeat(50)).routing_agrees);
    assert!(!compare_page(&"y".repeat(50), &"z".repeat(49)).routing_agrees);
}

#[test]
fn both_pages_empty() {
    let c = compare_page("", " \n\t");
    assert!(close(c.similarity, 1.0));
    assert!(c.exact);
    assert!(c.routing_agrees);
    assert_eq!(c.line_delta, 0);
}

#[test]
fn line_delta_is_signed() {
    assert_eq!(compare_page("a", "a\nb\nc").line_delta, 2);
    assert_eq!(compare_page("a\n\nb", "a").line_delta, -2);
    assert_eq!(compare_page("", "a").line_delta, 1);
}

fn page(sim: f64, routing: bool, delta: i64) -> PageComparison {
    PageComparison {
        routing_agrees: routing,
        similarity: sim,
        exact: close(sim, 1.0),
        line_delta: delta,
        reference_chars: 0,
        candidate_chars: 0,
    }
}

#[test]
fn gates_pass() {
    let mut acc = ParityAccumulator::new();
    for _ in 0..1000 {
        acc.add(&page(1.0, true, 0));
    }
    let r = acc.report(&Gates::SPEC);
    assert_eq!(r.pages, 1000);
    assert_eq!(r.verdicts.len(), 3);
    assert!(r.verdicts.iter().all(|v| v.passed));
    assert!(r.passed());
    assert_eq!(r.exact_rate, Some(1.0));
    assert_eq!(r.line_delta_histogram, [1000, 0, 0, 0]);
}

#[test]
fn routing_gate_fails() {
    let mut acc = ParityAccumulator::new();
    for i in 0..1000 {
        acc.add(&page(1.0, i >= 2, 0));
    }
    let r = acc.report(&Gates::SPEC);
    assert!(close(r.routing_rate.unwrap(), 0.998));
    let routing = &r.verdicts[0];
    assert_eq!(routing.name, "routing");
    assert!(!routing.passed);
    assert!(r.verdicts[1].passed && r.verdicts[2].passed);
    assert!(!r.passed());
}

#[test]
fn percentiles_are_nearest_rank_bucket_floors() {
    let mut acc = ParityAccumulator::new();
    for s in [0.1, 0.2, 0.3, 0.4] {
        acc.add(&page(s, true, 0));
    }
    let r = acc.report(&Gates {
        low_percentile: 10.0,
        ..Gates::SPEC
    });
    // Lower middle of an even count; p10 of four is the smallest.
    assert!(close(r.median.unwrap(), 0.2));
    assert!(close(r.low.unwrap(), 0.1));
    assert!(close(r.min.unwrap(), 0.1));
    assert!(close(r.mean.unwrap(), 0.25));
    // A value just under a threshold lands in the bucket below it, so the
    // verdict is the one the raw sample gives.
    let mut acc = ParityAccumulator::new();
    acc.add(&page(0.998_999_9, true, 0));
    assert!(!acc.report(&Gates::SPEC).verdicts[1].passed);
    let mut acc = ParityAccumulator::new();
    acc.add(&page(0.999, true, 0));
    assert!(acc.report(&Gates::SPEC).verdicts[1].passed);
    // Out-of-range input is clamped into the histogram.
    let mut acc = ParityAccumulator::new();
    acc.add(&page(1.5, true, 0));
    acc.add(&page(-0.5, true, 0));
    assert!(close(acc.report(&Gates::SPEC).low.unwrap(), 0.0));
}

#[test]
fn line_delta_histogram_and_extremes() {
    let mut acc = ParityAccumulator::new();
    for d in [0, -1, 2, 7, -3] {
        acc.add(&page(1.0, true, d));
    }
    let r = acc.report(&Gates::default());
    assert_eq!(r.line_delta_histogram, [1, 1, 1, 2]);
    assert_eq!(r.line_delta_max_abs, 7);
    assert!(close(r.line_delta_mean_abs.unwrap(), 13.0 / 5.0));
}

#[test]
fn partial_results_merge() {
    let pages: Vec<PageComparison> = (0..200)
        .map(|i| page(f64::from(i) / 200.0, i % 7 != 0, i64::from(i % 5) - 2))
        .collect();
    let mut whole = ParityAccumulator::new();
    let (mut a, mut b) = (ParityAccumulator::new(), ParityAccumulator::new());
    for (i, p) in pages.iter().enumerate() {
        whole.add(p);
        if i % 3 == 0 { a.add(p) } else { b.add(p) }
    }
    a.merge(&b);
    let (mut merged, mut single) = (a.report(&Gates::SPEC), whole.report(&Gates::SPEC));
    // The mean is a float sum, so its last bits depend on the order of
    // addition; every other field must be equal.
    assert!(close(merged.mean.unwrap(), single.mean.unwrap()));
    (merged.mean, single.mean) = (None, None);
    assert_eq!(merged, single);
    // Merging into or from an empty accumulator changes nothing.
    let mut empty = ParityAccumulator::new();
    empty.merge(&whole);
    assert_eq!(empty, whole);
    let before = whole.clone();
    whole.merge(&ParityAccumulator::new());
    assert_eq!(whole, before);
}

#[test]
fn empty_accumulator() {
    let acc = ParityAccumulator::new();
    let r = acc.report(&Gates::SPEC);
    assert_eq!(acc.pages(), 0);
    assert_eq!(r.pages, 0);
    assert_eq!(r.routing_rate, None);
    assert_eq!(r.median, None);
    assert_eq!(r.low, None);
    assert_eq!(r.min, None);
    assert!(r.verdicts.is_empty());
    assert!(!r.passed());
}
