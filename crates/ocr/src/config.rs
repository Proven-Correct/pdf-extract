//! Engine configuration (spec 0002 O8) and its validation.
//!
//! [`OcrConfig`] is a plain value with public fields so callers can use
//! struct-update syntax; its invariants are therefore checked when an engine
//! is built ([`OcrConfig::validate`]), not when the value is made. Every
//! check runs before the filesystem is touched, so a language name such as
//! `../eng` is refused before any `stat` (O8: a path separator or `..` must
//! never reach the filesystem).

use std::path::PathBuf;
use std::time::Duration;

use pdf_extract_core::{OcrUnavailable, UnavailableReason};
use pdf_extract_tesseract_sys as sys;

use crate::isolation::wire;

/// Lowest accepted resolution (O8).
pub const MIN_DPI: u32 = 1;
/// Highest accepted resolution (O8).
pub const MAX_DPI: u32 = 2400;

/// Highest `max_render_pixels` accepted with [`Isolation::Process`]
/// (66,977,776): every raster must fit one helper frame (64 MiB less the
/// `Recognize` header fields, at one byte per gray pixel), and a
/// raster may exceed `max_render_pixels` by
/// [`RASTER_PIXEL_SLACK`](pdf_extract_render::RASTER_PIXEL_SLACK) through
/// rounding. Refusing the configuration up front keeps the two isolation
/// modes' results identical (O26) instead of failing only the pages whose
/// raster happens to be large.
pub const MAX_PROCESS_RENDER_PIXELS: u64 =
    wire::MAX_RASTER_BYTES as u64 - pdf_extract_render::RASTER_PIXEL_SLACK;

/// How recognition is isolated from the calling process (O26).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isolation {
    /// Tesseract runs in this process: fastest, but an `abort()` inside
    /// Tesseract ends the process.
    InProcess,
    /// Each pool slot is a helper process, so a crash fails one page only.
    Process,
}

/// Tesseract's page segmentation modes (`--psm`), in Tesseract's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSegMode {
    /// Orientation and script detection only.
    OsdOnly,
    /// Automatic segmentation with orientation and script detection.
    AutoOsd,
    /// Automatic segmentation, no OSD and no OCR.
    AutoOnly,
    /// Fully automatic segmentation without OSD (the default).
    Auto,
    /// One column of text of variable sizes.
    SingleColumn,
    /// One uniform block of vertically aligned text.
    SingleBlockVertText,
    /// One uniform block of text.
    SingleBlock,
    /// One text line.
    SingleLine,
    /// One word.
    SingleWord,
    /// One word in a circle.
    CircleWord,
    /// One character.
    SingleChar,
    /// As much text as possible, in no particular order.
    SparseText,
    /// Sparse text with orientation and script detection.
    SparseTextOsd,
    /// One text line, bypassing Tesseract-specific hacks.
    RawLine,
}

impl PageSegMode {
    /// The sys crate's equivalent (same numbering as Tesseract).
    pub(crate) fn to_sys(self) -> sys::PageSegMode {
        match self {
            Self::OsdOnly => sys::PageSegMode::OsdOnly,
            Self::AutoOsd => sys::PageSegMode::AutoOsd,
            Self::AutoOnly => sys::PageSegMode::AutoOnly,
            Self::Auto => sys::PageSegMode::Auto,
            Self::SingleColumn => sys::PageSegMode::SingleColumn,
            Self::SingleBlockVertText => sys::PageSegMode::SingleBlockVertText,
            Self::SingleBlock => sys::PageSegMode::SingleBlock,
            Self::SingleLine => sys::PageSegMode::SingleLine,
            Self::SingleWord => sys::PageSegMode::SingleWord,
            Self::CircleWord => sys::PageSegMode::CircleWord,
            Self::SingleChar => sys::PageSegMode::SingleChar,
            Self::SparseText => sys::PageSegMode::SparseText,
            Self::SparseTextOsd => sys::PageSegMode::SparseTextOsd,
            Self::RawLine => sys::PageSegMode::RawLine,
        }
    }
}

/// Tesseract's engine modes (`--oem`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineMode {
    /// The legacy engine only. Not available in the vendored build (D-4).
    TesseractOnly,
    /// The LSTM engine only (the default).
    LstmOnly,
    /// Legacy and LSTM combined. Not available in the vendored build (D-4).
    TesseractLstmCombined,
    /// Whatever the model supports; LSTM when the legacy engine is absent.
    Default,
}

impl EngineMode {
    /// The sys crate's equivalent.
    pub(crate) fn to_sys(self) -> sys::EngineMode {
        match self {
            Self::TesseractOnly => sys::EngineMode::TesseractOnly,
            Self::LstmOnly => sys::EngineMode::LstmOnly,
            Self::TesseractLstmCombined => sys::EngineMode::TesseractLstmCombined,
            Self::Default => sys::EngineMode::Default,
        }
    }

    /// Whether the mode needs the legacy engine.
    fn needs_legacy(self) -> bool {
        matches!(self, Self::TesseractOnly | Self::TesseractLstmCombined)
    }
}

/// How an engine is set up (spec O8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrConfig {
    /// The directory holding `<lang>.traineddata`; `None` searches
    /// `$TESSDATA_PREFIX` and the usual install locations (O9).
    pub tessdata: Option<PathBuf>,
    /// Model names, each matching `[A-Za-z0-9_]+`; recognised together
    /// (Tesseract's `eng+deu`). Default `["eng"]`.
    pub languages: Vec<String>,
    /// Resolution pages are rendered at, 1–2400. Default 300.
    pub dpi: u32,
    /// The most Tesseract instances (and so concurrent recognitions) the
    /// engine keeps, at least 1. Default `min(available_parallelism, 4)`.
    pub max_instances: usize,
    /// The budget for one page, rendering and recognition together;
    /// non-zero. Default 60 s.
    pub page_timeout: Duration,
    /// Page segmentation mode. Default [`PageSegMode::Auto`].
    pub psm: PageSegMode,
    /// Engine mode. Default [`EngineMode::LstmOnly`].
    pub oem: EngineMode,
    /// The most pixels a page raster may hold; the resolution is lowered to
    /// fit (O2). At least 1, and at most [`MAX_PROCESS_RENDER_PIXELS`] with
    /// [`Isolation::Process`]. Default 40,000,000.
    pub max_render_pixels: u64,
    /// In-process or helper-process recognition. Default
    /// [`Isolation::InProcess`].
    pub isolation: Isolation,
    /// Where to look for the helper binary under [`Isolation::Process`]
    /// (O28); the Python binding passes its package directory.
    pub helper_dir: Option<PathBuf>,
    /// Whether the noise pre-check runs before recognition (run 0003 F12):
    /// a raster rendered at 144 dpi or more that is dominated by speckle is
    /// recognised at half the resolution, or skipped if it still is (see
    /// [`precheck`](crate::precheck)). Default `true`; `false` sends every
    /// raster to the backend as rendered.
    pub noise_precheck: bool,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            tessdata: None,
            languages: vec!["eng".to_owned()],
            dpi: 300,
            max_instances: default_max_instances(),
            page_timeout: Duration::from_secs(60),
            psm: PageSegMode::Auto,
            oem: EngineMode::LstmOnly,
            max_render_pixels: 40_000_000,
            isolation: Isolation::InProcess,
            helper_dir: None,
            noise_precheck: true,
        }
    }
}

/// `min(available_parallelism(), 4)`, and 1 when the parallelism is unknown.
fn default_max_instances() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().min(4))
}

/// Whether `name` is a language Tesseract may be given: `[A-Za-z0-9_]+`.
/// Stricter than the sys crate's check, which allows `+` because it takes
/// the joined list; here each name stands alone, so an empty name or a
/// stray `+` is refused too.
pub(crate) fn valid_language(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

impl OcrConfig {
    /// Checks every field against O8 (and the build's engine modes, D-4)
    /// without touching the filesystem.
    ///
    /// # Errors
    /// [`UnavailableReason::InitFailed`] naming the offending field.
    pub fn validate(&self) -> Result<(), OcrUnavailable> {
        let fail = |detail: String| Err(self.unavailable(UnavailableReason::InitFailed, detail));
        if self.languages.is_empty() {
            return fail("languages: at least one language is required".into());
        }
        if let Some(bad) = self.languages.iter().find(|l| !valid_language(l)) {
            return fail(format!(
                "languages: invalid language name {bad:?} (allowed: A-Z, a-z, 0-9, _)"
            ));
        }
        if !(MIN_DPI..=MAX_DPI).contains(&self.dpi) {
            return fail(format!(
                "dpi: {} is outside {MIN_DPI}..={MAX_DPI}",
                self.dpi
            ));
        }
        if self.max_instances == 0 {
            return fail("max_instances: must be at least 1".into());
        }
        if self.page_timeout.is_zero() {
            return fail("page_timeout: must be greater than zero".into());
        }
        if self.max_render_pixels == 0 {
            return fail("max_render_pixels: must be at least 1".into());
        }
        if self.isolation == Isolation::Process
            && self.max_render_pixels > MAX_PROCESS_RENDER_PIXELS
        {
            return fail(format!(
                "max_render_pixels: {} is over {MAX_PROCESS_RENDER_PIXELS}, the most \
                 Isolation::Process accepts (a raster must fit one helper frame)",
                self.max_render_pixels
            ));
        }
        if self.oem.needs_legacy() && !sys::LEGACY_ENGINE_BUILT {
            return fail(format!(
                "oem: {:?} needs the legacy engine: legacy engine not built",
                self.oem
            ));
        }
        Ok(())
    }

    /// The languages in Tesseract's `+`-joined form.
    pub(crate) fn joined_languages(&self) -> String {
        self.languages.join("+")
    }

    /// An [`OcrUnavailable`] for this configuration with nothing resolved
    /// yet.
    pub(crate) fn unavailable(&self, reason: UnavailableReason, detail: String) -> OcrUnavailable {
        OcrUnavailable {
            reason,
            tessdata: None,
            languages: self.languages.clone(),
            searched: Vec::new(),
            detail,
        }
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
