# Proposal

## Why

The project's behavioural contract is spread across the design spec
(`docs/design-spec.md`), three run specs (`docs/specs/0001`–`0003`) that
refine and revise it, and 83 ADRs. There is no single statement of what the
system does today. This change backfills one as OpenSpec capability specs, so
that later work, starting with Phase 3, can be proposed as deltas against it.

## What Changes

- Add capability specs that state the behaviour that has been **decided and
  built**: design-spec Phases 0–2, plus run 0003 (fork-hazard documentation,
  JBIG2/JPEG 2000 codec memory budgets, the noise pre-check before Tesseract,
  and its small fixes).
- Requirements state the contract as the final revised rows of the run specs
  give it. The mechanism (hayro mappings, H-patch details, revision history)
  is left out. Each scenario cites its source IDs (for example `[F5, T9]`) so
  every requirement can be traced back to the run spec that decided it.
- Where the code currently falls short of a requirement, the requirement is
  kept at full strength and the gap is tracked as an open issue. For example,
  internal #63 (crafted JPX images panic) violates the no-crash requirement R1, and
  internal #70 (the Linux small-stack harness measures 2 MiB) undermines its test.
- **Out of scope:** everything not yet built: Phase 3 hardening (fuzzing,
  sanitizers, `loom`, wheels, SBOM, the license pipeline beyond what CI
  already enforces), making process isolation the default (internal #53), and the
  Phase 4 migration of the first consumer. These will come later as their own changes.

No code changes. No behaviour changes.

## Capabilities

### New Capabilities

- `document-open`: opening from a path or bytes, content-only file-type
  detection, the size limit, parser-outcome classification, encryption
  (owner-password-only documents open; user-password documents raise at
  open), and xref repair warnings.
- `page-index`: an honest, bounded page count from a lazy page-tree walk,
  `max_pages`, page-tree defects (cycles, depth, brute-force fallback),
  iteration and random access, and lightweight page handles.
- `text-extraction`: the compat-profile `text()` shape, glyph-to-Unicode
  mapping, filtering to the visible box, line and block assembly, the
  compat-point register, damaged-content and font-substitution handling, and
  immutable `TextPage`s.
- `resource-limits`: every documented limit and its default, the
  per-page interpretation work budget, the per-call OCR deadline, and the
  JBIG2/JPEG 2000 codec memory budgets.
- `concurrency`: `Send + Sync` types, parallel use of separate and shared
  documents, GIL release and free-threaded CPython, safe concurrent
  `close()`, determinism, import-time inertness, and the documented
  `fork()` hazard.
- `error-model`: the Rust error types and the Python exception hierarchy,
  panics converted to `InternalError` at the boundary, the bounded-stack
  guarantee, no output on stdout or stderr, and structured warnings.
- `test-kit`: creating fixture documents with text placed at exact
  positions, and saving them to a path or to bytes.
- `output-parity`: black-box parity with PyMuPDF 1.28.2 on the corpora, the
  routing-decision gate, and the rule that every difference is either
  configurable back to parity or documented.
- `licensing`: only permissive licenses in the shipped dependency graph, and
  no MuPDF or PyMuPDF code read or ported.
- `ocr-rendering`: rendering a page for OCR at a configurable resolution,
  within its limits and budgets.
- `ocr-engine`: the Tesseract engine and its bounded pool, finding the
  language data, the noise pre-check, recognition into a `TextPage` in the
  compat shape, and classifying failures as host-level or page-level.
- `ocr-batch-extract`: the `extract()` helper that routes each page between
  the text layer and OCR.
- `ocr-process-isolation`: opt-in OCR in a child process, and how its
  failures are contained and classified.

### Modified Capabilities

None. There are no existing specs.

## Impact

- Adds `openspec/specs/` (once this change is archived). Code, APIs and
  dependencies are unchanged.
- `docs/design-spec.md`, `docs/specs/` and `docs/adr/` remain the record of
  mechanism and rationale. The new specs state the contract and cite those
  documents.
