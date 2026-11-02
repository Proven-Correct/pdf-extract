//! Unit tests of the engine's helpers: the deadline arithmetic, the
//! pre-check's charge to the budget (through the `ocr_with` seam) and the
//! pixel-to-point page building (spec 0002 O16, O17).

use std::sync::atomic::{AtomicUsize, Ordering};

use pdf_extract_core::BlockSeparator;

use super::*;
use crate::backend::{Linkage, OcrBlock, OcrLine, OcrWord};
use crate::config::Isolation;

fn word(text: &str, bbox_px: [u32; 4]) -> OcrWord {
    OcrWord {
        text: text.into(),
        bbox_px,
        confidence: 50.0,
    }
}

fn raster(image: &GrayImage, dpi: u32, scale: u32) -> Raster {
    Raster {
        height: image.height,
        dpi,
        scale,
    }
}

#[test]
fn a_huge_timeout_saturates_instead_of_panicking() {
    let now = Instant::now();
    assert_eq!(
        deadline_after(now, Duration::from_secs(5)),
        now + Duration::from_secs(5)
    );
    let far = deadline_after(now, Duration::MAX);
    assert!(far > now + Duration::from_secs(3600));
}

#[test]
fn a_screen_that_overruns_the_budget_is_a_timeout() {
    // The deadline passes while the screen runs, so its verdict is
    // discarded: this is the post-screen check, which an engine-level test
    // cannot reach (a budget short enough to expire during the screen has
    // already expired during the render).
    let deadline = Instant::now() + Duration::from_millis(5);
    let screened = screen_within(deadline, || {
        std::thread::sleep(Duration::from_millis(50));
        Screened::Skipped
    });
    assert!(screened.is_none(), "got {screened:?}");
}

/// A backend that counts its calls and recognises nothing. In-crate because
/// the test kit's `ScriptedBackend` implements this crate's traits as a
/// separate copy of the crate.
#[derive(Default)]
struct Counting(AtomicUsize);

impl OcrBackend for Counting {
    fn recognize(&self, _: &GrayImage, _: &OcrRequest) -> Result<OcrOutput, OcrPageError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(OcrOutput::default())
    }
    fn info(&self) -> BackendInfo {
        BackendInfo {
            name: "counting",
            version: String::new(),
            languages: Vec::new(),
            tessdata: None,
            model_sha256: Vec::new(),
            linkage: Linkage::Vendored,
            isolation: Isolation::InProcess,
        }
    }
}

#[test]
fn ocr_times_out_without_recognising_when_the_screen_step_finds_the_budget_spent() {
    // internal #95 C5: the unit tests above cover screen_within, this that the
    // engine acts on its verdict. A step reporting a spent budget must end
    // the page as a timeout before the backend is asked.
    let backend = Arc::new(Counting::default());
    let engine = OcrEngine::with_backend(backend.clone(), OcrConfig::default());
    assert!(engine.config().noise_precheck, "the step runs only with it");
    let mut pdf = pdf_extract_testkit::PdfBuilder::new();
    pdf.add_page(612.0, 792.0);
    let doc = pdf_extract_core::Document::from_bytes(
        pdf.to_bytes(),
        &pdf_extract_core::OpenOptions::default(),
    )
    .expect("open");
    let page = doc.page(0).expect("page");
    let opts = OcrOptions { dpi: Some(72) };
    let timeout = engine.config().page_timeout;
    // The step must be handed the page's own deadline, counted from the
    // start of ocr_with: one far in the future would let a real screen
    // overrun the budget unnoticed (internal #117).
    let mut handed = None;
    let before = Instant::now();
    let r = engine.ocr_with(&page, &opts, |deadline, _| {
        handed = Some((deadline, Instant::now()));
        None
    });
    let (deadline, called) = handed.expect("the screen step was not called");
    assert!(
        before + timeout <= deadline && deadline <= called + timeout,
        "the step's deadline is {:?} after the call began, budget {timeout:?}",
        deadline.saturating_duration_since(before)
    );
    assert!(
        matches!(r, Err(OcrError::Page(OcrPageError::Timeout(d))) if d == timeout),
        "{r:?}"
    );
    assert_eq!(backend.0.load(Ordering::SeqCst), 0, "the backend was asked");
    // The step ocr() passes keeps the screen's verdict, and the page is
    // recognised.
    assert!(engine.ocr(&page, &opts).is_ok());
    assert_eq!(backend.0.load(Ordering::SeqCst), 1);
}

#[test]
fn a_screen_within_the_budget_keeps_its_verdict() {
    let deadline = Instant::now() + Duration::from_secs(3600);
    assert!(matches!(
        screen_within(deadline, || Screened::Skipped),
        Some(Screened::Skipped)
    ));
    assert!(matches!(
        screen_within(deadline, || Screened::AsRendered),
        Some(Screened::AsRendered)
    ));
}

#[test]
fn boxes_are_scaled_to_points_and_flipped() {
    // A 144 dpi raster of 200 x 100 px is 100 x 50 pt.
    let image = GrayImage {
        width: 200,
        height: 100,
        data: vec![255; 200 * 100],
    };
    let out = OcrOutput {
        blocks: vec![
            OcrBlock {
                lines: vec![
                    OcrLine {
                        words: vec![word("Hi", [20, 10, 60, 30]), word("you", [80, 10, 140, 30])],
                    },
                    // Dropped: no words.
                    OcrLine { words: vec![] },
                ],
            },
            OcrBlock {
                lines: vec![OcrLine {
                    words: vec![word("", [0, 0, 1, 1]), word("x", [0, 60, 20, 80])],
                }],
            },
            // Dropped: no lines.
            OcrBlock { lines: vec![] },
        ],
    };
    let page = build_page(&out, raster(&image, 144, 1), 144, Vec::new());
    assert_eq!(page.dpi(), Some(144));
    assert_eq!(page.to_text(BlockSeparator::Newline), "Hi you\nx\n");
    assert_eq!(page.to_text(BlockSeparator::BlankLine), "Hi you\n\nx\n");
    let blocks = page.blocks();
    assert_eq!(blocks.len(), 2);
    let line = &blocks[0].lines()[0];
    // x: 20 px → 10 pt; 140 px → 70 pt. y: top 10 px → 50 − 5 = 45 pt,
    // bottom 30 px → 50 − 15 = 35 pt.
    assert_eq!(line.bbox(), [10.0, 35.0, 70.0, 45.0]);
    let first = &line.chars()[0];
    assert_eq!(first.text(), "H");
    assert_eq!(first.origin().0, 10.0);
}

#[test]
fn an_empty_output_is_an_empty_page() {
    let image = GrayImage {
        width: 1,
        height: 1,
        data: vec![255],
    };
    let page = build_page(
        &OcrOutput::default(),
        raster(&image, 300, 1),
        300,
        Vec::new(),
    );
    assert_eq!(page.to_text(BlockSeparator::Newline), "");
    assert!(page.blocks().is_empty());
}

#[test]
fn a_halved_raster_maps_back_with_the_exact_factor() {
    // Rendered at 301 dpi, 3509 px high; recognised at 150 dpi from the
    // 2 x 2 box-filtered raster. A backend pixel is two rendered pixels, so
    // a box is placed by 2 x 72 / 301 pt per pixel (not 72 / 150), and the
    // flip uses the rendered height.
    let out = OcrOutput {
        blocks: vec![OcrBlock {
            lines: vec![OcrLine {
                words: vec![word("Exhibit", [100, 100, 200, 120])],
            }],
        }],
    };
    let r = Raster {
        height: 3509,
        dpi: 301,
        scale: 2,
    };
    let page = build_page(&out, r, 150, Vec::new());
    assert_eq!(page.dpi(), Some(150));
    let per_px = 72.0f32 / 301.0;
    let height = 3509.0 * per_px;
    let bbox = page.blocks()[0].lines()[0].bbox();
    assert_eq!(
        bbox,
        [
            200.0 * per_px,
            height - 240.0 * per_px,
            400.0 * per_px,
            height - 200.0 * per_px
        ]
    );
    // At scale 1 the same box is half the size from the same origin.
    let page = build_page(&out, Raster { scale: 1, ..r }, 301, Vec::new());
    let one = page.blocks()[0].lines()[0].bbox();
    assert_eq!(one[0], 100.0 * per_px);
    // Pixel coordinates near u32::MAX do not overflow when doubled.
    let far = OcrOutput {
        blocks: vec![OcrBlock {
            lines: vec![OcrLine {
                words: vec![word("x", [u32::MAX - 1, 0, u32::MAX, 1])],
            }],
        }],
    };
    let page = build_page(&far, r, 150, Vec::new());
    assert!(page.blocks()[0].lines()[0].bbox()[2].is_finite());
}

#[test]
fn the_screen_takes_one_step_at_most() {
    let mut noise = GrayImage {
        width: 400,
        height: 300,
        data: vec![255; 400 * 300],
    };
    pdf_extract_testkit::corpus_d::add_noise(&mut noise.data, 7);
    // Below the floor nothing is looked at.
    assert!(matches!(screen(&noise, 143), Screened::AsRendered));
    // Pure noise: speckle at both resolutions.
    assert!(matches!(screen(&noise, 144), Screened::Skipped));
    // Clear: as rendered.
    let white = GrayImage {
        width: 400,
        height: 300,
        data: vec![255; 400 * 300],
    };
    assert!(matches!(screen(&white, 300), Screened::AsRendered));
    // Speckle but too small to halve: as rendered.
    let line = GrayImage {
        width: 8,
        height: 1,
        data: vec![0, 255, 0, 255, 0, 255, 0, 255],
    };
    assert_eq!(precheck::classify(&line), Verdict::Speckle);
    assert!(matches!(screen(&line, 300), Screened::AsRendered));
    // The warning texts (F14).
    let w = noise_warning(4, &Screened::Skipped, 300);
    assert_eq!(
        w.to_string(),
        "ocr: noise-dominated raster skipped (no convincing foreground at 300 or 150 dpi)"
    );
    assert_eq!(w.page, Some(4));
    let w = noise_warning(0, &Screened::Halved(white), 301);
    assert_eq!(
        w.to_string(),
        "ocr: noise-dominated raster recognised at 150 dpi (rendered at 301)"
    );
    assert_eq!(w.kind, WarningKind::OcrNoiseDownsampled);
}
