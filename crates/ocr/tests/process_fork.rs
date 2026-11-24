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
//!
//! "In flight" is observed, not assumed from a sleep (issue #6): every
//! helper holds its first page at a `rendezvous-on-recognize:2` (see the
//! helper's fault hooks), whose marker the parent's helper creates only
//! after reading the whole `Recognize` frame. The fork waits for that
//! marker, and nothing can complete the rendezvous before the fork.

mod common;

// Everything below is used only by the unix `main`; gated item by item so a
// non-unix build has no dead code under `-D warnings`.
#[cfg(unix)]
use std::path::Path;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use pdf_extract_core::{BlockSeparator, Document};
#[cfg(unix)]
use pdf_extract_ocr::isolation::client::{helper_log, set_helper_test_env};
#[cfg(unix)]
use pdf_extract_ocr::{OcrConfig, OcrEngine, OcrOptions, fork_resets};

#[cfg(unix)]
const OPTS: OcrOptions = OcrOptions { dpi: Some(150) };

#[cfg(unix)]
fn recognises(engine: &OcrEngine, doc: &Document) -> bool {
    engine
        .ocr(&doc.page(0).expect("page"), &OPTS)
        .is_ok_and(|tp| {
            tp.to_text(BlockSeparator::Newline)
                .contains("quick brown fox")
        })
}

/// How long the parent's helper may take to reach its first page: the
/// page budget (`page_timeout`'s default), which also bounds its wait at
/// the rendezvous.
#[cfg(unix)]
const MARKER_WAIT: Duration = Duration::from_secs(60);

/// Waits until `marker` exists, panicking at [`MARKER_WAIT`].
#[cfg(unix)]
fn await_marker(marker: &Path) {
    let deadline = Instant::now() + MARKER_WAIT;
    while !marker.exists() {
        assert!(
            Instant::now() < deadline,
            "the parent's helper did not reach its first page within {MARKER_WAIT:?} ({} absent)",
            marker.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
fn main() {
    if std::env::args().any(|a| a == "--list") {
        return;
    }
    // Every helper holds its first page until `meet` has two entries. The
    // parent's helper supplies the first. Nothing can supply the second
    // before the fork: it is the child's helper (armed too, so it never
    // waits) or the release file this thread writes after forking,
    // whichever comes first.
    let meet = common::scratch("process-fork-rendezvous");
    let fault = format!("rendezvous-on-recognize:2:{}", meet.display());
    set_helper_test_env(&[
        ("PDF_EXTRACT_OCR_HELPER_FAULT", &fault),
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
        // The engine's only helper is idle, so it takes this page. Its
        // marker means it has read the whole frame, so the other thread has
        // rendered, checked the helper out (releasing the pool's lock) and
        // made its last write; from there to poll(2) it runs only
        // stack-only code. The helper cannot answer until the rendezvous
        // completes, which nothing can do before the fork.
        await_marker(&meet.join(parent_helper.to_string()));

        // SAFETY: the only other thread is in (or entering) poll(2) waiting
        // for its helper, holding no lock and making no allocation (see
        // above); the child uses the engine (whose pool notices the new
        // pid) and leaves through _exit, never unwinding into the parent's
        // state.
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
        // Completes the rendezvous if the child's helper has not, so the
        // parent's page finishes whatever the child does.
        std::fs::write(meet.join("release"), b"").expect("release the rendezvous");
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
