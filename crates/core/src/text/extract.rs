//! One page through hayro into the text device: `Page::extract_chars`
//! (spec §3.3 signal sampling, §1.4 per-page limits, T34, T41).
//!
//! Per call, on the calling thread and its stack only:
//!
//! 1. rebuild the hayro `Page` from the index (T17);
//! 2. build a fresh `InterpreterCache` and `Context` (they are `!Send`; T41,
//!    §8.3) with an `InterpretBudget` from `max_ops_per_page`, a deadline
//!    `page_time_budget` from now, and a cancel flag shared with the device;
//! 3. interpret into a [`TextDevice`] whose visible box is the page's
//!    CropBox ∩ MediaBox in page space;
//! 4. map an exhausted operator or time budget to `PageError::Limit` (the
//!    partial output is discarded: a caller that set a budget asked for a
//!    whole page or an error) and every other signal to a
//!    [`Warning`](crate::Warning).
//!
//! **Character limit (T34, D-38).** When the device reaches
//! `max_chars_per_page` it raises the cancel flag, hayro stops at the next
//! operator with `BudgetExhaustion::Cancelled` (fork change H10g), and the
//! partial page is returned with a `CharLimit` warning. A budget that ran
//! out before the limit was reached stays an error: the first reason hayro
//! records is kept.
//!
//! **Signals (D-12, D-29).** The object-nesting, content-damage (H10a),
//! decode-limit (H10f) and font-substitution (H10e) counters are
//! thread-locals in the fork, sampled before step 1 and after step 3, which
//! is valid only because nothing in between leaves this thread; concurrent
//! extractions on one document therefore never charge each other.
//! Nesting-cap and form-cycle hits come from `Context::stats()`, which counts
//! for this page alone.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use hayro_interpret::{
    BudgetExhaustion, Context, InterpretBudget, InterpreterCache, InterpreterSettings,
    InterpreterWarning, RectExt, TransformExt, font_substitutions, interpret_page,
};
use hayro_syntax::{
    content_damage_events, image_memory_limit_hits, image_pixel_limit_hits,
    object_depth_limit_hits, stream_decode_limit_hits,
};
use kurbo::{Affine, Rect, Shape};

use super::details::{
    CONTENT_DAMAGED_DETAIL, FONT_SUBSTITUTED_DETAIL, FORM_CYCLE_DETAIL,
    IMAGE_DECODE_FAILURE_DETAIL, IMAGE_MEMORY_LIMIT_DETAIL, INTERPRETER_NESTING_DETAIL,
    OBJECT_DEPTH_DETAIL, UNSUPPORTED_FONT_DETAIL, image_pixel_limit_detail,
    stream_decode_limit_detail,
};
use super::device::TextDevice;
use super::options::Unmapped;
use super::record::PageChars;
use crate::document::Loaded;
use crate::error::PageError;
use crate::limits::{LimitKind, Limits};
use crate::page::Page;
use crate::warning::{WarningKind, WarningSink};

impl Page {
    /// Interprets this page and returns its glyphs as character records,
    /// with the page's warnings (spec §4.9; the line assembler builds on
    /// this).
    ///
    /// # Errors
    /// `Closed` if the document was closed before the call;
    /// `Limit { OpsPerPage | PageTimeBudget }` when the interpreter budget
    /// ran out.
    pub(crate) fn extract_chars(&self, unmapped: Unmapped) -> Result<PageChars, PageError> {
        let limits = self.limits();
        self.with_loaded(|loaded, index| extract(loaded, index, limits, unmapped))?
    }
}

/// hayro's interpreter warnings for one page. Atomics only because hayro's
/// sink must be `Send + Sync`; one page operation owns it. (hayro 0.7 raises
/// `ImageDecodeFailure` only when a device asks for pixels, which this one
/// never does, and does not raise `UnsupportedFont` at all; both are wired up
/// so a future hayro that does is heard.)
#[derive(Debug, Default)]
pub(super) struct SinkCounts {
    pub(super) unsupported_font: AtomicU64,
    pub(super) image_decode_failure: AtomicU64,
    pub(super) image_pixel_limit: AtomicU64,
    pub(super) image_memory_limit: AtomicU64,
}

impl SinkCounts {
    pub(super) fn note(&self, warning: InterpreterWarning) {
        let counter = match warning {
            InterpreterWarning::UnsupportedFont => &self.unsupported_font,
            InterpreterWarning::ImageDecodeFailure => &self.image_decode_failure,
            InterpreterWarning::ImagePixelLimit => &self.image_pixel_limit,
            InterpreterWarning::ImageMemoryLimit => &self.image_memory_limit,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

fn extract(
    loaded: &Loaded,
    index: usize,
    limits: &Limits,
    unmapped: Unmapped,
) -> Result<PageChars, PageError> {
    let start = Instant::now();
    let mut sink = WarningSink::new(Some(index));

    // --- signal window: synchronous, this thread only (module docs) ---
    let before = Counters::sample();

    let xref = loaded.pdf.xref();
    let Some(page) = loaded.index.hayro_page(xref, index) else {
        // Only possible when a lazy xref repair after open changed what the
        // leaf's identifier points to.
        sink.push(WarningKind::PageTreeNodeUnreadable, || {
            "page dictionary no longer resolves; page is empty".into()
        });
        return Ok(PageChars {
            chars: Vec::new(),
            markers: Vec::new(),
            warnings: sink.finish(),
        });
    };

    let counts = Arc::new(SinkCounts::default());
    let cancel = Arc::new(AtomicBool::new(false));
    let mut budget = InterpretBudget::new(
        Some(limits.max_ops_per_page),
        // A budget too large to represent is no deadline at all.
        start.checked_add(limits.page_time_budget),
    );
    budget.cancel = Some(cancel.clone());
    let settings = InterpreterSettings {
        budget,
        warning_sink: {
            let counts = counts.clone();
            Arc::new(move |w| counts.note(w))
        },
        ..InterpreterSettings::default()
    };

    let (initial, visible) = page_space(&page);
    // Passing the visible box as the context's box also lets hayro skip clip
    // rectangles that cover it.
    let cache = InterpreterCache::new();
    let mut context = Context::new(initial, visible, &cache, xref, settings);
    let mut device =
        TextDevice::new(visible, unmapped, limits.max_chars_per_page).with_cancel(cancel);
    interpret_page(&page, &mut context, &mut device);
    device.close_open_spans();

    let signals = Counters::sample().since(&before);
    // --- end of signal window ---

    let stats = context.stats();
    #[cfg(test)]
    tests_support::LAST_STATS.with(|s| s.set(Some(stats)));
    match stats.exhausted {
        Some(BudgetExhaustion::Ops) => {
            return Err(PageError::Limit {
                kind: LimitKind::OpsPerPage,
                value: limits.max_ops_per_page,
            });
        }
        Some(BudgetExhaustion::Deadline) => {
            return Err(PageError::Limit {
                kind: LimitKind::PageTimeBudget,
                value: u64::try_from(limits.page_time_budget.as_millis()).unwrap_or(u64::MAX),
            });
        }
        // Only the device raises the flag, and only at the character limit,
        // whose warning follows.
        Some(BudgetExhaustion::Cancelled) | None => {}
    }

    let (chars, markers, over_limit) = device.finish();
    sink.push_n(WarningKind::CharLimit, u64::from(over_limit > 0), || {
        format!(
            "character limit reached ({} characters); the rest of the page was dropped",
            limits.max_chars_per_page
        )
    });
    sink.push_n(WarningKind::ObjectDepth, signals.object_depth, || {
        OBJECT_DEPTH_DETAIL.into()
    });
    sink.push_n(WarningKind::StreamDecodeLimit, signals.decode_limit, || {
        stream_decode_limit_detail(limits)
    });
    sink.push_n(WarningKind::ContentDamaged, signals.content_damage, || {
        CONTENT_DAMAGED_DETAIL.into()
    });
    sink.push_n(
        WarningKind::FontLoadFailed,
        signals.font_substitutions,
        || FONT_SUBSTITUTED_DETAIL.into(),
    );
    sink.push_n(
        WarningKind::InterpreterNesting,
        u64::from(stats.nesting_cap_hits),
        || INTERPRETER_NESTING_DETAIL.into(),
    );
    sink.push_n(
        WarningKind::FormXObjectCycle,
        u64::from(stats.form_cycle_hits),
        || FORM_CYCLE_DETAIL.into(),
    );
    sink.push_n(
        WarningKind::UnsupportedFont,
        counts.unsupported_font.load(Ordering::Relaxed),
        || UNSUPPORTED_FONT_DETAIL.into(),
    );
    sink.push_n(
        WarningKind::ImageDecodeFailure,
        counts.image_decode_failure.load(Ordering::Relaxed),
        || IMAGE_DECODE_FAILURE_DETAIL.into(),
    );
    // Two channels report the same refusals (H13): the interpreter's sink, and
    // hayro-syntax's thread-local, which also hears decodes made outside the
    // interpreter's image path. Each refusal reaches both, so the larger count
    // is the number of images skipped; the text device never decodes images,
    // so on this path both are 0 unless a future hayro does.
    sink.push_n(
        WarningKind::ImagePixelLimit,
        signals
            .image_pixels
            .max(counts.image_pixel_limit.load(Ordering::Relaxed)),
        || image_pixel_limit_detail(limits),
    );
    // The same two channels report the codecs' memory-budget refusals (H14,
    // H15).
    sink.push_n(
        WarningKind::ImageMemoryLimit,
        signals
            .image_memory
            .max(counts.image_memory_limit.load(Ordering::Relaxed)),
        || IMAGE_MEMORY_LIMIT_DETAIL.into(),
    );

    Ok(PageChars {
        chars,
        markers,
        warnings: sink.finish(),
    })
}

/// The fork's thread-local signal counters at one moment (module docs). The
/// warning counts are the differences between two samples on this thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counters {
    /// H2a: objects refused for nesting deeper than the parser's cap.
    object_depth: u64,
    /// H10a: content bytes the lexer skipped to resynchronise (T35).
    content_damage: u64,
    /// H10f: stream decodes over `max_stream_decoded_bytes`.
    decode_limit: u64,
    /// H10e: fonts replaced by a substitute (T37).
    font_substitutions: u64,
    /// H13: images refused for exceeding `max_image_pixels` (as declared, as
    /// a codec's header reports, or as decoded).
    image_pixels: u64,
    /// H14, H15: images refused by a codec's memory budget.
    image_memory: u64,
}

impl Counters {
    fn sample() -> Self {
        Self {
            object_depth: object_depth_limit_hits(),
            content_damage: content_damage_events(),
            decode_limit: stream_decode_limit_hits(),
            font_substitutions: font_substitutions(),
            image_pixels: image_pixel_limit_hits(),
            image_memory: image_memory_limit_hits(),
        }
    }

    fn since(&self, before: &Self) -> Self {
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

/// The transform from PDF user space to page space (`initial_transform(false)`:
/// crop-box origin, `/Rotate` applied, y up) and the page's visible box,
/// CropBox ∩ MediaBox, in page space (T22).
fn page_space(page: &hayro_syntax::page::Page<'_>) -> (Affine, Rect) {
    let initial = page.initial_transform(false).to_kurbo();
    let visible = (initial * page.intersected_crop_box().to_kurbo().to_path(0.1)).bounding_box();
    (initial, visible)
}

/// Access for the unit tests in `text::tests`.
#[cfg(test)]
pub(super) mod tests_support {
    use std::cell::Cell;

    use hayro_interpret::InterpretStats;

    std::thread_local! {
        /// `Context::stats()` of the last page interpreted on this thread,
        /// for tests that check how interpretation ended.
        pub(super) static LAST_STATS: Cell<Option<InterpretStats>> = const { Cell::new(None) };
    }

    pub(in crate::text) fn sink_counts() -> super::SinkCounts {
        super::SinkCounts::default()
    }

    /// See [`LAST_STATS`].
    pub(in crate::text) fn last_stats() -> Option<InterpretStats> {
        LAST_STATS.with(Cell::get)
    }
}
