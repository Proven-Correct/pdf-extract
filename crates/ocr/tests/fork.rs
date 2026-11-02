//! Fork safety of the pool (spec 0002 O11; §4.8 "Fork"): a forked child
//! abandons the parent's instances, builds its own and recognises
//! correctly; the parent's pool is untouched.
//!
//! `harness = false`: the process must be single-threaded when it forks
//! (libtest runs tests on worker threads), so this binary has its own
//! `main` and forks from it.

mod common;

use pdf_extract_core::BlockSeparator;
use pdf_extract_ocr::{OcrEngine, OcrOptions, fork_resets};

const OPTS: OcrOptions = OcrOptions { dpi: Some(150) };

fn recognises(engine: &OcrEngine, doc: &pdf_extract_core::Document) -> bool {
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
    let engine =
        OcrEngine::tesseract_with_candidates(common::config(), &common::nowhere()).expect("engine");
    let doc = common::doc(common::body_page());
    assert!(recognises(&engine, &doc), "parent, before fork");
    assert_eq!(engine.pool_len(), Some(1));
    assert_eq!(fork_resets(), 0);

    // SAFETY: the process is single-threaded here (no test harness, no
    // threads spawned), so the child is a complete copy; it uses only
    // async-signal-unsafe code that is safe in a single-threaded child and
    // leaves through _exit, never unwinding into the parent's state.
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0, "fork failed");
    if pid == 0 {
        let ok = recognises(&engine, &doc) && fork_resets() == 1 && engine.pool_len() == Some(1);
        // SAFETY: _exit ends the child immediately, without running the
        // parent's atexit handlers or destructors on the copied state.
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
    assert!(recognises(&engine, &doc), "parent, after fork");
    assert_eq!(fork_resets(), 0, "the parent never resets");
    assert_eq!(engine.pool_len(), Some(1));
    println!("fork: child rebuilt its pool and recognised; parent unaffected: ok");
}

#[cfg(not(unix))]
fn main() {}
