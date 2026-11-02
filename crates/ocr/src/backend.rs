//! The backend seam (spec 0002 O33, §7.5): what an engine hands a raster to
//! and what it gets back.
//!
//! Two layers, because two kinds of backend exist:
//!
//! - [`OcrBackend`] is the public, object-safe trait an engine calls once per
//!   page. A backend given to `OcrEngine::with_backend` is called directly
//!   and owns its concurrency.
//! - [`RecognizerFactory`] / [`Recognizer`] describe a backend made of
//!   single-threaded *instances* (a Tesseract handle, a scripted stand-in),
//!   which the crate's pool (`pool.rs`) checks out one per page, clears and
//!   returns, or drops after a failure. `TesseractBackend` is such a pooled
//!   backend; the test kit's `ScriptedBackend` is both kinds, so the pool's
//!   bound, drop-on-error and clear-per-page rules are tested without
//!   Tesseract.

use std::path::PathBuf;
use std::time::Instant;

use pdf_extract_core::{OcrPageError, OcrUnavailable};
use pdf_extract_render::GrayImage;

use crate::config::{Isolation, PageSegMode};

/// What a backend is asked to do with one raster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OcrRequest {
    /// The raster's resolution (the `dpi'` it was rendered at).
    pub dpi: u32,
    /// When the page's budget runs out; measured from the start of
    /// rendering, so rendering time counts (O16).
    pub deadline: Instant,
    /// The page segmentation mode.
    pub psm: PageSegMode,
}

/// One recognised word, in raster pixels (origin top left, y down).
#[derive(Clone, Debug, PartialEq)]
pub struct OcrWord {
    /// The word's text (UTF-8; sanitised later, when the page is built).
    pub text: String,
    /// `[left, top, right, bottom]` in pixels.
    pub bbox_px: [u32; 4],
    /// The engine's confidence, 0–100 (not exposed on `TextPage`).
    pub confidence: f32,
}

/// One text line: its words in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OcrLine {
    /// The words.
    pub words: Vec<OcrWord>,
}

/// One layout block: its lines in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OcrBlock {
    /// The lines.
    pub lines: Vec<OcrLine>,
}

/// A backend's result for one raster. No blocks means nothing was found,
/// which is a success (an empty page), not an error.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OcrOutput {
    /// The blocks, in reading order.
    pub blocks: Vec<OcrBlock>,
}

/// How the backend's native library was obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linkage {
    /// Built from the repository's vendored sources.
    Vendored,
    /// The system's library, found by pkg-config.
    System,
}

/// What an engine runs (spec O13, design spec §14.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendInfo {
    /// The backend's name (`"tesseract"` for the production backend).
    pub name: &'static str,
    /// The backend library's version (Tesseract's, e.g. `"5.5.3"`).
    pub version: String,
    /// The languages loaded.
    pub languages: Vec<String>,
    /// The model directory, when the backend has one.
    pub tessdata: Option<PathBuf>,
    /// SHA-256 of each language's model file, in `languages` order,
    /// computed once when the engine was built.
    pub model_sha256: Vec<(String, [u8; 32])>,
    /// How the library was linked.
    pub linkage: Linkage,
    /// How recognition is isolated.
    pub isolation: Isolation,
}

/// A recognition backend an engine calls directly (O33).
///
/// Implementations must be safe to call from several threads at once.
/// A panic propagates to the engine's caller (and becomes `InternalError`
/// at the Python boundary).
pub trait OcrBackend: Send + Sync {
    /// Recognises one gray raster.
    ///
    /// # Errors
    /// [`OcrPageError::Recognition`] when the engine failed on this raster,
    /// [`OcrPageError::Timeout`] when `req.deadline` passed (the engine
    /// reports its own `page_timeout` whatever duration this carries).
    fn recognize(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError>;

    /// What this backend runs.
    fn info(&self) -> BackendInfo;
}

/// One single-threaded recognition instance kept in a pool.
#[doc(hidden)]
pub trait Recognizer: Send {
    /// Recognises one raster. After an `Err` (or a panic) the instance is
    /// dropped, never reused.
    ///
    /// # Errors
    /// As [`OcrBackend::recognize`].
    fn recognize(&mut self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError>;

    /// Forgets the last page (image and results), keeping models loaded;
    /// called before the instance goes back to the pool.
    fn clear(&mut self);

    /// Disposes of an instance that belongs to the process this one was
    /// forked from (O11, O30). The default leaks it: another thread of the
    /// parent may have been inside it at the fork, so freeing it is unsafe.
    /// A helper process overrides this to close its pipe ends only.
    fn abandon(self)
    where
        Self: Sized,
    {
        std::mem::forget(self);
    }
}

/// Makes pool instances.
#[doc(hidden)]
pub trait RecognizerFactory: Send + Sync + 'static {
    /// The instance type.
    type Instance: Recognizer;

    /// Creates (and initialises) one instance.
    ///
    /// # Errors
    /// Why no instance could be made; the page that needed it fails with
    /// this as `OcrError::Unavailable`.
    fn create(&self) -> Result<Self::Instance, OcrUnavailable>;

    /// What the instances run.
    fn info(&self) -> BackendInfo;
}
