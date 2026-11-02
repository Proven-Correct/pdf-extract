//! OCR for pdf-extract (run 0002 spec §1.2, §1.3, §1.6, §3.3
//! `pdf-extract-ocr`).
//!
//! An [`OcrEngine`] renders a page to gray with `pdf-extract-render` and
//! recognises it with a backend — in production [`TesseractBackend`],
//! a pool of in-process Tesseract instances — returning a `TextPage` whose
//! source is `TextSource::Ocr` and whose shape matches the text path's.
//!
//! - `config`: [`OcrConfig`] and its validation (O8).
//! - `tessdata`: the model directory search (O9), injectable for tests.
//! - `backend`: the [`OcrBackend`] seam and the pooled-instance traits.
//! - `pool`: the bounded, lazily filled, fork-aware instance pool (O10, O11),
//!   built on `sync`, the `std`/loom shim (`--cfg loom` builds re-export
//!   `Pool` for tests/loom_pool.rs).
//! - `tesseract`: the production backend and O7's up-front validation.
//! - `engine`: [`OcrEngine`], [`PageOcr`] and [`default_ocr_engine`].
//! - [`precheck`]: the noise pre-check that keeps speckle-dominated rasters
//!   away from Tesseract's layout analysis (run 0003 F11–F13).
//!
//! The crate has no `unsafe` code: every native call goes through
//! `pdf-extract-tesseract-sys`, whose `Instance` never leaves this crate.
//! Only the configuration's dedicated fields reach Tesseract; no Tesseract
//! variable is exposed, so the process-wide parameters the sys crate sets
//! once (O14) cannot be changed from here.
//!
//! # Forking
//!
//! A `fork()` copies only the calling thread. If another thread held a lock
//! at that instant, the child inherits it locked, with nobody to release
//! it, and the child deadlocks the first time it needs it. The locks this
//! crate and its sys crate take are: the `OnceLock` of the first
//! [`default_ocr_engine`] construction and the one of Tesseract's
//! process-wide initialisation (`pdf-extract-tesseract-sys`'s `GLOBAL_INIT`,
//! held for tens to hundreds of milliseconds while a model loads), the
//! engine's pool mutex, the helper log and test-environment mutexes of the
//! process-isolation client (`LOG`, `TEST_ENV`), and the tessdata
//! candidates lock (`OVERRIDE`). The Python binding adds one more: the
//! `OnceLock` of its own default engine (process-isolated, internal #53), held while
//! the first `pdf_extract.default_ocr_engine()` or engine-less `Page.ocr()`
//! builds it. No `atfork` handler is installed (see
//! `THEORY.md`); the hazard is documented, not defended. Three rules keep a
//! program clear of it:
//!
//! 1. **Build every engine before forking**, the default one included
//!    (call [`default_ocr_engine`] or [`OcrEngine::tesseract`] once). An
//!    engine built before the fork is reused by the child, which rebuilds
//!    its pool on first use (the pool notices the changed process id; its
//!    in-process instances are leaked and helper pipes are closed without
//!    waiting).
//! 2. **Do not fork while another thread may be inside** `Page::ocr`,
//!    [`OcrEngine::tesseract`], [`default_ocr_engine`] or
//!    `Document::extract` with `ocr` set.
//! 3. **In a threaded process, prefer `posix_spawn` or fork-then-`exec`**
//!    (or a start method that execs, such as `spawn`/`forkserver` in
//!    Python) to a bare `fork()`.
//!
//! A fork from a process that has not yet used OCR, or from a
//! single-threaded one, is safe.

#![forbid(unsafe_code)]

mod backend;
mod config;
mod engine;
#[doc(hidden)]
pub mod isolation;
mod pool;
pub mod precheck;
mod sync;
mod tessdata;
mod tesseract;

pub use backend::{
    BackendInfo, Linkage, OcrBackend, OcrBlock, OcrLine, OcrOutput, OcrRequest, OcrWord,
};
#[doc(hidden)]
pub use backend::{Recognizer, RecognizerFactory};
pub use config::{
    EngineMode, Isolation, MAX_DPI, MAX_PROCESS_RENDER_PIXELS, MIN_DPI, OcrConfig, PageSegMode,
};
pub use engine::{OcrEngine, OcrOptions, PageOcr, default_ocr_engine};
#[doc(hidden)]
pub use pool::fork_resets;
// The instance pool, reachable only from the loom build so that
// tests/loom_pool.rs model-checks the shipped code (design D5).
#[cfg(loom)]
#[doc(hidden)]
pub use pool::Pool;
#[doc(hidden)]
pub use tessdata::{Candidates, KNOWN_DIRS, resolutions, set_candidates_override};
pub use tesseract::TesseractBackend;

// The raster type of `OcrBackend::recognize`, so a backend can be written
// against this crate alone.
pub use pdf_extract_render::GrayImage;

static_assertions::assert_impl_all!(OcrEngine: Send, Sync);
static_assertions::assert_impl_all!(OcrConfig: Send, Sync);
static_assertions::assert_impl_all!(GrayImage: Send, Sync);
// Holds no `Rc` (or anything else thread-bound): shared across threads.
static_assertions::assert_impl_all!(TesseractBackend: Send, Sync);
