//! `polled` must fail a test whose closure panics, not hang it: a panic
//! unwinds past the poller's stop flag unless a drop guard sets it. Its own
//! test binary: `extract_threads.rs` counts process threads and must be the
//! only test in its binary, so this one cannot share it. Linux only.

#![cfg(target_os = "linux")]

mod thread_poll;

#[test]
#[should_panic(expected = "closure panicked")]
fn a_panicking_closure_propagates_instead_of_hanging() {
    thread_poll::polled(|| -> () { panic!("closure panicked") });
}
