//! Gate B (spec §4.14) and the private corpus-A run (design spec §13.2):
//! files scored as a corpus rather than one by one. Part of the `parity`
//! test binary; it reuses the manifest, expectation and extraction plumbing
//! of `parity.rs`.
//!
//! Every page of every scored file the oracle opened contributes one sample.
//! Texts are compared after a Python-compatible `.strip()` (the design-spec
//! metrics, §13.2, are defined on stripped text):
//!
//! 1. **opens**: the oracle opened the file ⇒ we open it (corpus B allows at
//!    most two documented exceptions, [`OPEN_EXCEPTIONS`]; corpus A none);
//! 2. **routing**: `len(text.strip()) >= 50` (the caller's OCR decision)
//!    agrees on at least the gate's share of pages;
//! 3. **similarity** `1 − levenshtein / max(len)` in characters: a minimum
//!    median and a minimum low percentile (B: p10; A: p1);
//! 4. exact-match rate and per-page line-count delta: reported only.
//!
//! Page pairing is by index. A page one side lacks (page-count mismatch, our
//! open failure, our page error) counts as empty text on that side, so it
//! costs routing and similarity instead of vanishing from the denominator. A
//! page the oracle itself failed to extract has no reference and is left out
//! (counted in the report). Files the oracle did not open contribute nothing.
//!
//! The per-page metrics and the aggregates come from the library's public
//! `pdf_extract_core::parity` module (`compare_page`, `ParityAccumulator`),
//! the same definitions a consumer measures a migration with; this file
//! adds the corpus plumbing, the page pairing, the floors and the report.
//! Percentiles use the nearest-rank rule, read from the accumulator's
//! 10⁻⁵-resolution histogram, so a reported percentile is at most 10⁻⁵
//! below the exact sample and every verdict is the exact one.
//!
//! Corpus A is private (it may hold confidential documents), so its run —
//! the ignored [`gate_a_private`] test driven by
//! `tools/oracle/run_private_corpus.sh` — prints and stores metrics and
//! lengths only, never page text.

use std::path::{Path, PathBuf};

use pdf_extract_core::parity::{
    self, Gates, PageComparison, ParityAccumulator, ROUTING_MIN_CHARS, compare_page, py_strip,
};
use sha2::{Digest, Sha256};

use super::{
    Class, Expectation, Observed, PageOut, expectation, expectation_in, manifest, oracle_observed,
    ours, report_path,
};

/// Files the oracle opens and we are allowed not to, each with the reason
/// (spec §4.14 (1): at most two). An entry that stops being needed fails the
/// gate, so the list never carries stale excuses.
const OPEN_EXCEPTIONS: &[(&str, &str)] = &[];

/// Minimum count of similarity files in corpus B. This floor guards against
/// silent corpus loss: a truncated manifest where all files are truncated or
/// skipped passes vacuously. The threshold must be raised deliberately when the
/// corpus grows (a decision documented in the ADR); lowering is a spec change.
const GATE_B_FILES_MIN: usize = 60;

/// How many of the least similar pages are listed.
const WORST_SHOWN: usize = 12;

/// One gate: the library's thresholds plus how this run reports. Changing
/// any threshold is a spec change.
#[derive(Debug)]
pub(super) struct Gate {
    pub name: &'static str,
    pub thresholds: Gates,
    /// Whether page text may appear in messages and the console summary.
    pub show_text: bool,
}

/// Spec §4.14: corpus B.
pub(super) const GATE_B: Gate = Gate {
    name: "B",
    thresholds: Gates {
        routing: 0.99,
        median: 0.98,
        low_percentile: 10.0,
        low_min: 0.90,
    },
    show_text: true,
};

/// Design spec §13.2 as restated in spec §4.14: the private corpus A, the
/// library's own `Gates::SPEC`.
pub(super) const GATE_A: Gate = Gate {
    name: "A",
    thresholds: Gates::SPEC,
    show_text: false,
};

/// One scored page: the library's comparison plus, for the console's list
/// of least similar pages, the stripped texts (never written to a report).
#[derive(Clone, Debug)]
pub(super) struct Sample {
    pub path: String,
    pub index: usize,
    pub ours: String,
    pub oracle: String,
    pub cmp: PageComparison,
}

impl Sample {
    pub(super) fn new(path: &str, index: usize, ours: &str, oracle: &str) -> Self {
        Self {
            path: path.to_owned(),
            index,
            // The oracle is the reference; ours is the candidate.
            cmp: compare_page(oracle, ours),
            ours: py_strip(ours).to_owned(),
            oracle: py_strip(oracle).to_owned(),
        }
    }

    pub(super) fn similarity(&self) -> f64 {
        self.cmp.similarity
    }

    pub(super) fn routing_agrees(&self) -> bool {
        self.cmp.routing_agrees
    }

    pub(super) fn exact(&self) -> bool {
        self.cmp.exact
    }

    pub(super) fn line_delta(&self) -> i64 {
        self.cmp.line_delta
    }
}

/// Pairs our pages with the oracle's (see the module docs) and returns the
/// samples plus the number of oracle pages that had no text to compare.
pub(super) fn pair_pages(path: &str, mine: &Observed, oracle: &Observed) -> (Vec<Sample>, usize) {
    let mut samples = Vec::new();
    let mut unscored = 0;
    for i in 0..mine.pages.len().max(oracle.pages.len()) {
        let reference = match oracle.pages.get(i) {
            Some(PageOut::Text(t)) => t.as_str(),
            None => "",
            // An oracle page error has no reference text; a hash-only page
            // (over 1 MiB) cannot be diffed.
            Some(PageOut::Error(_) | PageOut::Hash(_)) => {
                unscored += 1;
                continue;
            }
        };
        let text = match (mine.class, mine.pages.get(i)) {
            (Class::Opened, Some(PageOut::Text(t))) => t.as_str(),
            _ => "",
        };
        samples.push(Sample::new(path, i, text, reference));
    }
    (samples, unscored)
}

/// The library's aggregates over all samples, under `gate`'s thresholds.
pub(super) fn metrics(samples: &[Sample], gate: &Gate) -> parity::ParityReport {
    assert!(!samples.is_empty(), "no pages to score");
    let mut acc = ParityAccumulator::new();
    for s in samples {
        acc.add(&s.cmp);
    }
    acc.report(&gate.thresholds)
}

/// The failed verdicts of `m`, as messages (empty = pass).
pub(super) fn verdicts(gate: &Gate, m: &parity::ParityReport) -> Vec<String> {
    m.verdicts
        .iter()
        .filter(|v| !v.passed)
        .map(|v| match v.name {
            "routing" => format!("routing agreement {:.4} < {}", v.value, v.threshold),
            "median" => format!("similarity median {:.4} < {}", v.value, v.threshold),
            _ => format!(
                "similarity p{} {:.4} < {}",
                gate.thresholds.low_percentile, v.value, v.threshold
            ),
        })
        .collect()
}

/// One file to score: a display label, its path and its expectation.
pub(super) struct Input {
    pub label: String,
    pub path: PathBuf,
    pub expectation: Option<Expectation>,
}

/// Checks that the file count meets the gate B minimum floor. Returns an empty
/// string on pass, or a failure message on fail.
pub(super) fn check_gate_b_floor(files: usize) -> String {
    if files < GATE_B_FILES_MIN {
        format!("gate B has {files} files, minimum is {GATE_B_FILES_MIN}")
    } else {
        String::new()
    }
}

/// Scores `inputs` under `gate`, writes the JSON report to `report`, prints
/// a summary, and returns the failure messages (empty = pass).
pub(super) fn run(
    gate: &Gate,
    inputs: Vec<Input>,
    exceptions: &[(&str, &str)],
    report: &Path,
) -> Vec<String> {
    let mut failures = Vec::new();
    let mut samples = Vec::new();
    let (mut files, mut oracle_opened, mut unscored) = (0, 0, 0);
    let mut excused = Vec::new();
    for input in inputs {
        files += 1;
        let label = input.label;
        let Some(exp) = input.expectation else {
            failures.push(format!("{label}: no expectation (run tools/oracle/gen.py)"));
            continue;
        };
        let oracle = oracle_observed(&exp);
        let exception = exceptions.iter().find(|(p, _)| *p == label);
        let mine = match ours(input.path) {
            Ok((mine, _kinds)) => mine,
            Err(panic) => {
                failures.push(format!("{label}: our extraction panicked: {panic}"));
                continue;
            }
        };
        if oracle.class != Class::Opened {
            if exception.is_some() {
                failures.push(format!(
                    "{label}: open exception, but the oracle does not open it"
                ));
            }
            continue;
        }
        oracle_opened += 1;
        match (mine.class == Class::Opened, exception) {
            (true, Some(_)) => failures.push(format!(
                "{label}: listed as an open exception but opens now; remove the entry"
            )),
            (false, Some((_, why))) => excused.push(format!("{label} ({why})")),
            (false, None) => failures.push(format!(
                "{label}: the oracle opens it, we do not ({:?})",
                mine.failure
            )),
            (true, None) => {}
        }
        let (pages, skipped) = pair_pages(&label, &mine, &oracle);
        samples.extend(pages);
        unscored += skipped;
    }
    if samples.is_empty() {
        failures.push("no pages to score".to_owned());
        return failures;
    }

    let m = metrics(&samples, gate);
    failures.extend(verdicts(gate, &m));
    // p1 is reported for every gate (the §13.2 low percentile), whatever
    // percentile the gate itself checks.
    let p1 = metrics(&samples, &GATE_A).low;
    let mut worst: Vec<&Sample> = samples.iter().filter(|s| !s.exact()).collect();
    worst.sort_by(|a, b| a.similarity().total_cmp(&b.similarity()));
    worst.truncate(WORST_SHOWN);

    let json = serde_json::json!({
        "schema": 1,
        "gate": gate.name,
        "thresholds": {
            "routing_min_chars": ROUTING_MIN_CHARS,
            "routing_min_rate": gate.thresholds.routing,
            "similarity_min_median": gate.thresholds.median,
            "similarity_low_percentile": gate.thresholds.low_percentile,
            "similarity_min_low": gate.thresholds.low_min,
        },
        "metrics": {
            "files": files,
            "oracle_opened": oracle_opened,
            "open_exceptions": excused,
            "pages": m.pages,
            "pages_unscored": unscored,
            "routing_rate": m.routing_rate,
            "similarity_median": m.median,
            "similarity_low": m.low,
            "similarity_p1": p1,
            "similarity_min": m.min,
            "similarity_mean": m.mean,
            "exact_rate": m.exact_rate,
            "line_delta_mean_abs": m.line_delta_mean_abs,
            "line_delta_max_abs": m.line_delta_max_abs,
            "line_delta_pages_over_2": m.line_delta_histogram[3],
        },
        "pass": failures.is_empty(),
        "failures": failures,
        "pages": samples.iter().map(|s| serde_json::json!({
            "path": s.path,
            "index": s.index,
            "ours_len": s.ours.chars().count(),
            "oracle_len": s.oracle.chars().count(),
            "routing_agrees": s.routing_agrees(),
            "similarity": s.similarity(),
            "exact": s.exact(),
            "line_delta": s.line_delta(),
        })).collect::<Vec<_>>(),
    });
    if let Some(dir) = report.parent() {
        std::fs::create_dir_all(dir).expect("report directory");
    }
    std::fs::write(
        report,
        serde_json::to_string_pretty(&json).expect("report serialises") + "\n",
    )
    .expect("write the gate report");

    println!("gate {}: {m:#?}", gate.name);
    println!("least similar pages:");
    for s in &worst {
        println!("  {:.4} {} page {}", s.similarity(), s.path, s.index + 1);
        if gate.show_text {
            println!("    ours:   {}", excerpt(&s.ours));
            println!("    oracle: {}", excerpt(&s.oracle));
        }
    }
    failures
}

/// Short, escaped excerpt for the console.
fn excerpt(s: &str) -> String {
    let head: String = s.chars().take(160).collect();
    format!("{head:?}")
}

fn sha256_file(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every regular, non-hidden file under `dir`, sorted; the same walk as
/// `tools/oracle/gen.py`'s, so every file gets its expectation.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).unwrap_or_else(|e| panic!("{}: {e}", d.display()));
        for e in entries {
            let e = e.expect("directory entry");
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let kind = e.file_type().expect("file type");
            if kind.is_dir() {
                stack.push(e.path());
            } else if kind.is_file() {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

/// Gate B over every `similarity` fixture (spec §4.14).
#[test]
fn gate_b() {
    assert!(
        OPEN_EXCEPTIONS.len() <= 2,
        "spec §4.14 allows at most two open exceptions"
    );
    let root = super::fixtures_dir();
    let inputs: Vec<Input> = manifest()
        .into_iter()
        .filter(|e| e.gate == "similarity")
        .map(|e| Input {
            expectation: expectation(&e.sha256),
            path: root.join(&e.path),
            label: e.path,
        })
        .collect();
    let file_count = inputs.len();
    let floor_msg = check_gate_b_floor(file_count);
    assert!(
        floor_msg.is_empty(),
        "corpus B minimum counts not met: {floor_msg}"
    );
    let report = report_path().join("parity-report-b.json");
    let failures = run(&GATE_B, inputs, OPEN_EXCEPTIONS, &report);
    assert!(
        failures.is_empty(),
        "gate B fails:\n  {}\n(see {})",
        failures.join("\n  "),
        report.display()
    );
}

/// The corpus-A gate over a private directory (design spec §13.2). Never run
/// by CI: `tools/oracle/run_private_corpus.sh <dir>` writes the expectations
/// outside the repository and runs this with `PARITY_PRIVATE_DIR` (the
/// corpus), `PARITY_PRIVATE_EXPECTATIONS` and `PARITY_PRIVATE_REPORT` set.
#[test]
#[ignore = "private corpus; run through tools/oracle/run_private_corpus.sh"]
fn gate_a_private() {
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .unwrap_or_else(|| panic!("{name} is not set; use run_private_corpus.sh"))
    };
    let (dir, expectations) = (
        var("PARITY_PRIVATE_DIR"),
        var("PARITY_PRIVATE_EXPECTATIONS"),
    );
    let report = var("PARITY_PRIVATE_REPORT");
    let inputs = walk(&dir)
        .into_iter()
        .map(|path| Input {
            label: path
                .strip_prefix(&dir)
                .unwrap_or(&path)
                .display()
                .to_string(),
            expectation: expectation_in(&expectations, &sha256_file(&path)),
            path,
        })
        .collect();
    let failures = run(&GATE_A, inputs, &[], &report);
    assert!(
        failures.is_empty(),
        "gate A fails:\n  {}\n(see {})",
        failures.join("\n  "),
        report.display()
    );
}

/// The metric definitions themselves, on hand-made samples.
#[test]
fn gate_metric_rules() {
    // The metric definitions themselves are unit-tested in the library
    // (src/parity_tests.rs); here, that samples use them with the oracle as
    // the reference.
    let s = Sample::new("f", 0, "a\nb\nc", "a");
    assert_eq!(s.line_delta(), 2);

    let long = "x".repeat(60);
    let s = Sample::new("f", 0, &format!("  {long}\n"), &long);
    assert!(s.exact() && s.routing_agrees() && s.line_delta() == 0);
    let s = Sample::new("f", 0, &"y".repeat(49), &long);
    assert!(!s.routing_agrees() && !s.exact());

    // Pairing: a page we lack counts as empty; an oracle page error is
    // unscored; a page only we have is compared with empty text.
    let opened = |pages: Vec<PageOut>| Observed {
        class: Class::Opened,
        failure: None,
        pages,
    };
    let text = |t: &str| PageOut::Text(t.to_owned());
    let (pairs, skipped) = pair_pages(
        "f",
        &opened(vec![text("a"), text("b"), text("c")]),
        &opened(vec![text("a"), PageOut::Error("E".into())]),
    );
    assert_eq!(skipped, 1);
    assert_eq!(pairs.len(), 2);
    assert!(pairs[0].exact());
    assert_eq!((pairs[1].index, pairs[1].oracle.as_str()), (2, ""));
    let failed = Observed {
        class: Class::Failed,
        failure: Some("Corrupt".into()),
        pages: Vec::new(),
    };
    let (pairs, _) = pair_pages("f", &failed, &opened(vec![text(&long)]));
    assert_eq!(pairs[0].ours, "");
    assert!(!pairs[0].routing_agrees());

    let m = metrics(
        &[
            Sample::new("f", 0, "a", "a"),
            Sample::new("f", 1, "a\nb\nc\nd", "a"),
        ],
        &GATE_B,
    );
    assert!((m.exact_rate.unwrap() - 0.5).abs() < f64::EPSILON);
    assert_eq!((m.line_delta_max_abs, m.line_delta_histogram[3]), (3, 1));
    let v = verdicts(&GATE_B, &m);
    assert!(v.iter().any(|v| v.contains("p10")), "{v:?}");
    assert!(v.iter().any(|v| v.contains("median")), "{v:?}");
    let perfect = metrics(&[Sample::new("f", 0, "a", "a")], &GATE_A);
    assert!(verdicts(&GATE_A, &perfect).is_empty());
}

/// `run` itself on a temporary corpus: the open rule, exceptions (stale and
/// used), missing expectations, page pairing and the report, without any
/// fixture from the repository.
#[test]
fn gate_run_rules() {
    let tmp = std::env::temp_dir().join(format!("pdf-extract-gate-run-{}", std::process::id()));
    std::fs::create_dir_all(tmp.join("sub")).expect("temp dir");
    let pdf = tmp.join("sub/a.pdf");
    let text = "word ".repeat(12);
    std::fs::write(&pdf, pdf_extract_testkit_bytes(&text)).expect("write pdf");
    let junk = tmp.join("junk.pdf");
    std::fs::write(&junk, b"not a pdf").expect("write junk");
    std::fs::write(tmp.join(".hidden"), b"x").expect("write hidden");
    assert_eq!(walk(&tmp), vec![junk.clone(), pdf.clone()]);

    let exp = |ok: bool, pages: &[&str]| -> Expectation {
        let pages: Vec<_> = pages
            .iter()
            .enumerate()
            .map(|(i, t)| serde_json::json!({"index": i, "text": t, "sha256": null, "error": null}))
            .collect();
        serde_json::from_value(serde_json::json!({
            "schema": 1, "sha256": "x", "size": 0, "source": "x",
            "oracle": {"pymupdf": "1.28.2"},
            "open": {"ok": ok, "exception": if ok { None } else { Some("FileDataError") }, "needs_pass": false},
            "page_count": pages.len(), "pages": pages,
        }))
        .expect("expectation")
    };
    let report = tmp.join("out/report.json");
    let input = |label: &str, path: &Path, e: Option<Expectation>| Input {
        label: label.to_owned(),
        path: path.to_path_buf(),
        expectation: e,
    };
    let ours_text = format!("{}\n", text.trim_end());

    // Pass: one exact page; junk the oracle cannot open either is ignored.
    let failures = run(
        &GATE_A,
        vec![
            input("a", &pdf, Some(exp(true, &[&ours_text]))),
            input("junk", &junk, Some(exp(false, &[]))),
        ],
        &[],
        &report,
    );
    assert!(failures.is_empty(), "{failures:?}");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).expect("report")).expect("json");
    assert_eq!(json["metrics"]["pages"], 1);
    assert_eq!(json["metrics"]["oracle_opened"], 1);
    // No page text anywhere in the report (gate A runs on private corpora):
    // neither as a field nor as a value.
    assert!(json["pages"][0].get("text").is_none(), "no text in reports");
    let raw = std::fs::read_to_string(&report).expect("report");
    assert!(
        !raw.contains("word word"),
        "page text leaked into the report"
    );
    for key in ["ours", "oracle", "text"] {
        assert!(
            json["pages"][0].get(key).is_none(),
            "report page has a {key:?} field"
        );
    }

    // Fail: the oracle opens the junk file (we cannot), a stale exception,
    // a missing expectation, and a page we lack.
    let failures = run(
        &GATE_B,
        vec![
            input("a", &pdf, Some(exp(true, &[&ours_text, "second page"]))),
            input("junk", &junk, Some(exp(true, &["x"]))),
            input("none", &junk, None),
        ],
        &[("a", "stale"), ("junk", "used")],
        &report,
    );
    let joined = failures.join("\n");
    assert!(
        joined.contains("a: listed as an open exception"),
        "{joined}"
    );
    assert!(joined.contains("none: no expectation"), "{joined}");
    assert!(!joined.contains("junk: the oracle opens it"), "{joined}");
    let failures = run(
        &GATE_B,
        vec![input("junk", &junk, Some(exp(true, &["x"])))],
        &[],
        &report,
    );
    assert!(
        failures
            .iter()
            .any(|f| f.contains("the oracle opens it, we do not"))
    );
    let failures = run(
        &GATE_B,
        vec![input("junk", &junk, Some(exp(false, &[])))],
        &[("junk", "x")],
        &report,
    );
    assert!(
        failures
            .iter()
            .any(|f| f.contains("oracle does not open it"))
    );
    assert!(failures.iter().any(|f| f.contains("no pages to score")));
    std::fs::remove_dir_all(&tmp).expect("clean up");
}

/// The gate B file floor check, on hand-made counts.
#[test]
fn check_gate_b_floor_rules() {
    // Pass: at or above floor.
    assert_eq!(check_gate_b_floor(60), "");

    // Fail: below floor.
    let msg = check_gate_b_floor(59);
    assert!(msg.contains("59"));
    assert!(msg.contains("60"));
}

/// A one-page PDF showing `text` in Helvetica 12 pt, from the test kit.
fn pdf_extract_testkit_bytes(text: &str) -> Vec<u8> {
    let mut b = pdf_extract_testkit::PdfBuilder::new();
    b.add_page_a4()
        .insert_text(
            (72.0, 72.0),
            text,
            pdf_extract_testkit::TextStyle::size(12.0),
        )
        .expect("WinAnsi text");
    b.to_bytes()
}
