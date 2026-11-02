//! Tests of `Document::extract` (run 0002 §4.10) against an in-crate fake
//! recogniser. Core cannot depend on `pdf-extract-ocr` (D-2), so the same
//! oracle is exercised again through the real engine and `ScriptedBackend`
//! in `crates/ocr/tests/extract.rs`.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pdf_extract_testkit::{PdfBuilder, TextStyle};

use super::*;
use crate::error::{OcrPageError, OcrUnavailable, UnavailableReason};
use crate::{LimitKind, Limits, OpenOptions};

/// 60 code points of body text: never sparse at the default 50.
const BODY: &str = "The quick brown fox jumps over the lazy dog, twice. Done.  ";

/// A document of `n` pages; the pages in `sparse` carry a short label only,
/// the pages in `flood` an operator flood (for the ops budget), the rest
/// body text.
fn doc_with(n: usize, sparse: &[usize], flood: &[usize], limits: Limits) -> Document {
    let mut b = PdfBuilder::new();
    for i in 0..n {
        let page = b.add_page_a4();
        if flood.contains(&i) {
            page.raw_content(&"q Q ".repeat(2_000));
        } else if sparse.contains(&i) {
            page.insert_text((72.0, 72.0), &format!("p{i}"), TextStyle::default())
                .expect("label");
        } else {
            page.insert_text(
                (72.0, 72.0),
                &format!("{i:02} {BODY}"),
                TextStyle::default(),
            )
            .expect("body");
        }
    }
    Document::from_bytes(
        b.to_bytes(),
        &OpenOptions {
            max_pages: None,
            limits,
        },
    )
    .expect("builder document opens")
}

fn doc(n: usize, sparse: &[usize]) -> Document {
    doc_with(n, sparse, &[], Limits::default())
}

/// What the fake does for one page.
#[derive(Clone, Copy, Debug)]
enum Act {
    Text,
    Empty,
    Fail,
    Unavailable,
    Closed,
    Panic,
    /// Succeeds after 200 ms: a page still in flight when another worker
    /// finds OCR unavailable.
    Slow,
    /// What the engine's noise pre-check returns (run 0003 F13, F14):
    /// recognised text at half resolution with an `OcrNoiseDownsampled`
    /// warning, after an unrelated page warning.
    Downsampled,
    /// An empty page with an `OcrNoiseSkipped` warning.
    Skipped,
}

/// The engine's warning of kind `kind` on page `page`, as its `TextPage`
/// carries it.
fn noise_warning(page: usize, kind: WarningKind) -> Warning {
    Warning {
        page: Some(page),
        kind,
        detail: match kind {
            WarningKind::OcrNoiseSkipped => {
                "noise-dominated raster skipped (no convincing foreground at 300 or 150 dpi)"
            }
            _ => "noise-dominated raster recognised at 150 dpi (rendered at 300)",
        }
        .into(),
        count: 1,
    }
}

/// A recogniser scripted per page index, recording the pages it was asked
/// for in call order.
struct Fake {
    recognised: TextPage,
    acts: HashMap<usize, Act>,
    calls: Mutex<Vec<usize>>,
    /// Closes this document on the first call (the close-during-OCR case).
    close_on_call: Option<Document>,
}

impl Fake {
    fn new(acts: &[(usize, Act)]) -> Self {
        let src = doc(1, &[]);
        let recognised = src
            .page(0)
            .and_then(|p| p.text_page(&TextOptions::default()))
            .expect("source page");
        Self {
            recognised,
            acts: acts.iter().copied().collect(),
            calls: Mutex::new(Vec::new()),
            close_on_call: None,
        }
    }

    fn calls(&self) -> Vec<usize> {
        self.calls.lock().expect("calls").clone()
    }
}

impl PageRecognizer for Fake {
    fn recognize(&self, page: &Page, dpi: Option<u32>) -> Result<TextPage, OcrError> {
        assert_eq!(dpi, None, "extract uses the engine's own dpi");
        self.calls.lock().expect("calls").push(page.index());
        if let Some(d) = &self.close_on_call {
            d.close();
        }
        match self.acts.get(&page.index()).copied().unwrap_or(Act::Text) {
            Act::Text => Ok(self.recognised.clone()),
            Act::Empty => Ok(TextPage::new(Vec::new(), Vec::new())),
            Act::Fail => Err(OcrPageError::Recognition("scripted".into()).into()),
            Act::Unavailable => Err(OcrUnavailable {
                reason: UnavailableReason::NoTessdata,
                tessdata: None,
                languages: vec!["eng".into()],
                searched: Vec::new(),
                detail: "scripted".into(),
            }
            .into()),
            Act::Closed => Err(OcrError::Closed),
            Act::Panic => panic!("scripted recogniser panic"),
            Act::Slow => {
                std::thread::sleep(std::time::Duration::from_millis(200));
                Ok(self.recognised.clone())
            }
            Act::Downsampled => Ok(TextPage::new(
                self.recognised.blocks().to_vec(),
                vec![
                    Warning {
                        page: Some(page.index()),
                        kind: WarningKind::ImageDecodeFailure,
                        detail: "not a noise note".into(),
                        count: 1,
                    },
                    noise_warning(page.index(), WarningKind::OcrNoiseDownsampled),
                ],
            )),
            Act::Skipped => Ok(TextPage::new(
                Vec::new(),
                vec![noise_warning(page.index(), WarningKind::OcrNoiseSkipped)],
            )),
        }
    }
}

fn with_ocr<'a>(r: &'a Fake, max_ocr_pages: usize, workers: usize) -> ExtractOptions<'a> {
    ExtractOptions {
        ocr: Some(r),
        max_ocr_pages,
        workers,
        ..ExtractOptions::default()
    }
}

#[test]
fn text_pass_is_the_same_for_any_worker_count() {
    let d = doc(40, &[3, 7, 12]);
    let one = d.extract(&ExtractOptions::default()).expect("extract");
    assert_eq!(one.page_count, 40);
    assert_eq!(one.pages.len(), 40);
    assert_eq!(one.ocr_status, OcrStatus::NotNeeded);
    for (i, row) in one.pages.iter().enumerate() {
        let page = d.page(i).expect("page");
        assert_eq!(row.index, i);
        assert_eq!(row.text, page.text(&TextOptions::default()).expect("text"));
        assert_eq!(row.source, PageSource::Text);
        assert_eq!(row.ocr_error, None);
    }
    for workers in [0, 2, 4, 7, 40, 100] {
        let many = d
            .extract(&ExtractOptions {
                workers,
                ..ExtractOptions::default()
            })
            .expect("extract");
        assert_eq!(many, one, "workers = {workers}");
    }
}

#[test]
fn block_separator_comes_from_the_extract_options() {
    let d = doc(2, &[]);
    let r = d
        .extract(&ExtractOptions {
            block_separator: BlockSeparator::BlankLine,
            ..ExtractOptions::default()
        })
        .expect("extract");
    let page = d.page(0).expect("page");
    let expected = page
        .text_page(&TextOptions::default())
        .expect("text")
        .to_text(BlockSeparator::BlankLine);
    assert_eq!(r.pages[0].text, expected);
}

#[test]
fn an_empty_document_extracts_to_no_rows() {
    // A page tree with zero pages is refused at open, so a zero-page
    // `Document` cannot be built; `ranges` covers the n = 0 split.
    assert!(ranges(0, 4).is_empty());
    assert_eq!(ranges(5, 2), vec![0..3, 3..5]);
    assert_eq!(ranges(3, 8), vec![0..1, 1..2, 2..3]);
    assert_eq!(ranges(40, 4).iter().map(|r| r.len()).sum::<usize>(), 40);
}

#[test]
fn sparse_rule_trims_unicode_whitespace() {
    let spaces = " ".repeat(20);
    assert!(is_sparse(
        &format!("{spaces}{}{spaces}", "x".repeat(49)),
        50
    ));
    assert!(!is_sparse(
        &format!("{spaces}{}{spaces}", "x".repeat(50)),
        50
    ));
    // NBSP and EM SPACE are whitespace to `str::trim` (and to Python's
    // `str.strip`).
    assert!(is_sparse(&"\u{a0}\u{2003}".repeat(25), 50));
    // Interior whitespace counts.
    assert!(!is_sparse(&format!("x{}x", "\u{2003}".repeat(48)), 50));
    // Code points, not bytes.
    assert!(!is_sparse(&"é".repeat(50), 50));
    assert!(!is_sparse("", 0));
}

/// The table shared with the Python suite (spec 4.10: "a table of 12
/// strings is asserted on both sides"); tests/python/test_extract.py checks
/// the same rows against `len(text.strip()) < min_chars`.
#[test]
fn sparse_rule_matches_the_shared_table() {
    let doc: serde_json::Value =
        serde_json::from_str(include_str!("../../testkit/tests/data/sparse_rule.json"))
            .expect("valid JSON");
    let rows = doc["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 12);
    for row in rows {
        let text = row["text"].as_str().expect("text");
        let min_chars =
            usize::try_from(row["min_chars"].as_u64().expect("min_chars")).expect("fits usize");
        let sparse = row["sparse"].as_bool().expect("sparse");
        assert_eq!(is_sparse(text, min_chars), sparse, "{}", row["name"]);
    }
}

#[test]
fn budget_takes_the_first_sparse_pages_in_page_order() {
    let d = doc(40, &[3, 7, 12]);
    for workers in [1, 4] {
        let fake = Fake::new(&[]);
        let r = d.extract(&with_ocr(&fake, 2, workers)).expect("extract");
        assert_eq!(r.pages[3].source, PageSource::Ocr);
        assert_eq!(r.pages[7].source, PageSource::Ocr);
        assert_eq!(
            r.pages[3].text,
            fake.recognised.to_text(BlockSeparator::Newline)
        );
        assert_eq!(r.pages[12].source, PageSource::Text);
        assert_eq!(r.pages[12].ocr_error, None);
        assert_eq!(r.pages[12].text, "p12\n");
        assert_eq!(r.ocr_status, OcrStatus::BudgetExhausted);
        if workers == 1 {
            assert_eq!(fake.calls(), [3, 7]);
        } else {
            let set: BTreeSet<_> = fake.calls().into_iter().collect();
            assert_eq!(set, BTreeSet::from([3, 7]));
        }
        let ocr_rows = r
            .pages
            .iter()
            .filter(|p| p.source == PageSource::Ocr)
            .count();
        assert_eq!(ocr_rows, 2);
    }
}

#[test]
fn every_sparse_page_within_budget_is_ok() {
    let d = doc(10, &[1, 2]);
    let fake = Fake::new(&[]);
    let r = d.extract(&with_ocr(&fake, 500, 3)).expect("extract");
    assert_eq!(r.ocr_status, OcrStatus::Ok);
    assert_eq!(fake.calls().len(), 2);
}

#[test]
fn no_sparse_page_means_not_needed() {
    let d = doc(5, &[]);
    let fake = Fake::new(&[]);
    let r = d.extract(&with_ocr(&fake, 500, 1)).expect("extract");
    assert_eq!(r.ocr_status, OcrStatus::NotNeeded);
    assert!(fake.calls().is_empty());
    // min_chars = 0 makes nothing sparse.
    let d = doc(5, &[2]);
    let r = d
        .extract(&ExtractOptions {
            min_chars: 0,
            ..with_ocr(&fake, 500, 1)
        })
        .expect("extract");
    assert_eq!(r.ocr_status, OcrStatus::NotNeeded);
    assert!(fake.calls().is_empty());
}

#[test]
fn a_zero_budget_attempts_nothing() {
    let d = doc(5, &[2]);
    let fake = Fake::new(&[]);
    let r = d.extract(&with_ocr(&fake, 0, 4)).expect("extract");
    assert_eq!(r.ocr_status, OcrStatus::BudgetExhausted);
    assert_eq!(r.pages[2].source, PageSource::Text);
    assert!(fake.calls().is_empty());
}

#[test]
fn a_page_failure_falls_back_with_its_message() {
    let d = doc(40, &[3, 7, 12]);
    for workers in [1, 4] {
        let fake = Fake::new(&[(7, Act::Fail)]);
        let r = d.extract(&with_ocr(&fake, 500, workers)).expect("extract");
        assert_eq!(r.pages[7].source, PageSource::TextFallback);
        assert_eq!(r.pages[7].text, "p7\n");
        assert_eq!(
            r.pages[7].ocr_error.as_deref(),
            Some("recognition failed: scripted")
        );
        assert_eq!(r.pages[3].source, PageSource::Ocr);
        assert_eq!(r.pages[12].source, PageSource::Ocr);
        assert_eq!(r.ocr_status, OcrStatus::Partial);
    }
}

#[test]
fn budget_exhausted_outranks_partial() {
    let d = doc(40, &[3, 7, 12]);
    let fake = Fake::new(&[(7, Act::Fail)]);
    let r = d.extract(&with_ocr(&fake, 2, 1)).expect("extract");
    assert_eq!(r.ocr_status, OcrStatus::BudgetExhausted);
    assert_eq!(r.pages[7].source, PageSource::TextFallback);
}

#[test]
fn an_empty_result_falls_back_without_an_error() {
    let d = doc(40, &[3, 7, 12]);
    let fake = Fake::new(&[(7, Act::Empty)]);
    let r = d.extract(&with_ocr(&fake, 500, 1)).expect("extract");
    assert_eq!(r.pages[7].source, PageSource::TextFallback);
    assert_eq!(r.pages[7].ocr_error, None);
    assert_eq!(r.pages[7].text, "p7\n");
    assert_eq!(r.ocr_status, OcrStatus::Ok);
}

#[test]
fn unavailable_stops_all_further_ocr() {
    let d = doc(40, &[3, 7, 12]);
    let fake = Fake::new(&[(3, Act::Unavailable)]);
    let r = d.extract(&with_ocr(&fake, 500, 1)).expect("extract");
    assert_eq!(fake.calls(), [3]);
    assert_eq!(r.ocr_status, OcrStatus::Unavailable);
    for i in [3, 7, 12] {
        assert_eq!(r.pages[i].source, PageSource::TextFallback, "page {i}");
        assert_eq!(r.pages[i].text, format!("p{i}\n"));
        assert_eq!(
            r.pages[i].ocr_error.as_deref(),
            Some("OCR unavailable (no_tessdata): scripted"),
            "page {i}"
        );
    }
}

#[test]
fn unavailable_outranks_budget_exhausted_and_holds_in_parallel() {
    let d = doc(40, &[3, 7, 12, 20, 30]);
    let fake = Fake::new(&[(3, Act::Unavailable), (7, Act::Unavailable)]);
    let r = d.extract(&with_ocr(&fake, 4, 4)).expect("extract");
    assert_eq!(r.ocr_status, OcrStatus::Unavailable);
    // Every attempted-or-skipped page fell back; any page that finished
    // before the flag was seen kept its OCR text.
    for i in [3, 7] {
        assert_eq!(r.pages[i].source, PageSource::TextFallback);
        assert!(
            r.pages[i]
                .ocr_error
                .as_deref()
                .is_some_and(|m| m.starts_with("OCR unavailable"))
        );
    }
    for i in [12, 20] {
        assert_ne!(r.pages[i].source, PageSource::Text);
    }
    assert_eq!(r.pages[30].source, PageSource::Text);
}

#[test]
fn a_text_pass_limit_yields_empty_text_and_a_document_warning() {
    let d = doc_with(
        6,
        &[2],
        &[4],
        Limits {
            max_ops_per_page: 1_000,
            ..Limits::default()
        },
    );
    for workers in [1, 3] {
        let fake = Fake::new(&[]);
        let r = d.extract(&with_ocr(&fake, 500, workers)).expect("extract");
        assert_eq!(r.pages[4].text, "");
        assert_eq!(r.pages[4].source, PageSource::Text);
        assert_eq!(r.pages[4].ocr_error, None);
        // The limit page is not an OCR candidate (rendering would hit the
        // same budget); the genuinely sparse page is.
        assert_eq!(fake.calls(), [2]);
        let texts: Vec<String> = r.warnings.iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            ["page 4: limit ops_per_page exceeded while processing the page (1000)"]
        );
        assert_eq!(r.warnings[0].page, Some(4));
        assert_eq!(r.warnings[0].kind, WarningKind::PageLimit);
        let expected = PageError::Limit {
            kind: LimitKind::OpsPerPage,
            value: 1_000,
        };
        assert_eq!(r.warnings[0].detail, expected.to_string());
    }
}

#[test]
fn noise_notes_follow_page_limit_warnings_in_page_order() {
    // Page 1 runs out of the ops budget in the text pass (a page-limit
    // warning); 3 and 12 come back from OCR with noise notes, 7 without.
    let limits = Limits {
        max_ops_per_page: 1_000,
        ..Limits::default()
    };
    let d = doc_with(16, &[3, 7, 12], &[1], limits);
    let doc_warnings = d.warnings().len();
    for workers in [1, 4] {
        let fake = Fake::new(&[(3, Act::Skipped), (12, Act::Downsampled)]);
        let r = d.extract(&with_ocr(&fake, 500, workers)).expect("extract");
        assert_eq!(r.ocr_status, OcrStatus::Ok);
        // Skipped: the empty-OCR outcome of O23, no error.
        assert_eq!(r.pages[3].source, PageSource::TextFallback);
        assert_eq!(r.pages[3].ocr_error, None);
        assert_eq!(r.pages[3].text, "p3\n");
        assert_eq!(r.pages[12].source, PageSource::Ocr);
        assert_eq!(r.pages[7].source, PageSource::Ocr);
        let tail: Vec<_> = r.warnings[doc_warnings..].to_vec();
        let kinds: Vec<_> = tail.iter().map(|w| (w.kind, w.page)).collect();
        assert_eq!(
            kinds,
            [
                (WarningKind::PageLimit, Some(1)),
                (WarningKind::OcrNoiseSkipped, Some(3)),
                (WarningKind::OcrNoiseDownsampled, Some(12)),
            ],
            "workers = {workers}"
        );
        assert!(tail[1].detail.starts_with("page 3: "), "{}", tail[1].detail);
        assert_eq!(
            tail[1].to_string(),
            "page 3: ocr: noise-dominated raster skipped (no convincing foreground at 300 or \
             150 dpi)"
        );
        assert_eq!(
            tail[2].to_string(),
            "page 12: ocr: noise-dominated raster recognised at 150 dpi (rendered at 300)"
        );
        assert!(tail.iter().all(|w| w.count == 1));
    }
}

#[test]
fn document_warnings_are_carried() {
    let d = doc(2, &[]);
    let r = d.extract(&ExtractOptions::default()).expect("extract");
    assert_eq!(r.warnings, d.warnings());
}

#[test]
fn a_closed_document_is_an_error() {
    let d = doc(3, &[]);
    d.close();
    for workers in [1, 2] {
        let r = d.extract(&ExtractOptions {
            workers,
            ..ExtractOptions::default()
        });
        assert_eq!(r, Err(PageError::Closed));
    }
}

#[test]
fn a_recogniser_reporting_closed_fails_the_call() {
    let d = doc(10, &[2, 5]);
    for workers in [1, 2] {
        let fake = Fake::new(&[(5, Act::Closed)]);
        assert_eq!(
            d.extract(&with_ocr(&fake, 500, workers)),
            Err(PageError::Closed)
        );
    }
}

#[test]
fn closing_during_the_ocr_pass_fails_the_call() {
    let d = doc(10, &[2, 5]);
    let mut fake = Fake::new(&[]);
    fake.close_on_call = Some(d.clone());
    // Page 2's recognition closes the document; page 5 then cannot be
    // opened.
    assert_eq!(d.extract(&with_ocr(&fake, 500, 1)), Err(PageError::Closed));
}

#[test]
fn closing_from_another_thread_never_panics() {
    let mut closed = 0;
    for round in 0..100 {
        let d = doc(12, &[3]);
        let fake = Fake::new(&[]);
        let closer = d.clone();
        std::thread::scope(|s| {
            s.spawn(move || {
                if round % 2 == 0 {
                    std::thread::yield_now();
                }
                closer.close();
            });
            match d.extract(&with_ocr(&fake, 500, 1 + round % 4)) {
                Ok(r) => assert_eq!(r.pages.len(), 12),
                Err(e) => {
                    assert_eq!(e, PageError::Closed);
                    closed += 1;
                }
            }
        });
    }
    // The race is real: some rounds lose it.
    assert!(closed > 0, "no round saw the close");
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default()
}

#[test]
fn a_recogniser_panic_propagates_with_its_payload() {
    let d = doc(10, &[2, 5, 8]);
    for workers in [1, 4] {
        let fake = Fake::new(&[(5, Act::Panic)]);
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            d.extract(&with_ocr(&fake, 500, workers))
        }))
        .expect_err("the panic propagates");
        assert_eq!(panic_message(&*payload), "scripted recogniser panic");
    }
}

#[test]
fn run_parallel_resumes_a_panic_from_either_side() {
    let items = [0, 1, 2, 3];
    // Item 0 runs on the calling thread, the others on spawned ones.
    for bad in [0, 2] {
        let payload = std::panic::catch_unwind(|| {
            run_parallel(&items, |&i| {
                assert!(i != bad, "item {i}");
                i
            })
        })
        .expect_err("panics");
        assert_eq!(panic_message(&*payload), format!("item {bad}"));
    }
    assert_eq!(run_parallel(&items, |&i| i * 2), [0, 2, 4, 6]);
    assert_eq!(run_parallel(&[] as &[i32], |&i| i), Vec::<i32>::new());
}

#[test]
fn a_panicking_text_worker_stops_the_others() {
    // 30 pages over 3 ranges; the first page of one range panics, and every
    // other page waits until the panicking worker's thread has exited. Its
    // abort guard drops while the worker unwinds, before the thread-local
    // `RaiseOnExit` drops at thread exit, so once a waiting page returns the
    // flag is already raised. Each other worker therefore makes at most the
    // one call it had in hand (none, if it started after the flag rose):
    // the calls are the bad page plus a subset of the other range starts.
    // A flag set just before the panic would not do: the others could
    // return and pull their next page before the unwind reached the guard.
    // Without the guard every page runs.
    //
    // The bad page must be on a spawned worker: the calling thread runs the
    // first range and does not exit until it has joined the others, so its
    // exit cannot signal them. Both sides run the same guard, and
    // `run_parallel_resumes_a_panic_from_either_side` covers which side
    // panicked.
    struct RaiseOnExit(Arc<AtomicBool>);
    impl Drop for RaiseOnExit {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    thread_local! {
        static ON_EXIT: RefCell<Option<RaiseOnExit>> = const { RefCell::new(None) };
    }
    // Reached only if the panicking worker never exits, so that a
    // regression fails instead of hanging. It bounds each `bad` iteration as
    // a whole (one deadline, below), not each waiting call: per call, a
    // regression that ran the pages one after another would wait up to
    // SAFETY once per page.
    const SAFETY: Duration = Duration::from_secs(60);
    let ranges = ranges(30, 3);
    let starts: Vec<usize> = ranges.iter().map(|r| r.start).collect();
    for bad in [10, 20] {
        let exited = Arc::new(AtomicBool::new(false));
        let timed_out = AtomicBool::new(false);
        let calls = Mutex::new(Vec::new());
        let deadline = Instant::now() + SAFETY;
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_ranges(&ranges, |i| {
                calls.lock().expect("calls").push(i);
                if i == bad {
                    ON_EXIT.with(|e| *e.borrow_mut() = Some(RaiseOnExit(Arc::clone(&exited))));
                    panic!("page {i}");
                }
                while !exited.load(Ordering::Acquire) {
                    if Instant::now() > deadline {
                        timed_out.store(true, Ordering::Relaxed);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Ok(i)
            })
        }))
        .expect_err("the panic propagates");
        assert_eq!(panic_message(&*payload), format!("page {bad}"));
        assert!(
            !timed_out.into_inner(),
            "{bad}: the bad worker never exited"
        );
        let calls = calls.into_inner().expect("calls");
        assert!(calls.contains(&bad), "{bad}: {calls:?}");
        assert!(calls.iter().all(|i| starts.contains(i)), "{bad}: {calls:?}");
    }
    // Without a panic every index runs, in range order.
    let all = run_ranges(&ranges, Ok);
    let flat: Vec<usize> = all.into_iter().flat_map(|r| r.expect("ok")).collect();
    assert_eq!(flat, (0..30).collect::<Vec<_>>());
}

#[test]
fn run_ranges_ignores_a_caller_that_is_already_unwinding() {
    // `extract` can be called from a `Drop` that runs during a panic. The
    // calling thread runs the first range, so its abort guard drops while the
    // thread is panicking even though nothing in this call panicked. It must
    // not raise the flag: no panic of this call exists to resume, and the
    // other range would come back silently cut short. The slow second range
    // is still running when the first finishes, so a raised flag shows.
    struct Extracts<'a>(&'a Mutex<Vec<usize>>);
    impl Drop for Extracts<'_> {
        fn drop(&mut self) {
            let results = run_ranges(&[0..1, 1..30], |i| {
                self.0.lock().expect("calls").push(i);
                if i > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Ok(i)
            });
            assert!(results.iter().all(Result::is_ok));
        }
    }
    let calls = Mutex::new(Vec::new());
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _extracts = Extracts(&calls);
        panic!("outer panic");
    }))
    .expect_err("the outer panic propagates");
    assert_eq!(panic_message(&*payload), "outer panic");
    let mut calls = calls.into_inner().expect("calls");
    calls.sort_unstable();
    assert_eq!(calls, (0..30).collect::<Vec<_>>());
}

#[test]
fn no_page_starts_after_unavailable_in_parallel() {
    // 30 sparse pages; page 0 reports unavailable at once, every other
    // page is slow. Each worker can have at most one page in flight when
    // the flag rises, so at most `workers` calls happen, and every page not
    // called carries the unavailability message.
    let sparse: Vec<usize> = (0..30).collect();
    let d = doc(30, &sparse);
    let mut acts: Vec<(usize, Act)> = (1..30).map(|i| (i, Act::Slow)).collect();
    acts.push((0, Act::Unavailable));
    let fake = Fake::new(&acts);
    let workers = 3;
    let r = d.extract(&with_ocr(&fake, 500, workers)).expect("extract");
    assert_eq!(r.ocr_status, OcrStatus::Unavailable);
    let calls = fake.calls();
    assert!(calls.contains(&0));
    assert!(calls.len() <= workers, "{calls:?}");
    for row in &r.pages {
        if calls.contains(&row.index) && row.index != 0 {
            assert_eq!(row.source, PageSource::Ocr, "in flight: {}", row.index);
        } else {
            assert_eq!(row.source, PageSource::TextFallback, "page {}", row.index);
            assert_eq!(
                row.ocr_error.as_deref(),
                Some("OCR unavailable (no_tessdata): scripted"),
                "page {}",
                row.index
            );
        }
    }
}

#[test]
fn a_failing_worker_stops_the_others() {
    // Page 0 reports the document closed; the other workers stop pulling
    // pages instead of recognising the remaining 29 slow ones.
    let sparse: Vec<usize> = (0..30).collect();
    let d = doc(30, &sparse);
    let mut acts: Vec<(usize, Act)> = (1..30).map(|i| (i, Act::Slow)).collect();
    acts.push((0, Act::Closed));
    let fake = Fake::new(&acts);
    assert_eq!(d.extract(&with_ocr(&fake, 500, 3)), Err(PageError::Closed));
    assert!(fake.calls().len() <= 3, "{:?}", fake.calls());
    // Likewise after a panic.
    let mut acts: Vec<(usize, Act)> = (1..30).map(|i| (i, Act::Slow)).collect();
    acts.push((0, Act::Panic));
    let fake = Fake::new(&acts);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        d.extract(&with_ocr(&fake, 500, 3))
    }));
    assert!(r.is_err());
    assert!(fake.calls().len() <= 3, "{:?}", fake.calls());
}
