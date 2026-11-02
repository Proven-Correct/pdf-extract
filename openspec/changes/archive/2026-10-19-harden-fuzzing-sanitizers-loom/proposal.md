# Proposal

## Why

Phase 3 of the design spec (§16) requires hardening: fuzzing, sanitizers and `loom` model checking. Its exit criterion is fuzzing clean for 72 hours. None of these exist yet. No fuzz target, sanitizer job, `loom` test or scheduled workflow is in the repository.

The vendored image codecs still contain unaudited panic sites that input can reach (internal #76). Requirement R1 says a crafted image must not fail the whole page, but today it does: Rust callers get a panic and Python callers get `InternalError`.

The decision during the internal #63 fix pass was to fix this at the codec boundary together with fuzzing. Patching panic shapes one at a time does not end the chain. Fuzzing is what finds the reachable sites, and the boundary is what keeps them from failing a page in production.

This is the first of the three Phase 3 changes. The wheels, SBOM and internal #53 isolation-default change and the license-pipeline change follow it.

## What Changes

- **Fuzzing.** A `fuzz/` crate holds `cargo-fuzz` targets, in its own workspace and on the pinned nightly toolchain. The five targets come from design-spec §13.6:
  1. `open` + `page_count`
  2. `text_page` on every page
  3. render page 0 at 36 dpi
  4. the CMap parser
  5. the font-to-Unicode fallbacks

  Each target runs with tightened limits. Corpora B and C provide the seeds. Any panic, timeout beyond the page budget, or allocation beyond the limits counts as a finding.
- **Fallback entry points.** Core gets a `fuzzing` feature that exposes those fallback functions as a hidden module, so target 5 can drive them from raw bytes.
- **Small-stack corpus replay (§8.6).**
  - A committed `fuzz/regressions/` set of minimized reproducers is replayed on 512 KiB threads by an ordinary `cargo test`.
  - Each local fuzzing session ends by replaying its accumulated corpus on 512 KiB threads.
- **Runner cost is tightly capped.** The repository is internal, so Actions minutes are billed, and long CI fuzzing buys little over local runs.
  - **Local fuzzing.** Long fuzzing runs on a developer machine through `tools/fuzz/run.sh`, which is resumable and keeps a persistent local corpus.
  - **Clean-hours ledger.** The script keeps a committed per-target ledger, `fuzz/ledger.json`. A session with no finding adds the time it fuzzed, and any finding resets that target to zero. The 72-hour exit criterion is met when every target reaches 72 clean hours.
  - **CI.** A new `.github/workflows/hardening.yml` runs on `workflow_dispatch` only, never on a schedule. It offers a fuzz smoke run of about 10 minutes per target, and AddressSanitizer and ThreadSanitizer runs of the Rust test suite with the vendored Tesseract C++ instrumented through the FFI.
  - **Local sanitizers.** The same sanitizer runs are available locally through `tools/sanitizers/run.sh`.
  - **Per-push CI.** `ci.yml` and its trigger policy (ADR 0085) are unchanged, apart from the stable regression replay and the loom job. A new ADR records the dispatch-only hardening workflow and the local ledger.
- **`loom` model checking.**
  - The document close protocol is extracted into a small generic cell.
  - The OCR pool's synchronisation goes through a `cfg(loom)` shim.
  - `loom` tests cover close-during-use, release on last operation, pool checkout and return under `max_instances`, failed creation, and fork reset.
  - They run in `ci.yml` on the stable toolchain.
- **Codec panic boundary (internal #76).** The vendored `hayro-syntax` image filters run each codec decode (JBIG2, JPEG 2000, DCT, CCITT) inside `catch_unwind`. This becomes vendor patch 0028 (H18).
  - A caught panic becomes the existing `DecodeFailure::ImageDecode`. The image is skipped with an `image: decode failed` warning, and the rest of the page is processed.
  - Under `cfg(fuzzing)` the catch is compiled out, so fuzzing still treats every codec panic as a crash.
- **BREAKING (Rust API contract).** The Rust API no longer lets every panic propagate. Panics in image codecs are caught and become warnings. A new ADR partially supersedes ADR 0035.
- **No-output requirement narrowed.** The library stays silent on stdout and stderr when called from Python. For Rust callers, the process-global panic hook belongs to the application, so a caught codec panic is printed or not according to the application's hook.

## Capabilities

### New Capabilities
- `fuzzing`: the fuzz targets, their limit configuration and seeds, what counts as a finding, the committed regression set and its small-stack replay, local long-running fuzzing, the manual CI smoke run, and the committed clean-hours ledger behind the Phase 3 exit criterion.
- `sanitizer-checks`: on-demand (manual CI dispatch or local script) AddressSanitizer and ThreadSanitizer runs of the Rust test suite, including the vendored C++ reached through the Tesseract FFI, and the rules for suppressions.

### Modified Capabilities
- `error-model`:
  - "The Rust API lets panics propagate" gains the image-codec exception.
  - A new requirement covers skipping an image whose codec panics.
  - "The library writes nothing to stdout or stderr" is scoped to the Python package and to the application's panic hook.
  - "The library runs on 512 KiB thread stacks" adds the fuzz-corpus replay.
- `ocr-rendering`: "Hostile inputs render or hit a limit". A codec panic during rendering skips the image instead of propagating.
- `concurrency`: new requirements that the close protocol and the OCR pool are model-checked with `loom`.

## Impact

- **Vendored code:**
  - `vendor/hayro-syntax/src/filter/{jbig2,jpx,dct,ccitt}.rs` get a new patch 0028 (H18) and a test-only panic-injection hook behind a feature.
  - `vendor/hayro-syntax/Cargo.toml` declares the `check-cfg` for `fuzzing`.
- **Core:**
  - `crates/core/src/document.rs` changes: the close protocol moves into a generic cell in a new `sync` shim module.
  - Core gets a `fuzzing` feature with a hidden export of the `text/unicode.rs` fallbacks.
  - Lint configuration gets `check-cfg` entries for `loom` and `fuzzing`.
- **OCR:** `crates/ocr/src/pool.rs` moves to the `cfg(loom)` shim, and the pid lookup becomes a seam.
- **New files:**
  - `fuzz/` (own workspace and `Cargo.lock`, replicating the root `[patch.crates-io]`)
  - `.github/workflows/hardening.yml` (dispatch-only)
  - `tools/fuzz/run.sh`, `tools/fuzz/ledger.py`, `fuzz/ledger.json`
  - `tools/sanitizers/run.sh`
  - sanitizer suppression files under `tools/`
  - a regression-replay test
- **CI:**
  - `ci.yml` gains a `loom` job.
  - The hardening workflow and the local scripts use a pinned, dated nightly toolchain. The repository's pinned stable toolchain is unchanged.
  - Recurring Actions cost is near zero. Per push, the loom job and the regression replay add minutes. The dispatch-only smoke run costs about one runner-hour, and each sanitizer run one to two.
- **Dependencies:**
  - `loom` (MIT), only under `cfg(loom)`.
  - `libfuzzer-sys` (MIT/Apache-2.0, with NCSA-licensed libFuzzer), only in the separate `fuzz/` workspace and outside the shipped artifacts and `cargo-deny`.
- **ADRs:**
  - a new ADR partially superseding 0035 (codec panic boundary, panic-hook ownership)
  - a new ADR for the dispatch-only hardening workflow and local fuzzing with a committed ledger, alongside 0085
  - an addendum to 0040 (loom coverage of the close protocol)
- **Out of scope:**
  - Fixing the bugs fuzzing finds. They go through the ordinary issue workflow as issues.
  - The JBIG2 decode-termination internal issues #116 and #115, which fuzzing is expected to exercise.
  - Wheels, SBOM, internal #53 and the license pipeline.
