//! OCR agreement on a private directory of real scans (spec 0002 §3.8
//! "Private scans"), driven by `tools/oracle/run_private_scans.sh`; never
//! run on CI.
//!
//! Real scans have no ground truth, so the metric is agreement with the
//! oracle: per page, the character similarity of our 72-dpi and our
//! 300-dpi text with the oracle's 72-dpi text, and whether both sides would
//! route the page as sparse at `min_chars = 50` after OCR. The report holds
//! lengths, similarities and flags only, never text, because the scans may
//! be confidential. Every threshold is report-only in this run; the test
//! fails only when a file cannot be scored at all.
//!
//! Inputs (environment): `PRIVATE_SCANS_DIR` (the PDFs),
//! `PRIVATE_SCANS_EXPECTATIONS` (the oracle's schema-2 JSON from
//! `gen.py --ocr`), `PRIVATE_SCANS_REPORT` (where the report goes).

use std::path::{Path, PathBuf};

use serde_json::json;

use super::metrics::{quantile, similarity, sparse};
use super::{expectation, model_dir, recognise, sha256_hex};
use pdf_extract_ocr::{OcrConfig, OcrEngine};

/// `extract`'s default routing threshold (D-15).
const MIN_CHARS: usize = 50;

/// The bytes of `path`, or the report line for a file that cannot be read.
fn read_pdf(path: &Path, rel: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("{rel}: {e}"))
}

/// `path` relative to the scans root, as the report names it; the root
/// itself is `.` so a failure to list it still has a name.
fn relative(dir: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(dir).unwrap_or(path).display().to_string();
    if rel.is_empty() { ".".to_owned() } else { rel }
}

/// Every `*.pdf` under `dir`, sorted, dot-entries (as `gen.py`) and symlinked
/// directories skipped, plus a report line for each directory or entry that could not be
/// listed. Like an unreadable file, an unlistable directory is a per-input
/// error in the report, not a panic that loses everything scored so far.
fn pdfs(dir: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let (mut out, mut errors) = (Vec::new(), Vec::new());
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = match std::fs::read_dir(&d) {
            Ok(entries) => entries,
            Err(e) => {
                errors.push(format!("{}: {e}", relative(dir, &d)));
                continue;
            }
        };
        for e in entries {
            let p = match e {
                Ok(e) => e.path(),
                // Untested: an error on one entry of a listing that
                // succeeded (EIO, a stale NFS handle) cannot be provoked
                // portably. Accepted residual, internal #149.
                Err(e) => {
                    errors.push(format!("{}: {e}", relative(dir, &d)));
                    continue;
                }
            };
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            // `symlink_metadata`, not `is_dir()`: that follows symlinks, and a
            // link to an ancestor would relist every PDF once per level
            // until the OS gives up with ELOOP. Linked directories are not
            // descended into; a linked file is still listed.
            if p.symlink_metadata().is_ok_and(|m| m.is_dir()) {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "pdf") {
                out.push(p);
            }
        }
    }
    out.sort();
    (out, errors)
}

/// The scoring loop's input: each PDF under `dir` as `(path, report name,
/// bytes)`, or the report line for a directory or file that could not be
/// read. Listing and reading live here rather than inline in
/// `private_scans` so the non-ignored tests below exercise the same path
/// `private_scans` takes, without a model or real scans. Listing errors
/// come first; the files are read lazily, one at a time, so a large corpus
/// is never held in memory at once.
fn inputs(dir: &Path) -> impl Iterator<Item = Result<(PathBuf, String, Vec<u8>), String>> + '_ {
    let (paths, errors) = pdfs(dir);
    errors
        .into_iter()
        .map(Err)
        .chain(paths.into_iter().map(move |path| {
            let rel = relative(dir, &path);
            read_pdf(&path, &rel).map(|bytes| (path, rel, bytes))
        }))
}

#[test]
#[ignore = "private corpus; run through tools/oracle/run_private_scans.sh"]
fn private_scans() {
    let (Some(dir), Some(exps), Some(report)) = (
        std::env::var_os("PRIVATE_SCANS_DIR").map(PathBuf::from),
        std::env::var_os("PRIVATE_SCANS_EXPECTATIONS").map(PathBuf::from),
        std::env::var_os("PRIVATE_SCANS_REPORT").map(PathBuf::from),
    ) else {
        eprintln!("private_scans: SKIPPED - set PRIVATE_SCANS_DIR, _EXPECTATIONS and _REPORT");
        return;
    };
    let model = model_dir().expect("no eng.traineddata: set TESSDATA_PREFIX");
    let model_sha = sha256_hex(&std::fs::read(model.join("eng.traineddata")).expect("model"));
    let engine = OcrEngine::tesseract_with_candidates(
        OcrConfig {
            tessdata: Some(model),
            ..OcrConfig::default()
        },
        &super::common::nowhere(),
    )
    .expect("engine");

    let mut rows = Vec::new();
    let mut errors = Vec::new();
    let (mut sim72, mut sim300) = (Vec::new(), Vec::new());
    let (mut agree72, mut agree300, mut pages) = (0usize, 0usize, 0usize);
    for input in inputs(&dir) {
        let (path, rel, bytes) = match input {
            Ok(input) => input,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let sha = sha256_hex(&bytes);
        let exp = expectation(&exps, &sha);
        let ocr = &exp["ocr"];
        if ocr["tessdata_sha256"].as_str() != Some(model_sha.as_str()) {
            errors.push(format!("{rel}: expectation made with another model"));
            continue;
        }
        let Some(oracle_pages) = ocr["pages"].as_array() else {
            errors.push(format!("{rel}: expectation has no ocr block"));
            continue;
        };
        for (i, op) in oracle_pages.iter().enumerate() {
            let oracle = op["text"].as_str().unwrap_or("");
            let (r72, r300) = (
                recognise(&engine, &path, i, 72),
                recognise(&engine, &path, i, 300),
            );
            let ((t72, ms72), (t300, ms300)) = match (r72, r300) {
                (Ok(a), Ok(b)) => (a, b),
                (a, b) => {
                    errors.push(format!("{rel} page {i}: {:?} {:?}", a.err(), b.err()));
                    continue;
                }
            };
            let (s72, s300) = (similarity(&t72, oracle), similarity(&t300, oracle));
            let sparse_oracle = sparse(oracle, MIN_CHARS);
            let (a72, a300) = (
                sparse(&t72, MIN_CHARS) == sparse_oracle,
                sparse(&t300, MIN_CHARS) == sparse_oracle,
            );
            sim72.push(s72);
            sim300.push(s300);
            agree72 += usize::from(a72);
            agree300 += usize::from(a300);
            pages += 1;
            // Lengths, similarities and flags only: never the text.
            rows.push(json!({
                "file": rel, "page": i,
                "len_oracle72": oracle.chars().count(),
                "len_ours72": t72.chars().count(),
                "len_ours300": t300.chars().count(),
                "similarity72": s72, "similarity300": s300,
                "sparse_oracle72": sparse_oracle,
                "routing_agrees72": a72, "routing_agrees300": a300,
                "ms72": ms72, "ms300": ms300,
            }));
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let rate = |n: usize| {
        if pages == 0 {
            0.0
        } else {
            n as f64 / pages as f64
        }
    };
    let summary = json!({
        "pages": pages,
        "similarity72_median": quantile(&sim72, 0.5),
        "similarity72_p10": quantile(&sim72, 0.1),
        "similarity300_median": quantile(&sim300, 0.5),
        "similarity300_p10": quantile(&sim300, 0.1),
        "routing_agreement72": rate(agree72),
        "routing_agreement300": rate(agree300),
        "min_chars": MIN_CHARS,
    });
    let out = json!({
        "schema": 1, "model_sha256": model_sha, "summary": summary,
        "errors": errors, "pages": rows,
    });
    std::fs::write(
        &report,
        serde_json::to_string_pretty(&out).expect("json") + "\n",
    )
    .expect("write report");
    eprintln!(
        "private scans: {pages} pages; similarity to the oracle@72: ours@72 median {:.4} p10 \
         {:.4}, ours@300 median {:.4} p10 {:.4}; routing agreement @72 {:.4}, @300 {:.4} \
         (report-only)",
        summary["similarity72_median"].as_f64().unwrap_or(0.0),
        summary["similarity72_p10"].as_f64().unwrap_or(0.0),
        summary["similarity300_median"].as_f64().unwrap_or(0.0),
        summary["similarity300_p10"].as_f64().unwrap_or(0.0),
        rate(agree72),
        rate(agree300),
    );
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// A scratch directory that removes itself on drop, so a test that panics
/// (a failed assertion or a setup `expect`) cannot leak it. A leaked
/// mode-000 `locked` directory would also defeat the next run's
/// `remove_dir_all`, hence the chmod first.
struct Scratch(PathBuf);

impl Scratch {
    fn remove(dir: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0o700))
                .ok();
        }
        std::fs::remove_dir_all(dir).ok();
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        Self::remove(&self.0);
    }
}

/// A fresh, empty scratch directory for one test.
fn scratch(name: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("private-scans-{name}-{}", std::process::id()));
    Scratch::remove(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    Scratch(dir)
}

/// `inputs(dir)` split into the report names it yielded and its errors.
fn gather(dir: &Path) -> (Vec<String>, Vec<String>) {
    let (mut names, mut errors) = (Vec::new(), Vec::new());
    for input in inputs(dir) {
        match input {
            Ok((_, rel, _)) => names.push(rel),
            Err(e) => errors.push(e),
        }
    }
    (names, errors)
}

#[cfg(unix)]
#[test]
fn an_unreadable_pdf_is_reported_by_name_not_as_a_panic() {
    let dir = scratch("unreadable-pdf");
    std::fs::create_dir_all(dir.join("sub")).expect("sub");
    std::fs::write(dir.join("sub/good.pdf"), b"%PDF-").expect("good.pdf");
    // A dangling symlink: `pdfs()` lists it (not a directory, `.pdf`
    // extension) but reading it fails, the shape of a permission or I/O
    // error on a real file.
    std::os::unix::fs::symlink(dir.join("missing"), dir.join("sub/scan.pdf")).expect("symlink");
    let (names, errors) = gather(&dir);
    assert_eq!(names, ["sub/good.pdf"]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("sub/scan.pdf: "), "{errors:?}");
}

#[test]
fn an_unlistable_root_is_reported_not_as_a_panic() {
    let dir = scratch("unlistable-root");
    let file = dir.join("not-a-dir");
    std::fs::write(&file, b"").expect("file");
    for root in [file, dir.join("missing")] {
        let (names, errors) = gather(&root);
        assert!(names.is_empty(), "{names:?}");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with(".: "), "{errors:?}");
    }
}

#[cfg(unix)]
#[test]
fn an_unreadable_subdirectory_is_reported_by_name_not_as_a_panic() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("unreadable-dir");
    let locked = dir.join("locked");
    std::fs::create_dir_all(&locked).expect("locked");
    std::fs::write(dir.join("good.pdf"), b"%PDF-").expect("good.pdf");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    // Root reads a mode-000 directory anyway; there is nothing to test then.
    let enforced = std::fs::read_dir(&locked).is_err();
    let (names, errors) = gather(&dir);
    if !enforced {
        eprintln!("SKIPPED: permissions are not enforced for this user");
        return;
    }
    assert_eq!(names, ["good.pdf"]);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("locked: "), "{errors:?}");
}

#[cfg(unix)]
#[test]
fn a_symlink_to_an_ancestor_does_not_relist_every_pdf() {
    let dir = scratch("symlink-loop");
    std::fs::create_dir_all(dir.join("sub")).expect("sub");
    std::fs::write(dir.join("a.pdf"), b"%PDF-").expect("a.pdf");
    std::fs::write(dir.join("sub/b.pdf"), b"%PDF-").expect("b.pdf");
    // One link only: a second one (e.g. `sub/up -> ..`) would make the
    // unfixed listing branch exponentially, not just repeat each PDF per
    // level until ELOOP.
    std::os::unix::fs::symlink(".", dir.join("self")).expect("self link");
    let (names, errors) = gather(&dir);
    assert_eq!(names, ["a.pdf", "sub/b.pdf"]);
    assert!(errors.is_empty(), "{errors:?}");
}
