//! Deadline-aware pipe I/O for `pdf-extract-ocr`'s process isolation
//! (spec 0002 O27, O32, §3.5).
//!
//! The parent talks to a helper process over the helper's stdin/stdout
//! pipes on the calling thread — no thread is created — so a deadline can
//! only be enforced by waiting for readiness with `poll(2)` before each read
//! or write. The write end is switched to non-blocking so that a helper
//! that stops reading cannot block a large image write past the deadline:
//! a short write returns `WouldBlock` and the caller polls again.
//!
//! Two more protections belong to the same spawn-and-write path: helpers
//! must inherit no descriptor beyond 0–2 ([`cloexec_extra_fds`]), and a
//! write to a dead helper must not raise `SIGPIPE` in the caller
//! ([`write()`]).
//!
//! These are the only system calls the OCR crate needs that `std` does not
//! wrap; they live here because `pdf-extract-ocr` forbids `unsafe`.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::time::{Duration, Instant};

/// What to wait for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ready {
    /// Data to read, or end of file / hang-up (a read then returns 0).
    Read,
    /// Room to write, or an error / hang-up (a write then fails).
    Write,
}

/// Waits until `fd` is ready for `what` or `deadline` passes.
///
/// Returns `Ok(true)` when ready (including hang-up and error conditions,
/// which the following read or write reports), `Ok(false)` when the
/// deadline passed first. `EINTR` is retried with the remaining time.
///
/// # Errors
/// Any other `poll(2)` failure.
pub fn wait(fd: BorrowedFd<'_>, what: Ready, deadline: Instant) -> io::Result<bool> {
    let events = match what {
        Ready::Read => libc::POLLIN,
        Ready::Write => libc::POLLOUT,
    };
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let mut pfd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events,
            revents: 0,
        };
        // SAFETY: `pfd` is one valid, initialised pollfd that outlives the
        // call, and nfds is 1; `fd` is borrowed, so it stays open meanwhile.
        let n = unsafe { libc::poll(&raw mut pfd, 1, timeout_ms(remaining)) };
        match n {
            0 if remaining.is_zero() || Instant::now() >= deadline => return Ok(false),
            // Woke early (poll rounds to whole milliseconds): wait the rest.
            0 => {}
            n if n > 0 => return Ok(true),
            _ => {
                let err = io::Error::last_os_error();
                if err.kind() != io::ErrorKind::Interrupted {
                    return Err(err);
                }
            }
        }
    }
}

/// `remaining` as poll's millisecond timeout, rounded up so a sub-millisecond
/// remainder still waits, and clamped to `c_int`.
fn timeout_ms(remaining: Duration) -> libc::c_int {
    let ms = remaining.as_nanos().div_ceil(1_000_000);
    libc::c_int::try_from(ms).unwrap_or(libc::c_int::MAX)
}

/// `<sys/fcntl.h>`'s `F_SETNOSIGPIPE` (Darwin), which the `libc` crate
/// does not export.
#[cfg(target_vendor = "apple")]
const F_SETNOSIGPIPE: libc::c_int = 73;

/// Prepares the parent's end of a helper's stdin: non-blocking, and (on
/// macOS) never raising `SIGPIPE` when the helper has gone.
///
/// Only the parent's end is changed: the child's end is a separate open
/// file description and keeps blocking reads.
///
/// # Errors
/// The `fcntl(2)` failure.
pub fn set_nonblocking(fd: BorrowedFd<'_>) -> io::Result<()> {
    let raw = fd.as_raw_fd();
    // SAFETY: F_GETFL on a borrowed (open) descriptor reads its flags; no
    // memory is passed.
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above; F_SETFL takes an int argument.
    if unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    #[cfg(target_vendor = "apple")]
    // SAFETY: F_SETNOSIGPIPE on a borrowed descriptor takes an int; a write
    // to a pipe without readers then fails with EPIPE and raises nothing.
    if unsafe { libc::fcntl(raw, F_SETNOSIGPIPE, 1) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Writes to a pipe without letting `SIGPIPE` reach the process.
///
/// A helper that died leaves its stdin without a reader; writing to it
/// raises `SIGPIPE`, whose default action would end the *caller* — the
/// opposite of what process isolation promises, in any host that has not
/// ignored the signal (a C program, an embedded interpreter). macOS has a
/// per-descriptor switch, set by [`set_nonblocking`]. Elsewhere the signal
/// is blocked for this thread around the write and, if the write raised
/// it (`EPIPE`) and it was not already pending, consumed before the mask
/// is restored, so no disposition the host chose is ever changed.
///
/// # Errors
/// The `write(2)` failure (`WouldBlock`, `BrokenPipe`, …).
pub fn write(fd: BorrowedFd<'_>, buf: &[u8]) -> io::Result<usize> {
    #[cfg(target_vendor = "apple")]
    {
        raw_write(fd, buf)
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        write_masked(fd, buf)
    }
}

fn raw_write(fd: BorrowedFd<'_>, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: `buf` is a live slice of `buf.len()` bytes; `fd` is borrowed
    // and therefore open for the duration of the call.
    let n = unsafe { libc::write(fd.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
    usize::try_from(n).map_err(|_| io::Error::last_os_error())
}

#[cfg(not(target_vendor = "apple"))]
fn write_masked(fd: BorrowedFd<'_>, buf: &[u8]) -> io::Result<usize> {
    use std::mem::MaybeUninit;
    let mut pipe_set = MaybeUninit::<libc::sigset_t>::uninit();
    let mut old = MaybeUninit::<libc::sigset_t>::uninit();
    let mut pending = MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: every sigset_t is initialised by sigemptyset/pthread_sigmask/
    // sigpending before it is read; the pointers are to live locals.
    unsafe {
        libc::sigemptyset(pipe_set.as_mut_ptr());
        libc::sigaddset(pipe_set.as_mut_ptr(), libc::SIGPIPE);
        libc::sigpending(pending.as_mut_ptr());
    }
    // SAFETY: as above; `pending` was filled by sigpending.
    let was_pending = unsafe { libc::sigismember(pending.as_ptr(), libc::SIGPIPE) } == 1;
    // SAFETY: blocks SIGPIPE for this thread only, saving the old mask.
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, pipe_set.as_ptr(), old.as_mut_ptr()) };
    let result = raw_write(fd, buf);
    if !was_pending && matches!(&result, Err(e) if e.raw_os_error() == Some(libc::EPIPE)) {
        let zero = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: consumes the SIGPIPE this write raised (blocked, so
        // pending); a null info pointer is allowed; `zero` is live.
        unsafe { libc::sigtimedwait(pipe_set.as_ptr(), std::ptr::null_mut(), &raw const zero) };
    }
    // SAFETY: restores the mask saved above.
    unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, old.as_ptr(), std::ptr::null_mut()) };
    result
}

/// Makes every descriptor above 2 close-on-exec in the child `cmd` will
/// create, so the program it runs inherits only stdin, stdout and stderr —
/// whatever the host left open without `O_CLOEXEC` (a shell's or a
/// supervisor's descriptors, a C library's files). `Command` itself only
/// guarantees that *its own* pipes are close-on-exec.
///
/// The descriptors are marked rather than closed so that `Command`'s own
/// exec-error pipe keeps working (a failed exec is still reported as a
/// spawn error).
pub fn cloexec_extra_fds(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // The bound is computed here, in the parent, so the child between fork
    // and exec makes no sysconf/getrlimit call (neither is on POSIX's
    // async-signal-safe list); the descriptor limits are the same on both
    // sides of the fork.
    let bound = current_sweep_bound(FIRST_EXTRA_FD);
    // SAFETY: the hook runs in the forked child before exec and calls only
    // async-signal-safe functions (close_range/syscall, fcntl); it
    // allocates nothing and touches no lock.
    unsafe {
        cmd.pre_exec(move || {
            mark_cloexec_from(FIRST_EXTRA_FD, bound);
            Ok(())
        });
    }
}

/// The first descriptor a helper must not inherit (after stdin, stdout,
/// stderr).
const FIRST_EXTRA_FD: libc::c_int = 3;

/// The most descriptors the fallback sweep visits: `fcntl` on every number
/// up to a limit of 2^31 would stall a spawn for minutes, and a process
/// with a million open descriptors is not one this library serves.
/// Descriptors numbered at or above it are not marked (spec 0003 §2.2).
pub const MAX_SWEEP: libc::c_int = 1 << 20;

/// The exclusive upper bound of the descriptor sweep that starts at
/// `first`, from `sysconf(_SC_OPEN_MAX)` and the soft `RLIMIT_NOFILE`
/// (`None` when `getrlimit` failed). A `sysconf` answer that is negative
/// (−1: indeterminate or failed), not above `first`, or above
/// [`MAX_SWEEP`] is not trusted; the soft limit then decides, clamped to
/// `first + 1 ..= MAX_SWEEP`, and a missing or infinite limit means
/// `MAX_SWEEP`. For any `first` below `MAX_SWEEP` the range `first..bound`
/// is never empty, so an odd `sysconf` can no longer turn the sweep into a
/// no-op (spec 0003 F21). A plausible `sysconf` is trusted as it is: a
/// host that lowered its soft limit below a descriptor it still holds
/// leaves that descriptor unmarked on systems without `close_range`.
///
/// Edge: when `first` is already at or above [`MAX_SWEEP`] the bound is
/// `first + 1` (saturating at `c_int::MAX`), so the sweep visits exactly
/// `first` and exceeds the cap by that one descriptor rather than being
/// empty. Pinned by `the_sweep_bound_is_never_empty`; unreachable today
/// because the only caller passes `FIRST_EXTRA_FD`.
pub fn sweep_bound(sysconf: i64, rlimit: Option<u64>, first: libc::c_int) -> libc::c_int {
    let floor = i64::from(first) + 1;
    let ceiling = i64::from(MAX_SWEEP);
    if sysconf >= floor && sysconf <= ceiling {
        return libc::c_int::try_from(sysconf).unwrap_or(MAX_SWEEP);
    }
    let limit = rlimit.map_or(ceiling, |r| i64::try_from(r).unwrap_or(ceiling));
    // Not `clamp`: it panics when floor > ceiling (a `first` at or above
    // MAX_SWEEP), and this runs on the way to a fork.
    let bound = limit.min(ceiling).max(floor);
    libc::c_int::try_from(bound).unwrap_or(libc::c_int::MAX)
}

/// [`sweep_bound`] over this process's own limits.
// `c_long` and `rlim_t` are 64-bit on the targets built today, which makes
// the widening conversions below identities there, but not everywhere.
#[allow(clippy::useless_conversion)]
fn current_sweep_bound(first: libc::c_int) -> libc::c_int {
    // SAFETY: sysconf takes an integer name and reads no memory.
    let open_max = i64::from(unsafe { libc::sysconf(libc::_SC_OPEN_MAX) });
    let mut rl = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes one rlimit through a pointer to a live local.
    let ok = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut rl) } == 0;
    let soft = (ok && rl.rlim_cur != libc::RLIM_INFINITY).then(|| u64::from(rl.rlim_cur));
    sweep_bound(open_max, soft, first)
}

/// Sets `FD_CLOEXEC` on every open descriptor `>= first` (below `bound`
/// when the sweep is needed).
fn mark_cloexec_from(first: libc::c_int, bound: libc::c_int) {
    #[cfg(target_os = "linux")]
    {
        const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
        // SAFETY: close_range takes three integers; with CLOEXEC it only
        // sets flags. Kernels before 5.11 refuse it, and the loop below
        // does the same work.
        let r = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                first as libc::c_uint,
                libc::c_uint::MAX,
                CLOSE_RANGE_CLOEXEC,
            )
        };
        if r == 0 {
            return;
        }
    }
    sweep_cloexec(first, bound);
}

/// The portable fallback of [`mark_cloexec_from`], separate so a test can
/// run it over a bounded range: `close_range` has no upper bound a test
/// could pass, and marking every descriptor above the test's own would
/// leak into other tests of the same process.
fn sweep_cloexec(first: libc::c_int, bound: libc::c_int) {
    for fd in first..bound {
        // SAFETY: F_GETFD/F_SETFD on a number that may or may not be open:
        // an unused number fails with EBADF and nothing else happens.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

/// The conventional name of a signal number (`SIGABRT` for 6), for messages
/// such as `helper exited: signal 6 (SIGABRT)`; `None` for numbers outside
/// the common set.
pub fn signal_name(signal: i32) -> Option<&'static str> {
    const NAMES: [(libc::c_int, &str); 12] = [
        (libc::SIGHUP, "SIGHUP"),
        (libc::SIGINT, "SIGINT"),
        (libc::SIGQUIT, "SIGQUIT"),
        (libc::SIGILL, "SIGILL"),
        (libc::SIGTRAP, "SIGTRAP"),
        (libc::SIGABRT, "SIGABRT"),
        (libc::SIGBUS, "SIGBUS"),
        (libc::SIGFPE, "SIGFPE"),
        (libc::SIGKILL, "SIGKILL"),
        (libc::SIGSEGV, "SIGSEGV"),
        (libc::SIGPIPE, "SIGPIPE"),
        (libc::SIGTERM, "SIGTERM"),
    ];
    NAMES
        .iter()
        .find(|(n, _)| *n == signal)
        .map(|(_, name)| *name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::AsFd;

    #[test]
    fn a_pipe_with_nothing_to_read_times_out_and_then_becomes_ready() {
        let (mut r, mut w) = io::pipe().expect("pipe");
        let start = Instant::now();
        let ready = wait(r.as_fd(), Ready::Read, start + Duration::from_millis(30));
        assert!(!ready.expect("poll"));
        assert!(start.elapsed() >= Duration::from_millis(30));
        w.write_all(b"x").expect("write");
        assert!(wait(r.as_fd(), Ready::Read, Instant::now()).expect("poll"));
        let mut b = [0u8; 1];
        r.read_exact(&mut b).expect("read");
        // A closed writer reads as ready (EOF).
        drop(w);
        assert!(
            wait(
                r.as_fd(),
                Ready::Read,
                Instant::now() + Duration::from_secs(5)
            )
            .expect("poll")
        );
    }

    #[test]
    fn a_full_nonblocking_pipe_would_block_and_is_not_writable() {
        let (_r, mut w) = io::pipe().expect("pipe");
        set_nonblocking(w.as_fd()).expect("fcntl");
        let chunk = [0u8; 4096];
        let err = loop {
            if let Err(e) = w.write(&chunk) {
                break e;
            }
        };
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        let ready = wait(
            w.as_fd(),
            Ready::Write,
            Instant::now() + Duration::from_millis(10),
        );
        assert!(!ready.expect("poll"));
    }

    #[test]
    fn a_descriptor_that_is_not_open_is_an_error() {
        // A number far above any descriptor this process opens (a closed
        // real one could be reused by a parallel test at once).
        // SAFETY: (test) nothing is accessed through the number; fcntl and
        // poll only report EBADF / POLLNVAL for it.
        let fd = unsafe { BorrowedFd::borrow_raw(1_000_000) };
        assert!(set_nonblocking(fd).is_err());
        // POLLNVAL counts as ready: the following I/O reports the error.
        assert!(wait(fd, Ready::Read, Instant::now()).expect("poll"));
    }

    #[test]
    fn writing_to_a_pipe_without_readers_is_epipe_not_a_signal() {
        // With SIGPIPE at its default action the process would die here.
        // SAFETY: (test) swaps the disposition of SIGPIPE and restores it.
        let prev = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
        let (r, w) = io::pipe().expect("pipe");
        set_nonblocking(w.as_fd()).expect("fcntl");
        drop(r);
        let err = write(w.as_fd(), b"x").expect_err("no reader");
        // SAFETY: (test) restores the previous disposition.
        unsafe { libc::signal(libc::SIGPIPE, prev) };
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        let (_r2, w2) = io::pipe().expect("pipe");
        assert_eq!(write(w2.as_fd(), b"ok").expect("write"), 2);
    }

    #[test]
    fn the_sweep_bound_is_never_empty() {
        // sysconf indeterminate: the soft limit decides, or MAX_SWEEP.
        assert_eq!(sweep_bound(-1, Some(256), 3), 256);
        assert_eq!(sweep_bound(-1, None, 3), MAX_SWEEP);
        // sysconf not above `first`.
        assert_eq!(sweep_bound(2, Some(1024), 3), 1024);
        assert_eq!(sweep_bound(3, Some(1024), 3), 1024);
        // Absurd values on both sides clamp to MAX_SWEEP.
        assert_eq!(sweep_bound(1 << 40, Some(1 << 40), 3), MAX_SWEEP);
        assert_eq!(sweep_bound(-1, Some(u64::MAX), 3), MAX_SWEEP);
        // A plausible sysconf is used as it is, whatever the limit says.
        for rlimit in [None, Some(0), Some(1 << 40)] {
            assert_eq!(sweep_bound(1024, rlimit, 3), 1024);
        }
        assert_eq!(sweep_bound(i64::from(MAX_SWEEP), None, 3), MAX_SWEEP);
        assert_eq!(sweep_bound(4, None, 3), 4);
        // A limit at or below `first` still leaves one number to visit.
        assert_eq!(sweep_bound(-1, Some(0), 3), 4);
        assert_eq!(sweep_bound(0, Some(3), 3), 4);
        // A `first` past MAX_SWEEP does not panic (no `clamp`).
        assert_eq!(sweep_bound(-1, None, MAX_SWEEP), MAX_SWEEP + 1);
        assert_eq!(sweep_bound(-1, None, libc::c_int::MAX), libc::c_int::MAX);
        // This process's own limits give a non-empty sweep.
        assert!(current_sweep_bound(FIRST_EXTRA_FD) > FIRST_EXTRA_FD);
    }

    #[test]
    fn marking_sets_close_on_exec_on_an_inheritable_descriptor() {
        let file = std::fs::File::open("/dev/null").expect("open");
        let fd = file.as_raw_fd();
        // SAFETY: (test) F_GETFD/F_SETFD on a descriptor this test owns.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        let inheritable = flags & !libc::FD_CLOEXEC;
        // SAFETY: as above.
        let set = unsafe { libc::fcntl(fd, libc::F_SETFD, inheritable) };
        assert_eq!(set, 0);
        // Only the fallback sweep, over this descriptor alone: the
        // close_range path marks everything above `fd` in the shared test
        // process, and is covered by the helper-level test instead
        // (crates/ocr/tests/process.rs).
        sweep_cloexec(fd, fd + 1);
        // SAFETY: as above.
        let after = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_ne!(after & libc::FD_CLOEXEC, 0);
    }

    #[test]
    fn rounding_and_names() {
        assert_eq!(timeout_ms(Duration::ZERO), 0);
        assert_eq!(timeout_ms(Duration::from_micros(1)), 1);
        assert_eq!(timeout_ms(Duration::from_secs(u64::MAX)), libc::c_int::MAX);
        assert_eq!(signal_name(libc::SIGABRT), Some("SIGABRT"));
        assert_eq!(signal_name(libc::SIGKILL), Some("SIGKILL"));
        assert_eq!(signal_name(0), None);
    }
}
