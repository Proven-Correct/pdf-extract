//! `ScriptedBackend`: an OCR backend that returns what a test scripted
//! (run 0002 O33, §4.8), so every error path of the engine is testable
//! without Tesseract.
//!
//! It is usable both ways an engine reaches a backend: directly
//! (`OcrEngine::with_backend`, as [`OcrBackend`]) and as a factory of pool
//! instances (`OcrEngine::with_pool`, as [`RecognizerFactory`]), so the
//! production pool's bound, drop-on-error and clear-per-page behaviour can
//! be observed through its counters. Steps are consumed in order, one per
//! recognition, from a queue shared by every instance; when the queue is
//! empty the default step runs.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use pdf_extract_core::{OcrPageError, OcrUnavailable, UnavailableReason};
use pdf_extract_ocr::{
    BackendInfo, GrayImage, Isolation, Linkage, OcrBackend, OcrBlock, OcrLine, OcrOutput,
    OcrRequest, OcrWord, Recognizer, RecognizerFactory,
};

/// What one recognition does.
#[derive(Clone, Debug)]
pub enum Step {
    /// Returns this output.
    Output(OcrOutput),
    /// Returns this error.
    Error(OcrPageError),
    /// Panics with this message.
    Panic(String),
    /// Sleeps, then performs the inner step (a slow page).
    Sleep(Duration, Box<Step>),
}

impl Step {
    /// A timeout as Tesseract's monitor would report it.
    pub fn timeout() -> Self {
        Self::Error(OcrPageError::Timeout(Duration::ZERO))
    }

    /// A recognition failure with `message`.
    pub fn recognition(message: &str) -> Self {
        Self::Error(OcrPageError::Recognition(message.to_owned()))
    }

    /// An empty result: the engine found nothing.
    pub fn empty() -> Self {
        Self::Output(OcrOutput::default())
    }
}

/// Builds an output of one block from lines of `(word, [l, t, r, b])` in
/// raster pixels.
pub fn output(lines: &[&[(&str, [u32; 4])]]) -> OcrOutput {
    OcrOutput {
        blocks: vec![OcrBlock {
            lines: lines
                .iter()
                .map(|words| OcrLine {
                    words: words
                        .iter()
                        .map(|(text, bbox_px)| OcrWord {
                            text: (*text).to_owned(),
                            bbox_px: *bbox_px,
                            confidence: 90.0,
                        })
                        .collect(),
                })
                .collect(),
        }],
    }
}

/// Counters a test reads back.
#[derive(Debug, Default)]
struct Shared {
    steps: Mutex<VecDeque<Step>>,
    default: Mutex<Option<Step>>,
    create_failures: AtomicUsize,
    calls: AtomicUsize,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    clears: AtomicUsize,
    creates: AtomicUsize,
    drops: AtomicUsize,
    last_request: Mutex<Option<OcrRequest>>,
    last_image: Mutex<Option<(u32, u32)>>,
}

/// A scripted OCR backend (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct ScriptedBackend {
    shared: Arc<Shared>,
}

/// One pool instance of a [`ScriptedBackend`]; shares its script and
/// counters.
#[derive(Debug)]
pub struct ScriptedInstance {
    shared: Arc<Shared>,
}

impl Drop for ScriptedInstance {
    fn drop(&mut self) {
        self.shared.drops.fetch_add(1, Ordering::SeqCst);
    }
}

impl ScriptedBackend {
    /// A backend whose every recognition returns [`Step::empty`].
    pub fn new() -> Self {
        Self::default()
    }

    /// A backend whose recognitions perform `step` once the queue is empty.
    pub fn with_default(step: Step) -> Self {
        let b = Self::default();
        *b.shared
            .default
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(step);
        b
    }

    /// Queues `step` for the next recognition that finds the queue at this
    /// position.
    pub fn push(&self, step: Step) -> &Self {
        self.shared
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(step);
        self
    }

    /// Makes the next `n` instance creations fail with `InitFailed`.
    pub fn fail_next_creates(&self, n: usize) {
        self.shared.create_failures.store(n, Ordering::SeqCst);
    }

    /// Recognitions started.
    pub fn calls(&self) -> usize {
        self.shared.calls.load(Ordering::SeqCst)
    }

    /// The most recognitions that ran at the same time.
    pub fn max_in_flight(&self) -> usize {
        self.shared.max_in_flight.load(Ordering::SeqCst)
    }

    /// Times a pool instance was cleared after a page.
    pub fn clears(&self) -> usize {
        self.shared.clears.load(Ordering::SeqCst)
    }

    /// Pool instances created (successfully).
    pub fn creates(&self) -> usize {
        self.shared.creates.load(Ordering::SeqCst)
    }

    /// Pool instances dropped.
    pub fn drops(&self) -> usize {
        self.shared.drops.load(Ordering::SeqCst)
    }

    /// The request of the most recent recognition.
    pub fn last_request(&self) -> Option<OcrRequest> {
        *self
            .shared
            .last_request
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `(width, height)` of the most recent raster.
    pub fn last_image(&self) -> Option<(u32, u32)> {
        *self
            .shared
            .last_image
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Shared {
    fn next_step(&self) -> Step {
        let queued = self
            .steps
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        queued.unwrap_or_else(|| {
            self.default
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
                .unwrap_or_else(Step::empty)
        })
    }

    fn recognize(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self
            .last_request
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(*req);
        *self
            .last_image
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((img.width, img.height));
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(now, Ordering::SeqCst);
        // Decrements on every exit, panics included.
        struct Leave<'a>(&'a AtomicUsize);
        impl Drop for Leave<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _leave = Leave(&self.in_flight);
        let mut step = self.next_step();
        loop {
            match step {
                Step::Output(out) => return Ok(out),
                Step::Error(e) => return Err(e),
                Step::Panic(msg) => panic!("{msg}"),
                Step::Sleep(d, inner) => {
                    std::thread::sleep(d);
                    step = *inner;
                }
            }
        }
    }

    fn info() -> BackendInfo {
        BackendInfo {
            name: "scripted",
            version: "0".to_owned(),
            languages: vec!["eng".to_owned()],
            tessdata: None::<PathBuf>,
            model_sha256: Vec::new(),
            linkage: Linkage::Vendored,
            isolation: Isolation::InProcess,
        }
    }
}

impl OcrBackend for ScriptedBackend {
    fn recognize(&self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        self.shared.recognize(img, req)
    }

    fn info(&self) -> BackendInfo {
        Shared::info()
    }
}

impl Recognizer for ScriptedInstance {
    fn recognize(&mut self, img: &GrayImage, req: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        self.shared.recognize(img, req)
    }

    fn clear(&mut self) {
        self.shared.clears.fetch_add(1, Ordering::SeqCst);
    }
}

impl RecognizerFactory for ScriptedBackend {
    type Instance = ScriptedInstance;

    fn create(&self) -> Result<ScriptedInstance, OcrUnavailable> {
        let failing = self
            .shared
            .create_failures
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if failing {
            return Err(OcrUnavailable {
                reason: UnavailableReason::InitFailed,
                tessdata: None,
                languages: vec!["eng".to_owned()],
                searched: Vec::new(),
                detail: "scripted creation failure".to_owned(),
            });
        }
        self.shared.creates.fetch_add(1, Ordering::SeqCst);
        Ok(ScriptedInstance {
            shared: self.shared.clone(),
        })
    }

    fn info(&self) -> BackendInfo {
        Shared::info()
    }
}
