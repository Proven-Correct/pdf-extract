// Tests of the small-stack runner itself (common/stack.rs), which every
// "fits in 512 KiB" test relies on. Without these, a runner that stopped
// measuring (or measured wrongly) would leave the 512 KiB guarantee untested
// while every test using it kept passing.

#[path = "common/stack.rs"]
mod stack;

use std::thread;

use stack::{SMALL_STACK, current_thread_stack_size, on_small_stack, stack_fits};

const BIG_STACK: usize = 2 * 1024 * 1024;

fn on_stack_of(size: usize) -> Option<usize> {
    thread::Builder::new()
        .stack_size(size)
        .spawn(current_thread_stack_size)
        .expect("spawn")
        .join()
        .expect("join")
}

// Checks a runner thread's measurement against the size range itself, not
// only through `stack_fits`: a runner whose check always passed would satisfy
// `stack_fits` with any stack, so only the raw number shows that it measured.
fn assert_measured_small(measured: Option<usize>) {
    assert!(stack_fits(measured, SMALL_STACK), "{measured:?}");
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        let size = measured.expect("measurable on this platform");
        assert!(
            (SMALL_STACK..=SMALL_STACK + stack::STACK_SLACK).contains(&size),
            "{size}"
        );
    }
}

#[test]
fn the_runner_hands_out_a_stack_of_the_size_it_promises() {
    assert_measured_small(on_small_stack(current_thread_stack_size));
}

#[test]
fn a_big_stack_does_not_pass_for_a_small_one() {
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        return;
    }
    let measured = on_stack_of(BIG_STACK);
    assert!(
        measured.is_some_and(|m| m >= BIG_STACK),
        "a 2 MiB request measured {measured:?}"
    );
    assert!(!stack_fits(measured, SMALL_STACK));
}

// The internal #70 mechanism: threads that exited with bigger stacks leave them in
// glibc's cache, from which a 512 KiB request may be served. Whether it is
// is up to glibc; either way the runner must come back with a fitting stack.
#[test]
fn the_runner_survives_a_cache_of_bigger_stacks() {
    let big: Vec<_> = (0..8)
        .map(|_| {
            thread::Builder::new()
                .stack_size(BIG_STACK)
                .spawn(|| ())
                .expect("spawn")
        })
        .collect();
    for t in big {
        t.join().expect("join");
    }
    for _ in 0..8 {
        assert_measured_small(on_small_stack(current_thread_stack_size));
    }
}

// The runner fails, rather than running the body on a stack that is not the
// one the test says it is, when it cannot be given the size: here every
// thread is spawned at 2 MiB while the body wants 512 KiB.
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
#[should_panic(expected = "none measured as")]
fn a_stack_that_cannot_be_had_at_the_right_size_fails_the_test() {
    let _ = stack::run_on_stack_of(BIG_STACK, SMALL_STACK, || ());
}
