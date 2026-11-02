//! Unit tests of the parent side against scripted stand-in helpers: shell
//! scripts that read each request and answer with canned frames, so every
//! reply the real helper never sends (another build's version, an
//! unexpected kind, bad magic, an early exit, a hang, a reader that never
//! reads) can be produced on demand.

use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;

use super::*;
use crate::backend::{OcrBlock, OcrLine, OcrWord};
use crate::config::{EngineMode, PageSegMode};

/// What the stand-in does after its last answer.
const DRAIN: &str = "exec /bin/cat > /dev/null";
const SLEEP: &str = "exec /bin/sleep 30";

/// Pixels in every raster the stand-ins are asked to recognize, so each
/// can read exactly one `Recognize` frame before answering.
const PAGE: usize = 4;

/// The `Hello` every factory here sends.
fn hello_msg() -> Hello {
    Hello {
        proto_version: wire::PROTO_VERSION,
        oem: EngineMode::LstmOnly,
        psm: PageSegMode::Auto,
        languages: "eng".into(),
        tessdata: "/nonexistent".into(),
        debug_null: NULL_DEVICE.into(),
    }
}

/// Bytes of the `Hello` frame, which a stand-in reads before answering.
fn hello_len() -> usize {
    wire::encode(&Message::Hello(hello_msg())).len()
}

/// Bytes of a `Recognize` frame for an `image(PAGE)`.
const RECOGNIZE_LEN: usize = wire::HEADER_LEN + wire::RECOGNIZE_FIXED + PAGE;

/// A fresh directory under the temp dir, removed when dropped, so a run does
/// not leave `pxo-client-<pid>-*` behind for every stand-in it started.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pxo-client-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

thread_local! {
    /// The directories of the stand-ins this test has started (see `fake`).
    static SCRATCH_DIRS: std::cell::RefCell<Vec<Scratch>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A stand-in helper that, for each `(request, answer)` step, reads
/// exactly `request` bytes and then prints `answer` (raw bytes), and
/// finally runs `then`. The child's environment is empty, so commands are
/// absolute.
///
/// Like the real helper, it reads each request before answering. A
/// stand-in that answered first could exit before the parent had written
/// the request; the parent's write would then meet a closed pipe and
/// report the helper broken, never reading the answer already in the
/// pipe. `dd bs=1` reads a byte at a time, so it never takes bytes
/// belonging to a later request.
fn fake(name: &str, steps: &[(usize, &[u8])], then: &str) -> PathBuf {
    let scratch = Scratch::new(name);
    let dir = scratch.0.clone();
    // The stand-in's callers pass the returned path on to a factory and have
    // no handle to hold a guard in, so the guard lives until the test's thread
    // ends (the libtest thread runs each test and exits when it finishes, by
    // which time its helpers have been dropped and reaped).
    SCRATCH_DIRS.with(|dirs| dirs.borrow_mut().push(scratch));
    let mut script = String::from("#!/bin/sh\n");
    for (i, (request, answer)) in steps.iter().enumerate() {
        script.push_str(&format!(
            "/bin/dd bs=1 count={request} of=/dev/null 2>/dev/null\n"
        ));
        let replies = dir.join(format!("reply{i}.bin"));
        std::fs::write(&replies, answer).expect("write");
        script.push_str(&format!("/bin/cat '{}'\n", replies.display()));
    }
    script.push_str(then);
    script.push('\n');
    // The executable is written by a `cp` child, never by this process:
    // a descriptor open for writing here would be inherited by any helper
    // another test thread forks meanwhile, and until that child execs,
    // Linux refuses to run the script (ETXTBSY, "Text file busy").
    let source = dir.join("helper.txt");
    std::fs::write(&source, script).expect("write");
    let exe = dir.join("helper.sh");
    let copied = std::process::Command::new("/bin/cp")
        .arg(&source)
        .arg(&exe)
        .status()
        .expect("cp");
    assert!(copied.success(), "cp: {copied}");
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    exe
}

fn frames(msgs: &[Message]) -> Vec<u8> {
    msgs.iter().flat_map(wire::encode).collect()
}

fn ok_hello(version: &str) -> Message {
    Message::HelloOk(HelloOk {
        tesseract_version: "5.5.3".into(),
        library_version: version.into(),
        model_sha256: vec![("eng".into(), [3; 32])],
    })
}

fn factory(exe: PathBuf, start_timeout: Duration) -> HelperFactory {
    HelperFactory {
        exe,
        tessdata: PathBuf::from("/nonexistent"),
        hello: hello_msg(),
        start_timeout,
        languages: vec!["eng".into()],
        info: BackendInfo {
            name: "tesseract",
            version: "5.5.3".into(),
            languages: vec!["eng".into()],
            tessdata: None,
            model_sha256: Vec::new(),
            linkage: Linkage::Vendored,
            isolation: Isolation::Process,
        },
    }
}

const SOON: Duration = Duration::from_secs(10);

/// Starts a stand-in that reads `Hello`, answers `bytes` and runs `then`.
fn start_err(name: &str, bytes: &[u8], then: &str) -> OcrUnavailable {
    factory(fake(name, &[(hello_len(), bytes)], then), SOON)
        .start()
        .map(|_| ())
        .expect_err("start fails")
}

/// A helper that answered `Hello` and, when `reply` (raw bytes) is not
/// empty, will read one `Recognize` for an `image(PAGE)` and answer it with
/// `reply`; it then runs `then`. With an empty `reply` it reads nothing
/// after `Hello`, so `then` alone decides how it treats requests.
fn started(name: &str, reply: &[u8], then: &str) -> Helper {
    let ok = frames(&[ok_hello(pdf_extract_core::VERSION)]);
    let mut steps: Vec<(usize, &[u8])> = vec![(hello_len(), &ok)];
    if !reply.is_empty() {
        steps.push((RECOGNIZE_LEN, reply));
    }
    let f = factory(fake(name, &steps, then), SOON);
    assert_eq!(f.info().name, "tesseract");
    f.create().expect("started")
}

fn image(len: usize) -> GrayImage {
    GrayImage {
        width: u32::try_from(len).expect("small"),
        height: 1,
        data: vec![255; len],
    }
}

fn req(timeout: Duration) -> OcrRequest {
    OcrRequest {
        dpi: 300,
        deadline: Instant::now() + timeout,
        psm: PageSegMode::Auto,
    }
}

#[test]
fn a_helper_from_another_build_is_engine_not_built() {
    let e = start_err("version", &frames(&[ok_hello("0.0.0-other")]), DRAIN);
    assert_eq!(e.reason, UnavailableReason::EngineNotBuilt);
    assert!(e.detail.contains("0.0.0-other"), "{e}");
    assert!(e.detail.contains(pdf_extract_core::VERSION), "{e}");
}

#[test]
fn a_protocol_mismatch_names_both_builds() {
    // A helper speaking another protocol names only its own build (it is
    // not told ours); the parent adds this library's version, so the detail
    // shows both sides of the mismatch (§3.5).
    let detail = "protocol version 1 requested; this helper (pdf-extract 0.0.0-other) speaks 2";
    let msg = Message::HelloErr(reason::ENGINE_NOT_BUILT, detail.into());
    let e = start_err("proto", &frames(&[msg]), "exit 2");
    assert_eq!(e.reason, UnavailableReason::EngineNotBuilt);
    assert!(e.detail.starts_with(detail), "{e}");
    assert!(
        e.detail.contains(&format!(
            "this library is pdf-extract {}",
            pdf_extract_core::VERSION
        )),
        "{e}"
    );
}

#[test]
fn a_hello_err_carries_its_reason_and_the_helper_is_reaped() {
    let msg = Message::HelloErr(reason::LANGUAGE_DATA_MISSING, "no eng".into());
    let e = start_err("hello-err", &frames(&[msg]), "exit 1");
    assert_eq!(e.reason, UnavailableReason::LanguageDataMissing);
    assert_eq!(e.detail, "no eng");
    for (r, want) in [
        (reason::NO_TESSDATA, UnavailableReason::NoTessdata),
        (
            reason::LANGUAGE_DATA_MISSING,
            UnavailableReason::LanguageDataMissing,
        ),
        (reason::INIT_FAILED, UnavailableReason::InitFailed),
        (reason::ENGINE_NOT_BUILT, UnavailableReason::EngineNotBuilt),
        (200, UnavailableReason::InitFailed),
    ] {
        assert_eq!(reason_of(r), want);
    }
}

#[test]
fn bad_replies_to_hello_are_init_failures() {
    let e = start_err("unexpected", &frames(&[Message::Shutdown]), DRAIN);
    assert_eq!(e.reason, UnavailableReason::InitFailed);
    assert!(e.detail.contains("unexpected reply to Hello"), "{e}");
    let e = start_err("magic", b"garbage!garbage!", DRAIN);
    assert!(e.detail.contains("bad magic"), "{e}");
    // The helper read the Hello and went away without answering: the failure
    // is in reading the reply, not in sending.
    let e = start_err("exit", b"", "exit 7");
    assert_eq!(e.reason, UnavailableReason::InitFailed);
    assert_eq!(
        e.detail,
        "OCR helper failed to start (reading the reply to Hello): helper exited: exit status 7"
    );
}

#[test]
fn a_helper_that_never_answers_hello_is_killed() {
    let f = factory(fake("hang", &[], SLEEP), Duration::from_millis(200));
    let start = Instant::now();
    let e = f.start().map(|_| ()).expect_err("timeout");
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(e.detail.contains("did not start in time"), "{e}");
    // That it was killed and reaped is checked through the real binary
    // (tests/process.rs); here other tests spawn concurrently, so the pid
    // is not identifiable.
}

#[test]
fn a_missing_binary_is_engine_not_built() {
    let e = factory(PathBuf::from("/nonexistent/helper"), SOON)
        .start()
        .map(|_| ())
        .expect_err("spawn");
    assert_eq!(e.reason, UnavailableReason::EngineNotBuilt);
    assert!(e.detail.contains("cannot start OCR helper"), "{e}");
}

#[test]
fn replies_to_recognize_are_classified() {
    let out = OcrOutput {
        blocks: vec![OcrBlock {
            lines: vec![OcrLine {
                words: vec![OcrWord {
                    text: "hi".into(),
                    bbox_px: [1, 2, 3, 4],
                    confidence: 90.0,
                }],
            }],
        }],
    };
    let mut h = started("result", &frames(&[Message::Result(out.clone())]), DRAIN);
    assert!(h.id() > 0);
    assert_eq!(h.recognize(&image(PAGE), &req(SOON)), Ok(out));
    h.clear();
    drop(h);

    let mut h = started(
        "timeout",
        &frames(&[Message::PageErr(page_err::TIMEOUT, "late".into())]),
        DRAIN,
    );
    assert_eq!(
        h.recognize(&image(PAGE), &req(SOON)),
        Err(OcrPageError::Timeout(Duration::ZERO))
    );
    let mut h = started(
        "failed",
        &frames(&[Message::PageErr(page_err::RECOGNITION, "boom".into())]),
        DRAIN,
    );
    assert_eq!(
        h.recognize(&image(PAGE), &req(SOON)),
        Err(OcrPageError::Recognition("boom".into()))
    );
    let mut h = started("odd", &frames(&[ok_hello("x")]), DRAIN);
    assert_eq!(
        h.recognize(&image(PAGE), &req(SOON)),
        Err(OcrPageError::Recognition(
            "helper: unexpected reply to Recognize".into()
        ))
    );
    // Half a frame, then the helper exits.
    let mut h = started("truncated", &wire::MAGIC, "exit 5");
    assert_eq!(
        h.recognize(&image(PAGE), &req(SOON)),
        Err(OcrPageError::Recognition(
            "helper exited: exit status 5".into()
        ))
    );
    // Once reaped, writing reports the closed pipe without a second reap.
    assert!(h.recognize(&image(PAGE), &req(SOON)).is_err());
}

#[test]
fn a_helper_that_stops_reading_times_out_the_write() {
    let mut h = started("no-read", b"", SLEEP);
    let start = Instant::now();
    // Far more than a pipe buffer.
    let r = h.recognize(&image(4 << 20), &req(Duration::from_millis(100)));
    assert_eq!(r, Err(OcrPageError::Timeout(Duration::ZERO)));
    let elapsed = start.elapsed();
    assert!(elapsed >= GRACE, "{elapsed:?}");
    assert!(elapsed < GRACE + Duration::from_secs(2), "{elapsed:?}");
}

#[test]
fn a_helper_that_ignores_shutdown_is_killed_on_drop() {
    let h = started("ignore", b"", SLEEP);
    let pid = h.id();
    let start = Instant::now();
    drop(h);
    let elapsed = start.elapsed();
    assert!(elapsed >= SHUTDOWN_WAIT, "{elapsed:?}");
    let status = helper_log()
        .1
        .into_iter()
        .find(|(p, _)| *p == pid)
        .map(|(_, s)| s)
        .expect("reaped");
    assert_eq!(status.signal(), Some(9));
}

#[test]
fn an_abandoned_helper_only_loses_its_pipes() {
    let dir = Scratch::new("marker");
    let marker = dir.0.join("eof");
    let then = format!(
        "/bin/cat > /dev/null; /bin/cat /dev/null > '{}'",
        marker.display()
    );
    let mut h = started("abandon", b"", &then);
    let pid = h.id();
    // As if this process were a forked child of the helper's owner.
    h.owner = h.owner.wrapping_add(1);
    h.abandon();
    let until = Instant::now() + Duration::from_secs(10);
    while !marker.exists() {
        assert!(Instant::now() < until, "the helper never saw end of input");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        helper_log().1.iter().all(|(p, _)| *p != pid),
        "a foreign helper is never waited for"
    );
}

#[test]
fn a_raster_over_the_frame_limit_is_refused_without_writing() {
    let mut h = started("big", b"", DRAIN);
    let len = wire::MAX_FRAME as usize - wire::RECOGNIZE_FIXED + 1;
    let r = h.recognize(&image(len), &req(SOON));
    assert!(
        matches!(&r, Err(OcrPageError::Recognition(m)) if m.contains("does not fit")),
        "{r:?}"
    );
}

#[test]
fn statuses_are_described_with_signal_names() {
    assert_eq!(describe(ExitStatus::from_raw(3 << 8)), "exit status 3");
    assert_eq!(describe(ExitStatus::from_raw(6)), "signal 6 (SIGABRT)");
    assert_eq!(describe(ExitStatus::from_raw(9)), "signal 9 (SIGKILL)");
    assert_eq!(describe(ExitStatus::from_raw(60)), "signal 60");
    // A stopped status has neither a code nor a terminating signal.
    let stopped = ExitStatus::from_raw(0x117f);
    assert!(!describe(stopped).is_empty());
}

#[test]
fn the_helper_environment_is_minimal_plus_the_test_set() {
    set_helper_test_env(&[
        ("PDF_EXTRACT_OCR_HELPER_FAULT", "report-env"),
        ("LD_PRELOAD", "/tmp/x.so"),
    ]);
    let env = helper_env(Path::new("/models"));
    set_helper_test_env(&[]);
    assert!(
        env.iter().all(|(k, _)| k != "LD_PRELOAD"),
        "only fault names pass"
    );
    assert_eq!(env[0], ("TESSDATA_PREFIX".into(), "/models".into()));
    assert!(env.contains(&("PDF_EXTRACT_OCR_HELPER_FAULT".into(), "report-env".into())));
    let allowed = [
        "TESSDATA_PREFIX",
        "LANG",
        "LC_ALL",
        "PDF_EXTRACT_OCR_HELPER_FAULT",
        "PDF_EXTRACT_OCR_HELPER_FAULT_ARMED",
        "LLVM_PROFILE_FILE",
    ];
    for (k, _) in &env {
        assert!(allowed.iter().any(|a| k == a), "{k:?}");
    }
    let mut v = Vec::new();
    for i in 0..LOG_CAP + 5 {
        push_bounded(&mut v, i);
    }
    assert_eq!(v.len(), LOG_CAP);
    assert_eq!(v[0], 5);
}

#[test]
fn dropping_an_idle_helper_sends_exactly_a_shutdown_frame() {
    // Exit status 0 alone would not show the frame was sent: the real
    // helper also exits 0 at end of input. This stand-in records every
    // byte after `Hello` until end of input instead.
    let dir = Scratch::new("record");
    let record = dir.0.join("after-hello.bin");
    let h = started(
        "record",
        b"",
        &format!("exec /bin/cat > '{}'", record.display()),
    );
    let pid = h.id();
    drop(h);
    // Drop waited for the exit, so `cat` has written everything it read.
    let status = helper_log()
        .1
        .into_iter()
        .find(|(p, _)| *p == pid)
        .map(|(_, s)| s)
        .expect("reaped");
    assert_eq!(status.code(), Some(0), "{status:?}");
    let bytes = std::fs::read(&record).expect("record");
    let mut input = &bytes[..];
    assert_eq!(wire::read_frame(&mut input).ok(), Some(Message::Shutdown));
    assert!(input.is_empty(), "{} bytes after Shutdown", input.len());
}

#[test]
fn a_hello_the_helper_never_reads_fails_the_write() {
    // A stand-in that exits without reading, and a `Hello` far larger than
    // a pipe buffer: the write cannot complete before the read end closes,
    // so it is the write, not the read of the answer, that fails (EPIPE).
    let mut f = factory(fake("no-hello", &[], "exit 7"), SOON);
    f.hello.tessdata = "x".repeat(1 << 20);
    let e = f.start().map(|_| ()).expect_err("start fails");
    assert_eq!(e.reason, UnavailableReason::InitFailed);
    assert_eq!(
        e.detail,
        "OCR helper failed to start (sending Hello): helper exited: exit status 7"
    );
}
