//! Thread-count polling shared by `extract_threads.rs` and
//! `thread_poll_panic.rs`. A directory module, so cargo does not build it as
//! a test binary of its own. Linux only (`/proc`); the includers gate on it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

pub fn threads() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .expect("status")
        .lines()
        .find_map(|l| l.strip_prefix("Threads:"))
        .and_then(|v| v.trim().parse().ok())
        .expect("Threads: line")
}

/// Sets the flag on drop, so the poller is told to stop even when the
/// closure under test unwinds. Without it `thread::scope` would wait forever
/// on a poller that never sees the flag, and a panicking extraction would
/// hang the test instead of failing it.
struct StopOnDrop<'a>(&'a AtomicBool);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Runs `f` while a poller samples the thread count; returns `f`'s result
/// and the highest count seen (the poller included). If `f` panics, the
/// poller is stopped and the panic propagates.
pub fn polled<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let done = AtomicBool::new(false);
    thread::scope(|scope| {
        let poller = scope.spawn(|| {
            let mut peak = 0;
            while !done.load(Ordering::Acquire) {
                peak = peak.max(threads());
            }
            peak
        });
        let stop = StopOnDrop(&done);
        let r = f();
        drop(stop);
        (r, poller.join().expect("poller"))
    })
}
