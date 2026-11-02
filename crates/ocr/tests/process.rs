//! Process isolation through the real helper binary (spec 0002 O26–O32;
//! §4.9). Every test here builds only `Process` engines, so this test
//! binary itself must never initialise Tesseract.
//!
//! Fault hooks are armed on the helpers' environment through
//! `set_helper_test_env` — the parent's own environment never has them —
//! which is process-wide, so the tests take a lock and run one at a time.

mod common;

use std::process::ExitStatus;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use pdf_extract_core::UnavailableReason;
use pdf_extract_core::{BlockSeparator, Document, Limits, OcrError, OcrPageError};
use pdf_extract_ocr::isolation::client::{GRACE, helper_log, set_helper_test_env};
use pdf_extract_ocr::{Isolation, OcrConfig, OcrEngine, OcrOptions};
use pdf_extract_render::{RenderOptions, render_gray};

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
    set_helper_test_env(&[]);
    guard
}

/// Arms `fault` for helpers spawned from now on.
fn arm(fault: &str) {
    set_helper_test_env(&[
        ("PDF_EXTRACT_OCR_HELPER_FAULT", fault),
        ("PDF_EXTRACT_OCR_HELPER_FAULT_ARMED", "1"),
    ]);
}

const OPTS: OcrOptions = OcrOptions { dpi: Some(150) };

fn engine_with(cfg: OcrConfig) -> OcrEngine {
    OcrEngine::tesseract_with_candidates(cfg, &common::nowhere()).expect("process engine")
}

fn engine() -> OcrEngine {
    engine_with(common::process_config())
}

fn body() -> Document {
    common::doc(common::body_page())
}

fn ocr(e: &OcrEngine, doc: &Document) -> Result<String, OcrError> {
    e.ocr(&doc.page(0).expect("page"), &OPTS)
        .map(|tp| tp.to_text(BlockSeparator::Newline))
}

fn spawned() -> Vec<u32> {
    helper_log().0
}

/// The reaped status of `pid`, if it was reaped.
fn reaped(pid: u32) -> Option<ExitStatus> {
    helper_log()
        .1
        .into_iter()
        .rev()
        .find(|(p, _)| *p == pid)
        .map(|(_, s)| s)
}

/// The helper spawned most recently.
fn last_spawned() -> u32 {
    *spawned().last().expect("a helper was spawned")
}

fn recognition_message(r: Result<String, OcrError>) -> String {
    match r {
        Err(OcrError::Page(OcrPageError::Recognition(m))) => m,
        other => panic!("expected a recognition error, got {other:?}"),
    }
}

#[test]
fn construction_spawns_one_helper_and_the_parent_never_loads_tesseract() {
    let _g = serial();
    let before = spawned().len();
    let e = engine();
    assert_eq!(spawned().len(), before + 1, "exactly one helper");
    assert_eq!(e.pool_len(), Some(1));
    let info = e.info();
    assert_eq!(info.isolation, Isolation::Process);
    assert!(info.version.starts_with('5'), "{}", info.version);
    assert_eq!(info.languages, vec!["eng".to_owned()]);
    assert_eq!(info.model_sha256.len(), 1);
    assert_eq!(e.tessdata(), Some(common::tessdata().as_path()));
    let text = ocr(&e, &body()).expect("ocr");
    assert!(text.contains("quick brown fox"), "{text}");
    assert_eq!(spawned().len(), before + 1, "the page reused the helper");
    assert_eq!(pdf_extract_tesseract_sys::global_init_calls(), 0);
}

#[test]
fn an_aborting_helper_fails_the_page_and_is_replaced() {
    let _g = serial();
    arm("abort-on-recognize");
    let e = engine();
    let pid = last_spawned();
    let msg = recognition_message(ocr(&e, &body()));
    assert!(msg.contains("SIGABRT"), "{msg}");
    assert!(msg.starts_with("helper exited: signal "), "{msg}");
    if cfg!(target_os = "linux") {
        assert!(msg.contains("signal 6"), "{msg}");
    }
    assert!(reaped(pid).is_some(), "the crashed helper was reaped");
    assert_eq!(e.pool_len(), Some(0), "the dead slot was dropped");
    set_helper_test_env(&[]);
    let text = ocr(&e, &body()).expect("respawned helper");
    assert!(text.contains("quick brown fox"), "{text}");
    assert_ne!(last_spawned(), pid);
    assert_eq!(e.pool_len(), Some(1));
}

#[test]
fn an_exiting_helper_reports_its_status() {
    let _g = serial();
    arm("exit-on-recognize");
    let e = engine();
    let pid = last_spawned();
    let msg = recognition_message(ocr(&e, &body()));
    assert_eq!(msg, "helper exited: exit status 3");
    assert_eq!(reaped(pid).and_then(|s| s.code()), Some(3));
}

#[test]
fn a_garbage_frame_kills_the_helper() {
    let _g = serial();
    arm("garbage-on-recognize");
    let e = engine();
    let pid = last_spawned();
    let msg = recognition_message(ocr(&e, &body()));
    assert!(msg.contains("frame"), "{msg}");
    assert!(msg.starts_with("helper: frame too large"), "{msg}");
    assert!(reaped(pid).is_some(), "killed and reaped");
    set_helper_test_env(&[]);
    assert!(ocr(&e, &body()).is_ok(), "respawned");
}

#[test]
fn a_hanging_helper_is_killed_at_the_deadline() {
    let _g = serial();
    arm("hang-on-recognize");
    let timeout = Duration::from_millis(500);
    let e = engine_with(OcrConfig {
        page_timeout: timeout,
        ..common::process_config()
    });
    let pid = last_spawned();
    let start = Instant::now();
    let r = ocr(&e, &body());
    let elapsed = start.elapsed();
    assert!(
        matches!(r, Err(OcrError::Page(OcrPageError::Timeout(d))) if d == timeout),
        "{r:?}"
    );
    // The kill comes at timeout + GRACE; the slack covers the kill, the
    // reap and a loaded scheduler, not a second GRACE.
    let bound = timeout + GRACE + Duration::from_secs(2);
    assert!(elapsed < bound, "{elapsed:?} (bound {bound:?})");
    // waitpid returned: no zombie is left behind.
    let status = reaped(pid).expect("reaped");
    assert_eq!(status.code(), None, "killed by a signal: {status:?}");
}

/// The render options `OcrEngine::ocr` uses for a 72 dpi page.
const RENDER_72: RenderOptions = RenderOptions {
    dpi: 72,
    max_render_pixels: 40_000_000,
    render_annotations: true,
};

/// How long `doc`'s first page takes to render: the median of three, so a
/// single scheduling outlier does not set it.
fn median_render(doc: &Document) -> Duration {
    common::timing::measure(3, || {
        render_gray(&doc.page(0).expect("page"), &RENDER_72).expect("render");
    })
    .median
}

/// The form fan-out fixture at the smallest depth whose render takes at
/// least `floor`. Calibrating the depth keeps the render near `floor` on
/// fast and slow machines alike, which matters because the deadline test's
/// margin against the parent's kill is absolute (GRACE). One render per
/// depth: a first render costs about what later ones do.
fn slow_render_page(floor: Duration) -> Document {
    let limits = Limits {
        max_ops_per_page: u64::MAX,
        page_time_budget: Duration::from_secs(600),
        ..Limits::default()
    };
    for depth in 12..=24 {
        let doc = common::open(
            pdf_extract_testkit::hostile::form_fanout(depth),
            limits.clone(),
        );
        let start = Instant::now();
        render_gray(&doc.page(0).expect("page"), &RENDER_72).expect("render");
        if start.elapsed() >= floor {
            return doc;
        }
    }
    panic!("no fan-out depth renders in {floor:?}");
}

#[test]
fn the_deadline_counts_from_the_start_of_ocr_not_from_the_pipe_write() {
    let _g = serial();
    // A page whose render takes a measurable time R, and a helper that
    // sleeps S = timeout − R/2 before recognising. The helper's budget is
    // what the client sends with the request:
    // - clock from ocr(): it is left timeout − R₂ (R₂ the render inside
    //   ocr()), less than S whenever R₂ > R/2, so it reports a timeout;
    // - clock from the pipe write: it is left about the whole timeout, more
    //   than S by R/2, enough to recognise this one-word page, so the page
    //   succeeds.
    // A success is therefore also what the right clock gives when R₂ came
    // in under R/2. Renders swing by 2–3× under load (a thread moved to
    // another core, a neighbour's burst), so a success only means the
    // attempt was inconclusive: it is retried with a fresh measurement,
    // and the test fails only if no attempt times out. A wrong clock
    // succeeds every time. A render under 300 ms leaves the wrong clock
    // too little room to recognise (it would time out too), so the depth
    // was calibrated while renders ran slow: that attempt is inconclusive
    // and recalibrates (internal issue #117).
    let mut doc = slow_render_page(Duration::from_millis(600));
    let suspect = "the helper's budget counted from the pipe write, not the start of ocr()";
    common::timing::retry("helper deadline", suspect, 6, || {
        let render = median_render(&doc);
        if render <= Duration::from_millis(300) {
            doc = slow_render_page(Duration::from_millis(600));
            return Err(format!("render too fast: {render:?}; recalibrated"));
        }
        // Far more than the render inside ocr() can use up (the engine
        // would then time out without asking the helper at all).
        let timeout = render * 3 + Duration::from_secs(1);
        let slow = timeout - render / 2;
        arm(&format!("slow-on-recognize:{}", slow.as_millis()));
        let e = engine_with(OcrConfig {
            page_timeout: timeout,
            ..common::process_config()
        });
        let start = Instant::now();
        let r = e.ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(72) });
        let elapsed = start.elapsed();
        let tried = format!("render {render:?}, helper sleep {slow:?}, elapsed {elapsed:?}");
        if r.is_ok() {
            return Err(format!("{tried}: success"));
        }
        assert!(
            matches!(r, Err(OcrError::Page(OcrPageError::Timeout(d))) if d == timeout),
            "{tried}: {r:?}"
        );
        // The request reached the helper (whose slot a page error drops).
        assert_eq!(e.pool_len(), Some(0), "{tried}");
        // Answered cooperatively by the helper, not by the parent's kill:
        // the answer comes at timeout + (R₂ − R/2), so this holds while the
        // render inside ocr() stays under R/2 + 2 s.
        assert!(elapsed < timeout + Duration::from_secs(2), "{tried}");
        Ok(())
    });
}

#[test]
fn the_parent_kills_at_the_deadline_counted_from_the_start_of_ocr() {
    let _g = serial();
    // The previous test covers the deadline the helper is sent; this one
    // the parent's own kill (O27), with a helper that never answers. The
    // render takes R, so the kill comes at timeout + GRACE from the start
    // of ocr() with the right clock, and R₂ (the render inside ocr()) later
    // with a clock started at the pipe write. The upper bound sits Rmin/2
    // past the right kill, Rmin the fastest of the renders sampled just
    // before the attempt: load only slows a render, so R₂ falling under
    // Rmin/2 (which would hide a wrong clock) needs every sample to have
    // run at twice R₂, and Rmin/2 > 300 ms absorbs the kill and reap.
    // Under load the kill and reap can still take longer than that, so an
    // attempt over the bound is inconclusive and is retried with a fresh
    // measurement (internal issue #64); a wrong clock passes only on an attempt
    // where R₂ came in under Rmin/2.
    let mut doc = slow_render_page(Duration::from_secs(1));
    let suspect = "the kill clock started late, at the pipe write, not the start of ocr()";
    common::timing::retry("parent kill", suspect, 6, || {
        let render = common::timing::measure(3, || {
            render_gray(&doc.page(0).expect("page"), &RENDER_72).expect("render");
        });
        if render.min <= Duration::from_millis(600) {
            // The depth was calibrated while renders ran slow.
            doc = slow_render_page(Duration::from_secs(1));
            return Err(format!("render too fast: {render:?}; recalibrated"));
        }
        // Far more than the render inside ocr() can use up, so the request
        // is sent (the engine would otherwise time out before asking the
        // helper).
        let timeout = render.median * 3 + Duration::from_secs(1);
        arm("hang-on-recognize");
        let e = engine_with(OcrConfig {
            page_timeout: timeout,
            ..common::process_config()
        });
        let pid = last_spawned();
        let start = Instant::now();
        let r = e.ocr(&doc.page(0).expect("page"), &OcrOptions { dpi: Some(72) });
        let elapsed = start.elapsed();
        let tried = format!("render {render:?}, elapsed {elapsed:?}");
        assert!(
            matches!(r, Err(OcrError::Page(OcrPageError::Timeout(d))) if d == timeout),
            "{tried}: {r:?}"
        );
        let kill = timeout + GRACE;
        // The engine's clock check answers at R₂ ≥ timeout, before any
        // kill could; a slot kept after a request (a broken O10) answers
        // at the kill or later and must not pass for a slow render.
        if e.pool_len() == Some(1) && elapsed < kill {
            return Err(format!("{tried}: the render used up the budget"));
        }
        // Killed by the parent, not answered: the helper never replies.
        let status = reaped(pid).expect("reaped");
        assert_eq!(status.code(), None, "killed by a signal: {status:?}");
        assert_eq!(e.pool_len(), Some(0), "the request reached the helper");
        assert!(elapsed >= kill, "{tried}: killed early");
        if elapsed >= kill + render.min / 2 {
            return Err(format!(
                "{tried}: past the bound; the kill clock started late?"
            ));
        }
        Ok(())
    });
}

#[test]
fn dropping_the_engine_shuts_down_every_idle_helper() {
    let _g = serial();
    arm("slow-on-recognize:300");
    let e = engine_with(OcrConfig {
        max_instances: 3,
        ..common::process_config()
    });
    let before = spawned().len();
    let doc = body();
    std::thread::scope(|s| {
        for _ in 0..3 {
            s.spawn(|| ocr(&e, &doc).expect("page"));
        }
    });
    assert_eq!(e.pool_len(), Some(3));
    let pids: Vec<u32> = spawned().into_iter().collect();
    // The construction helper plus the two the concurrent pages added.
    let ours: Vec<u32> = pids[before - 1..].to_vec();
    assert_eq!(ours.len(), 3, "{pids:?}");
    let start = Instant::now();
    drop(e);
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    for pid in ours {
        let status = reaped(pid).expect("reaped on drop");
        assert_eq!(status.code(), Some(0), "pid {pid}: {status:?}");
    }
}

#[test]
fn helpers_start_with_only_the_standard_descriptors() {
    let _g = serial();
    // A descriptor the host left inheritable (as a shell, a supervisor or
    // a C library may): the helper must still not receive it (O30).
    let file = std::fs::File::open(env!("CARGO_BIN_EXE_pdf-extract-ocr-helper")).expect("open");
    let fd = std::os::fd::AsRawFd::as_raw_fd(&file);
    // SAFETY: (test) F_GETFD/F_SETFD on a descriptor this test owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFD);
        assert_eq!(libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC), 0);
    }
    arm("report-fds");
    let e = engine();
    let msg = recognition_message(ocr(&e, &body()));
    assert_eq!(msg, "fds=0,1,2", "inheritable fd {fd} leaked?");
    drop(file);
}

#[test]
fn helpers_get_only_the_allowed_environment() {
    let _g = serial();
    arm("report-env");
    let e = engine();
    let msg = recognition_message(ocr(&e, &body()));
    let names = msg.strip_prefix("env=").expect("env report");
    let allowed = [
        "TESSDATA_PREFIX",
        "LANG",
        "LC_ALL",
        "PDF_EXTRACT_OCR_HELPER_FAULT",
        "PDF_EXTRACT_OCR_HELPER_FAULT_ARMED",
        // Present only under `cargo llvm-cov` (an instrumented helper).
        "LLVM_PROFILE_FILE",
    ];
    for name in names.split(',') {
        assert!(
            // An instrumented helper's profiling runtime sets its own
            // `__LLVM_PROFILE_*` marker after start-up; it is not passed in.
            allowed.contains(&name) || name.starts_with("__LLVM_PROFILE"),
            "unexpected variable {name} in {msg}"
        );
    }
    assert!(names.split(',').any(|n| n == "TESSDATA_PREFIX"), "{msg}");
}

#[cfg(target_os = "linux")]
#[test]
fn proc_environ_of_a_live_helper_holds_only_allowed_variables() {
    let _g = serial();
    let e = engine();
    let pid = last_spawned();
    let raw = std::fs::read(format!("/proc/{pid}/environ")).expect("environ");
    for var in raw.split(|b| *b == 0).filter(|v| !v.is_empty()) {
        let name = String::from_utf8_lossy(var.split(|b| *b == b'=').next().unwrap_or_default());
        assert!(
            ["TESSDATA_PREFIX", "LANG", "LC_ALL", "LLVM_PROFILE_FILE"].contains(&name.as_ref())
                || name.starts_with("__LLVM_PROFILE"),
            "unexpected {name}"
        );
    }
    drop(e);
}

#[test]
fn the_pool_bounds_helpers_and_every_page_completes() {
    let _g = serial();
    arm("slow-on-recognize:200");
    let e = engine_with(OcrConfig {
        max_instances: 2,
        ..common::process_config()
    });
    let doc = body();
    let before = spawned().len();
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..4).map(|_| s.spawn(|| ocr(&e, &doc))).collect();
        for h in handles {
            let text = h.join().expect("thread").expect("page");
            assert!(text.contains("quick brown fox"));
        }
    });
    assert!(spawned().len() - before <= 1, "at most one helper added");
    assert_eq!(e.pool_len(), Some(2));
}

#[test]
fn a_model_tesseract_cannot_load_fails_inside_the_helper() {
    let _g = serial();
    let dir = common::scratch("process-empty-model");
    std::fs::write(dir.join("eng.traineddata"), b"").expect("write");
    let before = spawned().len();
    let err = OcrEngine::tesseract_with_candidates(
        OcrConfig {
            tessdata: Some(dir.clone()),
            ..common::process_config()
        },
        &common::nowhere(),
    )
    .expect_err("empty model");
    assert_eq!(err.reason, UnavailableReason::InitFailed, "{err}");
    assert_eq!(err.tessdata.as_deref(), Some(dir.as_path()));
    assert!(err.searched.is_empty(), "the explicit directory was chosen");
    let pid = *spawned().get(before).expect("one helper tried");
    assert_eq!(
        reaped(pid).and_then(|s| s.code()),
        Some(1),
        "exit 1 after HelloErr"
    );
}

#[test]
fn a_helper_that_vanishes_makes_the_respawn_unavailable() {
    let _g = serial();
    let dir = common::scratch("process-vanish");
    let exe = dir.join("pdf-extract-ocr-helper");
    std::fs::copy(env!("CARGO_BIN_EXE_pdf-extract-ocr-helper"), &exe).expect("copy helper");
    arm("exit-on-recognize");
    let e = engine_with(OcrConfig {
        helper_dir: Some(dir),
        ..common::process_config()
    });
    assert!(ocr(&e, &body()).is_err());
    std::fs::remove_file(&exe).expect("rm");
    match ocr(&e, &body()) {
        Err(OcrError::Unavailable(u)) => {
            assert_eq!(u.reason, UnavailableReason::EngineNotBuilt, "{u}");
            assert!(u.detail.contains("cannot start OCR helper"), "{u}");
        }
        other => panic!("expected EngineNotBuilt, got {other:?}"),
    }
}

#[test]
fn a_render_cap_too_large_for_one_frame_is_refused_at_construction() {
    let _g = serial();
    // An 80 MP cap would let a page of 8,000 × 9,000 pt OCRed with
    // `dpi: Some(72)` render a 72,000,000-pixel raster (about 78 MP at the
    // default 300 dpi, clamped), which no frame carries. The engine is refused
    // before any helper starts, rather than failing such pages only (O26);
    // the client's own per-raster check is covered by client_tests.
    let before = spawned().len();
    let r = OcrEngine::tesseract_with_candidates(
        OcrConfig {
            max_render_pixels: 80_000_000,
            ..common::process_config()
        },
        &common::nowhere(),
    );
    match r {
        Err(u) => {
            assert_eq!(u.reason, UnavailableReason::InitFailed, "{u}");
            assert!(u.detail.contains("max_render_pixels"), "{u}");
        }
        Ok(_) => panic!("an 80 MP cap was accepted under Process"),
    }
    assert_eq!(spawned().len(), before, "no helper was started");
}
