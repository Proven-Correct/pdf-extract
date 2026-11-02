//! Same-run timing for the tests that place a budget against a measured
//! duration (internal issue #64): the recognition-monitor test and the two
//! process deadline tests (the helper's budget, the parent's kill).
//!
//! Wall-clock samples on a loaded runner or under coverage instrumentation
//! are skewed upwards by bursts that come and go, so two rules apply:
//! - a budget is derived from the median of several samples taken just
//!   before the attempt it bounds, never from the slowest sample and
//!   never from a figure carried over from an earlier attempt (one slow
//!   sample must not poison every later budget);
//! - an attempt the timing made inconclusive is retried with a fresh
//!   measurement, up to a cap, and only the absence of any conclusive
//!   attempt fails the test. Each test arranges that a broken guarantee
//!   makes an attempt fail or stay inconclusive except on a measurement
//!   far off the samples beside it, so retrying makes a false pass only
//!   a little likelier, never routine.

use std::time::{Duration, Instant};

/// The spread of `n` timed runs.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub min: Duration,
    pub median: Duration,
    pub max: Duration,
}

/// Times `n` (odd, so the median is a sample) runs of `f`.
pub fn measure(n: usize, mut f: impl FnMut()) -> Timing {
    assert!(n % 2 == 1, "an odd sample count, not {n}");
    let mut samples: Vec<Duration> = (0..n)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    samples.sort();
    Timing {
        min: samples[0],
        median: samples[n / 2],
        max: samples[n - 1],
    }
}

/// Runs `attempt` until it is conclusive (`Ok`), at most `cap` times; an
/// `Err` is a note on why that attempt was inconclusive. A failed guarantee
/// is an assertion inside `attempt`, not an `Err`.
///
/// # Panics
/// When no attempt was conclusive, headed by `suspect` (the guarantee whose
/// breakage keeps every attempt inconclusive, which is likelier than the
/// timing failing `cap` times running) and listing every note under `what`.
pub fn retry(
    what: &str,
    suspect: &str,
    cap: usize,
    mut attempt: impl FnMut() -> Result<(), String>,
) {
    let mut notes = Vec::new();
    for _ in 0..cap {
        match attempt() {
            Ok(()) => return,
            Err(note) => notes.push(note),
        }
    }
    panic!("{what}: {suspect}? No conclusive attempt in {cap}: {notes:#?}");
}
