//! The panic boundary (spec T45, §2.2 "Python boundary", D-16).
//!
//! The Rust core lets panics propagate (D-16); this module is where they
//! stop. Every exported function and method body runs inside [`guarded`],
//! which catches an unwinding panic and raises `InternalError(message)`.
//! Without it PyO3 would raise `pyo3.PanicException`, which derives from
//! `BaseException`: `except Exception` would not catch it and a worker thread
//! in the caller would die.
//!
//! **Silence (T48).** Rust's default panic hook prints the panic message to
//! stderr before unwinding starts, which would break the "nothing on fd 1 or
//! 2" guarantee. [`install_panic_hook`] replaces it with a hook that records
//! the message and location in a thread-local instead; [`guarded`] puts that
//! text into the `InternalError`, so nothing is lost. The hook is
//! process-global only for *this* shared library: each Rust cdylib carries
//! its own copy of `std` and therefore its own hook, so other Rust extensions
//! in the same interpreter are unaffected.
//!
//! **Codec panics (H18).** The fork's image-codec boundary catches a codec
//! panic itself and turns it into an `image: decode failed` warning; the
//! hook still runs for it (silently) and records its message, which no
//! `guarded` unwind then takes. [`guarded`] therefore clears the slot on
//! entry, and uses a recorded message only when it belongs to the panic that
//! is unwinding (it starts with that panic's payload text). The second check
//! covers a panic re-raised with `resume_unwind`, which skips the hook: core's
//! `run_parallel` re-raises a worker thread's panic on the calling thread, so
//! the calling thread's slot may still hold a codec panic caught earlier in
//! the same call. An `InternalError` thus never carries a caught codec
//! panic's message.
//!
//! **Requirements.** Catching only works with `panic = "unwind"`, which the
//! workspace sets in every profile; under `abort` a panic would end the
//! process before reaching this code.

use std::any::Any;
use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Once;

use pyo3::{PyResult, Python};

use crate::errors::internal_error;

thread_local! {
    /// The formatted message of the most recent panic on this thread, set by
    /// the hook and taken by [`guarded`] while the same panic unwinds.
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Replaces the panic hook with one that records instead of printing.
///
/// Called from module initialisation; idempotent (a module can be
/// initialised more than once, e.g. in sub-interpreters or after reload).
pub(crate) fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        panic::set_hook(Box::new(|info| {
            let message = payload_text(info.payload());
            let text = match info.location() {
                Some(loc) => format!("{message} (at {}:{})", loc.file(), loc.line()),
                None => message,
            };
            // `try_with`: during thread teardown the slot may be gone; the
            // payload text is then still available to `guarded`.
            let _ = LAST_PANIC.try_with(|slot| *slot.borrow_mut() = Some(text));
        }));
    });
}

/// Runs `body`, turning a panic into `InternalError`.
///
/// `AssertUnwindSafe` is sound here: on a panic the only state `body` could
/// have left inconsistent is local to the failed call or lives in
/// `pdf-extract-core`, whose shared state is an `Option<Arc<_>>` replaced in
/// one assignment (lock poisoning is ignored there for that reason). Nothing
/// observed after the panic depends on `body`'s partial work.
pub(crate) fn guarded<T>(body: impl FnOnce() -> PyResult<T>) -> PyResult<T> {
    // A codec panic caught inside an earlier call left its message here.
    let _ = LAST_PANIC.try_with(|slot| slot.borrow_mut().take());
    match panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(result) => result,
        Err(payload) => {
            let text = payload_text(&*payload);
            let recorded = LAST_PANIC
                .try_with(|slot| slot.borrow_mut().take())
                .ok()
                .flatten()
                .filter(|r| r.starts_with(&text));
            let detail = recorded.unwrap_or(text);
            // Every guarded body runs attached (a panic inside `py.detach`
            // re-attaches while unwinding), so this `attach` is a no-op.
            Err(Python::attach(|py| {
                internal_error(
                    py,
                    format!(
                        "internal error in pdf-extract (this is a bug; please report it): {detail}"
                    ),
                )
            }))
        }
    }
}

/// The text of a panic payload: `panic!` produces a `&str` or a `String`;
/// anything else (`panic_any`) has no printable form.
fn payload_text(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic with a non-string payload".to_owned()
    }
}
