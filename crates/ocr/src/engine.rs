//! The engine (spec 0002 O7, O12, O15–O20): one page in, one `TextPage` out.
//!
//! [`OcrEngine::ocr`] renders the page with `pdf-extract-render` (under the
//! document's own limits, holding no engine lock), checks the page budget,
//! runs the noise pre-check (run 0003 F12–F14), hands the raster to the
//! backend and builds the result with core's `TextPageBuilder`, converting
//! pixel boxes to page points. The budget (`page_timeout`) starts before
//! rendering, so a slow render leaves less time for recognition (O16); the
//! pre-check is charged to it the same way.
//!
//! The pre-check runs here, in front of every backend, and not inside the
//! Tesseract backend: under `Isolation::Process` it therefore runs in the
//! parent, which still loads no native OCR code (O29), and the scripted
//! backend sees exactly what Tesseract would.
//!
//! An engine is a cheap `Arc` handle: clones share the backend and its pool.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use pdf_extract_core::ocr_support::TextPageBuilder;
use pdf_extract_core::{
    OcrError, OcrPageError, OcrUnavailable, Page, PageRecognizer, TextPage, Warning, WarningKind,
};
use pdf_extract_render::{GrayImage, RenderError, RenderOptions, render_gray};

use crate::backend::{
    BackendInfo, OcrBackend, OcrOutput, OcrRequest, Recognizer, RecognizerFactory,
};
use crate::config::OcrConfig;
use crate::pool::Pool;
use crate::precheck::{self, Verdict};
use crate::tessdata::Candidates;
use crate::tesseract::TesseractBackend;

/// Per-call options of [`OcrEngine::ocr`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OcrOptions {
    /// Resolution for this page; `None` uses the engine's `dpi`.
    pub dpi: Option<u32>,
}

/// How an engine reaches its backend.
trait Dispatch: Send + Sync {
    fn recognize(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrError>;
    fn info(&self) -> BackendInfo;
    fn pool_len(&self) -> Option<usize>;
}

/// A backend given to [`OcrEngine::with_backend`]: called directly, no pool.
struct Direct(Arc<dyn OcrBackend>);

impl Dispatch for Direct {
    fn recognize(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrError> {
        self.0.recognize(img, req).map_err(OcrError::Page)
    }
    fn info(&self) -> BackendInfo {
        self.0.info()
    }
    fn pool_len(&self) -> Option<usize> {
        None
    }
}

impl Dispatch for TesseractBackend {
    fn recognize(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrError> {
        self.run(img, req)
    }
    fn info(&self) -> BackendInfo {
        OcrBackend::info(self)
    }
    fn pool_len(&self) -> Option<usize> {
        Some(TesseractBackend::pool_len(self))
    }
}

impl<F: RecognizerFactory> Dispatch for Pool<F> {
    fn recognize(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrError> {
        self.run(|inst| inst.recognize(img, req))
    }
    fn info(&self) -> BackendInfo {
        self.factory().info()
    }
    fn pool_len(&self) -> Option<usize> {
        Some(self.len())
    }
}

struct Inner {
    config: OcrConfig,
    /// The backend's model directory, read once at construction.
    tessdata: Option<PathBuf>,
    backend: Box<dyn Dispatch>,
}

/// An OCR engine: configuration plus a backend (spec §3.3).
///
/// `Send + Sync`; clone it to share one backend and pool between threads.
///
/// **Forking.** An engine built before a `fork()` is reused by the child
/// (the pool rebuilds itself on first use), but a fork while another thread
/// is inside the engine holds the pool mutex locked in the child for good.
/// Build every engine first and see the
/// [crate-level `Forking` section](crate#forking) for the three rules.
#[derive(Clone)]
pub struct OcrEngine {
    inner: Arc<Inner>,
}

impl fmt::Debug for OcrEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OcrEngine")
            .field("config", &self.inner.config)
            .finish_non_exhaustive()
    }
}

/// `start + timeout`, or the furthest representable instant before it
/// overflows (a huge timeout means "no deadline", not a panic).
pub(crate) fn deadline_after(start: Instant, timeout: Duration) -> Instant {
    let mut t = timeout;
    loop {
        if let Some(d) = start.checked_add(t) {
            return d;
        }
        t /= 2;
    }
}

impl OcrEngine {
    /// Builds an in-process Tesseract engine, validating the configuration,
    /// the model directory and one instance up front (O7).
    ///
    /// # Errors
    /// [`OcrUnavailable`] when the configuration is out of range
    /// (`InitFailed`, naming the field), no model directory is found
    /// (`NoTessdata`), the chosen one lacks a language
    /// (`LanguageDataMissing`), Tesseract refuses to start (`InitFailed`),
    /// or process isolation was asked for and is not built
    /// (`EngineNotBuilt`).
    pub fn tesseract(cfg: OcrConfig) -> Result<Self, OcrUnavailable> {
        Self::tesseract_with_candidates(cfg, &Candidates::current())
    }

    /// As [`OcrEngine::tesseract`] with injected resolution inputs (tests).
    #[doc(hidden)]
    pub fn tesseract_with_candidates(
        cfg: OcrConfig,
        candidates: &Candidates,
    ) -> Result<Self, OcrUnavailable> {
        let backend = TesseractBackend::with_candidates(&cfg, candidates)?;
        Ok(Self::from_dispatch(cfg, Box::new(backend)))
    }

    /// An engine over a caller's backend, which is called directly: no
    /// validation, no model resolution and no pool (O33).
    pub fn with_backend(backend: Arc<dyn OcrBackend>, cfg: OcrConfig) -> Self {
        Self::from_dispatch(cfg, Box::new(Direct(backend)))
    }

    /// An engine over a pool of instances made by `factory`, empty until
    /// the first page and bounded by `cfg.max_instances`; no validation.
    /// Exercises the production pool without Tesseract.
    #[doc(hidden)]
    pub fn with_pool<F: RecognizerFactory>(factory: Arc<F>, cfg: OcrConfig) -> Self {
        let pool = Pool::new(factory, cfg.max_instances, None);
        Self::from_dispatch(cfg, Box::new(pool))
    }

    fn from_dispatch(config: OcrConfig, backend: Box<dyn Dispatch>) -> Self {
        let tessdata = backend.info().tessdata;
        Self {
            inner: Arc::new(Inner {
                config,
                tessdata,
                backend,
            }),
        }
    }

    /// The configuration the engine was built with.
    pub fn config(&self) -> &OcrConfig {
        &self.inner.config
    }

    /// The model directory in use, when the backend has one.
    pub fn tessdata(&self) -> Option<&Path> {
        self.inner.tessdata.as_deref()
    }

    /// What the engine runs (O13).
    pub fn info(&self) -> BackendInfo {
        self.inner.backend.info()
    }

    /// Instances the pool holds, idle or busy; `None` for an unpooled
    /// backend (test observability for O7 and O10).
    #[doc(hidden)]
    pub fn pool_len(&self) -> Option<usize> {
        self.inner.backend.pool_len()
    }

    /// Renders and recognises `page` (O16–O19).
    ///
    /// With [`OcrConfig::noise_precheck`] (the default) and a rendered
    /// resolution of at least [`precheck::MIN_DPI`], a raster the
    /// [`precheck`] classifies as speckle is box-filtered to half the
    /// resolution and checked again: if that is clear it is recognised at
    /// half the resolution (the page's `dpi()` says so, its boxes stay in
    /// page points, and it carries a [`WarningKind::OcrNoiseDownsampled`]
    /// warning); otherwise recognition is skipped and the page is empty
    /// with a [`WarningKind::OcrNoiseSkipped`] warning (run 0003 F13, F14).
    ///
    /// # Errors
    /// [`OcrError::Closed`] when the document is closed;
    /// [`OcrError::Page`] when rendering hit a document limit (`Limit`; the
    /// backend is then not called), the page budget ran out (`Timeout`,
    /// carrying `page_timeout`) or recognition failed (`Recognition`);
    /// [`OcrError::Unavailable`] when a replacement pool instance could not
    /// be created.
    pub fn ocr(&self, page: &Page, opts: &OcrOptions) -> Result<TextPage, OcrError> {
        self.ocr_with(page, opts, |deadline, screen| {
            screen_within(deadline, screen)
        })
    }

    /// [`OcrEngine::ocr`] with the deadline-charged pre-check step
    /// injected: `screen_step(deadline, screen)` runs `screen` and returns
    /// `None` when the budget ran out. A seam for the unit tests only (internal #95
    /// C5): render and screen both scale with the pixel count, so no page
    /// makes the real screen overrun a budget the render left standing, and
    /// only a substituted step can pin that the engine honours the check.
    fn ocr_with(
        &self,
        page: &Page,
        opts: &OcrOptions,
        screen_step: impl FnOnce(Instant, &dyn Fn() -> Screened) -> Option<Screened>,
    ) -> Result<TextPage, OcrError> {
        let cfg = &self.inner.config;
        let start = Instant::now();
        let deadline = deadline_after(start, cfg.page_timeout);
        let timeout = || OcrError::Page(OcrPageError::Timeout(cfg.page_timeout));
        let rendered = render_gray(
            page,
            &RenderOptions {
                dpi: opts.dpi.unwrap_or(cfg.dpi),
                max_render_pixels: cfg.max_render_pixels,
                render_annotations: true,
            },
        )
        .map_err(|e| match e {
            RenderError::Closed => OcrError::Closed,
            RenderError::Limit { kind, value } => {
                OcrError::Page(OcrPageError::Limit { kind, value })
            }
        })?;
        // The render consumed the whole budget: recognising now could only
        // time out, after doing work.
        if Instant::now() >= deadline {
            return Err(timeout());
        }
        let mut warnings = rendered.warnings;
        let screened = if cfg.noise_precheck {
            match screen_step(deadline, &|| screen(&rendered.image, rendered.dpi)) {
                Some(screened) => screened,
                None => return Err(timeout()),
            }
        } else {
            Screened::AsRendered
        };
        let (image, scale) = match &screened {
            Screened::AsRendered => (&rendered.image, 1),
            Screened::Halved(small) => (small, 2),
            Screened::Skipped => {
                warnings.push(noise_warning(page.index(), &screened, rendered.dpi));
                return Ok(TextPageBuilder::new(rendered.dpi, warnings).finish());
            }
        };
        let dpi = rendered.dpi / scale;
        let req = OcrRequest {
            dpi,
            deadline,
            psm: cfg.psm,
        };
        let output = match self.inner.backend.recognize(image, &req) {
            // A backend's timeout is the page's budget, whatever it measured.
            Err(OcrError::Page(OcrPageError::Timeout(_))) => return Err(timeout()),
            other => other?,
        };
        if scale != 1 {
            warnings.push(noise_warning(page.index(), &screened, rendered.dpi));
        }
        Ok(build_page(
            &output,
            Raster {
                height: rendered.image.height,
                dpi: rendered.dpi,
                scale,
            },
            dpi,
            warnings,
        ))
    }
}

/// Runs the pre-check `screen` and charges it to the page budget like the
/// render (O16): `None` when it finished at or past `deadline`, so the
/// caller reports a timeout instead of recognising on a spent budget. A
/// separate function so the post-screen check can be tested with a screen
/// that overruns; a real one cannot be made to overrun while the render
/// check before it still passes.
fn screen_within(deadline: Instant, screen: impl FnOnce() -> Screened) -> Option<Screened> {
    let screened = screen();
    (Instant::now() < deadline).then_some(screened)
}

/// What the noise pre-check made of a rendered raster (run 0003 F12, F13).
#[derive(Debug)]
enum Screened {
    /// Recognise the raster as rendered: clear, below the resolution floor,
    /// or too small to halve.
    AsRendered,
    /// Speckle at the rendered resolution, clear at half of it: recognise
    /// this 2 × 2 box-filtered raster.
    Halved(GrayImage),
    /// Speckle at both resolutions: do not recognise.
    Skipped,
}

/// Runs the pre-check on `image`, rendered at `dpi` (spec 0003 F12, F13).
/// Exactly one downsample step is ever taken.
fn screen(image: &GrayImage, dpi: u32) -> Screened {
    if dpi < precheck::MIN_DPI || precheck::classify(image) == Verdict::Clear {
        return Screened::AsRendered;
    }
    match precheck::downsample2(image) {
        None => Screened::AsRendered,
        Some(small) if precheck::classify(&small) == Verdict::Clear => Screened::Halved(small),
        Some(_) => Screened::Skipped,
    }
}

/// The page warning for a downsampled or skipped page rendered at `dpi`
/// (run 0003 F14).
fn noise_warning(page: usize, screened: &Screened, dpi: u32) -> Warning {
    let half = dpi / 2;
    let (kind, detail) = match screened {
        Screened::Skipped => (
            WarningKind::OcrNoiseSkipped,
            format!(
                "noise-dominated raster skipped (no convincing foreground at {dpi} or {half} dpi)"
            ),
        ),
        _ => (
            WarningKind::OcrNoiseDownsampled,
            format!("noise-dominated raster recognised at {half} dpi (rendered at {dpi})"),
        ),
    };
    Warning {
        page: Some(page),
        kind,
        detail,
        count: 1,
    }
}

/// The geometry of the rendered raster that the backend's pixel boxes are
/// mapped back onto.
#[derive(Clone, Copy, Debug)]
struct Raster {
    /// Height of the raster as rendered, in pixels.
    height: u32,
    /// Resolution it was rendered at.
    dpi: u32,
    /// Rendered pixels per backend pixel: 1, or 2 for a downsampled raster.
    scale: u32,
}

/// Converts pixel boxes (origin top left, y down, in the backend's raster)
/// to page points (origin bottom left, y up) and builds the page recognised
/// at `dpi` (O17, O18; run 0003 F13).
///
/// The rendered raster covers the page's visible box with `/Rotate`
/// applied, which is the text path's page space, so a rendered pixel's top
/// edge `y` is at `height_pt − y × 72 / dpi'`. The raster's height is the
/// visible height rounded up to a whole pixel, so boxes may sit up to one
/// pixel high. A downsampled raster's pixel `p` is rendered pixel
/// `p × scale` exactly, and the flip uses the rendered height, so boxes keep
/// their meaning whatever the trailing odd row the box filter dropped.
fn build_page(
    output: &OcrOutput,
    raster: Raster,
    dpi: u32,
    warnings: Vec<pdf_extract_core::Warning>,
) -> TextPage {
    #[allow(clippy::cast_precision_loss)]
    let pt_per_px = 72.0 / raster.dpi.max(1) as f32;
    #[allow(clippy::cast_precision_loss)]
    let height_pt = raster.height as f32 * pt_per_px;
    // In u64: a pixel coordinate times 2 may not fit u32. At scale 1 the
    // values (and so the f32 results) are exactly those of run 0002.
    let px = |v: u32| -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let p = (u64::from(v) * u64::from(raster.scale)) as f32;
        p * pt_per_px
    };
    let to_pt = |[l, t, r, b]: [u32; 4]| -> [f32; 4] {
        [px(l), height_pt - px(b), px(r), height_pt - px(t)]
    };
    let mut builder = TextPageBuilder::new(dpi, warnings);
    for block in &output.blocks {
        builder.begin_block();
        for line in &block.lines {
            // The backend reports no line box; the builder's line box is
            // the union of its word boxes either way.
            let boxes: Vec<[f32; 4]> = line.words.iter().map(|w| to_pt(w.bbox_px)).collect();
            let Some(first) = boxes.first() else {
                continue;
            };
            let union = boxes.iter().fold(*first, |[a, b, c, d], [e, f, g, h]| {
                [a.min(*e), b.min(*f), c.max(*g), d.max(*h)]
            });
            builder.begin_line(union);
            for (word, bbox) in line.words.iter().zip(boxes) {
                builder.word(&word.text, bbox);
            }
        }
    }
    builder.finish()
}

/// `page.ocr(&engine, &opts)` for core's [`Page`] (D-2: an extension trait,
/// because core cannot depend on the engine).
pub trait PageOcr {
    /// Renders and recognises this page with `engine`; see
    /// [`OcrEngine::ocr`].
    ///
    /// # Errors
    /// As [`OcrEngine::ocr`].
    fn ocr(&self, engine: &OcrEngine, opts: &OcrOptions) -> Result<TextPage, OcrError>;
}

impl PageOcr for Page {
    fn ocr(&self, engine: &OcrEngine, opts: &OcrOptions) -> Result<TextPage, OcrError> {
        engine.ocr(self, opts)
    }
}

/// Lets `Document::extract` recognise sparse pages with this engine (core
/// sees OCR only through this trait, D-2). `dpi: None` is the engine's own
/// configured resolution, as for [`OcrOptions::default`].
impl PageRecognizer for OcrEngine {
    fn recognize(&self, page: &Page, dpi: Option<u32>) -> Result<TextPage, OcrError> {
        self.ocr(page, &OcrOptions { dpi })
    }
}

/// The process-wide default engine (O12): built on first use from
/// [`OcrConfig::default`] and `TESSDATA_PREFIX`. A failure is stored and
/// returned on every later call without retrying (the host will not have
/// changed in a way the caller expects us to notice). Fork-aware through
/// the pool (O11): a forked child reuses the engine and rebuilds its pool.
///
/// A **panic** during the first construction is not stored: it propagates to
/// the caller (ADR 35) and leaves the `OnceLock` empty, so the next call
/// constructs again; only an `Err` is remembered.
///
/// **Forking.** The first construction holds the `OnceLock` of this
/// function (and, inside it, Tesseract's one-time initialisation lock) for
/// tens to hundreds of milliseconds; a `fork()` while another thread is
/// inside it leaves that lock held forever in the child. Call this once
/// before forking. See the [crate-level `Forking` section](crate#forking).
///
/// # Errors
/// The stored [`OcrUnavailable`] of the first attempt.
pub fn default_ocr_engine() -> Result<&'static OcrEngine, &'static OcrUnavailable> {
    static DEFAULT: OnceLock<Result<OcrEngine, OcrUnavailable>> = OnceLock::new();
    DEFAULT
        .get_or_init(|| OcrEngine::tesseract(OcrConfig::default()))
        .as_ref()
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
