//! The vocabulary of the batch helper `Document::extract` (run 0002 §1.4,
//! §3.3 `extract.rs`; design spec §12.4): its options, its per-page and
//! per-document results, and [`PageRecognizer`], the seam through which it
//! calls an OCR engine.
//!
//! The engine lives in `pdf-extract-ocr` (it links Tesseract; this crate
//! must not, D-2), so core sees OCR only through the object-safe
//! [`PageRecognizer`] trait, which the engine implements. That keeps
//! `cargo test -p pdf-extract-core` free of the C++ build while the policy
//! of which pages to recognise stays here, next to the text path it falls
//! back to.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::document::Document;
use crate::error::{OcrError, PageError};
use crate::page::Page;
use crate::text::{BlockSeparator, TextOptions, TextPage};
use crate::warning::{Warning, WarningKind};

/// Something that can turn a page into text by recognition: in practice
/// `pdf_extract_ocr::OcrEngine`; in tests, any stand-in.
///
/// Object-safe, so [`ExtractOptions`] can hold `&dyn PageRecognizer` and the
/// core needs no generic parameter for the engine type. `Send + Sync`
/// because `extract` calls it from several worker threads at once.
pub trait PageRecognizer: Send + Sync {
    /// Recognises `page` at `dpi` (`None`: the recogniser's own default).
    ///
    /// # Errors
    /// [`OcrError::Unavailable`] when OCR cannot run in this process,
    /// [`OcrError::Page`] when this page failed, [`OcrError::Closed`] when
    /// the document was closed first.
    fn recognize(&self, page: &Page, dpi: Option<u32>) -> Result<TextPage, OcrError>;
}

/// Options of `Document::extract` (run 0002 O22). The defaults are the
/// design spec's §12.4 policy: OCR off, and a page whose whitespace-trimmed
/// text has fewer than 50 code points counts as sparse.
#[derive(Clone)]
pub struct ExtractOptions<'a> {
    /// A page whose trimmed text has fewer code points than this is
    /// *sparse* and is a candidate for OCR. Default 50.
    pub min_chars: usize,
    /// The recogniser for sparse pages; `None` (the default) disables OCR.
    pub ocr: Option<&'a dyn PageRecognizer>,
    /// At most this many sparse pages, the first in page order, are
    /// recognised. Default 500.
    pub max_ocr_pages: usize,
    /// Worker threads for the text pass and for OCR; 1 (the default) runs
    /// on the calling thread.
    pub workers: usize,
    /// How blocks are separated in each page's text. Default
    /// [`BlockSeparator::Newline`].
    pub block_separator: BlockSeparator,
    /// Options of the text pass.
    pub text: TextOptions,
}

impl Default for ExtractOptions<'_> {
    fn default() -> Self {
        Self {
            min_chars: 50,
            ocr: None,
            max_ocr_pages: 500,
            workers: 1,
            block_separator: BlockSeparator::Newline,
            text: TextOptions::default(),
        }
    }
}

impl fmt::Debug for ExtractOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A recogniser has no `Debug` requirement; say only whether one is
        // set.
        f.debug_struct("ExtractOptions")
            .field("min_chars", &self.min_chars)
            .field("ocr", &self.ocr.map(|_| "<recognizer>"))
            .field("max_ocr_pages", &self.max_ocr_pages)
            .field("workers", &self.workers)
            .field("block_separator", &self.block_separator)
            .field("text", &self.text)
            .finish()
    }
}

/// Where a [`PageResult`]'s text came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSource {
    /// The text layer, and OCR was not attempted.
    Text,
    /// OCR, which produced non-empty text.
    Ocr,
    /// The text layer, after OCR was attempted and failed or found nothing;
    /// [`PageResult::ocr_error`] says why.
    TextFallback,
}

/// How OCR went for the whole document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrStatus {
    /// No page was sparse, or OCR was not requested.
    NotNeeded,
    /// Every attempted recognition succeeded.
    Ok,
    /// OCR could not run; sparse pages kept their text layer.
    Unavailable,
    /// At least one page's recognition failed.
    Partial,
    /// More pages were sparse than `max_ocr_pages` allowed (takes precedence
    /// over [`Partial`](Self::Partial)).
    BudgetExhausted,
}

/// The outcome for one page of `Document::extract`.
#[derive(Clone, Debug, PartialEq)]
pub struct PageResult {
    /// The page's 0-based index.
    pub index: usize,
    /// The page's text (from [`source`](Self::source)).
    pub text: String,
    /// Where the text came from.
    pub source: PageSource,
    /// Why OCR did not replace the text layer, when it was attempted and
    /// did not.
    pub ocr_error: Option<String>,
}

/// The result of `Document::extract`: one row per page, in page order.
#[derive(Clone, Debug, PartialEq)]
pub struct Extraction {
    /// The document's page count (also `pages.len()`).
    pub page_count: usize,
    /// One result per page, in page order.
    pub pages: Vec<PageResult>,
    /// How OCR went for the document.
    pub ocr_status: OcrStatus,
    /// Document-level warnings, then page-level failures of the text pass
    /// (prefixed `page N: `), then the OCR notes of recognised pages that
    /// tell why a page's OCR text is empty or came from a half-resolution
    /// raster (the noise pre-check's warnings, detail prefixed
    /// `page N: `), in page order.
    pub warnings: Vec<Warning>,
}

static_assertions::assert_impl_all!(Extraction: Send, Sync);
static_assertions::assert_impl_all!(PageResult: Send, Sync);
static_assertions::assert_impl_all!(ExtractOptions<'static>: Send, Sync);

impl Document {
    /// The batch helper of design spec §12.4 (run 0002 O22–O25): every
    /// page's text layer, then OCR of the sparse pages, one row per page in
    /// page order.
    ///
    /// 1. **Text pass** over every page, `opts.workers` threads each taking
    ///    a contiguous range of pages. A page whose pass runs out of a
    ///    per-page budget gets `text = ""` and a
    ///    [`WarningKind::PageLimit`] warning instead of failing the call.
    /// 2. A page is **sparse** when its text, trimmed of Unicode whitespace,
    ///    has fewer than `opts.min_chars` code points (Python's
    ///    `len(text.strip()) < min_chars`).
    /// 3. With `opts.ocr`, the first `opts.max_ocr_pages` sparse pages **in
    ///    page order** are recognised, across the same number of threads.
    ///    The first [`OcrError::Unavailable`] stops OCR for the rest of the
    ///    document. Which pages are *attempted* depends only on page order
    ///    and `min_chars` (C6); once OCR turns out unavailable with
    ///    `workers > 1`, pages already in flight on other workers keep their
    ///    outcome, so which of them end as `Ocr` depends on timing.
    /// 4. A recognised page whose `TextPage` carries a noise pre-check
    ///    warning ([`WarningKind::OcrNoiseDownsampled`],
    ///    [`WarningKind::OcrNoiseSkipped`]) adds one warning of that kind to
    ///    [`Extraction::warnings`], after the page-limit ones, in page
    ///    order, its detail the page's warning text prefixed `page N: `
    ///    (run 0003 F14): `PageResult` has no warnings of its own, and a
    ///    skipped page would otherwise look like a blank one.
    ///
    /// `workers` of 0 is treated as 1; with 1 no thread is created. Worker
    /// threads are scoped and joined before this returns; a panic in one
    /// is resumed on the calling thread after the others stop, each
    /// finishing only the page it is on.
    ///
    /// # Errors
    /// [`PageError::Closed`] when the document is (or becomes) closed
    /// before every page is done; the partial result is discarded.
    pub fn extract(&self, opts: &ExtractOptions<'_>) -> Result<Extraction, PageError> {
        let page_count = self.page_count();
        let workers = opts.workers.max(1);

        let texts = text_pass(self, opts, workers)?;
        let mut warnings = self.warnings();
        let mut pages = Vec::with_capacity(page_count);
        let mut sparse = Vec::new();
        for (index, outcome) in texts.into_iter().enumerate() {
            let text = match outcome {
                Ok(text) => {
                    if is_sparse(&text, opts.min_chars) {
                        sparse.push(index);
                    }
                    text
                }
                // Not an OCR candidate although its text is empty: the
                // renderer runs the same interpreter under the same budget,
                // so it would fail the same way (O23: `source = Text`).
                Err(e) => {
                    warnings.push(Warning {
                        page: Some(index),
                        kind: WarningKind::PageLimit,
                        detail: e.to_string(),
                        count: 1,
                    });
                    String::new()
                }
            };
            pages.push(PageResult {
                index,
                text,
                source: PageSource::Text,
                ocr_error: None,
            });
        }

        let ocr_status = match opts.ocr {
            Some(recognizer) if !sparse.is_empty() => {
                let budget_exhausted = sparse.len() > opts.max_ocr_pages;
                sparse.truncate(opts.max_ocr_pages);
                let outcome = ocr_pass(
                    self,
                    recognizer,
                    &sparse,
                    opts,
                    workers,
                    &mut pages,
                    &mut warnings,
                )?;
                if outcome.unavailable {
                    OcrStatus::Unavailable
                } else if budget_exhausted {
                    OcrStatus::BudgetExhausted
                } else if outcome.failed {
                    OcrStatus::Partial
                } else {
                    OcrStatus::Ok
                }
            }
            _ => OcrStatus::NotNeeded,
        };

        Ok(Extraction {
            page_count,
            pages,
            ocr_status,
            warnings,
        })
    }
}

/// O22 (2): fewer than `min_chars` code points once Unicode whitespace
/// (`char::is_whitespace`, Unicode `White_Space`) is trimmed from both ends.
/// This is not quite the set Python's `str.strip()` removes: Python also
/// strips U+001C..U+001F, which Rust does not count as whitespace. The text
/// path replaces those controls (0001 D-10), so the difference cannot show in
/// extracted text; the parity gate (`py_strip` in `tests/parity/gate_b.rs`)
/// and `sparse_rule.json` account for it where they compare against Python.
fn is_sparse(text: &str, min_chars: usize) -> bool {
    text.trim().chars().count() < min_chars
}

/// A text-pass outcome that does not fail the call: the page's text, or the
/// limit it ran out of.
type TextOutcome = Result<String, PageError>;

/// Splits `0..n` into `workers` contiguous ranges of near-equal length (the
/// first `n % workers` one longer), dropping empty ones.
fn ranges(n: usize, workers: usize) -> Vec<std::ops::Range<usize>> {
    let (base, extra) = (n / workers, n % workers);
    let mut start = 0;
    (0..workers)
        .map(|w| {
            let len = base + usize::from(w < extra);
            let r = start..start + len;
            start += len;
            r
        })
        .filter(|r| !r.is_empty())
        .collect()
}

/// Runs `f` on `items` across `workers` threads: the calling thread takes
/// the first item, `workers - 1` scoped threads the rest. Results are in
/// item order. A panic in any thread is resumed here after all have
/// joined, with its own payload (`thread::scope`'s own panic would replace
/// it with a generic message).
fn run_parallel<I: Sync, R: Send>(items: &[I], f: impl Fn(&I) -> R + Sync) -> Vec<R> {
    let Some((first, rest)) = items.split_first() else {
        return Vec::new();
    };
    if rest.is_empty() {
        return vec![f(first)];
    }
    std::thread::scope(|scope| {
        let f = &f;
        let handles: Vec<_> = rest
            .iter()
            .map(|item| scope.spawn(move || f(item)))
            .collect();
        // Caught so the spawned threads are joined (and their own panics
        // seen) before any panic leaves the scope.
        let mine = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(first)));
        let theirs: Vec<_> = handles.into_iter().map(|h| h.join()).collect();
        let mut out = Vec::with_capacity(items.len());
        for r in std::iter::once(mine).chain(theirs) {
            match r {
                Ok(v) => out.push(v),
                Err(payload) => std::panic::resume_unwind(payload),
            }
        }
        out
    })
}

/// O22 (1): every page's text, `workers` contiguous ranges in parallel.
/// `Closed` (or any non-limit error) fails the whole pass.
fn text_pass(
    doc: &Document,
    opts: &ExtractOptions<'_>,
    workers: usize,
) -> Result<Vec<TextOutcome>, PageError> {
    let chunks = run_ranges(&ranges(doc.page_count(), workers), |index| {
        let page = doc.page(index)?;
        match page.text_page(&opts.text) {
            Ok(tp) => Ok(Ok(tp.to_text(opts.block_separator))),
            Err(e @ PageError::Limit { .. }) => Ok(Err(e)),
            Err(e) => Err(e),
        }
    });
    let mut out = Vec::with_capacity(doc.page_count());
    for chunk in chunks {
        out.extend(chunk?);
    }
    Ok(out)
}

/// Runs `f` on every index of each range, one range per worker
/// ([`run_parallel`]), stopping a range at its first `Err`. A panic in one
/// worker stops the others before their next index, since the call ends in
/// that panic and the rest of their work would be discarded. A range cut
/// short that way is never seen: `run_parallel` resumes a panic whenever one
/// occurred. With several faults, which panic is resumed can then depend on
/// timing (a worker stopped early never reaches its own).
fn run_ranges<R: Send>(
    ranges: &[std::ops::Range<usize>],
    f: impl Fn(usize) -> Result<R, PageError> + Sync,
) -> Vec<Result<Vec<R>, PageError>> {
    /// Raises the flag when its worker unwinds. Whether the thread was
    /// already unwinding at the start is remembered: `extract` called from a
    /// `Drop` during a panic would otherwise raise it on a normal return and
    /// cut the other ranges short with no panic to resume.
    struct AbortOnPanic<'a>(&'a AtomicBool, bool);
    impl Drop for AbortOnPanic<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() && !self.1 {
                self.0.store(true, Ordering::Release);
            }
        }
    }
    let abort = AtomicBool::new(false);
    run_parallel(ranges, |range| {
        let _guard = AbortOnPanic(&abort, std::thread::panicking());
        let mut out = Vec::with_capacity(range.len());
        for index in range.clone() {
            if abort.load(Ordering::Acquire) {
                break;
            }
            out.push(f(index)?);
        }
        Ok(out)
    })
}

/// What the OCR pass saw, for [`OcrStatus`].
struct OcrOutcome {
    unavailable: bool,
    failed: bool,
}

/// The result of recognising one selected page.
enum Recognised {
    /// Non-empty OCR text, and the page's OCR notes.
    Text(String, Vec<Warning>),
    /// Recognition ran and found nothing (or was skipped by the engine's
    /// noise pre-check), and the page's OCR notes.
    Empty(Vec<Warning>),
    /// This page failed (`OcrPageError`).
    Failed(String),
    /// OCR is unavailable; this page was the one that found out, or was
    /// not started after another did.
    Unavailable(String),
}

/// The warnings of an OCR `TextPage` that `extract` repeats at document
/// level (step 4 of `Document::extract`), each as `page N: <its text>`.
fn ocr_notes(index: usize, tp: &TextPage) -> Vec<Warning> {
    tp.warnings()
        .iter()
        .filter(|w| {
            matches!(
                w.kind,
                WarningKind::OcrNoiseDownsampled | WarningKind::OcrNoiseSkipped
            )
        })
        .map(|w| Warning {
            page: Some(index),
            kind: w.kind,
            detail: format!("page {index}: {}: {}", w.kind.category(), w.detail),
            count: 1,
        })
        .collect()
}

/// O22 (3) and O23: recognises `selected` (sparse pages, in page order,
/// already cut to the budget), rewrites their rows and appends their OCR
/// notes to `warnings` in page order.
///
/// Workers pull the next page from a shared cursor rather than a fixed
/// range: which pages are recognised is fixed by `selected` (C6), only the
/// order of the calls varies, and recognition times vary far more than
/// text-pass times. The first `Unavailable` raises a flag that every worker
/// checks before starting a page, so no recognition starts after it.
fn ocr_pass(
    doc: &Document,
    recognizer: &dyn PageRecognizer,
    selected: &[usize],
    opts: &ExtractOptions<'_>,
    workers: usize,
    pages: &mut [PageResult],
    warnings: &mut Vec<Warning>,
) -> Result<OcrOutcome, PageError> {
    let cursor = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    // Set when any worker fails the call (`Closed`) or panics, so the others
    // stop pulling pages instead of recognising ones whose results will be
    // discarded.
    let abort = AtomicBool::new(false);
    // The unavailability message, set once by the first worker to see it.
    let unavailable: Mutex<Option<String>> = Mutex::new(None);
    let results: Vec<Mutex<Option<Recognised>>> =
        selected.iter().map(|_| Mutex::new(None)).collect();

    let worker = |_: &()| -> Result<(), PageError> {
        /// Raises `abort` when the worker leaves by panic or by `Err`.
        struct Abort<'a>(&'a AtomicBool, bool);
        impl Drop for Abort<'_> {
            fn drop(&mut self) {
                if !self.1 {
                    self.0.store(true, Ordering::Release);
                }
            }
        }
        let mut guard = Abort(&abort, false);
        let result = (|| loop {
            if abort.load(Ordering::Acquire) {
                return Ok(());
            }
            let slot = cursor.fetch_add(1, Ordering::Relaxed);
            let Some(&index) = selected.get(slot) else {
                return Ok(());
            };
            if stop.load(Ordering::Acquire) {
                // Left for the fill-in below.
                continue;
            }
            let page = doc.page(index)?;
            let outcome = match recognizer.recognize(&page, None) {
                Ok(tp) => {
                    let text = tp.to_text(opts.block_separator);
                    let notes = ocr_notes(index, &tp);
                    // Whitespace-only counts as empty: it would replace a
                    // text layer with nothing useful.
                    if text.trim().is_empty() {
                        Recognised::Empty(notes)
                    } else {
                        Recognised::Text(text, notes)
                    }
                }
                Err(OcrError::Page(e)) => Recognised::Failed(e.to_string()),
                Err(e @ OcrError::Unavailable(_)) => {
                    let message = e.to_string();
                    let mut first = unavailable.lock().unwrap_or_else(PoisonError::into_inner);
                    first.get_or_insert_with(|| message.clone());
                    stop.store(true, Ordering::Release);
                    Recognised::Unavailable(message)
                }
                Err(OcrError::Closed) => return Err(PageError::Closed),
            };
            *results[slot].lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome);
        })();
        guard.1 = result.is_ok();
        result
    };
    let threads = vec![(); workers.min(selected.len())];
    for r in run_parallel(&threads, worker) {
        r?;
    }

    let unavailable = unavailable
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner);
    let mut outcome = OcrOutcome {
        unavailable: unavailable.is_some(),
        failed: false,
    };
    for (&index, result) in selected.iter().zip(results) {
        let result = result.into_inner().unwrap_or_else(PoisonError::into_inner);
        let row = &mut pages[index];
        match result {
            Some(Recognised::Text(text, notes)) => {
                row.text = text;
                row.source = PageSource::Ocr;
                warnings.extend(notes);
            }
            Some(Recognised::Empty(notes)) => {
                row.source = PageSource::TextFallback;
                warnings.extend(notes);
            }
            Some(Recognised::Failed(message)) => {
                outcome.failed = true;
                row.source = PageSource::TextFallback;
                row.ocr_error = Some(message);
            }
            Some(Recognised::Unavailable(message)) => {
                row.source = PageSource::TextFallback;
                row.ocr_error = Some(message);
            }
            // Skipped after OCR became unavailable (a page is only ever
            // skipped once the flag, and so the message, is set).
            None => {
                row.source = PageSource::TextFallback;
                row.ocr_error.clone_from(&unavailable);
            }
        }
    }
    Ok(outcome)
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod extract_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// A recogniser that always reports the document closed; enough to
    /// show the trait is usable as `&dyn` from an options value.
    struct Refuses;

    impl PageRecognizer for Refuses {
        fn recognize(&self, _page: &Page, _dpi: Option<u32>) -> Result<TextPage, OcrError> {
            Err(OcrError::Closed)
        }
    }

    #[test]
    fn defaults_match_the_spec() {
        let o = ExtractOptions::default();
        assert_eq!(o.min_chars, 50);
        assert!(o.ocr.is_none());
        assert_eq!(o.max_ocr_pages, 500);
        assert_eq!(o.workers, 1);
        assert_eq!(o.block_separator, BlockSeparator::Newline);
        assert_eq!(o.text, TextOptions::default());
    }

    #[test]
    fn recognizer_is_object_safe_and_callable_through_options() {
        let r = Refuses;
        let dyn_r: &dyn PageRecognizer = &r;
        let o = ExtractOptions {
            ocr: Some(dyn_r),
            ..ExtractOptions::default()
        };
        let debug = format!("{o:?}");
        assert!(debug.contains("<recognizer>"), "{debug}");
        assert!(format!("{:?}", ExtractOptions::default()).contains("ocr: None"));
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/corpus-c/enc_aes_128_owner.pdf"
        ))
        .expect("fixture");
        let doc = crate::Document::from_bytes(bytes, &crate::OpenOptions::default())
            .expect("fixture opens");
        let page = doc.page(0).expect("page 0");
        let recognizer = o.ocr.expect("set above");
        assert!(matches!(
            recognizer.recognize(&page, Some(72)),
            Err(OcrError::Closed)
        ));
    }

    #[test]
    fn result_types_compare_and_clone() {
        let row = PageResult {
            index: 0,
            text: "x\n".into(),
            source: PageSource::TextFallback,
            ocr_error: Some("recognition failed: boom".into()),
        };
        let e = Extraction {
            page_count: 1,
            pages: vec![row.clone()],
            ocr_status: OcrStatus::Partial,
            warnings: Vec::new(),
        };
        assert_eq!(e.clone(), e);
        assert_eq!(e.pages[0], row);
        for s in [PageSource::Text, PageSource::Ocr, PageSource::TextFallback] {
            assert!(!format!("{s:?}").is_empty());
        }
        for s in [
            OcrStatus::NotNeeded,
            OcrStatus::Ok,
            OcrStatus::Unavailable,
            OcrStatus::Partial,
            OcrStatus::BudgetExhausted,
        ] {
            assert!(!format!("{s:?}").is_empty());
        }
    }
}
