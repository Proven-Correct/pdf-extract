//! The parent side of process isolation (spec 0002 O26–O30, O32, §3.5).
//!
//! A `Helper` is one child process plus the parent's ends of its stdin
//! and stdout pipes; `HelperFactory` makes them for the pool, which treats
//! a helper exactly like an in-process instance: one page at a time,
//! returned after success, dropped after any failure.
//!
//! **Deadlines without threads (O32).** Everything happens on the calling
//! thread. Before each read the parent `poll`s the helper's stdout until
//! the page deadline plus [`GRACE`] (the deadline itself is sent to the
//! child, so the common case ends cooperatively with a `PageErr`); the
//! parent's stdin end is non-blocking, so a helper that stops reading
//! cannot hold a large image write past the deadline either. On expiry the
//! helper is killed with `SIGKILL` and reaped, and the page is a timeout.
//!
//! **Failures.** A helper that exits, crashes or closes its output is
//! reaped and the page fails with `helper exited: <status>`; a malformed
//! frame gets the helper killed and the page fails with `helper: <what>`.
//! The pool then drops the slot and the next checkout spawns a fresh
//! helper (O27). Every helper this module spawns is reaped by it, so no
//! zombie outlives a failure.
//!
//! **Drop and fork (O30).** Dropping an idle helper sends `Shutdown`,
//! closes its stdin and waits up to [`SHUTDOWN_WAIT`] for it to exit before
//! killing it. In a forked child the helpers belong to the parent: drop
//! then only closes this process's copies of the pipe ends, never signals
//! or waits.
//!
//! **Environment (§3.5).** A helper starts with an empty environment plus
//! `TESSDATA_PREFIX` (the resolved model directory), `LANG`/`LC_ALL` when
//! set here, and the fault-hook variables only when this process has them
//! (or a test set them with [`set_helper_test_env`]). Its stderr is the
//! null device. Every descriptor above 2 is made close-on-exec in the
//! forked child before the helper runs (`Command` only guarantees that for
//! its own pipes), so the helper starts with exactly fds 0, 1 and 2.
//!
//! **SIGPIPE.** Writes go through `pipe::write`, which never lets a dead
//! helper's closed pipe raise `SIGPIPE` in the caller, whatever the host's
//! signal disposition.
//!
//! **Respawn time.** A replacement helper is started inside the `ocr()`
//! call that needs it and is given the engine's start-up allowance
//! (`max(page_timeout, 60 s) + 2 s`), not the page's remaining budget: a
//! model load cannot be made shorter than it is, and failing every respawn
//! of an engine with a short `page_timeout` would make it unusable.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use pdf_extract_core::{OcrPageError, OcrUnavailable, UnavailableReason};
use pdf_extract_render::GrayImage;
use pdf_extract_tesseract_sys::pipe::{self, Ready};

use super::discovery;
use super::helper::{FAULT_ARMED_ENV, FAULT_ENV};
use super::wire::{self, Hello, HelloOk, Message, RecognizeHeader, WireError, page_err, reason};
use crate::backend::{BackendInfo, Linkage, OcrOutput, OcrRequest, Recognizer, RecognizerFactory};
use crate::config::{Isolation, OcrConfig};
use crate::engine::deadline_after;
use crate::tessdata::{self, Candidates};

/// Transport allowance on top of the page deadline (§3.5).
pub const GRACE: Duration = Duration::from_secs(2);
/// How long a dropped helper is given to exit after `Shutdown` (§3.5).
pub const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);
/// How long a helper whose output closed is given to finish exiting before
/// it is killed (it normally is a zombie already).
const EXIT_WAIT: Duration = Duration::from_secs(1);
/// The null device the helper's Tesseract writes its debug output to.
const NULL_DEVICE: &str = "/dev/null";
/// The model directory variable, passed to the helper explicitly.
const TESSDATA_ENV: &str = "TESSDATA_PREFIX";
/// Locale variables passed through when set.
const LOCALE_ENV: [&str; 2] = ["LANG", "LC_ALL"];

/// Extra variables for helpers spawned from now on (test seam, O31: arms
/// fault hooks on the *helper's* environment, not this process's).
///
/// A `fork()` while another thread holds this mutex leaves it locked in the
/// child (as does `LOG` below); see the
/// [crate-level `Forking` section](crate#forking).
static TEST_ENV: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// What this process spawned and reaped, most recent last, bounded.
struct Log {
    spawned: Vec<u32>,
    reaped: Vec<(u32, ExitStatus)>,
}

/// The spawn/reap log. Holding this mutex across a `fork()` in another
/// thread deadlocks the child; see the
/// [crate-level `Forking` section](crate#forking).
static LOG: Mutex<Log> = Mutex::new(Log {
    spawned: Vec::new(),
    reaped: Vec::new(),
});

/// Entries kept per list in [`LOG`].
const LOG_CAP: usize = 1024;

fn log() -> std::sync::MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(PoisonError::into_inner)
}

fn push_bounded<T>(v: &mut Vec<T>, item: T) {
    if v.len() == LOG_CAP {
        v.remove(0);
    }
    v.push(item);
}

/// Sets the fault-hook variables for helpers spawned after this call,
/// replacing the previous set (tests only). Only the two
/// `PDF_EXTRACT_OCR_HELPER_FAULT*` names are accepted; any other name is
/// ignored, so this seam cannot widen the helper's environment (no
/// `LD_PRELOAD` through it).
#[doc(hidden)]
pub fn set_helper_test_env(vars: &[(&str, &str)]) {
    *TEST_ENV.lock().unwrap_or_else(PoisonError::into_inner) = vars
        .iter()
        .filter(|(k, _)| [FAULT_ENV, FAULT_ARMED_ENV].contains(k))
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
}

/// Process ids of helpers spawned by this process, and the exit statuses
/// of those it reaped, oldest first (test observability, §4.9).
#[doc(hidden)]
#[must_use]
pub fn helper_log() -> (Vec<u32>, Vec<(u32, ExitStatus)>) {
    let log = log();
    (log.spawned.clone(), log.reaped.clone())
}

/// `exit status 3`, `signal 6 (SIGABRT)`.
pub(crate) fn describe(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        format!("exit status {code}")
    } else if let Some(sig) = status.signal() {
        match pipe::signal_name(sig) {
            Some(name) => format!("signal {sig} ({name})"),
            None => format!("signal {sig}"),
        }
    } else {
        status.to_string()
    }
}

/// The helper's environment (§3.5).
fn helper_env(tessdata: &Path) -> Vec<(OsString, OsString)> {
    let mut env = vec![(
        OsString::from(TESSDATA_ENV),
        tessdata.as_os_str().to_owned(),
    )];
    for name in LOCALE_ENV.into_iter().chain([FAULT_ENV, FAULT_ARMED_ENV]) {
        if let Some(v) = std::env::var_os(name) {
            env.push((name.into(), v));
        }
    }
    // Under `cargo llvm-cov` the helper is an instrumented binary too; it
    // needs the profile path to record its coverage (§4.9). Never compiled
    // into a normal build.
    #[cfg(coverage)]
    if let Some(v) = std::env::var_os("LLVM_PROFILE_FILE") {
        env.push(("LLVM_PROFILE_FILE".into(), v));
    }
    for (k, v) in TEST_ENV
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
    {
        env.push((k.into(), v.into()));
    }
    env
}

/// Reads from the helper's stdout, waiting no later than `deadline`.
struct TimedRead<'a> {
    inner: &'a mut ChildStdout,
    deadline: Instant,
}

impl Read for TimedRead<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !pipe::wait(self.inner.as_fd(), Ready::Read, self.deadline)? {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.inner.read(buf)
    }
}

/// Writes to the helper's (non-blocking) stdin, waiting no later than
/// `deadline` for room.
struct TimedWrite<'a> {
    inner: &'a mut ChildStdin,
    deadline: Instant,
}

impl Write for TimedWrite<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        loop {
            if !pipe::wait(self.inner.as_fd(), Ready::Write, self.deadline)? {
                return Err(io::ErrorKind::TimedOut.into());
            }
            match pipe::write(self.inner.as_fd(), buf) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                other => return other,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// How a request failed, before it becomes a page error.
enum Failure {
    /// The deadline passed (helper killed).
    Timeout,
    /// The helper went away or broke the protocol; the message is final.
    Broken(String),
}

/// One helper process.
#[derive(Debug)]
pub(crate) struct Helper {
    child: Child,
    /// `None` once closed by `Drop` (end of input after `Shutdown`).
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    /// The process that spawned the helper; any other (a forked child of
    /// it) must not signal, wait for or talk to it.
    owner: u32,
    /// Reaped already: nothing left to shut down.
    reaped: bool,
}

impl Helper {
    /// Spawns `exe` with `env` (stderr to the null device).
    fn spawn(exe: &Path, env: Vec<(OsString, OsString)>) -> io::Result<Self> {
        let mut cmd = Command::new(exe);
        cmd.env_clear()
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // `Command` makes only its own pipes close-on-exec; anything else
        // the host left open would reach the helper (O30).
        pipe::cloexec_extra_fds(&mut cmd);
        let mut child = cmd.spawn()?;
        push_bounded(&mut log().spawned, child.id());
        // Both were requested as pipes above, so `spawn` filled both.
        let stdin = child.stdin.take().expect("stdin was set up as a pipe");
        let stdout = child.stdout.take().expect("stdout was set up as a pipe");
        let nonblocking = pipe::set_nonblocking(stdin.as_fd());
        let mut helper = Self {
            child,
            stdin: Some(stdin),
            stdout,
            owner: std::process::id(),
            reaped: false,
        };
        if let Err(e) = nonblocking {
            helper.reap();
            return Err(e);
        }
        Ok(helper)
    }

    /// Kills and reaps the helper; returns how it ended.
    fn reap(&mut self) -> String {
        // Fails only when the process has already exited, which is fine.
        let _ = self.child.kill();
        match self.child.wait() {
            Ok(status) => self.reaped_with(status),
            Err(e) => {
                self.reaped = true;
                format!("unknown status ({e})")
            }
        }
    }

    fn reaped_with(&mut self, status: ExitStatus) -> String {
        self.reaped = true;
        push_bounded(&mut log().reaped, (self.child.id(), status));
        describe(status)
    }

    /// Waits for exit until `until`, without blocking past it.
    fn wait_until(&mut self, until: Instant) -> Option<ExitStatus> {
        let mut pause = Duration::from_millis(1);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) => {}
                Err(_) => return None,
            }
            let now = Instant::now();
            if now >= until {
                return None;
            }
            std::thread::sleep(pause.min(until - now));
            pause = (pause * 2).min(Duration::from_millis(20));
        }
    }

    /// The helper went away (its output closed or the pipe broke). It is
    /// normally a zombie already; it is given until [`EXIT_WAIT`] from now
    /// or `deadline`, whichever is first, before it is killed, so a
    /// failure never outlasts the page budget.
    fn exited(&mut self, deadline: Instant) -> Failure {
        let until = (Instant::now() + EXIT_WAIT).min(deadline);
        if let Some(status) = self.wait_until(until) {
            return Failure::Broken(format!("helper exited: {}", self.reaped_with(status)));
        }
        self.reap();
        Failure::Broken("helper closed its output and was killed".into())
    }

    /// The helper broke the protocol: kill it.
    fn violated(&mut self, what: &str) -> Failure {
        self.reap();
        Failure::Broken(format!("helper: {what}"))
    }

    /// A transport or decoding failure, classified.
    fn failed(&mut self, e: WireError, deadline: Instant) -> Failure {
        match e {
            WireError::Io(e) if e.kind() == io::ErrorKind::TimedOut => {
                self.reap();
                Failure::Timeout
            }
            WireError::Closed | WireError::Truncated | WireError::Io(_) => self.exited(deadline),
            other => self.violated(&other.to_string()),
        }
    }

    fn write(&mut self, bytes: &[&[u8]], deadline: Instant) -> Result<(), Failure> {
        let result = match self.stdin.as_mut() {
            Some(inner) => {
                let mut w = TimedWrite { inner, deadline };
                bytes.iter().try_for_each(|b| w.write_all(b))
            }
            None => Err(io::ErrorKind::BrokenPipe.into()),
        };
        result.map_err(|e| self.failed(WireError::Io(e), deadline))
    }

    fn read(&mut self, deadline: Instant) -> Result<Message, Failure> {
        let result = wire::read_frame(&mut TimedRead {
            inner: &mut self.stdout,
            deadline,
        });
        result.map_err(|e| self.failed(e, deadline))
    }

    /// Sends `Hello` and reads the answer, waiting until `deadline`.
    fn hello(&mut self, hello: &Hello, deadline: Instant) -> Result<HelloOk, (u8, String)> {
        let frame = wire::encode(&Message::Hello(hello.clone()));
        // `step` names where the failure happened, so a detail (and a test)
        // can tell a Hello the helper never read from an answer it never gave.
        let broken = |step: &'static str| {
            move |f: Failure| match f {
                Failure::Timeout => (
                    reason::INIT_FAILED,
                    "OCR helper did not start in time".to_owned(),
                ),
                Failure::Broken(m) => (
                    reason::INIT_FAILED,
                    format!("OCR helper failed to start ({step}): {m}"),
                ),
            }
        };
        self.write(&[&frame], deadline)
            .map_err(broken("sending Hello"))?;
        match self
            .read(deadline)
            .map_err(broken("reading the reply to Hello"))?
        {
            Message::HelloOk(ok) => Ok(ok),
            Message::HelloErr(r, detail) => {
                // It exits after refusing; collect it now.
                let until = (Instant::now() + EXIT_WAIT).min(deadline);
                match self.wait_until(until) {
                    Some(status) => {
                        self.reaped_with(status);
                    }
                    None => {
                        self.reap();
                    }
                }
                Err((r, detail))
            }
            _ => Err(broken("reading the reply to Hello")(
                self.violated("unexpected reply to Hello"),
            )),
        }
    }

    /// Runs one page in the helper (O27).
    fn request(&mut self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        // `OcrConfig::validate` keeps every raster under this; the check
        // stays for engines built around the validation (tests, scripted
        // pools), and so no oversized length ever reaches the pipe.
        let limit = wire::MAX_RASTER_BYTES;
        if img.data.len() > limit {
            return Err(OcrPageError::Recognition(format!(
                "helper: a {}-byte raster does not fit in one frame (limit {limit})",
                img.data.len()
            )));
        }
        let remaining = req.deadline.saturating_duration_since(Instant::now());
        let header = RecognizeHeader {
            width: img.width,
            height: img.height,
            dpi: req.dpi,
            psm: req.psm,
            deadline_ms: u32::try_from(remaining.as_millis()).unwrap_or(u32::MAX),
        };
        let deadline = deadline_after(req.deadline, GRACE);
        let prefix = wire::recognize_prefix(&header, img.data.len());
        let fail = |f: Failure| match f {
            Failure::Timeout => OcrPageError::Timeout(Duration::ZERO),
            Failure::Broken(m) => OcrPageError::Recognition(m),
        };
        self.write(&[&prefix, &img.data], deadline).map_err(fail)?;
        match self.read(deadline).map_err(fail)? {
            Message::Result(out) => Ok(out),
            // The pool drops a helper after any page error; ending it here
            // rather than in a polite `Drop` keeps the page within its
            // budget even when the reply came at the last moment.
            Message::PageErr(kind, detail) => {
                self.reap();
                Err(if kind == page_err::TIMEOUT {
                    OcrPageError::Timeout(Duration::ZERO)
                } else {
                    OcrPageError::Recognition(detail)
                })
            }
            _ => Err(fail(self.violated("unexpected reply to Recognize"))),
        }
    }

    /// Process id (test observability).
    #[cfg(test)]
    pub(crate) fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        // In a forked child the helper is the parent's: closing our copies
        // of the pipe ends (the fields' own drops) is all we may do (O30).
        if self.reaped || std::process::id() != self.owner {
            return;
        }
        let deadline = Instant::now() + SHUTDOWN_WAIT;
        let frame = wire::encode(&Message::Shutdown);
        // A helper that already exited makes this fail; it is reaped below
        // either way. Closing stdin afterwards gives it end of input too.
        if let Some(mut inner) = self.stdin.take() {
            let _ = TimedWrite {
                inner: &mut inner,
                deadline,
            }
            .write_all(&frame);
        }
        match self.wait_until(deadline) {
            Some(status) => {
                self.reaped_with(status);
            }
            None => {
                self.reap();
            }
        }
    }
}

impl Recognizer for Helper {
    fn recognize(&mut self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        self.request(img, req)
    }

    /// The helper clears its instance after every page itself.
    fn clear(&mut self) {}

    /// Dropping is exactly right after a fork: `Drop` sees the foreign
    /// process id and only closes this process's pipe ends.
    fn abandon(self) {
        drop(self);
    }
}

/// Spawns helpers for one engine.
#[derive(Debug)]
pub(crate) struct HelperFactory {
    exe: PathBuf,
    tessdata: PathBuf,
    hello: Hello,
    /// How long a starting helper may take to answer `Hello`.
    start_timeout: Duration,
    languages: Vec<String>,
    info: BackendInfo,
}

impl HelperFactory {
    fn unavailable(&self, reason: UnavailableReason, detail: String) -> OcrUnavailable {
        OcrUnavailable {
            reason,
            tessdata: Some(self.tessdata.clone()),
            languages: self.languages.clone(),
            searched: Vec::new(),
            detail,
        }
    }

    /// Spawns one helper and completes the hand-shake.
    fn start(&self) -> Result<(Helper, HelloOk), OcrUnavailable> {
        let mut helper = Helper::spawn(&self.exe, helper_env(&self.tessdata)).map_err(|e| {
            self.unavailable(
                UnavailableReason::EngineNotBuilt,
                format!("cannot start OCR helper {}: {e}", self.exe.display()),
            )
        })?;
        let deadline = deadline_after(Instant::now(), self.start_timeout);
        let ok = helper.hello(&self.hello, deadline).map_err(|(r, detail)| {
            // A helper refuses with EngineNotBuilt only when it is from
            // another build (a protocol mismatch). `Hello` does not carry
            // our version and that helper's code is not ours to change, so
            // the parent names its own side here.
            let detail = if r == reason::ENGINE_NOT_BUILT {
                format!(
                    "{detail}; this library is pdf-extract {}",
                    pdf_extract_core::VERSION
                )
            } else {
                detail
            };
            self.unavailable(reason_of(r), detail)
        })?;
        if ok.library_version != pdf_extract_core::VERSION {
            return Err(self.unavailable(
                UnavailableReason::EngineNotBuilt,
                format!(
                    "OCR helper {} is from pdf-extract {}, this library is {}",
                    self.exe.display(),
                    ok.library_version,
                    pdf_extract_core::VERSION
                ),
            ));
        }
        Ok((helper, ok))
    }
}

/// A `HelloErr` reason as core's reason.
fn reason_of(r: u8) -> UnavailableReason {
    match r {
        reason::NO_TESSDATA => UnavailableReason::NoTessdata,
        reason::LANGUAGE_DATA_MISSING => UnavailableReason::LanguageDataMissing,
        reason::ENGINE_NOT_BUILT => UnavailableReason::EngineNotBuilt,
        _ => UnavailableReason::InitFailed,
    }
}

impl RecognizerFactory for HelperFactory {
    type Instance = Helper;

    fn create(&self) -> Result<Helper, OcrUnavailable> {
        self.start().map(|(helper, _)| helper)
    }

    fn info(&self) -> BackendInfo {
        self.info.clone()
    }
}

/// O28 then O9 then O29: locates the helper, resolves the model directory
/// here (the helper never searches), spawns the first helper and validates
/// the engine inside it. Returns the factory and that first helper.
///
/// # Errors
/// `EngineNotBuilt` for a missing helper, a failed spawn or a helper from
/// another build; resolution errors as the in-process engine reports them;
/// the helper's own `HelloErr` reason otherwise.
pub(crate) fn build(
    cfg: &OcrConfig,
    candidates: &Candidates,
) -> Result<(HelperFactory, Helper), OcrUnavailable> {
    let exe = discovery::locate(cfg.helper_dir.as_deref())
        .map_err(|detail| cfg.unavailable(UnavailableReason::EngineNotBuilt, detail))?;
    let resolved = tessdata::resolve(cfg.tessdata.as_deref(), candidates, &cfg.languages)?;
    let with_search = |mut u: OcrUnavailable| {
        u.searched.clone_from(&resolved.searched);
        u
    };
    let Some(dir) = resolved.dir.to_str() else {
        return Err(with_search(OcrUnavailable {
            reason: UnavailableReason::InitFailed,
            tessdata: Some(resolved.dir.clone()),
            languages: cfg.languages.clone(),
            searched: Vec::new(),
            detail: "process isolation needs a UTF-8 model directory path".into(),
        }));
    };
    let mut factory = HelperFactory {
        exe,
        tessdata: resolved.dir.clone(),
        hello: Hello {
            proto_version: wire::PROTO_VERSION,
            oem: cfg.oem,
            psm: cfg.psm,
            languages: cfg.joined_languages(),
            tessdata: dir.to_owned(),
            debug_null: NULL_DEVICE.to_owned(),
        },
        start_timeout: cfg.page_timeout.max(super::helper::WARMUP_BUDGET) + GRACE,
        languages: cfg.languages.clone(),
        info: BackendInfo {
            name: "tesseract",
            version: String::new(),
            languages: cfg.languages.clone(),
            tessdata: Some(resolved.dir.clone()),
            model_sha256: Vec::new(),
            linkage: match pdf_extract_tesseract_sys::LINKAGE {
                pdf_extract_tesseract_sys::Linkage::Vendored => Linkage::Vendored,
                pdf_extract_tesseract_sys::Linkage::System => Linkage::System,
            },
            isolation: Isolation::Process,
        },
    };
    let (first, ok) = factory.start().map_err(with_search)?;
    factory.info.version = ok.tesseract_version;
    factory.info.model_sha256 = ok.model_sha256;
    Ok((factory, first))
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
