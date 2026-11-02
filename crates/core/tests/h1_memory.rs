//! Memory bound of fork change H1 (spec §4.5, memory bullet; gating).
//!
//! With the page tree no longer walked at open, what `Pdf::new` keeps is
//! hayro's xref map (about 25 bytes per entry at rest, about 50 while the map
//! rehashes) plus a constant. The bound is stated per xref entry so that it
//! tracks the one cost H1 leaves in place, and the first `pages()` call must
//! cost at least ten times as much, which is what H1 defers.
//!
//! This is its own test binary because it installs a counting global
//! allocator: every allocation in the process is counted, so no other test
//! may run alongside it. It holds exactly one `#[test]` for the same reason.
//! The allocator is the only `unsafe` here; `pdf-extract-core` itself
//! forbids unsafe code, and this file is a separate crate.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use hayro_syntax::Pdf;
use pdf_extract_testkit::hostile::{TreeShape, page_tree};

/// Forwards to the system allocator and tracks live bytes and their peak.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

impl Counting {
    fn added(size: usize) {
        let live = LIVE.fetch_add(size, Relaxed) + size;
        PEAK.fetch_max(live, Relaxed);
    }
}

// SAFETY: every method forwards to `System` with the caller's arguments
// unchanged, so `System`'s guarantees carry over; the counters are atomics and
// never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged; see the impl comment.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            Self::added(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged; see the impl comment.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            Self::added(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged; see the impl comment.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded unchanged; see the impl comment.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            // Count the move as free-then-allocate; the peak may then miss
            // the instant both blocks exist, which `System` may avoid anyway.
            LIVE.fetch_sub(layout.size(), Relaxed);
            Self::added(new_size);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Retained bytes per xref entry `Pdf::new` may keep (measured: ~33).
const MAX_RETAINED_PER_ENTRY: usize = 64;
/// Peak bytes per xref entry during `Pdf::new` (measured: ~49).
const MAX_PEAK_PER_ENTRY: usize = 96;
/// How much more the first `pages()` call must cost than `Pdf::new` kept.
const MIN_PAGES_FACTOR: usize = 10;

#[test]
fn new_retains_only_the_xref_and_pages_cost_is_deferred() {
    // Generated before measuring: the input buffer is the baseline.
    let data = page_tree(TreeShape::Flat(100_000));

    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    let pdf = Pdf::new(data).expect("opens");
    let retained = LIVE.load(Relaxed) - base;
    let peak = PEAK.load(Relaxed) - base;
    let entries = pdf.len();

    let before_pages = LIVE.load(Relaxed);
    assert_eq!(pdf.pages().len(), 100_000);
    let pages_cost = LIVE.load(Relaxed) - before_pages;

    let report = format!(
        "entries {entries}, retained {retained} B ({} B/entry), peak {peak} B ({} B/entry), \
         first pages() {pages_cost} B",
        retained / entries,
        peak / entries
    );
    assert!(entries > 200_000, "fixture shape changed: {report}");
    assert!(
        retained < MAX_RETAINED_PER_ENTRY * entries,
        "Pdf::new retained too much: {report}"
    );
    assert!(
        peak < MAX_PEAK_PER_ENTRY * entries,
        "Pdf::new peaked too high: {report}"
    );
    assert!(
        pages_cost >= MIN_PAGES_FACTOR * retained,
        "pages() was not deferred: {report}"
    );
}
