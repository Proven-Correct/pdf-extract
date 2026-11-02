//! Helpers shared by the pdf-extract-ocr integration tests: the model
//! directory, scratch directories, test pages and the error-rate metric.

// Each test crate includes this module and uses only part of it.
#![allow(dead_code)]

pub mod adversarial;
pub mod timing;

use std::path::{Path, PathBuf};

use pdf_extract_core::{Document, Limits, OpenOptions};
use pdf_extract_ocr::{Candidates, OcrConfig};
use pdf_extract_testkit::{PdfBuilder, TextStyle};

/// The directory holding `eng.traineddata` for recognition tests.
///
/// CI exports `TESSDATA_PREFIX` pointing at the pinned `tessdata_fast`
/// model (spec D-17); locally the first known location that has it. No model
/// is a failure, not a skip: a silently skipped OCR suite would look green.
pub fn tessdata() -> PathBuf {
    Candidates::from_env()
        .ordered()
        .into_iter()
        .find(|d| d.join("eng.traineddata").is_file())
        .unwrap_or_else(|| {
            panic!(
                "no eng.traineddata found: set TESSDATA_PREFIX to a directory holding it \
                 (CI installs tessdata_fast; see .github/actions/tessdata)"
            )
        })
}

/// Candidates that find nothing: no environment, no known directories.
pub fn nowhere() -> Candidates {
    Candidates {
        prefix: None,
        known: Vec::new(),
    }
}

/// A configuration reading the test model explicitly.
pub fn config() -> OcrConfig {
    OcrConfig {
        tessdata: Some(tessdata()),
        ..OcrConfig::default()
    }
}

/// A fresh, empty directory unique to this process and `name`.
pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pxo-ocr-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir scratch");
    dir
}

/// Makes `dir/eng.traineddata` a symlink to the test model (no 4–23 MB
/// copy).
#[cfg(unix)]
pub fn link_model(dir: &Path) {
    std::fs::create_dir_all(dir).expect("mkdir");
    std::os::unix::fs::symlink(
        tessdata().join("eng.traineddata"),
        dir.join("eng.traineddata"),
    )
    .expect("symlink model");
}

/// The body text of [`body_page`], one entry per printed line.
pub const BODY: [&str; 8] = [
    "The quick brown fox jumps over the lazy dog near the river bank.",
    "Pack my box with five dozen liquor jugs before the summer ends.",
    "A journey of a thousand miles begins with a single small step.",
    "Bright vixens jump; dozy fowl quack while the farmer sleeps late.",
    "Every morning the market opens at seven and closes after noon.",
    "Numbers such as 2026, 3.14 and 42 appear in ordinary documents.",
    "How vexingly quick daft zebras jump over the wooden fence today.",
    "The final line of this paragraph ends the clean body sample.",
];

/// An A4 page of [`BODY`] in 11 pt Helvetica: the stand-in for corpus D's
/// `clean_body_11pt` page, which unit 12 commits (rendered here by the
/// renderer under test from vector text).
pub fn body_page() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    for (i, line) in BODY.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let y = 100.0 + 16.0 * i as f32;
        page.insert_text((72.0, y), line, TextStyle::size(11.0))
            .expect("WinAnsi text");
    }
    b.to_bytes()
}

/// The bytes of committed corpus-D file `corpus-d/<name>.pdf` (spec
/// 0002 §3.8): an image-only page rasterised by the oracle.
pub fn corpus_d(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/corpus-d")
        .join(format!("{name}.pdf"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The ground truth of corpus-D file `name`.
pub fn corpus_d_truth(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/corpus-d")
        .join(format!("{name}.txt"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Corpus D's twelve clean pages (Appendix C), one per test-kit source.
pub fn corpus_d_clean() -> Vec<&'static str> {
    pdf_extract_testkit::corpus_d::sources()
        .iter()
        .map(|s| s.name)
        .collect()
}

/// A blank US Letter page.
pub fn blank_page() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    b.add_page(612.0, 792.0);
    b.to_bytes()
}

/// A page densely filled with 8 pt text: enough work that Tesseract cannot
/// finish within a millisecond.
pub fn dense_page() -> Vec<u8> {
    let mut b = PdfBuilder::new();
    let page = b.add_page_a4();
    for i in 0..80 {
        #[allow(clippy::cast_precision_loss)]
        let y = 30.0 + 10.0 * i as f32;
        page.insert_text((20.0, y), BODY[i % BODY.len()], TextStyle::size(8.0))
            .expect("WinAnsi text");
    }
    b.to_bytes()
}

/// Opens `bytes` under `limits`.
pub fn open(bytes: Vec<u8>, limits: Limits) -> Document {
    let opts = OpenOptions {
        limits,
        ..OpenOptions::default()
    };
    Document::from_bytes(bytes, &opts).expect("test document opens")
}

/// Opens `bytes` with default limits.
pub fn doc(bytes: Vec<u8>) -> Document {
    open(bytes, Limits::default())
}

/// Collapses runs of whitespace to one space and trims (O34's
/// normalisation; the test texts are ASCII, so NFC is the identity).
fn normalise(s: &str) -> Vec<char> {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .collect()
}

/// Character error rate of `got` against `truth` (O34).
pub fn cer(got: &str, truth: &str) -> f64 {
    let (a, b) = (normalise(got), normalise(truth));
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != cb);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    #[allow(clippy::cast_precision_loss)]
    let rate = prev[b.len()] as f64 / b.len().max(1) as f64;
    rate
}

/// The directory holding the helper binary cargo built for these tests.
pub fn helper_dir() -> PathBuf {
    Path::new(env!("CARGO_BIN_EXE_pdf-extract-ocr-helper"))
        .parent()
        .expect("helper has a directory")
        .to_path_buf()
}

/// [`config`] with process isolation, finding the helper through
/// `helper_dir` (spec O28).
pub fn process_config() -> OcrConfig {
    OcrConfig {
        isolation: pdf_extract_ocr::Isolation::Process,
        helper_dir: Some(helper_dir()),
        ..config()
    }
}

/// Corpus D's `sparse_below_min_chars_noise` page (built from the committed
/// clean file, as the quality gate builds it): the one corpus-D page the
/// noise pre-check downsamples at 300 dpi (spec 0003 F16).
pub fn sparse_noise_page() -> Vec<u8> {
    pdf_extract_testkit::corpus_d::noise_variant(&corpus_d("sparse_below_min_chars"))
        .expect("noise variant builds")
}

/// A one-page A4 PDF whose image is corpus D's noise field on white and
/// nothing else: speckle at 300 dpi and at 150, so the pre-check skips it
/// (spec 0003 §4.2).
pub fn pure_noise_pdf() -> Vec<u8> {
    use pdf_extract_testkit::corpus_d::{pure_noise_page, scan_pdf};
    scan_pdf(&[pure_noise_page()])
}
