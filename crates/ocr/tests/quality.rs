//! The OCR quality gate on corpus D (spec 0002 O34, §3.8, §4.12; design
//! spec §13.4), run by the `corpus-d` CI job and by developers with
//! `cargo test --release -p pdf-extract-ocr --test quality -- --include-ignored --nocapture`.
//!
//! For each of the 52 corpus-D pages (40 committed, 12 noise variants built
//! here by the test kit and checked against their `[[built]]` pins first)
//! it recognises the page at 72 and at 300 dpi, scores both and the oracle's
//! committed 72-dpi text against the ground truth (CER/WER per D-10), and
//! applies O34 over the per-subset means:
//!
//! - (a) clean: ours@72 <= oracle@72 + 0.01;
//! - (b) clean: ours@300 < min(ours@72, oracle@72);
//! - (c) noise, skew, blur: (a) only.
//!
//! The low-resolution and JPEG pages are scored and reported but not gated:
//! O34 names no gate for them. WER, the order-insensitive bag-of-words F1
//! (`metrics::bow_f1`, B24: gate (a) is reading-order sensitive), latency,
//! the process-isolation overhead (Q1) and RSS are reported, never asserted
//! (ADR 13): shared CI runners make timing noise, not signal. Everything
//! lands in
//! `target/ocr-report.json`.
//!
//! The committed numbers hold for one model only, tessdata_fast
//! `eng.traineddata` (D-17); an expectation made with a different model than
//! the one in use fails the test, because the comparison would be
//! meaningless. That is one upfront failure (`quality/model_pin.rs`) naming
//! the directory found, both SHAs and the `TESSDATA_PREFIX` remedy, ahead of
//! the per-page check, which would otherwise repeat the mismatch once per
//! page without saying why. With no model at all the test skips with a
//! message.
//!
//! Measurement mode (unit 13, O35): with `PDF_EXTRACT_TESSDATA=<dir>` the
//! test scores whatever `eng.traineddata` that directory holds, in
//! report-only form: the model pin check, the gates and the process-isolation
//! pass are skipped, `Init` latency is recorded, and the report goes to
//! `target/ocr-report-measure.json` so the pinned gate's report is never
//! overwritten. `tools/oracle/measure_tessdata.sh` drives it. The variable
//! only ever relaxes checks when it is set; CI does not set it.
//!
//! The noise pre-check (spec 0003 F16, F17, §4.2): with the pinned model,
//! at 300 dpi only `sparse_below_min_chars_noise` carries an `ocr:` warning
//! (recognised at 150 dpi, CER ≤ 0.05, under 5 s: a functional stall-vs-fixed
//! bound, the stall being 16–25 s), and at 72 dpi no page does. The report
//! gains, per page and resolution, the pre-check's verdict and statistics
//! and its time (`precheck_ms`, measured here on a separate render of the
//! page: report-only). The process-isolation pass also covers the noise
//! subset, whose text must equal the in-process text byte for byte; and the
//! clean subset is recognised a second time with the pre-check off, which
//! must give byte-identical text (on a clean page the pre-check changes
//! nothing). The adversarial pages of Appendix A.6 are pinned too: a page
//! with an inverted upper half and one with a gray figure are read in full
//! at 300 dpi, the inverted sparse noise page is recognised at 150 dpi, and
//! the inverted noise field is skipped, both in under 5 s.
//!
//! `PDF_EXTRACT_NOISE_PRECHECK=0`, a switch of this test only (not a library
//! variable), runs everything with the pre-check off — run-0002 behaviour —
//! and skips the pins that describe the pre-check; it checks that the
//! stalling page is then recognised at 300 dpi with no `ocr:` warning, and
//! writes `ocr-report-precheck-off.json` with 0002 §4.12's CER means beside
//! the measured ones (report-only: the numbers are host-dependent, D-12).
//!
//! `private_scans` (in `quality/private.rs`) is the corpus-A counterpart,
//! driven by `tools/oracle/run_private_scans.sh`.

mod common;
#[path = "quality/metrics.rs"]
mod metrics;
#[path = "quality/model_pin.rs"]
mod model_pin;
#[path = "quality/private.rs"]
mod private;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pdf_extract_core::{BlockSeparator, Document, OpenOptions};
use pdf_extract_ocr::precheck::{self, OtsuClass, Verdict};
use pdf_extract_ocr::{Candidates, Isolation, OcrConfig, OcrEngine, OcrOptions};
use pdf_extract_render::{RenderOptions, render_gray};
use pdf_extract_testkit::corpus_d;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use metrics::{bow_f1, cer, mean, wer};

/// The gate tolerance of O34(a): one CER point.
const TOLERANCE: f64 = 0.01;
/// Subsets gated by (a) and (b), and by (a) only.
const GATED_AB: &[&str] = &["clean"];
const GATED_A: &[&str] = &["noise", "skew", "blur"];
/// Every subset, in report order.
const SUBSETS: &[&str] = &["clean", "noise", "skew", "blur", "lowres", "jpeg"];
/// The one corpus-D page the noise pre-check acts on (spec 0003 F16).
const SPECKLED: &str = "sparse_below_min_chars_noise";
/// F16's bounds on that page at 300 dpi: CER, and wall-clock time (a
/// stall-vs-fixed bound: ≈ 20 ms fixed, 16–25 s stalled).
const SPECKLED_MAX_CER: f64 = 0.05;
const SPECKLED_MAX_MS: f64 = 5_000.0;

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

pub fn target_dir() -> PathBuf {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace.join("target"), |d| workspace.join(d))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The model directory the engine would use (`TESSDATA_PREFIX`, then the
/// known locations), or `None` when nothing holds `eng.traineddata`.
pub fn model_dir() -> Option<PathBuf> {
    Candidates::from_env()
        .ordered()
        .into_iter()
        .find(|d| d.join("eng.traineddata").is_file())
}

/// Peak resident set size of this process so far, in bytes.
pub fn peak_rss() -> u64 {
    // SAFETY: getrusage writes into the zeroed struct we pass and nothing
    // else; RUSAGE_SELF is always valid.
    let usage = unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &raw mut u);
        u
    };
    let raw = u64::try_from(usage.ru_maxrss).unwrap_or(0);
    // Linux reports KiB, macOS bytes.
    if cfg!(target_os = "macos") {
        raw
    } else {
        raw * 1024
    }
}

/// The oracle's committed expectation for the file with hash `sha`.
pub fn expectation(dir: &Path, sha: &str) -> Value {
    let path = dir.join(format!("{sha}.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (run tools/oracle/gen.py --ocr)", path.display()));
    serde_json::from_str(&text).expect("expectation JSON")
}

/// One scored corpus-D page.
struct Doc {
    name: String,
    subset: String,
    path: PathBuf,
    sha256: String,
    truth: String,
}

fn str_of<'a>(row: &'a toml::Value, key: &str) -> &'a str {
    row.get(key)
        .and_then(toml::Value::as_str)
        .unwrap_or_else(|| panic!("manifest row without `{key}`: {row:?}"))
}

/// The 40 committed files and the 12 built noise variants, the latter
/// built (or reused) now and required to match their pins.
fn corpus() -> Vec<Doc> {
    let root = fixtures_dir();
    let text = std::fs::read_to_string(root.join("manifest.toml")).expect("manifest");
    let manifest: toml::Table = toml::from_str(&text).expect("manifest parses");
    let rows = |key: &str| -> Vec<toml::Value> {
        manifest
            .get(key)
            .and_then(toml::Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let truth = |rel: &str| std::fs::read_to_string(root.join(rel)).expect("ground truth");
    let mut docs = Vec::new();
    for row in rows("fixture") {
        if row.get("gate").and_then(toml::Value::as_str) != Some("ocr") {
            continue;
        }
        let rel = str_of(&row, "path");
        docs.push(Doc {
            name: rel
                .trim_start_matches("corpus-d/")
                .trim_end_matches(".pdf")
                .to_owned(),
            subset: str_of(&row, "subset").to_owned(),
            path: root.join(rel),
            sha256: str_of(&row, "sha256").to_owned(),
            truth: truth(str_of(&row, "truth")),
        });
    }
    let built_rows = rows("built");
    let pins: Vec<(&str, &str)> = built_rows
        .iter()
        .map(|r| (str_of(r, "path"), str_of(r, "sha256")))
        .collect();
    let built = corpus_d::build_noise(&root, &corpus_d::default_build_dir(), &pins)
        .expect("noise variants build");
    let mut drift = String::new();
    for b in &built {
        if b.pinned.as_deref() != Some(b.sha256.as_str()) {
            let _ = writeln!(
                drift,
                "{}: built {}, pinned {:?}",
                b.path.display(),
                b.sha256,
                b.pinned
            );
        }
    }
    assert!(
        drift.is_empty(),
        "corpus-D noise variants differ from their manifest.toml [[built]] pins; the committed \
         expectations belong to other bytes:\n{drift}Re-pin (spec §3.8): cargo run -p \
         pdf-extract-testkit --example corpus_d_noise -- DIR; then in tools/oracle: \
         mkcorpusd.py --pin-built DIR; gen.py --ocr --tessdata .cache/tessdata_fast --built DIR \
         ../../tests/fixtures/corpus-d; mkcorpusd.py --verify --built DIR"
    );
    for (row, b) in built_rows.iter().zip(&built) {
        assert_eq!(format!("{}.pdf", b.name), str_of(row, "path"), "pin order");
        docs.push(Doc {
            name: b.name.clone(),
            subset: str_of(row, "subset").to_owned(),
            path: b.path.clone(),
            sha256: b.sha256.clone(),
            truth: truth(str_of(row, "truth")),
        });
    }
    docs
}

/// One recognised page.
pub struct Recognised {
    /// `to_text(Newline)`.
    pub text: String,
    /// Wall-clock milliseconds of the `ocr` call (render, pre-check and
    /// recognition).
    pub ms: f64,
    /// The `TextPage`'s resolution.
    pub dpi: Option<u32>,
    /// Its `ocr:` warnings, as displayed.
    pub ocr_warnings: Vec<String>,
}

/// Recognises page `page` of `path` at `dpi`.
pub fn recognise_page(
    engine: &OcrEngine,
    path: &Path,
    page: usize,
    dpi: u32,
) -> Result<Recognised, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    recognise_bytes(engine, bytes, page, dpi)
}

/// Recognises page `page` of the PDF `bytes` at `dpi`.
pub fn recognise_bytes(
    engine: &OcrEngine,
    bytes: Vec<u8>,
    page: usize,
    dpi: u32,
) -> Result<Recognised, String> {
    let doc = Document::from_bytes(bytes, &OpenOptions::default()).map_err(|e| e.to_string())?;
    let page = doc.page(page).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let tp = engine
        .ocr(&page, &OcrOptions { dpi: Some(dpi) })
        .map_err(|e| e.to_string())?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(Recognised {
        text: tp.to_text(BlockSeparator::Newline),
        ms,
        dpi: tp.dpi(),
        ocr_warnings: tp
            .warnings()
            .iter()
            .map(ToString::to_string)
            .filter(|w| w.starts_with("ocr:"))
            .collect(),
    })
}

/// Recognises page 0 of `path` at `dpi`; returns the text and the
/// wall-clock milliseconds of the `ocr` call (render + recognition).
pub fn recognise(
    engine: &OcrEngine,
    path: &Path,
    page: usize,
    dpi: u32,
) -> Result<(String, f64), String> {
    recognise_page(engine, path, page, dpi).map(|r| (r.text, r.ms))
}

/// The noise pre-check on page 0 of `path` rendered at `dpi`, as the engine
/// runs it (spec 0003 F12, F13), for the report: its statistics, the
/// action an engine with the pre-check on takes (whether or not this run's
/// engine has it on), and the time of the check alone (the render is
/// not counted). Report-only.
fn precheck_report(path: &Path, dpi: u32) -> Value {
    let doc = Document::open(path, &OpenOptions::default()).expect("opens");
    let page = doc.page(0).expect("page 0");
    let rendered = render_gray(
        &page,
        &RenderOptions {
            dpi,
            max_render_pixels: OcrConfig::default().max_render_pixels,
            render_annotations: true,
        },
    )
    .expect("renders");
    let start = Instant::now();
    let stats = precheck::stats(&rendered.image);
    let action = if rendered.dpi < precheck::MIN_DPI || stats.verdict() == Verdict::Clear {
        "as_rendered"
    } else {
        match precheck::downsample2(&rendered.image) {
            Some(half) if precheck::classify(&half) == Verdict::Clear => "downsampled",
            Some(_) => "skipped",
            None => "as_rendered",
        }
    };
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    let class = match stats.class {
        OtsuClass::Empty => "empty",
        OtsuClass::ForegroundDark => "foreground_dark",
        OtsuClass::BackgroundDark => "background_dark",
        OtsuClass::None => "none",
    };
    json!({
        "verdict": format!("{:?}", stats.verdict()).to_lowercase(),
        "action_with_precheck": action,
        "ms": ms,
        "threshold": stats.threshold,
        "class": class,
        "foreground_dark": stats.foreground_dark,
        "foreground": stats.foreground,
        "isolation": stats.isolation,
    })
}

/// Spec 0003 F16 on one page: at 300 dpi only the speckled page carries
/// an `ocr:` warning — one, saying it was recognised at 150 dpi — with
/// `dpi() == 150`, CER ≤ 0.05 and the call under 5 s; at 72 dpi no page
/// carries one.
fn check_f16(
    name: &str,
    truth: &str,
    r72: &Recognised,
    r300: &Recognised,
    failures: &mut Vec<String>,
) {
    if !r72.ocr_warnings.is_empty() {
        failures.push(format!(
            "F16 {name}: ocr warnings at 72 dpi: {:?}",
            r72.ocr_warnings
        ));
    }
    if name != SPECKLED {
        if !r300.ocr_warnings.is_empty() {
            failures.push(format!(
                "F16 {name}: ocr warnings at 300 dpi: {:?}",
                r300.ocr_warnings
            ));
        }
        return;
    }
    let expected = "ocr: noise-dominated raster recognised at 150 dpi (rendered at 300)";
    if r300.ocr_warnings != [expected] {
        failures.push(format!(
            "F16 {name}: expected one {expected:?}, got {:?}",
            r300.ocr_warnings
        ));
    }
    if r300.dpi != Some(150) {
        failures.push(format!("F16 {name}: dpi {:?}, expected 150", r300.dpi));
    }
    let rate = cer(&r300.text, truth);
    if rate > SPECKLED_MAX_CER {
        failures.push(format!(
            "F16 {name}: CER {rate:.4} at 300 dpi > {SPECKLED_MAX_CER}: {:?}",
            r300.text
        ));
    }
    if r300.ms >= SPECKLED_MAX_MS {
        failures.push(format!(
            "F16 {name}: {:.0} ms at 300 dpi, not under {SPECKLED_MAX_MS} (the stall is back?)",
            r300.ms
        ));
    }
}

/// 0002 §4.12's CER means (`oracle@72`, `ours@72`, `ours@300`), measured
/// by unit 12 before the pre-check existed: what a switch-off run should
/// reproduce, up to host differences (report-only).
fn reference_0002() -> Value {
    json!({
        "clean": [0.1331, 0.0315, 0.0084],
        "noise": [0.1289, 0.0383, 0.0863],
        "skew": [0.1551, 0.0590, 0.0037],
        "blur": [0.1276, 0.0491, 0.0085],
        "sparse_below_min_chars_noise_cer300": 0.93,
    })
}

/// Spec 0003 §4.2's Tesseract pins on the adversarial pages of Appendix
/// A.6, recognised at 300 dpi from one-page PDFs whose render is the
/// transformed raster. Returns the report rows.
fn adversarial_pins(engine: &OcrEngine, failures: &mut Vec<String>) -> Value {
    use common::adversarial::{corpus300, flat, invert, render300, to_pdf};
    let sparse_truth =
        std::fs::read_to_string(fixtures_dir().join("corpus-d/sparse_below_min_chars.txt"))
            .expect("ground truth");
    let body = corpus300("clean_body_11pt");
    let cases = [
        ("inverted upper 45 %", invert(body.clone(), 0, 45)),
        ("gray figure 40-70 %", flat(body, 128, 40, 70)),
        (
            "inverted sparse noise",
            invert(render300(common::sparse_noise_page()), 0, 100),
        ),
        (
            "inverted noise field",
            invert(render300(common::pure_noise_pdf()), 0, 100),
        ),
    ];
    let mut rows = Vec::new();
    for (name, image) in cases {
        let r = match recognise_bytes(engine, to_pdf(&image), 0, 300) {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("A.6 {name}: OCR failed: {e}"));
                continue;
            }
        };
        let chars = r.text.chars().filter(|c| !c.is_whitespace()).count();
        let mut fail = |what: String| failures.push(format!("A.6 {name}: {what}"));
        match name {
            "inverted upper 45 %" | "gray figure 40-70 %" => {
                let floor = if name.starts_with("inverted") {
                    2_500
                } else {
                    1_400
                };
                if chars < floor {
                    fail(format!("{chars} characters, expected at least {floor}"));
                }
                if r.dpi != Some(300) || !r.ocr_warnings.is_empty() {
                    fail(format!("dpi {:?}, warnings {:?}", r.dpi, r.ocr_warnings));
                }
            }
            "inverted sparse noise" => {
                let expected =
                    "ocr: noise-dominated raster recognised at 150 dpi (rendered at 300)";
                let rate = cer(&r.text, &sparse_truth);
                if r.dpi != Some(150) || r.ocr_warnings != [expected] || rate > SPECKLED_MAX_CER {
                    fail(format!(
                        "dpi {:?}, warnings {:?}, CER {rate:.4}: {:?}",
                        r.dpi, r.ocr_warnings, r.text
                    ));
                }
            }
            _ => {
                let expected = "ocr: noise-dominated raster skipped (no convincing foreground at \
                                300 or 150 dpi)";
                if !r.text.is_empty() || r.ocr_warnings != [expected] {
                    fail(format!("text {:?}, warnings {:?}", r.text, r.ocr_warnings));
                }
            }
        }
        if name.contains("noise") && r.ms >= SPECKLED_MAX_MS {
            fail(format!("{:.0} ms, not under {SPECKLED_MAX_MS}", r.ms));
        }
        rows.push(json!({
            "name": name,
            "chars": chars,
            "dpi": r.dpi,
            "ocr_warnings": r.ocr_warnings,
            "ms": r.ms,
        }));
    }
    Value::Array(rows)
}

/// Per-subset means of one metric column.
fn means(rows: &[Value], subset: &str, key: &str) -> f64 {
    let v: Vec<f64> = rows
        .iter()
        .filter(|r| r["subset"] == subset)
        .map(|r| r[key].as_f64().expect("number"))
        .collect();
    mean(&v)
}

#[test]
#[ignore = "needs tessdata (CI job corpus-d); run with --include-ignored"]
fn corpus_d_quality() {
    let measure = std::env::var_os("PDF_EXTRACT_TESSDATA").map(PathBuf::from);
    let dir = match &measure {
        Some(d) if d.join("eng.traineddata").is_file() => Some(d.clone()),
        Some(d) => panic!(
            "PDF_EXTRACT_TESSDATA={} has no eng.traineddata",
            d.display()
        ),
        None => model_dir(),
    };
    let Some(dir) = dir else {
        // The gate SHALL run on CI (output-parity spec), so there a missing
        // model fails instead of skipping green. Same `CI` convention as
        // tests/python/ocrkit.py (`_required`) and tests/common/mod.rs
        // (`tessdata`); locally, without the model, it stays a skip.
        let msg = "quality: no eng.traineddata found; set TESSDATA_PREFIX to the pinned \
                   tessdata_fast model (tools/oracle/mkcorpusd.py --fetch-model)";
        assert!(
            std::env::var_os("CI").is_none_or(|v| v.is_empty()),
            "{msg}: a skipped quality gate would look green under CI"
        );
        eprintln!("SKIPPED - {msg}");
        return;
    };
    let model_sha = sha256_hex(&std::fs::read(dir.join("eng.traineddata")).expect("model"));
    // Test-only switch (spec 0003 §4.2): run-0002 behaviour, no F16 pins.
    let noise_precheck = std::env::var_os("PDF_EXTRACT_NOISE_PRECHECK").is_none_or(|v| v != "0");
    let docs = corpus();
    assert_eq!(docs.len(), 52, "corpus D has 52 pages (Appendix C)");
    let expectations = fixtures_dir().join("expectations");
    if measure.is_none() {
        // One failure that names the cause, ahead of the per-page check below
        // (which this makes unreachable for a mismatch, and stays as the
        // guard should the two ever diverge).
        let pinned: std::collections::BTreeSet<String> = docs
            .iter()
            .map(|d| {
                let exp = expectation(&expectations, &d.sha256);
                exp["ocr"]["tessdata_sha256"]
                    .as_str()
                    .unwrap_or("<none>")
                    .to_owned()
            })
            .collect();
        let source = model_pin::model_source(&dir, &Candidates::from_env());
        let remedy = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("crates/ocr sits two levels below the workspace root")
            .join("tools/oracle/.cache/tessdata_fast");
        if let Some(msg) = model_pin::model_pin_failure(&model_sha, &pinned, &dir, &source, &remedy)
        {
            panic!("{msg}");
        }
    }

    let rss_before = peak_rss();
    let config = OcrConfig {
        tessdata: Some(dir.clone()),
        noise_precheck,
        ..OcrConfig::default()
    };
    let nowhere = common::nowhere();
    let init_start = Instant::now();
    let engine = OcrEngine::tesseract_with_candidates(config.clone(), &nowhere).expect("engine");
    let init_ms = init_start.elapsed().as_secs_f64() * 1000.0;
    let rss_after_init = peak_rss();
    // Measurement mode skips the process engine: it is Q1's concern, and a
    // second engine would blur the per-instance RSS figure.
    let process = measure.is_none().then(|| {
        OcrEngine::tesseract_with_candidates(
            OcrConfig {
                isolation: Isolation::Process,
                helper_dir: Some(common::helper_dir()),
                ..config.clone()
            },
            &nowhere,
        )
        .expect("process engine")
    });
    // F17 on the clean subset: the pre-check off gives the same text as on
    // (on a clean page it changes nothing). Gate mode with the pre-check on
    // only; otherwise the two engines would be the same.
    let unchecked = (measure.is_none() && noise_precheck).then(|| {
        OcrEngine::tesseract_with_candidates(
            OcrConfig {
                noise_precheck: false,
                ..config.clone()
            },
            &nowhere,
        )
        .expect("engine without the pre-check")
    });
    // F16 pins: gate mode with the pre-check on.
    let pins = measure.is_none() && noise_precheck;

    let mut failures = Vec::new();
    let mut rows = Vec::new();
    for d in &docs {
        let exp = expectation(&expectations, &d.sha256);
        let ocr = &exp["ocr"];
        let used = ocr["tessdata_sha256"].as_str().unwrap_or("<none>");
        if measure.is_none() && used != model_sha {
            failures.push(format!(
                "{}: the expectation was made with model {used}, the engine uses {model_sha} \
                 (corpus D's numbers hold for tessdata_fast only, spec D-17)",
                d.name
            ));
            continue;
        }
        let oracle = ocr["pages"][0]["text"].as_str().expect("oracle OCR text");
        // Sequential on purpose: one page at a time keeps the latencies
        // comparable (they are still report-only).
        let (r72, r300) = (
            recognise_page(&engine, &d.path, 0, 72),
            recognise_page(&engine, &d.path, 0, 300),
        );
        let (r72, r300) = match (r72, r300) {
            (Ok(a), Ok(b)) => (a, b),
            (a, b) => {
                failures.push(format!(
                    "{}: OCR failed: {:?} {:?}",
                    d.name,
                    a.err(),
                    b.err()
                ));
                continue;
            }
        };
        let (t72, ms72, t300, ms300) = (&r72.text, r72.ms, &r300.text, r300.ms);
        // Q1: the cost of process isolation, on the clean pages; the noise
        // pages are recognised too, for parity with the pre-check on
        // (spec 0003 F17), but not timed into the overhead.
        // The text/dpi/warnings comparison below overlaps O26 in
        // tests/process_parity.rs for the clean pages and the noise page it
        // names. It stays on purpose (maintainer decision, internal #55 B25): this loop
        // is the only place the other noise variants are checked for
        // process/in-process parity.
        let parity = d.subset == "clean" || d.subset == "noise";
        let ms300_process = if let (true, Some(process)) = (parity, &process) {
            match recognise_page(process, &d.path, 0, 300) {
                Ok(p) => {
                    if p.text != *t300 {
                        failures.push(format!("{}: process text differs from in-process", d.name));
                    }
                    if (p.dpi, &p.ocr_warnings) != (r300.dpi, &r300.ocr_warnings) {
                        failures.push(format!(
                            "{}: process dpi/warnings {:?} {:?} differ from in-process {:?} {:?}",
                            d.name, p.dpi, p.ocr_warnings, r300.dpi, r300.ocr_warnings
                        ));
                    }
                    (d.subset == "clean").then_some(p.ms)
                }
                Err(e) => {
                    failures.push(format!("{}: process OCR failed: {e}", d.name));
                    None
                }
            }
        } else {
            None
        };
        if let (true, Some(unchecked)) = (d.subset == "clean", &unchecked) {
            match recognise(unchecked, &d.path, 0, 300) {
                Ok((t, _)) if t == *t300 => {}
                Ok(_) => failures.push(format!(
                    "{}: text with the pre-check off differs from the text with it on",
                    d.name
                )),
                Err(e) => {
                    failures.push(format!("{}: OCR without the pre-check failed: {e}", d.name))
                }
            }
        }
        let (pre72, pre300) = (precheck_report(&d.path, 72), precheck_report(&d.path, 300));
        if pins {
            check_f16(&d.name, &d.truth, &r72, &r300, &mut failures);
        }
        if measure.is_none() && !noise_precheck && d.name == SPECKLED {
            // F17: the switch restores the stall's outcome (its CER is
            // reported; it is ≈ 0.93 on the development machine).
            if r300.dpi != Some(300) || !r300.ocr_warnings.is_empty() {
                failures.push(format!(
                    "F17 {}: with the pre-check off: dpi {:?}, warnings {:?}",
                    d.name, r300.dpi, r300.ocr_warnings
                ));
            }
        }
        rows.push(json!({
            "name": d.name,
            "subset": d.subset,
            "truth_chars": metrics::normalise(&d.truth).chars().count(),
            "cer_oracle72": cer(oracle, &d.truth),
            "cer_ours72": cer(t72, &d.truth),
            "cer_ours300": cer(t300, &d.truth),
            "wer_oracle72": wer(oracle, &d.truth),
            "wer_ours72": wer(t72, &d.truth),
            "wer_ours300": wer(t300, &d.truth),
            "bow_f1_oracle72": bow_f1(oracle, &d.truth),
            "bow_f1_ours72": bow_f1(t72, &d.truth),
            "bow_f1_ours300": bow_f1(t300, &d.truth),
            "ms72": ms72,
            "ms300": ms300,
            "ms300_process": ms300_process,
            "dpi72": r72.dpi,
            "dpi300": r300.dpi,
            "ocr_warnings72": r72.ocr_warnings,
            "ocr_warnings300": r300.ocr_warnings,
            "precheck_verdict72": pre72["verdict"],
            "precheck_verdict300": pre300["verdict"],
            "precheck_ms72": pre72["ms"],
            "precheck_ms300": pre300["ms"],
            "precheck72": pre72,
            "precheck300": pre300,
        }));
    }
    let adversarial = if pins {
        adversarial_pins(&engine, &mut failures)
    } else {
        Value::Null
    };
    let peak = peak_rss();

    // Per-subset means, the gates and their margins.
    let mut summary = BTreeMap::new();
    let mut gates = Vec::new();
    for &subset in SUBSETS {
        let m = |k: &str| means(&rows, subset, k);
        let (o72, u72, u300) = (m("cer_oracle72"), m("cer_ours72"), m("cer_ours300"));
        summary.insert(
            subset,
            json!({
                "pages": rows.iter().filter(|r| r["subset"] == subset).count(),
                "cer_oracle72": o72, "cer_ours72": u72, "cer_ours300": u300,
                "wer_oracle72": m("wer_oracle72"), "wer_ours72": m("wer_ours72"),
                "wer_ours300": m("wer_ours300"),
                "bow_f1_oracle72": m("bow_f1_oracle72"),
                "bow_f1_ours72": m("bow_f1_ours72"),
                "bow_f1_ours300": m("bow_f1_ours300"),
                "ms72": m("ms72"), "ms300": m("ms300"),
            }),
        );
        if measure.is_some() {
            continue;
        }
        if GATED_AB.contains(&subset) || GATED_A.contains(&subset) {
            let margin = o72 + TOLERANCE - u72;
            gates.push(json!({"gate": "a", "subset": subset, "margin": margin}));
            if margin < 0.0 {
                failures.push(format!(
                    "O34(a) {subset}: ours@72 CER {u72:.4} > oracle@72 {o72:.4} + {TOLERANCE}"
                ));
            }
        }
        if GATED_AB.contains(&subset) {
            let bound = u72.min(o72);
            gates.push(json!({"gate": "b", "subset": subset, "margin": bound - u300}));
            if u300 >= bound {
                failures.push(format!(
                    "O34(b) {subset}: ours@300 CER {u300:.4} is not below min(ours@72 {u72:.4}, \
                     oracle@72 {o72:.4})"
                ));
            }
        }
    }
    let process: Vec<(f64, f64)> = rows
        .iter()
        .filter_map(|r| Some((r["ms300"].as_f64()?, r["ms300_process"].as_f64()?)))
        .collect();
    let (inproc_ms, process_ms) = (
        mean(&process.iter().map(|p| p.0).collect::<Vec<_>>()),
        mean(&process.iter().map(|p| p.1).collect::<Vec<_>>()),
    );
    let report = json!({
        "schema": 1,
        "model_sha256": model_sha,
        "tessdata": dir,
        "summary": summary,
        "gates": gates,
        "process_isolation": {
            "pages": process.len(),
            "ms300_in_process": inproc_ms,
            "ms300_process": process_ms,
            "overhead": if inproc_ms > 0.0 { process_ms / inproc_ms - 1.0 } else { 0.0 },
        },
        "rss": {
            "before_engine_bytes": rss_before,
            "peak_bytes": peak,
            // Sequential use keeps one in-process instance busy; the growth
            // is that instance plus the page buffers.
            "per_instance_bytes": peak.saturating_sub(rss_before),
            "after_init_bytes": rss_after_init,
        },
        "init_ms": init_ms,
        "mode": if measure.is_some() { "measure" } else { "gate" },
        "noise_precheck": noise_precheck,
        "adversarial": adversarial,
        "reference_0002": if noise_precheck { Value::Null } else { reference_0002() },
        "failures": failures,
        "pages": rows,
    });
    let out = target_dir().join(if measure.is_some() {
        "ocr-report-measure.json"
    } else if !noise_precheck {
        "ocr-report-precheck-off.json"
    } else {
        "ocr-report.json"
    });
    std::fs::create_dir_all(target_dir()).expect("target dir");
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&report).expect("json") + "\n",
    )
    .expect("write report");

    eprintln!("subset   pages  oracle@72  ours@72  ours@300  (CER means)  ms@72  ms@300");
    for &subset in SUBSETS {
        let s = &summary[subset];
        let f = |k: &str| s[k].as_f64().unwrap_or(0.0);
        eprintln!(
            "{subset:8} {:5}  {:9.4}  {:7.4}  {:8.4}               {:5.0}  {:6.0}",
            s["pages"],
            f("cer_oracle72"),
            f("cer_ours72"),
            f("cer_ours300"),
            f("ms72"),
            f("ms300")
        );
    }
    // Order-insensitive companion to the CER gates (B24); never asserted.
    eprintln!("subset   pages  bag-of-words F1 (report-only): oracle@72  ours@72  ours@300");
    for &subset in SUBSETS {
        let s = &summary[subset];
        let f = |k: &str| s[k].as_f64().unwrap_or(0.0);
        eprintln!(
            "{subset:8} {:5}  {:29}  {:9.4}  {:7.4}  {:8.4}",
            s["pages"],
            "",
            f("bow_f1_oracle72"),
            f("bow_f1_ours72"),
            f("bow_f1_ours300"),
        );
    }
    eprintln!(
        "process isolation at 300 dpi: {process_ms:.0} ms vs {inproc_ms:.0} ms in process; \
         peak RSS {} MB; report {}",
        peak / 1_000_000,
        out.display()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
