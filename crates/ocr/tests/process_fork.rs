//! Fork safety of process isolation (spec 0002 O30; §4.9 "Fork"): the
//! unit-08 fork test with `Isolation::Process`, forking while the parent
//! has a page in flight in its helper. The child abandons the parent's
//! helpers (closing only its own copies of their pipes), spawns its own and
//! recognises; the parent's helper finishes the page started before the
//! fork and serves the next one.
//!
//! `harness = false`: the fork happens from this binary's own `main`, with
//! exactly one other thread (the one whose page is in flight), so the
//! child's state is known.

mod common;

use std::time::Duration;

use pdf_extract_core::{BlockSeparator, Document};
use pdf_extract_ocr::isolation::client::{helper_log, set_helper_test_env};
use pdf_extract_ocr::{OcrConfig, OcrEngine, OcrOptions, fork_resets};

const OPTS: OcrOptions = OcrOptions { dpi: Some(150) };

fn recognises(engine: &OcrEngine, doc: &Document) -> bool {
    engine
        .ocr(&doc.page(0).expect("page"), &OPTS)
        .is_ok_and(|tp| {
            tp.to_text(BlockSeparator::Newline)
                .contains("quick brown fox")
        })
}

#[cfg(unix)]
fn main() {
    if std::env::args().any(|a| a == "--list") {
        return;
    }
    // Every helper sleeps 1.5 s before its first page, so the parent's page
    // is certainly in flight when the fork happens.
    set_helper_test_env(&[
        ("PDF_EXTRACT_OCR_HELPER_FAULT", "slow-on-recognize:1500"),
        ("PDF_EXTRACT_OCR_HELPER_FAULT_ARMED", "1"),
    ]);
    let engine = OcrEngine::tesseract_with_candidates(
        OcrConfig {
            max_instances: 2,
            ..common::process_config()
        },
        &common::nowhere(),
    )
    .expect("engine");
    let doc = common::doc(common::body_page());
    let parent_helper = *helper_log().0.last().expect("spawned");

    std::thread::scope(|s| {
        let in_flight = s.spawn(|| recognises(&engine, &doc));
        std::thread::sleep(Duration::from_millis(300));

        // SAFETY: the only other thread is blocked in poll(2) waiting for
        // its helper, holding no lock the child needs; the child uses the
        // engine (whose pool notices the new pid) and leaves through
        // _exit, never unwinding into the parent's state.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            let ok = recognises(&engine, &doc)
                && fork_resets() == 1
                && engine.pool_len() == Some(1)
                && helper_log().0.last() != Some(&parent_helper);
            // SAFETY: _exit ends the child at once, without running the
            // parent's destructors (which would shut down its helpers).
            unsafe { libc::_exit(i32::from(!ok)) };
        }
        let mut status = 0;
        // SAFETY: `pid` is our child; `status` is a valid out-pointer.
        let waited = unsafe { libc::waitpid(pid, &raw mut status, 0) };
        assert_eq!(waited, pid);
        assert!(
            libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
            "child failed: status {status:#x}"
        );
        assert!(
            in_flight.join().expect("thread"),
            "the page in flight at the fork completed in the parent's helper"
        );
    });
    assert!(recognises(&engine, &doc), "parent, after the fork");
    assert_eq!(fork_resets(), 0, "the parent never resets");
    assert!(
        helper_log().1.iter().all(|(p, _)| *p != parent_helper),
        "the parent's helper was never reaped"
    );
    println!("process fork: child used its own helper; parent's helper unaffected: ok");
}

#[cfg(not(unix))]
fn main() {}
