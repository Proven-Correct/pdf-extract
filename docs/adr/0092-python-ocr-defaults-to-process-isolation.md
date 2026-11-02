---
id: 0092
title: Python OCR defaults to process isolation; Rust stays in-process
date: 2026-10-22
status: Accepted
summary: OcrEngine(), default_ocr_engine() and Page.ocr() without an engine run Tesseract in helper processes from Python (the binding has its own process-isolated default engine); Rust's OcrConfig::default() and default_ocr_engine() stay InProcess; a missing helper is engine_not_built with no fallback.
---

# 0092. Python OCR defaults to process isolation; Rust stays in-process

## Context

This record partially supersedes ADR 0061, for the Python binding only.
ADR 0061 made in-process OCR the default and left flipping it to spec Q1,
"once measured". Run 0002 measured the cost of process isolation at 1.6 %
on clean 300 dpi pages, and 0.3 % in a noisy re-measurement. Both are far
under the 10 % bar. Process isolation contains Tesseract's `ASSERT_HOST`
aborts, which otherwise end the caller's process (R1, internal issue #53).

Three things blocked the flip, as the fix-pass comment on internal #53 records:

1. **Helper discovery.** Python's default engine was the Rust crate's. That
   engine is built with `helper_dir: None`, so it looks for the helper
   beside the *interpreter*.
2. **The raster cap.** Process mode caps `max_render_pixels` at 66,977,776,
   the most that fits one helper frame.
3. **Shipping the helper.** Nothing guaranteed that a wheel ships the
   helper. ADR 0091 now enforces it.

A Rust binary has no package directory. Flipping the Rust default would
oblige every Rust caller, tests and examples included, to install the
helper beside its executable.

## Decision

1. **Python.** `OcrEngine(isolation=...)` defaults to `"process"`.
   `default_ocr_engine()`, and `Page.ocr()` without an engine, use the
   binding's own process-wide engine. It is a `OnceLock` in
   `crates/python/src/ocr.rs`, built from `OcrConfig::default()` with
   `isolation = Process` and `helper_dir` set to the extension module's
   directory, and it keeps the semantics of O12:
   - an `Err` is stored and returned on every later call;
   - a panic is not stored, so the next call builds again.
2. **Rust.** `OcrConfig::default()` and `pdf_extract_ocr::default_ocr_engine()`
   stay `InProcess`, unchanged.
3. **A missing helper.** It is `OcrUnavailableError(reason=
   "engine_not_built")`, stored like any other failure. There is no
   automatic fallback to in-process OCR. When the package directory lacks
   the helper, the message names `tools/build/stage-helper.sh` and
   `isolation='in_process'`.
4. **The raster cap.** A Python `max_render_pixels` above the cap is
   refused with a `ValueError` that names `isolation='in_process'` as the
   way to use a larger value.

## Alternatives considered

- **Flip `OcrConfig::default()` too.** Every Rust caller would need the
  helper beside its executable or `PDF_EXTRACT_OCR_HELPER` set. The user
  chose Python only.
- **Fall back to in-process when the helper is missing.** That turns a
  broken install into an uncontained one, silently, which is the exposure
  internal #53 removes.
- **A keyed `default_ocr_engine_with(config)` in the Rust crate.** It adds
  API for one caller. A binding-local `OnceLock` is a few lines.

## Consequences

- From Python, a Tesseract abort fails one page (`OcrPageError`) instead of
  the interpreter, with no code change by the caller.
- Every Python OCR call pays the IPC and a helper's memory per pool slot.
  The measured cost is 0.3 % to 1.6 %. On macOS each helper spawn costs
  about 130 ms under a high fd limit (internal #56 B12), once per pool slot.
- **BREAKING for Python callers** in two cases:
  - `max_render_pixels` above 66,977,776 now needs `isolation="in_process"`.
  - Source or development installs without a staged helper lose default
    OCR until they run `tools/build/stage-helper.sh`.
- The binding's `OnceLock` is one more lock that a `fork()` can strand. It
  is listed in the `Forking` section of the `pdf-extract-ocr` crate docs.
- Python and Rust now differ in one default. The ocr-engine and
  ocr-process-isolation specs state both.
