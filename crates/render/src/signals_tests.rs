//! Unit tests of the warning fold (spec 0002 O3, O4).

use hayro_interpret::{InterpretStats, InterpreterWarning};
use pdf_extract_core::{Limits, WarningKind};

use super::{Counters, SinkCounts, Warnings};

#[test]
fn no_signal_is_no_warning() {
    let mut w = Warnings::new(3);
    w.add_render(
        &Limits::default(),
        &Counters::default(),
        &InterpretStats::default(),
        &SinkCounts::default(),
    );
    assert!(w.finish().is_empty());
}

#[test]
fn every_detail_is_the_text_paths() {
    // One source for both paths (core's `render_support`), so a warning
    // reads the same whichever path raised it. Every kind the renderer
    // raises is checked against the shared definition, so a literal
    // reintroduced here fails this test.
    use pdf_extract_core::render_support::{
        CONTENT_DAMAGED_DETAIL, FONT_SUBSTITUTED_DETAIL, FORM_CYCLE_DETAIL,
        IMAGE_DECODE_FAILURE_DETAIL, IMAGE_MEMORY_LIMIT_DETAIL, INTERPRETER_NESTING_DETAIL,
        OBJECT_DEPTH_DETAIL, UNSUPPORTED_FONT_DETAIL, image_pixel_limit_detail,
        stream_decode_limit_detail,
    };
    let limits = Limits {
        max_image_pixels: 1_234_567,
        max_stream_decoded_bytes: 7_654_321,
        ..Limits::default()
    };
    let signals = Counters {
        object_depth: 1,
        content_damage: 1,
        decode_limit: 1,
        font_substitutions: 1,
        ..Counters::default()
    };
    let stats = InterpretStats {
        nesting_cap_hits: 1,
        form_cycle_hits: 1,
        ..InterpretStats::default()
    };
    let sink = SinkCounts::default();
    for w in [
        InterpreterWarning::UnsupportedFont,
        InterpreterWarning::ImageDecodeFailure,
        InterpreterWarning::ImagePixelLimit,
        InterpreterWarning::ImageMemoryLimit,
    ] {
        sink.note(w);
    }
    let mut w = Warnings::new(0);
    w.add_render(&limits, &signals, &stats, &sink);
    let got: Vec<_> = w.finish().into_iter().map(|w| (w.kind, w.detail)).collect();
    assert_eq!(
        got,
        [
            (WarningKind::ObjectDepth, OBJECT_DEPTH_DETAIL.to_owned()),
            (
                WarningKind::StreamDecodeLimit,
                stream_decode_limit_detail(&limits)
            ),
            (
                WarningKind::ContentDamaged,
                CONTENT_DAMAGED_DETAIL.to_owned()
            ),
            (
                WarningKind::FontLoadFailed,
                FONT_SUBSTITUTED_DETAIL.to_owned()
            ),
            (
                WarningKind::InterpreterNesting,
                INTERPRETER_NESTING_DETAIL.to_owned()
            ),
            (WarningKind::FormXObjectCycle, FORM_CYCLE_DETAIL.to_owned()),
            (
                WarningKind::UnsupportedFont,
                UNSUPPORTED_FONT_DETAIL.to_owned()
            ),
            (
                WarningKind::ImageDecodeFailure,
                IMAGE_DECODE_FAILURE_DETAIL.to_owned()
            ),
            (
                WarningKind::ImagePixelLimit,
                image_pixel_limit_detail(&limits)
            ),
            (
                WarningKind::ImageMemoryLimit,
                IMAGE_MEMORY_LIMIT_DETAIL.to_owned()
            ),
        ]
    );
}

#[test]
fn every_sink_warning_is_counted() {
    let sink = SinkCounts::default();
    for w in [
        InterpreterWarning::UnsupportedFont,
        InterpreterWarning::ImageDecodeFailure,
        InterpreterWarning::ImageDecodeFailure,
        InterpreterWarning::ImagePixelLimit,
        InterpreterWarning::ImageMemoryLimit,
    ] {
        sink.note(w);
    }
    let stats = InterpretStats {
        nesting_cap_hits: 2,
        form_cycle_hits: 1,
        ..InterpretStats::default()
    };
    let mut w = Warnings::new(7);
    w.add_render(&Limits::default(), &Counters::default(), &stats, &sink);
    let got: Vec<_> = w
        .finish()
        .into_iter()
        .map(|w| (w.page, w.kind, w.count, w.to_string()))
        .collect();
    let kinds: Vec<_> = got.iter().map(|g| (g.1, g.2)).collect();
    assert_eq!(
        kinds,
        [
            (WarningKind::InterpreterNesting, 2),
            (WarningKind::FormXObjectCycle, 1),
            (WarningKind::UnsupportedFont, 1),
            (WarningKind::ImageDecodeFailure, 2),
            (WarningKind::ImagePixelLimit, 1),
            (WarningKind::ImageMemoryLimit, 1),
        ]
    );
    assert!(got.iter().all(|g| g.0 == Some(7)));
    assert!(got[3].3.starts_with("image: decode failed"), "{}", got[3].3);
    assert!(got[4].3.starts_with("image: pixel limit"), "{}", got[4].3);
    assert!(got[5].3.starts_with("image: memory limit"), "{}", got[5].3);
}

#[test]
fn thread_local_signals_become_warnings() {
    let before = Counters::default();
    let after = Counters {
        object_depth: 1,
        content_damage: 2,
        decode_limit: 3,
        font_substitutions: 4,
        image_pixels: 5,
        image_memory: 6,
    };
    let signals = after.since(&before);
    // The pixel- and memory-limit counts are the larger of the two channels.
    let sink = SinkCounts::default();
    sink.note(InterpreterWarning::ImagePixelLimit);
    for _ in 0..7 {
        sink.note(InterpreterWarning::ImageMemoryLimit);
    }
    let mut w = Warnings::new(0);
    w.add_render(
        &Limits::default(),
        &signals,
        &InterpretStats::default(),
        &sink,
    );
    let kinds: Vec<_> = w.finish().into_iter().map(|w| (w.kind, w.count)).collect();
    assert_eq!(
        kinds,
        [
            (WarningKind::ObjectDepth, 1),
            (WarningKind::StreamDecodeLimit, 3),
            (WarningKind::ContentDamaged, 2),
            (WarningKind::FontLoadFailed, 4),
            (WarningKind::ImagePixelLimit, 5),
            (WarningKind::ImageMemoryLimit, 7),
        ]
    );
    // Counters never run backwards; a stale sample saturates at zero.
    assert_eq!(before.since(&after), Counters::default());
}

#[test]
fn counts_saturate_at_u32_max() {
    let mut w = Warnings::new(0);
    w.push_n(WarningKind::ContentDamaged, u64::MAX, || "x".into());
    assert_eq!(w.finish()[0].count, u32::MAX);
}
