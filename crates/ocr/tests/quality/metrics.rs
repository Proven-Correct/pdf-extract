//! The error-rate metrics of spec 0002 O34 / D-10, shared by the corpus-D
//! gate and the private-scans report.
//!
//! Text is NFC-normalised, whitespace-collapsed (runs of Unicode whitespace
//! become one space; leading and trailing removed) and case-preserving.
//! CER is the Levenshtein distance over code points divided by the
//! reference length in code points; WER the same over whitespace-split
//! tokens. NFC matters even though corpus D's truth is ASCII: OCR output is
//! not, and the private-scans similarity compares two OCR outputs.
//!
//! `bow_f1` is the exception to D-10: an order-insensitive companion to
//! CER/WER, reported but never gated (see its doc comment).

use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;

/// The normalised form both metrics work on.
pub fn normalise(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    nfc.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Levenshtein distance between two sequences: two rows, O(len(a) ×
/// len(b)) time, O(len(b)) memory.
pub fn levenshtein<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[allow(clippy::cast_precision_loss)] // texts are far below 2^52 units
fn rate(distance: usize, reference: usize) -> f64 {
    if reference == 0 {
        // An empty reference: any output is all error, none is perfect.
        return if distance == 0 { 0.0 } else { 1.0 };
    }
    distance as f64 / reference as f64
}

/// Character error rate of `got` against `truth`.
pub fn cer(got: &str, truth: &str) -> f64 {
    let (g, t): (Vec<char>, Vec<char>) = (
        normalise(got).chars().collect(),
        normalise(truth).chars().collect(),
    );
    rate(levenshtein(&g, &t), t.len())
}

/// Word error rate of `got` against `truth`.
pub fn wer(got: &str, truth: &str) -> f64 {
    let (g, t) = (normalise(got), normalise(truth));
    let (g, t): (Vec<&str>, Vec<&str>) = (
        g.split(' ').filter(|w| !w.is_empty()).collect(),
        t.split(' ').filter(|w| !w.is_empty()).collect(),
    );
    rate(levenshtein(&g, &t), t.len())
}

/// Word-multiset F1 of `got` against `truth`: the F1 of precision and recall
/// over the bags of whitespace-split tokens (after the same normalisation as
/// CER/WER), so reading order does not matter. 1 is a perfect bag; two empty
/// texts score 1, and one empty text against a non-empty one scores 0.
///
/// Why it exists (B24 of internal #55): the oracle interleaves the two columns of
/// two-column pages, so CER/WER against the in-order ground truth penalises
/// reading order as much as recognition (oracle CER ~0.73 at 72 dpi on those
/// pages). This metric separates the two. Tokens are compared exactly, so it
/// is still case- and punctuation-sensitive. It is report-only: no gate has
/// been derived from it.
#[allow(clippy::cast_precision_loss)] // token counts are far below 2^52
pub fn bow_f1(got: &str, truth: &str) -> f64 {
    let (g, t) = (normalise(got), normalise(truth));
    let mut bag: HashMap<&str, usize> = HashMap::new();
    for w in t.split(' ').filter(|w| !w.is_empty()) {
        *bag.entry(w).or_insert(0) += 1;
    }
    let truth_n: usize = bag.values().sum();
    let (mut got_n, mut overlap) = (0usize, 0usize);
    for w in g.split(' ').filter(|w| !w.is_empty()) {
        got_n += 1;
        if let Some(c) = bag.get_mut(w).filter(|c| **c > 0) {
            *c -= 1;
            overlap += 1;
        }
    }
    if got_n == 0 && truth_n == 0 {
        return 1.0;
    }
    if overlap == 0 {
        return 0.0;
    }
    let (p, r) = (
        overlap as f64 / got_n as f64,
        overlap as f64 / truth_n as f64,
    );
    2.0 * p * r / (p + r)
}

/// Character similarity of two texts with no ground truth (private
/// scans): `1 - distance / max(len)`, 1 for two empty texts.
pub fn similarity(a: &str, b: &str) -> f64 {
    let (a, b): (Vec<char>, Vec<char>) = (
        normalise(a).chars().collect(),
        normalise(b).chars().collect(),
    );
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    1.0 - rate(levenshtein(&a, &b), longest)
}

/// `extract`'s sparse rule (D-15): fewer than `min_chars` code points once
/// Unicode whitespace is trimmed from both ends.
pub fn sparse(text: &str, min_chars: usize) -> bool {
    text.trim().chars().count() < min_chars
}

/// The mean of `values` (0 for none).
#[allow(clippy::cast_precision_loss)]
pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

/// The `q`-quantile (0–1) of `values` by nearest rank (0 for none).
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn quantile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let idx = ((q * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1;
    v[idx]
}

#[test]
fn metrics_follow_d10() {
    assert_eq!(normalise("  a\u{2003}b\n\nc  "), "a b c");
    // NFC: a decomposed é equals the composed one.
    assert_eq!(cer("e\u{301}", "\u{e9}"), 0.0);
    assert!((cer("abd", "abc") - 1.0 / 3.0).abs() < 1e-12);
    assert!((cer("ab", "abc") - 1.0 / 3.0).abs() < 1e-12);
    // Case is preserved.
    assert!((cer("ABC", "abc") - 1.0).abs() < 1e-12);
    assert!((wer("the cat sat", "the cat sat down") - 0.25).abs() < 1e-12);
    assert_eq!(wer("x", ""), 1.0);
    assert_eq!(bow_f1("x", ""), 0.0);
    assert_eq!(cer("", ""), 0.0);
    assert_eq!(similarity("", ""), 1.0);
    assert!((similarity("abcd", "abce") - 0.75).abs() < 1e-12);
    assert!(sparse(" \u{a0}abc\u{2003} ", 4));
    assert!(!sparse("abcd", 4));
    assert_eq!(quantile(&[3.0, 1.0, 2.0, 4.0], 0.5), 2.0);
    assert_eq!(quantile(&[3.0, 1.0, 2.0, 4.0], 0.1), 1.0);
    assert_eq!(mean(&[1.0, 2.0]), 1.5);
}

#[test]
fn bow_f1_ignores_order_but_not_content() {
    // Reordering (a two-column interleave) costs nothing.
    assert_eq!(bow_f1("c a b", "a b c"), 1.0);
    assert!(wer("c a b", "a b c") > 0.0);
    assert_eq!(bow_f1("", ""), 1.0);
    assert_eq!(bow_f1("", "a"), 0.0);
    assert_eq!(bow_f1("a", "b"), 0.0);
    // Multiset, not set: got = {a, a}, truth = {a, b} has overlap 1,
    // P = R = 1/2, F1 = 1/2.
    assert!((bow_f1("a a", "a b") - 0.5).abs() < 1e-12);
    // got = {a, b, x}, truth = {a, b, c, d}: overlap 2, P = 2/3, R = 1/2,
    // F1 = 4/7.
    assert!((bow_f1("a b x", "a b c d") - 4.0 / 7.0).abs() < 1e-12);
    // Normalisation matches CER/WER: whitespace runs and NFC.
    assert_eq!(bow_f1("e\u{301}\n\n a", "a \u{e9}"), 1.0);
    // Case is preserved.
    assert_eq!(bow_f1("A", "a"), 0.0);
}
