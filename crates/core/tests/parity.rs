//! Gates C and B (spec §4.13, §4.14, D-21): our extraction against the
//! checked-in oracle expectations, with no PyMuPDF anywhere (L2). This is the
//! `parity` CI job.
//!
//! Inputs, all under `tests/fixtures/`: `manifest.toml` (path, hash, gate per
//! fixture), `expectations/<sha256>.json` (the oracle's open outcome and page
//! texts, written once by `tools/oracle/gen.py` from the files on disk) and
//! `snapshots/<sha256>.txt` (our pinned output for `divergence` fixtures).
//! Each fixture is opened from disk, as the oracle opened it, with default
//! options, on a 512 KiB stack (spec §2.3), and compared by its gate:
//!
//! - `exact`: same open outcome class (opened / needs a password / failed)
//!   and, when opened, the same page count and every page's text equal.
//! - `divergence`: our transcript equals the snapshot **and** differs from
//!   the oracle; a divergence that disappears fails, so the manifest gets
//!   cleaned rather than carrying a stale exception.
//! - `hostile` (D-27, spec revision 10): the same open outcome class as
//!   the oracle, our transcript equal to the snapshot, and exactly the
//!   entry's `warnings` kinds; never the oracle's text, which for over-deep
//!   input depends on where the nesting sits.
//! - `similarity` (corpus B) is not judged per file: [`gate_b`] scores every
//!   page of every such fixture together (open rate, OCR-routing agreement,
//!   similarity median and 10th percentile) and writes
//!   `target/parity-report-b.json`; `gate_c` leaves these entries alone.
//!   `skip` is ignored by both.
//!
//! Any entry with a `warnings` list must report exactly those warning
//! kinds, document and pages together.
//!
//! Every run writes `target/parity-report.json` (per-page lengths,
//! Levenshtein distance and similarity via `strsim`), which
//! `tools/oracle/score.py` pretty-prints. `PARITY_BLESS=1` (re)writes the
//! snapshots of divergence and hostile entries from our current output before checking
//! them; it never touches expectations, which only the oracle writes.
//!
//! Snapshot / transcript format: per page `--- page N ---\n` followed by the
//! page text (or `--- page N error: <PageError> ---\n`), `--- no pages ---\n`
//! for a document without pages, `--- open failed: <OpenError variant> ---\n`
//! when opening fails. Equality with the oracle is decided on the structured
//! values, never on transcripts, since the two name failures differently.

mod common;
// A subdirectory, not `tests/gate_b.rs`, which cargo would build as a
// separate test binary.
#[path = "parity/gate_b.rs"]
mod gate_b;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use pdf_extract_core::{Document, OpenError, OpenOptions, TextOptions};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// The PyMuPDF release every expectation must come from (spec §3.6).
const ORACLE_VERSION: &str = "1.28.2";

// Minimum counts per gate in corpus C. These floors guard against silent corpus
// loss: a truncated manifest that passes vacuously (all gates vacuous on zero
// fixtures). Thresholds must be raised deliberately when the corpus grows (a
// decision documented in the ADR); lowering is a spec change.
const GATE_C_EXACT_MIN: usize = 264;
const GATE_C_DIVERGENCE_MIN: usize = 19;
const GATE_C_HOSTILE_MIN: usize = 9;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ---------------------------------------------------------------------------
// On-disk formats (spec §3.6)

#[derive(Debug, Deserialize)]
struct Manifest {
    #[serde(default)]
    fixture: Vec<Entry>,
    /// Corpus D's noise variants (run 0002 §3.8): built at test time by the
    /// test kit, never committed. Only their expectations concern this file.
    #[serde(default)]
    built: Vec<BuiltEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct BuiltEntry {
    path: String,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
struct Entry {
    path: String,
    sha256: String,
    #[serde(default)]
    compat_points: Vec<String>,
    gate: String,
    divergence: Option<String>,
    snapshot: Option<String>,
    warnings: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct Expectation {
    schema: u32,
    sha256: String,
    size: u64,
    source: String,
    oracle: OracleInfo,
    open: OracleOpen,
    page_count: usize,
    pages: Vec<OraclePage>,
    /// Schema 2 (run 0002 §3.8): the oracle's 72-dpi OCR of every page.
    ocr: Option<OracleOcr>,
}

#[derive(Debug, Deserialize)]
struct OracleOcr {
    dpi: u32,
    language: String,
    tessdata_sha256: String,
    pages: Vec<OraclePage>,
}

#[derive(Debug, Deserialize)]
struct OracleInfo {
    pymupdf: String,
}

#[derive(Debug, Deserialize)]
struct OracleOpen {
    ok: bool,
    exception: Option<String>,
    needs_pass: bool,
}

#[derive(Debug, Deserialize)]
struct OraclePage {
    index: usize,
    text: Option<String>,
    sha256: Option<String>,
    error: Option<String>,
}

fn built_rows() -> Vec<BuiltEntry> {
    let text = std::fs::read_to_string(fixtures_dir().join("manifest.toml")).expect("manifest");
    toml::from_str::<Manifest>(&text)
        .expect("manifest parses")
        .built
}

/// The pinned OCR model of every committed OCR expectation (spec D-17).
const OCR_MODEL_SHA256: &str = "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2";

fn manifest() -> Vec<Entry> {
    let text = std::fs::read_to_string(fixtures_dir().join("manifest.toml")).expect("manifest");
    toml::from_str::<Manifest>(&text)
        .expect("manifest parses")
        .fixture
}

fn expectation(sha: &str) -> Option<Expectation> {
    expectation_in(&fixtures_dir().join("expectations"), sha)
}

/// The expectation for `sha` in `dir` (the private-corpus run keeps its
/// expectations outside the repository).
fn expectation_in(dir: &Path, sha: &str) -> Option<Expectation> {
    let path = dir.join(format!("{sha}.json"));
    let text = std::fs::read_to_string(&path).ok()?;
    Some(serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

// ---------------------------------------------------------------------------
// Observations, ours and the oracle's, in one shape

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Opened,
    Password,
    Failed,
}

#[derive(Clone, Debug)]
enum PageOut {
    Text(String),
    /// The oracle stored only the hash (text over 1 MiB).
    Hash(String),
    Error(String),
}

#[derive(Clone, Debug)]
struct Observed {
    class: Class,
    /// Why opening failed (our `OpenError` variant or the oracle's
    /// exception class).
    failure: Option<String>,
    pages: Vec<PageOut>,
}

impl Observed {
    fn texts(&self) -> Vec<Option<&str>> {
        self.pages
            .iter()
            .map(|p| match p {
                PageOut::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Two pages agree: equal texts (or a text matching a stored hash), or both
/// failed (the messages differ between the two implementations by nature).
fn page_eq(a: &PageOut, b: &PageOut) -> bool {
    match (a, b) {
        (PageOut::Text(x), PageOut::Text(y)) => x == y,
        (PageOut::Text(t), PageOut::Hash(h)) | (PageOut::Hash(h), PageOut::Text(t)) => {
            sha256_hex(t.as_bytes()) == *h
        }
        (PageOut::Hash(x), PageOut::Hash(y)) => x == y,
        (PageOut::Error(_), PageOut::Error(_)) => true,
        _ => false,
    }
}

/// The exact-gate equality: outcome class, then (when opened) page count and
/// every page.
fn agrees(ours: &Observed, oracle: &Observed) -> bool {
    ours.class == oracle.class
        && (ours.class != Class::Opened
            || (ours.pages.len() == oracle.pages.len()
                && ours
                    .pages
                    .iter()
                    .zip(&oracle.pages)
                    .all(|(a, b)| page_eq(a, b))))
}

fn transcript(o: &Observed) -> String {
    let mut s = String::new();
    match o.class {
        Class::Opened if o.pages.is_empty() => s.push_str("--- no pages ---\n"),
        Class::Opened => {
            for (i, p) in o.pages.iter().enumerate() {
                match p {
                    PageOut::Text(t) => {
                        let _ = write!(s, "--- page {} ---\n{t}", i + 1);
                    }
                    PageOut::Hash(h) => {
                        let _ = writeln!(s, "--- page {} sha256 {h} ---", i + 1);
                    }
                    PageOut::Error(e) => {
                        let _ = writeln!(s, "--- page {} error: {e} ---", i + 1);
                    }
                }
            }
        }
        Class::Password | Class::Failed => {
            let why = o.failure.as_deref().unwrap_or("?");
            let _ = writeln!(s, "--- open failed: {why} ---");
        }
    }
    s
}

fn oracle_observed(e: &Expectation) -> Observed {
    let class = if !e.open.ok {
        Class::Failed
    } else if e.open.needs_pass {
        Class::Password
    } else {
        Class::Opened
    };
    let pages = e
        .pages
        .iter()
        .map(|p| match (&p.error, &p.text, &p.sha256) {
            (Some(err), _, _) => PageOut::Error(err.clone()),
            (None, Some(t), _) => PageOut::Text(t.clone()),
            (None, None, Some(h)) => PageOut::Hash(h.clone()),
            (None, None, None) => panic!("{}: page {} has no text", e.source, p.index),
        })
        .collect();
    Observed {
        class,
        failure: e.open.exception.clone(),
        pages,
    }
}

/// The variant name of a `Debug`-formatted error (`NotPdf`, `Corrupt`, ...).
fn variant(debug: &str) -> String {
    debug
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect()
}

/// Opens `path` from disk with default options and extracts every page, on
/// a 512 KiB stack. Returns the observation and the sorted warning kinds of
/// the document and all its pages.
///
/// A panic in our extraction comes back as `Err(payload)`; the caller
/// attributes it to the fixture and carries on, so one regression never
/// hides the verdict on the others (nor the report).
fn ours(path: PathBuf) -> Result<(Observed, Vec<String>), String> {
    common::try_on_small_stack(move || extract_from_disk(&path)).map_err(|payload| {
        payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_owned())
    })
}

/// The body of [`ours`], run on the small-stack thread.
fn extract_from_disk(path: &Path) -> (Observed, Vec<String>) {
    let doc = match Document::open(path, &OpenOptions::default()) {
        Ok(doc) => doc,
        Err(e) => {
            let class = if matches!(e, OpenError::PasswordRequired) {
                Class::Password
            } else {
                Class::Failed
            };
            let failure = Some(variant(&format!("{e:?}")));
            let observed = Observed {
                class,
                failure,
                pages: Vec::new(),
            };
            return (observed, Vec::new());
        }
    };
    let opts = TextOptions::default();
    let mut kinds: BTreeSet<String> = doc
        .warnings()
        .iter()
        .map(|w| format!("{:?}", w.kind))
        .collect();
    let mut pages = Vec::new();
    for page in doc.pages() {
        match page.text_page(&opts) {
            Ok(tp) => {
                kinds.extend(tp.warnings().iter().map(|w| format!("{:?}", w.kind)));
                pages.push(PageOut::Text(tp.to_text(opts.block_separator)));
            }
            Err(e) => pages.push(PageOut::Error(format!("{e:?}"))),
        }
    }
    // A lazy repair seen during extraction is reported by the document.
    kinds.extend(doc.warnings().iter().map(|w| format!("{:?}", w.kind)));
    let observed = Observed {
        class: Class::Opened,
        failure: None,
        pages,
    };
    (observed, kinds.into_iter().collect())
}

// ---------------------------------------------------------------------------
// Report

/// Per-page metrics for `parity-report.json`.
fn page_metrics(ours: &Observed, oracle: &Observed) -> Vec<serde_json::Value> {
    let (a, b) = (ours.texts(), oracle.texts());
    (0..a.len().max(b.len()))
        .map(|i| {
            let x = a.get(i).copied().flatten();
            let y = b.get(i).copied().flatten();
            // Unstripped texts, as this report always compared them; the
            // similarity formula is the library's (`parity::similarity`).
            let (distance, similarity) = match (x, y) {
                (Some(x), Some(y)) if x == y => (Some(0), Some(1.0)),
                (Some(x), Some(y)) => (
                    Some(strsim::levenshtein(x, y)),
                    Some(pdf_extract_core::parity::similarity(x, y)),
                ),
                _ => (None, None),
            };
            serde_json::json!({
                "index": i,
                "ours_len": x.map(|t| t.chars().count()),
                "oracle_len": y.map(|t| t.chars().count()),
                "equal": matches!((x, y), (Some(x), Some(y)) if x == y),
                "distance": distance,
                "similarity": similarity,
            })
        })
        .collect()
}

/// The workspace's `target/`, or `CARGO_TARGET_DIR` (a relative value is
/// taken from the workspace root, where cargo is normally invoked; the test
/// itself runs in the crate directory, and an absolute value wins in
/// `join`).
fn report_path() -> PathBuf {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace.join("target"), |d| workspace.join(d))
}

/// Shortens a transcript for a failure message.
fn clip(s: &str) -> String {
    const MAX: usize = 1500;
    if s.chars().count() <= MAX {
        format!("{s:?}")
    } else {
        let head: String = s.chars().take(MAX).collect();
        format!("{head:?}… ({} chars)", s.chars().count())
    }
}

/// Checks one entry; `Err` carries the failure message.
fn check(
    entry: &Entry,
    ours: &Observed,
    kinds: &[String],
    oracle: &Observed,
) -> Result<(), String> {
    let same = agrees(ours, oracle);
    let mine = transcript(ours);
    match entry.gate.as_str() {
        // Corpus D is image-only: the text path must agree with the oracle's
        // (both empty), exactly. Its OCR is scored by crates/ocr's quality.rs.
        "exact" | "ocr" if !same => {
            return Err(format!(
                "exact gate: differs from the oracle\n  ours:   {}\n  oracle: {}",
                clip(&mine),
                clip(&transcript(oracle))
            ));
        }
        "divergence" => {
            pinned(entry, &mine)?;
            if same {
                return Err(format!(
                    "divergence gate: now equals the oracle ({:?}); make the entry exact",
                    entry.divergence
                ));
            }
        }
        "hostile" => {
            if ours.class != oracle.class {
                return Err(format!(
                    "hostile gate: open outcome {:?}, the oracle's is {:?}",
                    ours.class, oracle.class
                ));
            }
            pinned(entry, &mine)?;
            if entry.warnings.is_none() {
                return Err("hostile gate without a warnings list".to_owned());
            }
        }
        "exact" | "ocr" => {}
        other => return Err(format!("unknown gate {other:?}")),
    }
    if let Some(want) = &entry.warnings {
        let mut want = want.clone();
        want.sort();
        if want != kinds {
            return Err(format!("warning kinds {kinds:?}, manifest says {want:?}"));
        }
    }
    Ok(())
}

/// Our transcript `mine` must equal the entry's snapshot (`PARITY_BLESS=1`
/// writes it first).
fn pinned(entry: &Entry, mine: &str) -> Result<(), String> {
    let rel = entry.snapshot.as_deref().ok_or("gate needs a snapshot")?;
    if rel != format!("snapshots/{}.txt", entry.sha256) {
        return Err(format!("snapshot {rel:?} is not snapshots/<sha256>.txt"));
    }
    let path = fixtures_dir().join(rel);
    if std::env::var("PARITY_BLESS").as_deref() == Ok("1") {
        std::fs::write(&path, mine).map_err(|e| format!("{rel}: {e}"))?;
    }
    let pinned = std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
    if pinned != mine {
        return Err(format!(
            "snapshot gate: output differs from {rel}\n  ours:     {}\n  snapshot: {}",
            clip(mine),
            clip(&pinned)
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests

/// Checks that each gate's count meets its minimum floor. Returns an empty
/// string on pass, or a failure message on fail.
fn check_gate_floors(counts: &BTreeMap<String, (usize, usize)>) -> String {
    let mut failures = Vec::new();

    if let Some((n, _)) = counts.get("exact")
        && *n < GATE_C_EXACT_MIN
    {
        failures.push(format!(
            "exact gate has {n} fixtures, minimum is {GATE_C_EXACT_MIN}"
        ));
    }

    if let Some((n, _)) = counts.get("divergence")
        && *n < GATE_C_DIVERGENCE_MIN
    {
        failures.push(format!(
            "divergence gate has {n} fixtures, minimum is {GATE_C_DIVERGENCE_MIN}"
        ));
    }

    if let Some((n, _)) = counts.get("hostile")
        && *n < GATE_C_HOSTILE_MIN
    {
        failures.push(format!(
            "hostile gate has {n} fixtures, minimum is {GATE_C_HOSTILE_MIN}"
        ));
    }

    failures.join("\n")
}

/// Gate C over every manifest entry that has an expectation and a gate.
#[test]
fn gate_c() {
    let root = fixtures_dir();
    let mut failures = Vec::new();
    let mut rows = Vec::new();
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for entry in manifest() {
        // Corpus B is scored as a whole by `gate_b`.
        if entry.gate == "skip" || entry.gate == "similarity" {
            continue;
        }
        let Some(exp) = expectation(&entry.sha256) else {
            failures.push(format!(
                "{}: no expectation (run tools/oracle/gen.py)",
                entry.path
            ));
            continue;
        };
        let oracle = oracle_observed(&exp);
        let (mine, kinds, result) = match ours(root.join(&entry.path)) {
            Ok((mine, kinds)) => {
                let result = check(&entry, &mine, &kinds, &oracle);
                (mine, kinds, result)
            }
            Err(panic) => {
                let failed = Observed {
                    class: Class::Failed,
                    failure: Some("panic".to_owned()),
                    pages: Vec::new(),
                };
                let problem = format!("our extraction panicked: {panic}");
                (failed, Vec::new(), Err(problem))
            }
        };
        let tally = counts.entry(entry.gate.clone()).or_default();
        tally.0 += 1;
        if let Err(problem) = &result {
            tally.1 += 1;
            failures.push(format!("{} [{}]: {problem}", entry.path, entry.gate));
        }
        rows.push(serde_json::json!({
            "path": entry.path,
            "gate": entry.gate,
            "compat_points": entry.compat_points,
            "pass": result.is_ok(),
            "problem": result.err(),
            "ours_class": format!("{:?}", mine.class),
            "oracle_class": format!("{:?}", oracle.class),
            "ours_pages": mine.pages.len(),
            "oracle_pages": oracle.pages.len(),
            "agrees": agrees(&mine, &oracle),
            "warnings": kinds,
            "pages": page_metrics(&mine, &oracle),
        }));
    }
    let floor_failures = check_gate_floors(&counts);
    let summary: BTreeMap<_, _> = counts
        .iter()
        .map(|(gate, (n, failed))| {
            (
                gate.clone(),
                serde_json::json!({"fixtures": n, "failed": failed}),
            )
        })
        .collect();
    let report = serde_json::json!({ "schema": 1, "summary": summary, "fixtures": rows });
    let dir = report_path();
    std::fs::create_dir_all(&dir).expect("target directory");
    std::fs::write(
        dir.join("parity-report.json"),
        serde_json::to_string_pretty(&report).expect("report serialises") + "\n",
    )
    .expect("write parity-report.json");
    println!("gate C: {summary:?}");
    assert!(
        floor_failures.is_empty(),
        "corpus C minimum counts not met:\n\n{floor_failures}"
    );
    assert!(
        failures.is_empty(),
        "{} fixture(s) fail gate C:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Every expectation belongs to exactly one manifest fixture (same hash,
/// source path and size, oracle 1.28.2) or `[[built]]` row (same hash,
/// `source = "built/<path>"`; the bytes are checked against the pin by the
/// test kit's `corpus_d_noise` test, since they exist only after a build),
/// and every gated fixture and every built row has one. Schema 1 has no
/// `ocr` block; schema 2 (run 0002 §3.8) has one at 72 dpi from the pinned
/// model for every page.
#[test]
fn check_expectations() {
    let root = fixtures_dir();
    let entries = manifest();
    // Expectations are keyed by hash, so every file that has one must have
    // a distinct hash. `skip` entries have none, and corpus D's ground-truth
    // files legitimately repeat (every variant of a page has the same text).
    let gated: Vec<&Entry> = entries.iter().filter(|e| e.gate != "skip").collect();
    let by_sha: BTreeMap<&str, &Entry> = gated.iter().map(|e| (e.sha256.as_str(), *e)).collect();
    let built = built_rows();
    let built_by_sha: BTreeMap<&str, &BuiltEntry> =
        built.iter().map(|b| (b.sha256.as_str(), b)).collect();
    assert_eq!(
        by_sha.len(),
        gated.len(),
        "two manifest entries share a hash"
    );
    assert_eq!(
        built_by_sha.len(),
        built.len(),
        "two built rows share a hash"
    );
    assert!(
        built_by_sha.keys().all(|h| !by_sha.contains_key(h)),
        "a built row shares a hash with a fixture"
    );
    let mut seen = BTreeSet::new();
    for item in std::fs::read_dir(root.join("expectations")).expect("expectations dir") {
        let path = item.expect("dir entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("UTF-8 name");
        if name == ".gitkeep" {
            continue;
        }
        let sha = name
            .strip_suffix(".json")
            .unwrap_or_else(|| panic!("{name}: not a .json file"));
        let exp = expectation(sha).expect("readable");
        assert!(matches!(exp.schema, 1 | 2), "{name}: schema {}", exp.schema);
        assert_eq!(exp.sha256, sha, "{name}: sha256 field");
        assert_eq!(exp.oracle.pymupdf, ORACLE_VERSION, "{name}: oracle version");
        check_ocr_block(name, &exp);
        if let Some(row) = built_by_sha.get(sha) {
            assert_eq!(exp.source, format!("built/{}", row.path), "{name}: source");
            assert!(
                exp.ocr.is_some(),
                "{name}: a built file's expectation has no OCR"
            );
        } else {
            let entry = by_sha
                .get(sha)
                .unwrap_or_else(|| panic!("{name}: no manifest fixture has this hash"));
            assert_eq!(exp.source, entry.path, "{name}: source");
            let bytes = std::fs::read(root.join(&entry.path)).expect("fixture");
            assert_eq!(exp.size, bytes.len() as u64, "{name}: size");
            assert_eq!(
                sha256_hex(&bytes),
                sha,
                "{name}: the fixture's bytes hash differently"
            );
            if entry.gate == "ocr" {
                assert!(
                    exp.ocr.is_some(),
                    "{name}: an ocr fixture's expectation has no OCR"
                );
            }
        }
        assert_eq!(exp.page_count, exp.pages.len(), "{name}: page_count");
        for (i, p) in exp.pages.iter().enumerate() {
            assert_eq!(p.index, i, "{name}: page order");
            if let (Some(t), Some(h)) = (&p.text, &p.sha256) {
                assert_eq!(&sha256_hex(t.as_bytes()), h, "{name}: page {i} hash");
            }
        }
        seen.insert(sha.to_owned());
    }
    for e in &entries {
        if e.gate != "skip" {
            assert!(seen.contains(&e.sha256), "{}: no expectation", e.path);
        }
    }
    for b in &built {
        assert!(seen.contains(&b.sha256), "built/{}: no expectation", b.path);
    }
}

/// The `ocr` block, when present, is schema 2's: 72 dpi, `eng`, the pinned
/// model, one page per page in order, each text hashing to its `sha256`.
fn check_ocr_block(name: &str, exp: &Expectation) {
    let Some(ocr) = &exp.ocr else {
        assert_eq!(exp.schema, 1, "{name}: schema 2 without an ocr block");
        return;
    };
    assert_eq!(exp.schema, 2, "{name}: an ocr block needs schema 2");
    assert_eq!(ocr.dpi, 72, "{name}: ocr dpi");
    assert_eq!(ocr.language, "eng", "{name}: ocr language");
    assert_eq!(ocr.tessdata_sha256, OCR_MODEL_SHA256, "{name}: ocr model");
    assert_eq!(ocr.pages.len(), exp.page_count, "{name}: ocr pages");
    for (i, p) in ocr.pages.iter().enumerate() {
        assert_eq!(p.index, i, "{name}: ocr page order");
        let (Some(t), Some(h)) = (&p.text, &p.sha256) else {
            panic!("{name}: ocr page {i} has no text or hash");
        };
        assert_eq!(&sha256_hex(t.as_bytes()), h, "{name}: ocr page {i} hash");
    }
}

/// The oracle tools carry their own copies of the pinned model hash (Python
/// and shell cannot share this file's constant); they must all agree with it,
/// or an expectation regenerated with a different model would pass here. The
/// README and the measurement doc quote the hash for deployers to pin; they
/// are checked too, so a model bump cannot leave stale advice behind.
/// Each extractor insists on finding its line, so a reformatted tool fails
/// the test instead of silently skipping the comparison.
#[test]
fn oracle_tools_pin_the_same_model_hash() {
    fn tool(rel: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/oracle")
            .join(rel);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }
    // `MODEL_SHA256 = "<hex>"` at the start of a line.
    fn python_constant(source: &str, rel: &str) -> String {
        source
            .lines()
            .find_map(|l| l.strip_prefix("MODEL_SHA256 = \""))
            .and_then(|rest| rest.strip_suffix('"'))
            .unwrap_or_else(|| panic!("{rel}: no `MODEL_SHA256 = \"...\"` line"))
            .to_owned()
    }
    // The `fast|tessdata_fast|<blob>|<sha256>` row of the models array.
    fn shell_row(source: &str, rel: &str) -> String {
        source
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("\"fast|tessdata_fast|"))
            .and_then(|l| l.trim_matches('"').rsplit('|').next())
            .unwrap_or_else(|| panic!("{rel}: no `fast|tessdata_fast|...` row"))
            .to_owned()
    }

    for rel in ["gen.py", "mkcorpusd.py"] {
        assert_eq!(python_constant(&tool(rel), rel), OCR_MODEL_SHA256, "{rel}");
    }
    let rel = "measure_tessdata.sh";
    assert_eq!(shell_row(&tool(rel), rel), OCR_MODEL_SHA256, "{rel}");

    // The prose that tells deployers which model to pin quotes the hash, in
    // full and in abbreviated form (`7d4322bd…170b2`, `7d4322bd2a774972...`).
    // Scope of this check, per file:
    //   * full copies: every run of exactly 64 hex digits on a line that
    //     `pin_lines` admits must equal the pin, and there must be exactly
    //     `full` of them;
    //   * abbreviated copies: every run of 8..64 hex digits followed by
    //     `...` or `…`, plus any hex tail directly after the marker, in the
    //     text `abbrev_scope` yields for a line, must be a prefix (head) and
    //     suffix (tail) of the pin, and there must be exactly `abbrev` of
    //     them.
    // The counts make a reworded doc that drops or adds a copy fail rather
    // than pass vacuously. The measurement doc's table also lists the *other*
    // models' hashes, so there the full-copy check sees non-table lines and
    // the `| fast |` row, and the abbreviated check sees non-table lines and
    // only the fast column of the SHA-256 row.
    // Out of scope: copies in other files (specs and ADRs are frozen records
    // of the decision; the fixture JSON is covered by the ocr-pin check
    // above), abbreviations with no `...`/`…` marker or under 8 digits, and
    // abbreviations of the other models' hashes.
    struct DocPins {
        rel: &'static str,
        pin_lines: fn(&str) -> bool,
        full: usize,
        abbrev_scope: fn(&str) -> Option<&str>,
        abbrev: usize,
    }
    let docs = [
        DocPins {
            rel: "README.md",
            pin_lines: |_| true,
            full: 1,
            abbrev_scope: |l| Some(l),
            abbrev: 0,
        },
        DocPins {
            rel: "docs/reference/tessdata-measurement.md",
            pin_lines: |l| !l.starts_with('|') || l.starts_with("| fast |"),
            full: 2,
            abbrev_scope: |l| match l.strip_prefix("| SHA-256 |") {
                // Cells are fast | best | combined; only the first is ours.
                Some(cells) => cells.split('|').next(),
                None if l.starts_with('|') => None,
                None => Some(l),
            },
            abbrev: 1,
        },
        DocPins {
            rel: "tools/oracle/README.md",
            pin_lines: |_| true,
            full: 0,
            abbrev_scope: |l| Some(l),
            abbrev: 1,
        },
    ];
    for doc in docs {
        let rel = doc.rel;
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel);
        let text =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let copies: Vec<&str> = text
            .lines()
            .filter(|l| (doc.pin_lines)(l))
            .flat_map(hex64_tokens)
            .collect();
        assert_eq!(
            copies.len(),
            doc.full,
            "{rel}: expected {} quoted model hashes, found {copies:?}",
            doc.full
        );
        for copy in copies {
            assert_eq!(copy, OCR_MODEL_SHA256, "{rel}");
        }
        let abbreviated: Vec<AbbrevHash> = text
            .lines()
            .filter_map(doc.abbrev_scope)
            .flat_map(abbreviated_hashes)
            .collect();
        assert_eq!(
            abbreviated.len(),
            doc.abbrev,
            "{rel}: expected {} abbreviated model hashes, found {abbreviated:?}",
            doc.abbrev
        );
        for copy in abbreviated {
            assert!(
                copy.is_abbreviation_of(OCR_MODEL_SHA256),
                "{rel}: {copy:?} is not an abbreviation of the pinned model hash"
            );
        }
    }
}

/// A hash quoted as `head...tail` or `head…tail`; `tail` may be empty.
#[derive(Debug, PartialEq, Eq)]
struct AbbrevHash<'a> {
    head: &'a str,
    tail: &'a str,
}

impl AbbrevHash<'_> {
    /// `head` must open the full hash and `tail` close it, without the two
    /// overlapping (a long head and tail could otherwise both match a hash
    /// they cannot both be disjoint parts of).
    fn is_abbreviation_of(&self, full: &str) -> bool {
        self.head.len() + self.tail.len() <= full.len()
            && full.starts_with(self.head)
            && full.ends_with(self.tail)
    }
}

/// Abbreviated hashes in `line`: a maximal hex run of 8..64 digits followed
/// directly by `...` or `…`, then the maximal (possibly empty) hex run after
/// the marker. A 40-digit git blob id written this way is returned too, on
/// purpose: the caller scopes the lines, and a blob abbreviation inside a
/// scoped line should be noticed rather than skipped.
fn abbreviated_hashes(line: &str) -> Vec<AbbrevHash<'_>> {
    let bytes = line.as_bytes();
    let run_end = |from: usize| {
        from + bytes[from..]
            .iter()
            .take_while(|b| b.is_ascii_hexdigit())
            .count()
    };
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_hexdigit() {
            i += 1;
            continue;
        }
        let end = run_end(i);
        let head = &line[i..end];
        let marker = ["...", "\u{2026}"]
            .into_iter()
            .find(|m| line[end..].starts_with(m));
        if let Some(marker) = marker.filter(|_| (8..64).contains(&head.len())) {
            let tail_start = end + marker.len();
            let tail = &line[tail_start..run_end(tail_start)];
            found.push(AbbrevHash { head, tail });
        }
        i = end;
    }
    found
}

/// The abbreviated-hash extractor and prefix/suffix rule on hand-made lines:
/// what it finds, what it ignores, and that a wrong copy is rejected.
#[test]
fn abbreviated_hash_extraction_and_check() {
    let pin = OCR_MODEL_SHA256;
    let found = abbreviated_hashes("`7d4322bd\u{2026}170b2` and `7d4322bd2a774972...`");
    assert_eq!(
        found,
        [
            AbbrevHash {
                head: "7d4322bd",
                tail: "170b2"
            },
            AbbrevHash {
                head: "7d4322bd2a774972",
                tail: ""
            },
        ]
    );
    assert!(found.iter().all(|c| c.is_abbreviation_of(pin)));

    // One wrong digit in the head or in the tail is rejected.
    for bad in [
        "7d4322be\u{2026}170b2",
        "7d4322bd...170b3",
        "7d4322bd2a774973...",
    ] {
        let copies = abbreviated_hashes(bad);
        assert_eq!(copies.len(), 1, "{bad}");
        assert!(!copies[0].is_abbreviation_of(pin), "{bad}");
    }
    let overlapping = AbbrevHash {
        head: &pin[..40],
        tail: &pin[30..],
    };
    assert!(!overlapping.is_abbreviation_of(pin));

    // Not abbreviations of this kind: too short, no marker, a full hash,
    // prose before an ellipsis, a marker before the digits.
    for ignored in [
        "7d4322b...",
        "7d4322bd2a774972",
        pin,
        "see the model... then",
        "...7d4322bd2a774972",
    ] {
        assert!(abbreviated_hashes(ignored).is_empty(), "{ignored}");
    }
}

/// The maximal runs of exactly 64 hex digits in `line` (a SHA-256). Longer
/// runs (git blob ids are 40) and shorter ones are not hashes of this kind.
fn hex64_tokens(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !c.is_ascii_hexdigit())
        .filter(|t| t.len() == 64)
}

/// Every snapshot belongs to a divergence entry (no orphans after a
/// fixture changes or becomes exact).
#[test]
fn snapshots_are_referenced() {
    let root = fixtures_dir();
    let referenced: BTreeSet<String> = manifest()
        .into_iter()
        .filter(|e| e.gate == "divergence" || e.gate == "hostile")
        .filter_map(|e| e.snapshot)
        .collect();
    for item in std::fs::read_dir(root.join("snapshots")).expect("snapshots dir") {
        let name = item.expect("dir entry").file_name();
        let name = name.to_str().expect("UTF-8 name");
        if name != ".gitkeep" {
            let rel = format!("snapshots/{name}");
            assert!(
                referenced.contains(&rel),
                "{rel}: no divergence entry uses it"
            );
        }
    }
}

/// The comparison rules themselves, on hand-made observations.
#[test]
fn comparison_rules() {
    let opened = |pages: Vec<PageOut>| Observed {
        class: Class::Opened,
        failure: None,
        pages,
    };
    let text = |t: &str| PageOut::Text(t.to_owned());
    assert!(agrees(
        &opened(vec![text("a\n")]),
        &opened(vec![text("a\n")])
    ));
    assert!(!agrees(
        &opened(vec![text("a\n")]),
        &opened(vec![text("b\n")])
    ));
    assert!(!agrees(&opened(vec![]), &opened(vec![text("")])));
    let hash = PageOut::Hash(sha256_hex(b"a\n"));
    assert!(agrees(&opened(vec![text("a\n")]), &opened(vec![hash])));
    assert!(agrees(
        &opened(vec![PageOut::Error("x".into())]),
        &opened(vec![PageOut::Error("y".into())])
    ));
    let failed = |why: &str| Observed {
        class: Class::Failed,
        failure: Some(why.to_owned()),
        pages: Vec::new(),
    };
    assert!(agrees(&failed("NotPdf"), &failed("FileDataError")));
    assert!(!agrees(&failed("NotPdf"), &opened(vec![])));
    assert_eq!(transcript(&opened(vec![])), "--- no pages ---\n");
    assert_eq!(
        transcript(&opened(vec![text("a\n"), PageOut::Error("E".into())])),
        "--- page 1 ---\na\n--- page 2 error: E ---\n"
    );
    assert_eq!(
        transcript(&failed("NotPdf")),
        "--- open failed: NotPdf ---\n"
    );
    assert_eq!(variant("Corrupt(\"x\")"), "Corrupt");
    assert_eq!(variant("PasswordRequired"), "PasswordRequired");
}

/// The gate floor checks, on hand-made counts.
#[test]
fn check_gate_floors_rules() {
    // Pass: all counts at or above floor.
    let mut counts = BTreeMap::new();
    counts.insert("exact".to_owned(), (GATE_C_EXACT_MIN, 0));
    counts.insert("divergence".to_owned(), (19, 0));
    counts.insert("hostile".to_owned(), (9, 0));
    assert_eq!(check_gate_floors(&counts), "");

    // Fail: exact below floor.
    let mut counts = BTreeMap::new();
    counts.insert("exact".to_owned(), (GATE_C_EXACT_MIN - 1, 0));
    counts.insert("divergence".to_owned(), (19, 0));
    counts.insert("hostile".to_owned(), (9, 0));
    let msg = check_gate_floors(&counts);
    assert!(msg.contains("exact"));
    assert!(msg.contains(&(GATE_C_EXACT_MIN - 1).to_string()));
    assert!(msg.contains(&GATE_C_EXACT_MIN.to_string()));

    // Fail: divergence below floor.
    let mut counts = BTreeMap::new();
    counts.insert("exact".to_owned(), (GATE_C_EXACT_MIN, 0));
    counts.insert("divergence".to_owned(), (18, 0));
    counts.insert("hostile".to_owned(), (9, 0));
    let msg = check_gate_floors(&counts);
    assert!(msg.contains("divergence"));
    assert!(msg.contains("18"));
    assert!(msg.contains("19"));

    // Fail: hostile below floor.
    let mut counts = BTreeMap::new();
    counts.insert("exact".to_owned(), (GATE_C_EXACT_MIN, 0));
    counts.insert("divergence".to_owned(), (19, 0));
    counts.insert("hostile".to_owned(), (8, 0));
    let msg = check_gate_floors(&counts);
    assert!(msg.contains("hostile"));
    assert!(msg.contains("8"));
    assert!(msg.contains("9"));

    // Fail: multiple gates below floor.
    let mut counts = BTreeMap::new();
    counts.insert("exact".to_owned(), (0, 0));
    counts.insert("divergence".to_owned(), (0, 0));
    counts.insert("hostile".to_owned(), (0, 0));
    let msg = check_gate_floors(&counts);
    assert!(msg.contains("exact"));
    assert!(msg.contains("divergence"));
    assert!(msg.contains("hostile"));
}
