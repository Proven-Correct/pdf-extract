//! Model-checks the document close protocol (`CloseCell`, ADR-0040) with
//! loom: every interleaving, within loom's preemption bound, of operations
//! racing `close()` (concurrency spec, "The close protocol is
//! model-checked").
//!
//! Compiled only under `--cfg loom`, where the crate's `sync` shim is loom's
//! and `CloseCell` is re-exported. The cell holds a `Counter`, not a parsed
//! document, so no hayro type (with its own `std` locks) is built inside the
//! model. Run with:
//!
//! ```text
//! RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 \
//!     cargo test -p pdf-extract-core --test loom_close --release
//! ```
//!
//! Each operation mirrors `Page::with_loaded`: take a clone through
//! `get()` (or fail as closed), use it, drop it. Each close mirrors
//! `Document::close`: `close()`, then drop the returned `Arc` outside the
//! lock. Loom itself also fails a case on a deadlock or a leaked `Arc`.

#![cfg(loom)]

use loom::sync::Arc;
use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::thread;
use pdf_extract_core::CloseCell;

/// Stand-in for `Loaded`: knows whether it was freed, how many operations
/// are using it, and how many closes have finished with it.
struct Counter {
    /// Times this value was freed; must end at exactly one.
    freed: Arc<AtomicUsize>,
    /// Operations currently holding a clone.
    in_use: Arc<AtomicUsize>,
    /// Closes whose `close()` call has returned.
    closes_done: Arc<AtomicUsize>,
    /// A field an operation reads, to show it sees a whole value.
    payload: u64,
}

impl Drop for Counter {
    fn drop(&mut self) {
        // Freed only after the cell let go (some close returned) and no
        // operation still holds it.
        assert!(
            self.closes_done.load(Ordering::SeqCst) >= 1,
            "freed before any close() returned"
        );
        assert_eq!(
            self.in_use.load(Ordering::SeqCst),
            0,
            "freed while an operation held it"
        );
        self.freed.fetch_add(1, Ordering::SeqCst);
    }
}

const PAYLOAD: u64 = 0x5eed;

struct Fixture {
    cell: Arc<CloseCell<Counter>>,
    freed: Arc<AtomicUsize>,
    in_use: Arc<AtomicUsize>,
    closes_done: Arc<AtomicUsize>,
}

impl Fixture {
    fn new() -> Self {
        let freed = Arc::new(AtomicUsize::new(0));
        let in_use = Arc::new(AtomicUsize::new(0));
        let closes_done = Arc::new(AtomicUsize::new(0));
        let cell = Arc::new(CloseCell::new(Counter {
            freed: freed.clone(),
            in_use: in_use.clone(),
            closes_done: closes_done.clone(),
            payload: PAYLOAD,
        }));
        Self {
            cell,
            freed,
            in_use,
            closes_done,
        }
    }

    /// One page operation. Returns whether it ran (`false`: closed).
    fn operation(&self) -> bool {
        let Some(loaded) = self.cell.get() else {
            return false;
        };
        self.in_use.fetch_add(1, Ordering::SeqCst);
        // Never a partially closed state: the clone is whole and alive.
        assert_eq!(loaded.payload, PAYLOAD);
        assert_eq!(self.freed.load(Ordering::SeqCst), 0, "used after free");
        self.in_use.fetch_sub(1, Ordering::SeqCst);
        drop(loaded);
        true
    }

    /// `Document::close`: the returned `Arc` is dropped after the lock is
    /// released, and after the close is counted as done. Returns whether
    /// this close took the value.
    fn close(&self) -> bool {
        let taken = self.cell.close();
        assert!(self.cell.is_closed());
        self.closes_done.fetch_add(1, Ordering::SeqCst);
        let took = taken.is_some();
        drop(taken);
        took
    }

    fn spawn<R: 'static>(&self, f: impl FnOnce(&Fixture) -> R + 'static) -> thread::JoinHandle<R> {
        let me = Self {
            cell: self.cell.clone(),
            freed: self.freed.clone(),
            in_use: self.in_use.clone(),
            closes_done: self.closes_done.clone(),
        };
        thread::spawn(move || f(&me))
    }
}

/// Close racing an operation: the operation either completes on the whole
/// value or fails as closed, an operation started after `close()` returned
/// always fails as closed (even while the first may still be running), and
/// the value is freed exactly once.
#[test]
fn close_races_an_operation() {
    loom::model(|| {
        let fx = Fixture::new();
        let op = fx.spawn(Fixture::operation);
        assert!(fx.close(), "the only close takes the value");
        let late = fx.spawn(Fixture::operation);
        assert!(!late.join().expect("late operation"), "ran after close");
        op.join().expect("operation thread");
        assert!(fx.cell.is_closed());
        assert!(fx.cell.get().is_none(), "closed cell hands nothing out");
        assert_eq!(fx.freed.load(Ordering::SeqCst), 1);
    });
}

/// Release by the last operation: with two operations in flight around a
/// close, the value is freed exactly once, after both operations and the
/// close are done with it (the `Counter` drop asserts the "after").
#[test]
fn the_last_operation_releases_the_value() {
    loom::model(|| {
        let fx = Fixture::new();
        let a = fx.spawn(Fixture::operation);
        let b = fx.spawn(Fixture::operation);
        assert!(fx.close(), "the only close takes the value");
        a.join().expect("operation a");
        b.join().expect("operation b");
        assert_eq!(fx.freed.load(Ordering::SeqCst), 1);
        assert_eq!(fx.in_use.load(Ordering::SeqCst), 0);
    });
}

/// Double close: two concurrent closes and an operation. Freed exactly
/// once, and the operation always finishes (loom reports a deadlock
/// otherwise); only one close takes the value.
#[test]
fn a_double_close_releases_once() {
    loom::model(|| {
        let fx = Fixture::new();
        let op = fx.spawn(Fixture::operation);
        let other = fx.spawn(Fixture::close);
        let mine = fx.close();
        let theirs = other.join().expect("second close");
        op.join().expect("operation thread");
        assert!(mine ^ theirs, "exactly one close takes the value");
        assert_eq!(fx.freed.load(Ordering::SeqCst), 1);
    });
}
