//! The production backend: in-process Tesseract instances in a pool
//! (spec 0002 O7, O10, O13, O16).
//!
//! [`TesseractBackend::new`] performs O7's validation once, in order —
//! configuration (O8), model directory (O9), one `Init`, one recognition of
//! a 1 × 1 white image to force model loading — and keeps that instance as
//! the pool's first member. Model checksums (O13) are computed here too, so
//! `info()` never touches the filesystem.
//!
//! The sys crate's `Instance` is `Send + !Sync` and never leaves this
//! crate: the pool hands one to one page at a time. Under
//! `Isolation::Process` the pool holds helper processes instead
//! (`isolation::client`), and the helper binary reuses this module's
//! instance, checksum and warm-up code on its side of the pipe.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pdf_extract_core::{OcrError, OcrPageError, OcrUnavailable, UnavailableReason};
use pdf_extract_render::GrayImage;
use pdf_extract_tesseract_sys as sys;
use sha2::{Digest, Sha256};

use crate::backend::{
    BackendInfo, Linkage, OcrBackend, OcrBlock, OcrLine, OcrOutput, OcrRequest, OcrWord,
    Recognizer, RecognizerFactory,
};
use crate::config::{EngineMode, Isolation, OcrConfig, PageSegMode};
#[cfg(unix)]
use crate::isolation::client::{self, HelperFactory};
use crate::pool::Pool;
use crate::tessdata::{self, Candidates};

/// The backend's name in [`BackendInfo`].
const NAME: &str = "tesseract";

/// Resolution given to the warm-up recognition; any plausible value works.
const WARMUP_DPI: u32 = 300;

/// Makes initialised Tesseract instances for one model directory and
/// language set.
#[derive(Debug)]
pub(crate) struct TesseractFactory {
    tessdata: PathBuf,
    languages: Vec<String>,
    joined: String,
    oem: EngineMode,
    info: BackendInfo,
}

/// One Tesseract handle.
#[derive(Debug)]
pub(crate) struct TesseractInstance(pub(crate) sys::Instance);

impl TesseractFactory {
    fn unavailable(&self, detail: String) -> OcrUnavailable {
        OcrUnavailable {
            reason: UnavailableReason::InitFailed,
            tessdata: Some(self.tessdata.clone()),
            languages: self.languages.clone(),
            searched: Vec::new(),
            detail,
        }
    }
}

impl RecognizerFactory for TesseractFactory {
    type Instance = TesseractInstance;

    fn create(&self) -> Result<TesseractInstance, OcrUnavailable> {
        sys::Instance::new(&self.tessdata, &self.joined, self.oem.to_sys())
            .map(TesseractInstance)
            .map_err(|e| self.unavailable(e.message))
    }

    fn info(&self) -> BackendInfo {
        self.info.clone()
    }
}

/// A sys error during recognition as a page error. A timeout carries no
/// meaningful duration here; the engine substitutes its `page_timeout`.
fn page_error(e: &sys::Error) -> OcrPageError {
    match e.kind {
        sys::ErrorKind::Timeout => OcrPageError::Timeout(std::time::Duration::ZERO),
        _ => OcrPageError::Recognition(e.message.clone()),
    }
}

impl Recognizer for TesseractInstance {
    fn recognize(&mut self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        let inst = &mut self.0;
        let err = |e: sys::Error| page_error(&e);
        inst.set_image(&img.data, img.width, img.height, img.width)
            .map_err(err)?;
        inst.set_source_resolution(req.dpi).map_err(err)?;
        inst.set_page_seg_mode(req.psm.to_sys()).map_err(err)?;
        // A zero remainder is refused by the sys crate as an expired
        // deadline (Timeout) without entering Tesseract.
        let remaining = req.deadline.saturating_duration_since(Instant::now());
        inst.recognize(remaining).map_err(err)?;
        let blocks = inst.blocks().map_err(err)?;
        Ok(OcrOutput {
            blocks: blocks
                .into_iter()
                .map(|b| OcrBlock {
                    lines: b
                        .lines
                        .into_iter()
                        .map(|l| OcrLine {
                            words: l
                                .words
                                .into_iter()
                                .map(|w| OcrWord {
                                    text: w.text,
                                    bbox_px: w.bbox,
                                    confidence: w.confidence,
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        })
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}

/// O7 (4): a model can pass `Init` and still fail on first use; one
/// recognition of a white pixel loads everything now, not on a page. Shared
/// by the in-process backend and the helper process (O29).
pub(crate) fn warm_up(
    inst: &mut TesseractInstance,
    psm: PageSegMode,
    budget: Duration,
) -> Result<(), String> {
    let warmup = GrayImage {
        width: 1,
        height: 1,
        data: vec![u8::MAX],
    };
    let req = OcrRequest {
        dpi: WARMUP_DPI,
        deadline: crate::engine::deadline_after(Instant::now(), budget),
        psm,
    };
    inst.recognize(&warmup, &req)
        .map_err(|e| format!("warm-up recognition failed: {e}"))?;
    inst.clear();
    Ok(())
}

/// SHA-256 of a file, streamed (models are 4–23 MB).
pub(crate) fn sha256_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(hasher.finalize().into())
}

/// Tesseract with a pool of instances (O10): in-process handles, or one
/// helper process per slot under [`Isolation::Process`] (O26). The
/// production [`OcrBackend`].
pub struct TesseractBackend {
    pool: Pools,
}

/// The pool, by isolation; both kinds share the pool code.
enum Pools {
    InProcess(Pool<TesseractFactory>),
    #[cfg(unix)]
    Process(Pool<HelperFactory>),
}

impl std::fmt::Debug for TesseractBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TesseractBackend")
            .field("info", &OcrBackend::info(self))
            .finish_non_exhaustive()
    }
}

impl TesseractBackend {
    /// Validates `cfg` and the host once (O7) and returns a backend whose
    /// pool holds the instance that was validated.
    ///
    /// # Errors
    /// [`OcrUnavailable`] with the reason of O7.
    pub fn new(cfg: &OcrConfig) -> Result<Self, OcrUnavailable> {
        Self::with_candidates(cfg, &Candidates::current())
    }

    /// As [`TesseractBackend::new`] with injected resolution inputs.
    #[doc(hidden)]
    pub fn with_candidates(
        cfg: &OcrConfig,
        candidates: &Candidates,
    ) -> Result<Self, OcrUnavailable> {
        cfg.validate()?;
        if cfg.isolation == Isolation::Process {
            return Self::process(cfg, candidates);
        }
        let resolved = tessdata::resolve(cfg.tessdata.as_deref(), candidates, &cfg.languages)?;
        let fail = |reason, detail: String| OcrUnavailable {
            reason,
            tessdata: Some(resolved.dir.clone()),
            languages: cfg.languages.clone(),
            searched: resolved.searched.clone(),
            detail,
        };
        let mut model_sha256 = Vec::with_capacity(cfg.languages.len());
        for lang in &cfg.languages {
            let path = tessdata::model_path(&resolved.dir, lang);
            let digest = sha256_file(&path).map_err(|e| {
                fail(
                    UnavailableReason::LanguageDataMissing,
                    format!("cannot read {}: {e}", path.display()),
                )
            })?;
            model_sha256.push((lang.clone(), digest));
        }
        let factory = TesseractFactory {
            tessdata: resolved.dir.clone(),
            languages: cfg.languages.clone(),
            joined: cfg.joined_languages(),
            oem: cfg.oem,
            info: BackendInfo {
                name: NAME,
                version: sys::version(),
                languages: cfg.languages.clone(),
                tessdata: Some(resolved.dir.clone()),
                model_sha256,
                linkage: match sys::LINKAGE {
                    sys::Linkage::Vendored => Linkage::Vendored,
                    sys::Linkage::System => Linkage::System,
                },
                isolation: Isolation::InProcess,
            },
        };
        let mut first = factory
            .create()
            .map_err(|e| fail(UnavailableReason::InitFailed, e.detail))?;
        warm_up(&mut first, cfg.psm, cfg.page_timeout)
            .map_err(|detail| fail(UnavailableReason::InitFailed, detail))?;
        Ok(Self {
            pool: Pools::InProcess(Pool::new(Arc::new(factory), cfg.max_instances, Some(first))),
        })
    }

    /// O29: the first helper validates the engine; this process never
    /// loads Tesseract.
    #[cfg(unix)]
    fn process(cfg: &OcrConfig, candidates: &Candidates) -> Result<Self, OcrUnavailable> {
        let (factory, first) = client::build(cfg, candidates)?;
        Ok(Self {
            pool: Pools::Process(Pool::new(Arc::new(factory), cfg.max_instances, Some(first))),
        })
    }

    #[cfg(not(unix))]
    fn process(cfg: &OcrConfig, _: &Candidates) -> Result<Self, OcrUnavailable> {
        Err(cfg.unavailable(
            UnavailableReason::EngineNotBuilt,
            "process isolation is not available on this platform".into(),
        ))
    }

    /// Recognises on a pooled instance; a replacement instance that cannot
    /// be created is [`OcrError::Unavailable`] (O10).
    pub(crate) fn run(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrError> {
        match &self.pool {
            Pools::InProcess(p) => p.run(|inst| inst.recognize(img, req)),
            #[cfg(unix)]
            Pools::Process(p) => p.run(|inst| inst.recognize(img, req)),
        }
    }

    /// Instances (or helper processes) in existence (test observability).
    pub(crate) fn pool_len(&self) -> usize {
        match &self.pool {
            Pools::InProcess(p) => p.len(),
            #[cfg(unix)]
            Pools::Process(p) => p.len(),
        }
    }
}

impl OcrBackend for TesseractBackend {
    /// As the engine's pooled path; a failure to create a replacement
    /// instance, which this signature cannot carry as `OcrUnavailable`, is
    /// reported as a recognition failure with that message.
    fn recognize(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        self.run(img, req).map_err(|e| match e {
            OcrError::Page(p) => p,
            other => OcrPageError::Recognition(other.to_string()),
        })
    }

    fn info(&self) -> BackendInfo {
        match &self.pool {
            Pools::InProcess(p) => p.factory().info(),
            #[cfg(unix)]
            Pools::Process(p) => p.factory().info(),
        }
    }
}
