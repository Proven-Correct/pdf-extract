//! Process isolation (spec 0002 §1.5, §3.5): each pool slot is a
//! `pdf-extract-ocr-helper` child process running one Tesseract instance,
//! so an `abort()` inside Tesseract fails one page instead of ending the
//! caller's process.
//!
//! - `wire`: the frame format both sides compile.
//! - `discovery`: where the helper binary is looked for (O28, D-16).
//! - `client`: the parent side — spawning, the hand-shake, deadlines,
//!   killing and reaping, `Drop` and fork handling (O27, O29, O30, O32).
//! - `helper`: the child side, the whole of the helper binary's `main`.
//!
//! Process isolation is Unix-only (it relies on `poll(2)` and signals);
//! elsewhere asking for it is `EngineNotBuilt`.

pub(crate) mod wire;

#[cfg(unix)]
pub mod client;
#[cfg(unix)]
pub(crate) mod discovery;
#[cfg(unix)]
pub mod helper;
