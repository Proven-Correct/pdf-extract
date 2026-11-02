//! The page's warnings: hayro's signals around one render, folded into
//! [`Warning`]s the way the text path folds them (spec 0002 O3, O4).
//!
//! Two channels, as in core's `text/extract.rs`:
//!
//! - **Thread-local counters** in the fork (object depth H2a, content damage
//!   H10a, decode-limit hits H10f, font substitutions H10e, image pixel-limit
//!   refusals H13, codec memory-budget refusals H14/H15), sampled before and
//!   after the render. Valid only because hayro's renderer runs entirely on
//!   the calling thread (it is built with `num_threads: 0` and no thread pool;
//!   spec Appendix B.1), so nothing between the two samples can be charged to
//!   another page.
//! - **hayro-interpret's warning sink** (`UnsupportedFont`,
//!   `ImageDecodeFailure`, `ImagePixelLimit`, `ImageMemoryLimit`), counted in
//!   atomics because the sink must be `Send + Sync`; one render owns it.
//!
//! An image refused by the pixel cap or a codec's memory budget reaches both
//! channels once (H13, H14, H15: the interpreter's sink, and hayro-syntax's
//! thread-local, which also hears a refusal on a path outside the
//! interpreter's image code), so the larger of the two counts is the number of
//! images skipped. Every detail string comes from core through
//! `render_support`, so a warning reads the same whichever path raised it.

use std::sync::atomic::{AtomicU64, Ordering};

use hayro_interpret::{InterpretStats, InterpreterWarning, font_substitutions};
use hayro_syntax::{
    content_damage_events, image_memory_limit_hits, image_pixel_limit_hits,
    object_depth_limit_hits, stream_decode_limit_hits,
};
use pdf_extract_core::render_support::{
    CONTENT_DAMAGED_DETAIL, FONT_SUBSTITUTED_DETAIL, FORM_CYCLE_DETAIL,
    IMAGE_DECODE_FAILURE_DETAIL, IMAGE_MEMORY_LIMIT_DETAIL, INTERPRETER_NESTING_DETAIL,
    OBJECT_DEPTH_DETAIL, UNSUPPORTED_FONT_DETAIL, image_pixel_limit_detail,
    stream_decode_limit_detail,
};
use pdf_extract_core::{Limits, Warning, WarningKind};

/// The fork's thread-local counters at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counters {
    object_depth: u64,
    content_damage: u64,
    decode_limit: u64,
    font_substitutions: u64,
    image_pixels: u64,
    image_memory: u64,
}

impl Counters {
    pub(crate) fn sample() -> Self {
        Self {
            object_depth: object_depth_limit_hits(),
            content_damage: content_damage_events(),
            decode_limit: stream_decode_limit_hits(),
            font_substitutions: font_substitutions(),
            image_pixels: image_pixel_limit_hits(),
            image_memory: image_memory_limit_hits(),
        }
    }

    /// The events between `before` and `self` (counters only grow).
    pub(crate) fn since(&self, before: &Self) -> Self {
        Self {
            object_depth: self.object_depth.saturating_sub(before.object_depth),
            content_damage: self.content_damage.saturating_sub(before.content_damage),
            decode_limit: self.decode_limit.saturating_sub(before.decode_limit),
            font_substitutions: self
                .font_substitutions
                .saturating_sub(before.font_substitutions),
            image_pixels: self.image_pixels.saturating_sub(before.image_pixels),
            image_memory: self.image_memory.saturating_sub(before.image_memory),
        }
    }
}

/// hayro-interpret's warnings for one render.
#[derive(Debug, Default)]
pub(crate) struct SinkCounts {
    unsupported_font: AtomicU64,
    image_decode_failure: AtomicU64,
    image_pixel_limit: AtomicU64,
    image_memory_limit: AtomicU64,
}

impl SinkCounts {
    pub(crate) fn note(&self, warning: InterpreterWarning) {
        let counter = match warning {
            InterpreterWarning::UnsupportedFont => &self.unsupported_font,
            InterpreterWarning::ImageDecodeFailure => &self.image_decode_failure,
            InterpreterWarning::ImagePixelLimit => &self.image_pixel_limit,
            InterpreterWarning::ImageMemoryLimit => &self.image_memory_limit,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

/// Folds counts into one warning per kind, in a fixed order, with the
/// page's index (core's `WarningSink` is crate-private; this is the same
/// fold for the few kinds a render can raise).
#[derive(Debug)]
pub(crate) struct Warnings {
    page: usize,
    list: Vec<Warning>,
}

impl Warnings {
    pub(crate) fn new(page: usize) -> Self {
        Self {
            page,
            list: Vec::new(),
        }
    }

    /// Records `n` occurrences of `kind`; nothing when `n == 0`. Each kind
    /// is pushed at most once per render, so no merging is needed.
    pub(crate) fn push_n(&mut self, kind: WarningKind, n: u64, detail: impl FnOnce() -> String) {
        if n == 0 {
            return;
        }
        self.list.push(Warning {
            page: Some(self.page),
            kind,
            detail: detail(),
            count: u32::try_from(n).unwrap_or(u32::MAX),
        });
    }

    /// Adds every signal of one completed render.
    pub(crate) fn add_render(
        &mut self,
        limits: &Limits,
        signals: &Counters,
        stats: &InterpretStats,
        sink: &SinkCounts,
    ) {
        self.push_n(WarningKind::ObjectDepth, signals.object_depth, || {
            OBJECT_DEPTH_DETAIL.into()
        });
        self.push_n(WarningKind::StreamDecodeLimit, signals.decode_limit, || {
            stream_decode_limit_detail(limits)
        });
        self.push_n(WarningKind::ContentDamaged, signals.content_damage, || {
            CONTENT_DAMAGED_DETAIL.into()
        });
        self.push_n(
            WarningKind::FontLoadFailed,
            signals.font_substitutions,
            || FONT_SUBSTITUTED_DETAIL.into(),
        );
        self.push_n(
            WarningKind::InterpreterNesting,
            u64::from(stats.nesting_cap_hits),
            || INTERPRETER_NESTING_DETAIL.into(),
        );
        self.push_n(
            WarningKind::FormXObjectCycle,
            u64::from(stats.form_cycle_hits),
            || FORM_CYCLE_DETAIL.into(),
        );
        self.push_n(
            WarningKind::UnsupportedFont,
            sink.unsupported_font.load(Ordering::Relaxed),
            || UNSUPPORTED_FONT_DETAIL.into(),
        );
        self.push_n(
            WarningKind::ImageDecodeFailure,
            sink.image_decode_failure.load(Ordering::Relaxed),
            || IMAGE_DECODE_FAILURE_DETAIL.into(),
        );
        self.push_n(
            WarningKind::ImagePixelLimit,
            signals
                .image_pixels
                .max(sink.image_pixel_limit.load(Ordering::Relaxed)),
            || image_pixel_limit_detail(limits),
        );
        self.push_n(
            WarningKind::ImageMemoryLimit,
            signals
                .image_memory
                .max(sink.image_memory_limit.load(Ordering::Relaxed)),
            || IMAGE_MEMORY_LIMIT_DETAIL.into(),
        );
    }

    pub(crate) fn finish(self) -> Vec<Warning> {
        self.list
    }
}

#[cfg(test)]
#[path = "signals_tests.rs"]
mod tests;
