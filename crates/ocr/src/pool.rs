//! The instance pool (spec 0002 O10, O11).
//!
//! A `Mutex<State>` plus a `Condvar`. Instances are created lazily, up to
//! `max`, by the thread that needs one, *outside* the lock (an `Init` loads
//! a model for tens to hundreds of milliseconds and must not stall callers
//! that could reuse an idle instance). A caller that finds every instance
//! busy and the pool full waits on the condvar with no timeout: each busy
//! page is bounded by its own `page_timeout`, so the wait is too.
//!
//! **Drop on error.** After a successful page the instance is cleared and
//! returned. After an error, a timeout or a panic it is dropped — its
//! internal state is unknown — and the slot is freed, so the next checkout
//! creates a replacement. The release runs in a guard's `Drop`, so a panic
//! inside recognition cannot leak a slot and starve later callers.
//!
//! **Fork safety (O11, O30).** The pool records the process id. A checkout
//! in a process with a different id (a `fork()`ed child) abandons every
//! instance the parent had through `Recognizer::abandon`: an in-process
//! instance is *leaked*, not dropped, because another parent thread may
//! have been inside Tesseract when the fork happened and the child's copy
//! of that instance can be mid-update; a helper process only has this
//! process's copies of its pipe ends closed. The child then starts
//! empty and pays one `Init` for its first page. A generation number makes
//! an instance checked out before the reset return to nowhere (it is abandoned
//! too), so the counts of the new pool stay exact.
//!
//! **Forking hazard.** The pool mutex is one of the locks a `fork()` from a
//! threaded process can leave held forever in the child; the process-id
//! check above only helps a child that gets as far as its first checkout.
//! See the [crate-level `Forking` section](crate#forking).
//!
//! The lock is never held while recognising or creating, and every lock
//! acquisition recovers from poisoning: the protected state is only
//! counters and a `Vec` updated by single statements, so a poisoned lock
//! still guards consistent data.
//!
//! **Model checking.** The lock and condvar come from the crate's `sync`
//! shim, so tests/loom_pool.rs runs this code under loom; the pid source is
//! a field (`std::process::id` outside that test) so a loom test can stage
//! a fork reset. `Pool` and its constructors are `pub` only for the loom
//! build's re-export; the module is private.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError};

use pdf_extract_core::{OcrError, OcrPageError};

use crate::backend::{Recognizer, RecognizerFactory};
use crate::sync::{Condvar, Mutex, MutexGuard};

/// How many times a pool in this process found itself in a forked child and
/// started over (test observability for O11).
static FORK_RESETS: AtomicUsize = AtomicUsize::new(0);

/// See [`FORK_RESETS`].
#[doc(hidden)]
pub fn fork_resets() -> usize {
    FORK_RESETS.load(Ordering::Relaxed)
}

/// A bounded pool of instances made by `F`.
pub struct Pool<F: RecognizerFactory> {
    factory: Arc<F>,
    max: usize,
    /// The current process id: `std::process::id`, or a loom test's stand-in.
    pid_source: fn() -> u32,
    state: Mutex<State<F::Instance>>,
    available: Condvar,
}

struct State<I> {
    /// The process the instances belong to.
    pid: u32,
    /// Bumped by each fork reset; a checked-out instance from an older
    /// generation is not returned.
    generation: u64,
    /// Instances ready for a page.
    idle: Vec<I>,
    /// Instances in existence (idle or checked out) plus slots reserved by
    /// a creation in progress; never above `max`.
    live: usize,
}

/// One checked-out instance; returns or drops it when dropped.
struct Slot<'a, F: RecognizerFactory> {
    pool: &'a Pool<F>,
    instance: Option<F::Instance>,
    generation: u64,
    /// Set by a successful page; anything else (an error, a panic
    /// unwinding through) drops the instance.
    healthy: bool,
}

impl<F: RecognizerFactory> Drop for Slot<'_, F> {
    fn drop(&mut self) {
        let Some(mut instance) = self.instance.take() else {
            return;
        };
        if self.healthy {
            // Outside the lock: Clear frees the page's image and results.
            // If it panics, the instance is dropped by the unwind and the
            // guard gives its slot back, so no later caller waits for it.
            let guard = Reservation::new(self.pool, self.generation);
            instance.clear();
            guard.disarm();
        }
        let mut state = self.pool.lock();
        self.pool.check_pid(&mut state);
        if state.generation != self.generation {
            // Checked out before a fork reset: it belongs to the abandoned
            // pool, and is abandoned like the rest of it.
            drop(state);
            instance.abandon();
            return;
        }
        if self.healthy {
            state.idle.push(instance);
        } else {
            state.live -= 1;
            drop(state);
            drop(instance);
        }
        self.pool.available.notify_one();
    }
}

/// A counted slot not (or no longer) backed by an instance in the pool's
/// hands: dropped while armed, it decrements `live` (if no fork reset has
/// zeroed the count since) and wakes one waiter.
struct Reservation<'a, F: RecognizerFactory> {
    pool: &'a Pool<F>,
    generation: u64,
    armed: bool,
}

impl<'a, F: RecognizerFactory> Reservation<'a, F> {
    fn new(pool: &'a Pool<F>, generation: u64) -> Self {
        Self {
            pool,
            generation,
            armed: true,
        }
    }

    /// The slot is accounted for elsewhere; dropping does nothing.
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl<F: RecognizerFactory> Drop for Reservation<'_, F> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = self.pool.lock();
        self.pool.check_pid(&mut state);
        if state.generation == self.generation {
            state.live -= 1;
        }
        drop(state);
        self.pool.available.notify_one();
    }
}

impl<F: RecognizerFactory> Pool<F> {
    /// A pool of at most `max` (≥ 1) instances, starting with `first` if
    /// given (the instance engine construction validated, O7).
    pub fn new(factory: Arc<F>, max: usize, first: Option<F::Instance>) -> Self {
        Self::with_pid_source(factory, max, first, std::process::id)
    }

    /// As [`Self::new`], with `pid` standing in for `std::process::id`, so a
    /// model-checking test can make the pool see a fork.
    pub fn with_pid_source(
        factory: Arc<F>,
        max: usize,
        first: Option<F::Instance>,
        pid: fn() -> u32,
    ) -> Self {
        let live = usize::from(first.is_some());
        Self {
            factory,
            max: max.max(1),
            pid_source: pid,
            state: Mutex::new(State {
                pid: pid(),
                generation: 0,
                idle: first.into_iter().collect(),
                live,
            }),
            available: Condvar::new(),
        }
    }

    pub fn factory(&self) -> &F {
        &self.factory
    }

    fn lock(&self) -> MutexGuard<'_, State<F::Instance>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Instances in existence, idle or busy (test observability).
    #[cfg_attr(
        loom,
        expect(
            clippy::len_without_is_empty,
            reason = "test observability; public only for the loom re-export"
        )
    )]
    pub fn len(&self) -> usize {
        let mut state = self.lock();
        self.check_pid(&mut state);
        state.live
    }

    /// Abandons the instances of the process that forked us (O11).
    ///
    /// A reset frees every slot at once, so it wakes every waiter. Without
    /// that, a reset made by a returning pre-fork slot (whose
    /// generation-mismatch path in `Slot::drop` changes no count and so
    /// notifies nobody) left a caller already waiting on the full pool
    /// asleep with free slots and nobody left to wake it (found by
    /// tests/loom_pool.rs, `a_return_across_a_fork_reset`).
    fn check_pid(&self, state: &mut State<F::Instance>) {
        let pid = (self.pid_source)();
        if state.pid != pid {
            std::mem::take(&mut state.idle)
                .into_iter()
                .for_each(Recognizer::abandon);
            state.live = 0;
            state.pid = pid;
            state.generation += 1;
            FORK_RESETS.fetch_add(1, Ordering::Relaxed);
            self.available.notify_all();
        }
    }

    /// Runs `f` on a pooled instance (O10).
    ///
    /// # Errors
    /// `f`'s error as [`OcrError::Page`] (the instance is then dropped), or
    /// [`OcrError::Unavailable`] when a needed instance could not be
    /// created.
    pub fn run<R>(
        &self,
        f: impl FnOnce(&mut F::Instance) -> Result<R, OcrPageError>,
    ) -> Result<R, OcrError> {
        let mut slot = self.checkout()?;
        let instance = slot
            .instance
            .as_mut()
            .expect("a slot holds its instance until dropped");
        let result = f(instance);
        slot.healthy = result.is_ok();
        drop(slot);
        result.map_err(OcrError::Page)
    }

    fn checkout(&self) -> Result<Slot<'_, F>, OcrError> {
        let mut state = self.lock();
        loop {
            self.check_pid(&mut state);
            let generation = state.generation;
            if let Some(instance) = state.idle.pop() {
                return Ok(self.slot(instance, generation));
            }
            if state.live < self.max {
                // Reserve the slot, then create outside the lock. The guard
                // gives the reservation back on an error *or a panic* in
                // `create`; without it a panicking factory would leave the
                // slot counted forever and, at `max`, every later caller
                // would wait for it.
                state.live += 1;
                drop(state);
                let reservation = Reservation::new(self, generation);
                return match self.factory.create() {
                    Ok(instance) => {
                        reservation.disarm();
                        Ok(self.slot(instance, generation))
                    }
                    Err(e) => {
                        drop(reservation);
                        Err(OcrError::Unavailable(e))
                    }
                };
            }
            state = self
                .available
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn slot(&self, instance: F::Instance, generation: u64) -> Slot<'_, F> {
        Slot {
            pool: self,
            instance: Some(instance),
            generation,
            healthy: false,
        }
    }
}

impl<F: RecognizerFactory> Drop for Pool<F> {
    /// An engine dropped in a forked child must not free the parent's
    /// instances either (O11); in the creating process they are dropped
    /// normally.
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        if state.pid != (self.pid_source)() {
            std::mem::take(&mut state.idle)
                .into_iter()
                .for_each(Recognizer::abandon);
        }
    }
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
