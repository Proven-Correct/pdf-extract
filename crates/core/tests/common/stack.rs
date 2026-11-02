//! The 512 KiB small-stack runner shared by the core and render test crates
//! (core's `tests/common` includes this file; render's `tests/common` and
//! core's text unit tests, `src/text/tests/mod.rs`, pull it in by `#[path]`),
//! and the measurement that makes the size a fact instead of a request.
//!
//! `Builder::stack_size(512 KiB)` is only a request. On Linux, glibc serves a
//! new thread from its cache of exited threads' stacks, taking the smallest
//! free block of at least the requested size, even one up to four times
//! larger (`get_cached_stack` in nptl/allocatestack.c). Rust's own worker
//! threads run on 2 MiB stacks, within that bound, so a 512 KiB request can
//! be handed a 2 MiB block and a test that means to prove "fits in 512 KiB"
//! proves nothing (the same mechanism as internal issue #70, which the Python harness
//! handles in `tests/python/pdfs.py::on_small_stack`; this mirrors it).
//!
//! So the thread measures its own stack before running the test body. One
//! that is too big is parked, holding its block so the next spawn cannot be
//! given it, and another is spawned, until one fits or [`MAX_PARKED`] have
//! been refused, which fails the test: a stack that cannot be had at the
//! right size means the guarantee cannot be tested, not that it holds.
//! On platforms other than Linux and macOS (the only ones CI runs) the
//! measurement is unavailable, `stack_fits(None, _)` returns true, and the
//! size is taken on trust.
//!
//! This is `unsafe` (pthread FFI through `libc`, a dev-dependency of both
//! crates), so it lives in the test crates and not in `pdf-extract-testkit`,
//! which is `#![forbid(unsafe_code)]`. Core's unit-test build only denies
//! `unsafe`, so its include can `#[allow]` it (ADR 0089).

// Each test crate includes this module and uses only part of it.
#![allow(dead_code)]

use std::sync::mpsc;
use std::thread;

/// The thread stack size the spec runs every hostile fixture on (§2.3): small
/// enough that unbounded recursion overflows quickly, large enough for
/// legitimate parsing and interpretation.
pub const SMALL_STACK: usize = 512 * 1024;

/// How far above the requested size a measured stack may be and still be the
/// one asked for: macOS reports 512 KiB as 524,288 plus a few KiB, Linux
/// exactly; a reused or default stack is hundreds of KiB larger or more.
pub const STACK_SLACK: usize = 64 * 1024;

/// Bound on the oversized threads one call parks. glibc's stack cache holds
/// 40 MiB by default, so at most 20 of Rust's 2 MiB blocks; needing more
/// means something other than the cache is at work.
pub const MAX_PARKED: usize = 64;

/// The calling thread's actual stack size in bytes, or `None` where it cannot
/// be asked (neither macOS nor Linux). On Linux `pthread_getattr_np` reports,
/// for a thread other than the main one, the block it was given with the
/// guard page excluded.
#[cfg(target_os = "macos")]
pub fn current_thread_stack_size() -> Option<usize> {
    // SAFETY: `pthread_self` is always valid for the calling thread, and
    // `pthread_get_stacksize_np` only reads it.
    Some(unsafe { libc::pthread_get_stacksize_np(libc::pthread_self()) })
}

/// See the macOS variant.
#[cfg(target_os = "linux")]
pub fn current_thread_stack_size() -> Option<usize> {
    use std::mem::MaybeUninit;

    let mut attr = MaybeUninit::<libc::pthread_attr_t>::zeroed();
    let mut size = 0usize;
    // SAFETY: `attr` is a zeroed pthread_attr_t that `pthread_getattr_np`
    // initialises (on success) and `pthread_attr_destroy` releases; `size` is
    // a valid out-pointer; the attr is only read between the two.
    unsafe {
        let rc = libc::pthread_getattr_np(libc::pthread_self(), attr.as_mut_ptr());
        assert_eq!(rc, 0, "pthread_getattr_np failed");
        let rc = libc::pthread_attr_getstacksize(attr.as_ptr(), &raw mut size);
        libc::pthread_attr_destroy(attr.as_mut_ptr());
        assert_eq!(rc, 0, "pthread_attr_getstacksize failed");
    }
    Some(size)
}

/// See the macOS variant.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn current_thread_stack_size() -> Option<usize> {
    None
}

/// Whether a thread that measured `measured` bytes has the `want`-byte stack
/// it asked for. Unmeasurable (`None`) is taken on trust.
pub fn stack_fits(measured: Option<usize>, want: usize) -> bool {
    measured.is_none_or(|m| (want..=want + STACK_SLACK).contains(&m))
}

/// The stack a body gets in a sanitizer run instead of [`SMALL_STACK`].
/// AddressSanitizer's redzones and ThreadSanitizer's shadow bookkeeping
/// inflate every frame several times over, so the 512 KiB claim is not the
/// claim being tested there; the body still runs, on a stack big enough that
/// only a real (unbounded) recursion could overflow it.
pub const SANITIZER_STACK: usize = 64 * 1024 * 1024;

/// Whether this is a sanitizer run (`tools/sanitizers/run.sh` sets
/// `PXE_SANITIZER`). Under one, [`run_on_stack_of`] neither requests the
/// small stack nor measures it; the tests of the measurement itself
/// (`stack_size.rs`) are skipped by name in run.sh.
pub fn sanitizer_run() -> bool {
    std::env::var_os("PXE_SANITIZER").is_some_and(|v| !v.is_empty())
}

/// Runs `f` on a fresh thread with a [`SMALL_STACK`] stack and returns its
/// result, or the panic payload if it panicked. A stack overflow aborts the
/// whole test process, which is the failure these tests exist to rule out.
pub fn try_on_small_stack<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> thread::Result<T> {
    run_on_stack_of(SMALL_STACK, SMALL_STACK, f)
}

/// [`try_on_small_stack`] with a panic on the test thread if `f` panicked.
pub fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    try_on_small_stack(f).expect("small-stack thread panicked")
}

/// Spawns threads requesting `request` bytes until one measures as `want`
/// bytes, then runs `f` there. `request != want` is how the tests show the
/// check refuses a stack of the wrong size.
pub fn run_on_stack_of<T: Send + 'static, F: FnOnce() -> T + Send + 'static>(
    request: usize,
    want: usize,
    f: F,
) -> thread::Result<T> {
    if sanitizer_run() {
        return thread::Builder::new()
            .stack_size(SANITIZER_STACK)
            .spawn(f)
            .expect("spawn sanitizer-run thread")
            .join();
    }
    // Parked threads wait on their own channel; dropping the sender releases
    // them. They are held only while spawning, because it is the cache lookup
    // at spawn time that must not find them; once a good thread is running
    // its block is no longer in the cache.
    let mut parked: Vec<(thread::JoinHandle<Option<T>>, mpsc::Sender<F>)> = Vec::new();
    let mut f = Some(f);
    loop {
        let (size_tx, size_rx) = mpsc::channel();
        let (work_tx, work_rx) = mpsc::channel::<F>();
        let handle = thread::Builder::new()
            .stack_size(request)
            .spawn(move || {
                size_tx
                    .send(current_thread_stack_size())
                    .expect("report stack size");
                // Err when the spawner decided to park or drop this thread.
                work_rx.recv().ok().map(|work| work())
            })
            .expect("spawn small-stack thread");
        let measured = size_rx.recv().expect("stack size report");
        if stack_fits(measured, want) {
            work_tx
                .send(f.take().expect("work is sent once"))
                .expect("send work");
            for (held, release) in parked {
                drop(release);
                held.join().expect("parked thread exits cleanly");
            }
            return handle
                .join()
                .map(|r| r.expect("the fitting thread ran its work"));
        }
        assert!(
            parked.len() < MAX_PARKED,
            "{} threads asked for a {request}-byte stack and none measured as {want} bytes \
             (last: {measured:?})",
            parked.len() + 1,
        );
        parked.push((handle, work_tx));
    }
}
