//! O24's thread bound for `Document::extract` (run 0002 §4.10): no thread
//! with `workers = 1`, at most `workers` threads during the call (the
//! caller plus `workers - 1` spawned; the assertions check exactly that),
//! none after.
//!
//! The OCR pass is measured by thread identity, not by counting: the
//! recogniser records the id of every thread that calls it. Counting
//! `/proc/self/status` is unreliable *during* the call, because the
//! text-pass threads just joined can still be listed there for a moment
//! after `pthread_join` returns and overlap the OCR pass's own threads.
//!
//! The text pass has no hook to record ids from without product code, so
//! for `workers = 1` a poller thread watches `/proc/self/status` for the
//! whole call: a stale entry can only lower what it sees relative to a
//! baseline taken once the count is stable, never raise it, so "never
//! above the baseline" is robust. Its own test binary with a single test,
//! so no other test's threads move the count. Linux only: on macOS this file
//! compiles to nothing, so the local dev loop never runs it (nor
//! `thread_poll_panic.rs`) and internal #55 C4's thread bound is checked by CI's ubuntu
//! job alone. There is no macOS port, deliberately.

#![cfg(target_os = "linux")]

mod thread_poll;

use std::collections::HashSet;
use std::sync::Mutex;
use std::thread::{self, ThreadId};
use std::time::Duration;

use pdf_extract_core::{
    Document, ExtractOptions, OcrError, OpenOptions, Page, PageRecognizer, TextOptions, TextPage,
};
use pdf_extract_testkit::PdfBuilder;
use thread_poll::{polled, threads};

/// Waits until no joined thread is still listed: the count stops falling.
fn settle() -> usize {
    let mut last = threads();
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(10));
        let now = threads();
        if now == last {
            return now;
        }
        last = now;
    }
    last
}

/// Whether the count falls to `n` or below within a second: a joined thread
/// can stay listed in `/proc` briefly after `pthread_join` returns.
fn drops_to(n: usize) -> bool {
    (0..100).any(|_| {
        threads() <= n || {
            thread::sleep(Duration::from_millis(10));
            false
        }
    })
}

/// Records which threads recognise, and answers with an empty page.
struct Sampler {
    empty: TextPage,
    ids: Mutex<Vec<ThreadId>>,
}

impl PageRecognizer for Sampler {
    fn recognize(&self, _: &Page, _: Option<u32>) -> Result<TextPage, OcrError> {
        self.ids.lock().expect("ids").push(thread::current().id());
        // Long enough that every worker gets a page.
        thread::sleep(Duration::from_millis(5));
        Ok(self.empty.clone())
    }
}

/// Runs `extract` while a poller samples the thread count; returns the
/// distinct recognising threads and the highest count seen.
fn run(d: &Document, empty: &TextPage, workers: usize) -> (HashSet<ThreadId>, usize) {
    let s = Sampler {
        empty: empty.clone(),
        ids: Mutex::new(Vec::new()),
    };
    let (r, peak) = polled(|| {
        d.extract(&ExtractOptions {
            ocr: Some(&s),
            workers,
            ..ExtractOptions::default()
        })
    });
    assert_eq!(r.expect("extract").pages.len(), 40);
    let ids = s.ids.into_inner().expect("ids");
    assert_eq!(ids.len(), 40, "every page is sparse and recognised");
    (ids.into_iter().collect(), peak)
}

#[test]
fn thread_counts_follow_workers() {
    // No text and a long operator run per page: every page is sparse, and
    // the text pass takes long enough for the poller to see any thread.
    let mut b = PdfBuilder::new();
    for _ in 0..40 {
        b.add_page_a4().raw_content(&"q Q ".repeat(20_000));
    }
    let d = Document::from_bytes(b.to_bytes(), &OpenOptions::default()).expect("opens");
    let empty = d
        .page(0)
        .and_then(|p| p.text_page(&TextOptions::default()))
        .expect("empty page");
    let caller = thread::current().id();

    // workers = 1: only the caller recognises, and nothing is ever above
    // the baseline (the test thread, libtest's main thread, the poller).
    let baseline = settle() + 1;
    let (ids, peak) = run(&d, &empty, 1);
    assert_eq!(
        ids,
        HashSet::from([caller]),
        "workers 1: OCR ran off the caller"
    );
    assert!(
        peak <= baseline,
        "workers 1: {peak} threads > baseline {baseline}"
    );
    assert!(drops_to(baseline - 1), "workers 1: threads left behind");

    // Text pass alone, workers = 4: at most 3 spawned besides the caller.
    // All of one pass's threads are spawned before any is joined, so a
    // stale `/proc` entry cannot push the peak past this bound; anything
    // over it is a real extra thread. Must run after the workers = 1 case
    // (the bound is relative to a settled baseline).
    let before = settle();
    let (r, peak) = polled(|| {
        d.extract(&ExtractOptions {
            workers: 4,
            ..ExtractOptions::default()
        })
    });
    assert_eq!(r.expect("extract").pages.len(), 40);
    assert!(
        peak > before + 1,
        "text pass: poller saw no worker ({peak} <= {before} + 1)"
    );
    assert!(
        peak <= before + 1 + 3,
        "text pass: {peak} threads > {before} + poller + 3"
    );
    assert!(drops_to(before), "text pass: threads left behind");

    // workers = 4: several threads, at most 4 including the caller.
    let before = settle();
    let (ids, peak) = run(&d, &empty, 4);
    assert!(ids.len() > 1, "workers 4 ran on one thread");
    assert!(
        ids.len() <= 4,
        "workers 4: {} recognising threads",
        ids.len()
    );
    // The poller does see spawned threads (it would have caught one above).
    assert!(
        peak > before + 1,
        "poller saw no worker: {peak} <= {before} + 1"
    );
    assert!(drops_to(before), "workers 4: threads left behind");
}
