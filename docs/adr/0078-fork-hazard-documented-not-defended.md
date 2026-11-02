---
id: 0078
title: The fork-deadlock hazard is documented, not defended
date: 2026-10-05
status: Accepted
summary: No atfork handlers are installed; the fork hazard of the OCR locks is stated in every place a caller looks and the safe patterns are spawn/forkserver or building engines before forking.
---

# 0078. The fork-deadlock hazard is documented, not defended

## Context

Spec 0002 section 2.4 left a residual: a `fork()` while another thread is inside the first `default_ocr_engine()` construction, the process-wide Tesseract initialisation (`OnceLock`), an engine's pool lock, or the process-isolation client / tessdata-override locks leaves that lock held forever in the child, and the child's next OCR call deadlocks. The deadlock was reproduced during run 0002. Spec 0003 section 1.1 (A1) asked whether to defend against it. User decision, 2026-10-05: no.

## Decision

No `atfork` handlers of any kind, and no CI test that reproduces the deadlock (it would hang on failure). Instead the hazard is stated once and quoted wherever a caller looks before forking: the `# Forking` section of the `pdf-extract-ocr` crate docs with cross-references from each lock holder, the Python docstrings and pyclass doc (including that CPython 3.14 moved the POSIX default start method from `fork` to `forkserver`), the README, ARCHITECTURE.md (every lock named) and THEORY.md (why no handler). The safe patterns are: build every engine, the default one included, before forking; do not fork while another thread may be inside OCR; prefer `spawn`/`forkserver`/`posix_spawn`.

## Alternatives considered

- **`pthread_atfork` handlers that reset or release the locks** - a `OnceLock` caught mid-initialisation has no safe reset; releasing a pool lock in the child would hand it an instance another thread was using.
- **Process-wide handlers installed by the library** - a library installing global `pthread_atfork` handlers into its host process is itself a global side effect.
- **Make `Isolation::Process` the default so the parent holds no OCR state** - deferred to Phase 3 with wheel packaging (spec 0003 section 2.1, A3); ADR 0061 stands.

## Consequences

Callers who fork a threaded process during OCR can still deadlock; this is a documented residual, not a bug to fix. Any new lock added to the OCR path must be added to the hazard statement and its cross-reference. 0002 section 2.4 stays; O12 was corrected in wording only (a panic in the first construction leaves the `OnceLock` empty; see the addendum on ADR 0035).
