//! Tests of fork change H10a: the content-stream lexer skips bytes that start
//! no token and continues, counting each skipped byte in the thread-local
//! `content_damage_events()` (spec §3.2, oracle §4.9b).
//!
//! Upstream `UntypedIter::next` returned `None` at the first stray `)`, `]`
//! or `>>`, losing the rest of the stream without any signal.

use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::content::{TypedIter, UntypedIter};
use hayro_syntax::content_damage_events;

/// The operators of `data`, in order, and how far the damage counter moved.
fn operators(data: &[u8]) -> (Vec<String>, u64) {
    let before = content_damage_events();
    let mut iter = UntypedIter::new(data);
    let mut ops = Vec::new();
    while let Some(i) = iter.next() {
        ops.push(String::from_utf8_lossy(i.operator).into_owned());
    }
    (ops, content_damage_events() - before)
}

#[test]
fn stray_delimiters_are_skipped_and_counted() {
    // `) ) ] > >` are five bytes that start no token; `junk` is an unknown
    // operator, which is a token and therefore not damage.
    let (ops, skipped) = operators(b"(A) Tj ) ) ] >> junk (B) Tj");
    assert_eq!(ops, ["Tj", "junk", "Tj"]);
    assert_eq!(skipped, 5);
}

#[test]
fn operands_before_a_skip_are_dropped() {
    // `(A)` belongs to an instruction broken by the stray `)`, so it is not
    // left on the stack for the following operator.
    let mut iter = UntypedIter::new(b"(A) ) (B) Tj");
    let i = iter.next().expect("Tj");
    assert_eq!(i.operands().count(), 1);
    assert!(iter.next().is_none());
}

#[test]
fn a_well_formed_stream_is_not_damage() {
    let (ops, skipped) =
        operators(b"q 1 0 0 1 0 0 cm BT /F1 12 Tf 10 10 Td [(A) -20 (B)] TJ ET Q % comment\n");
    assert_eq!(ops, ["q", "cm", "BT", "Tf", "Td", "TJ", "ET", "Q"]);
    assert_eq!(skipped, 0);
}

#[test]
fn an_operator_inside_a_tj_array_is_still_tolerated() {
    // Upstream's `operator-in-TJ-array` leniency: a content operator inside
    // an array is skipped as part of the array, which still parses.
    let mut iter = TypedIter::new(b"BT [(A) Tw (B)] TJ ET");
    assert!(matches!(iter.next(), Some(TypedInstruction::BeginText(_))));
    assert!(matches!(iter.next(), Some(TypedInstruction::ShowTexts(_))));
    assert!(matches!(iter.next(), Some(TypedInstruction::EndText(_))));
    let (_, skipped) = operators(b"BT [(A) Tw (B)] TJ ET");
    assert_eq!(skipped, 0);
}

#[test]
fn a_malformed_name_run_is_skipped_whole() {
    // `a#zz` is a run of regular characters that is not a valid name: it is
    // skipped as one run (four bytes), not retried at each byte.
    let (ops, skipped) = operators(b"a#zz Q");
    assert_eq!(ops, ["Q"]);
    assert_eq!(skipped, 4);
    // Linear time on a long run (quadratic rescanning would take minutes).
    let mut data = b"a".to_vec();
    data.extend(std::iter::repeat_n(b'#', 4 << 20));
    data.extend_from_slice(b" Q");
    let start = std::time::Instant::now();
    let (ops, skipped) = operators(&data);
    assert_eq!(ops, ["Q"]);
    assert_eq!(skipped, (4 << 20) + 1);
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn a_long_run_of_stray_bytes_is_linear() {
    let mut data = b"(A) Tj ".to_vec();
    data.extend(std::iter::repeat_n(b')', 4 << 20));
    data.extend_from_slice(b" (B) Tj");
    let (ops, skipped) = operators(&data);
    assert_eq!(ops, ["Tj", "Tj"]);
    assert_eq!(skipped, 4 << 20);
}

#[test]
fn a_failed_operand_still_ends_the_stream() {
    // Not resynchronised (see the fork's comment): an unterminated string
    // scans to the end, and an over-deep array is H2a's pinned case. Neither
    // counts as skipped bytes.
    let (ops, skipped) = operators(b"(A) Tj (unterminated Tj (B) Tj");
    assert_eq!(ops, ["Tj"]);
    assert_eq!(skipped, 0);
}

#[test]
fn the_counter_is_per_thread() {
    let before = content_damage_events();
    std::thread::spawn(|| operators(b") ) )")).join().unwrap();
    assert_eq!(content_damage_events(), before);
}

/// The operators of `data` with, for each inline image (`BI`), the length
/// of its data, and how far the damage counter moved.
fn operators_and_images(data: &[u8]) -> (Vec<String>, Vec<usize>, u64) {
    use hayro_syntax::object::Object;
    let before = content_damage_events();
    let mut iter = UntypedIter::new(data);
    let (mut ops, mut images) = (Vec::new(), Vec::new());
    while let Some(i) = iter.next() {
        ops.push(String::from_utf8_lossy(i.operator).into_owned());
        if &**i.operator == b"BI"
            && let Some(Object::Stream(s)) = i.operands().next()
        {
            images.push(s.raw_data().len());
        }
    }
    (ops, images, content_damage_events() - before)
}

#[test]
fn inline_image_end_detection_is_unchanged() {
    // The image data `9jEI z~>` contains a false `EI`. Upstream rejects it
    // because what follows (`z~> EI Q …`) does not read as content without
    // skipping bytes; the probe that decides this must not resynchronise
    // (it would accept the false `EI` and truncate the image), and must not
    // count the bytes it looked at as damage.
    let data = b"BI /W 4 /H 1 /BPC 8 /CS /G /F /A85 ID 9jEI z~> EI Q BT (A) Tj ET \
                 BI /W 1 /H 1 /BPC 8 /CS /G ID A EI Q";
    let (ops, images, skipped) = operators_and_images(data);
    assert_eq!(ops, ["BI", "Q", "BT", "Tj", "ET", "BI", "Q"]);
    assert_eq!(images, [9, 2]);
    assert_eq!(skipped, 0);
}

#[test]
fn stray_bytes_around_inline_images_are_counted_once() {
    let data = b") BI /W 4 /H 1 /BPC 8 /CS /G /F /A85 ID 9jEI z~> EI ] Q \
                 BT (A) Tj ET >> BI /W 1 /H 1 /BPC 8 /CS /G ID A EI ) ) Q";
    let (ops, images, skipped) = operators_and_images(data);
    assert_eq!(ops, ["BI", "Q", "BT", "Tj", "ET", "BI", "Q"]);
    assert_eq!(images, [9, 2]);
    assert_eq!(skipped, 6);
}
