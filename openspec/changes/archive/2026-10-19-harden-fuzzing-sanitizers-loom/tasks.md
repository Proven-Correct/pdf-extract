# Tasks

## 1. Codec panic boundary (internal #76)

- [x] 1.1 In the vendored `hayro-syntax`, add `guard_codec` in `filter/mod.rs`, gated `cfg(all(feature = "std", not(fuzzing)))`. Wrap only the codec calls in `filter/{dct,ccitt,jbig2,jpx}.rs` with it, mapping a caught panic to `DecodeFailure::ImageDecode`. Add `check-cfg` for `cfg(fuzzing)` to the vendored `Cargo.toml`. Verify that `cargo build` in core passes, and that `cargo build --features hayro-syntax/…` with `RUSTFLAGS=--cfg fuzzing` compiles the guard out (a `cfg` assert in a unit test).
- [x] 1.2 Add the `codec-panic-hook` feature to `hayro-syntax`: a thread-local arm flag and `arm_codec_panic_for_test()` that make the next guarded codec call panic inside the guard. Forward it as core's `test-hooks` feature, and enable it in core's and render's dev-dependencies. Verify that a core unit test arms the hook and sees the panic flag cleared after one decode.
- [x] 1.3 Commit the vendor edits as their own commit with subject `H18: …`, and run `tools/vendor/export-patches.sh` to produce `vendor/patches/hayro/0028-H18-…patch`. Verify that `tools/vendor/check-patches.sh` reports 28 patches and that `tools/vendor/test-check-patches.sh` passes.
- [x] 1.4 Add Rust tests for the error-model scenarios:
  - A render with the hook armed returns a raster with exactly one `ImageDecodeFailure` warning and no panic. Cover JBIG2, JPX, DCT and CCITT fixtures from the testkit.
  - A panic outside the codec, in an interpreter-path test hook, still propagates.

  Verify that the tests fail with the guard removed and pass with it.
- [x] 1.5 In `crates/python/src/guard.rs`, clear `LAST_PANIC` on entry to `guarded`. Enable `codec-panic-hook` in the binding and add `_native._arm_codec_panic_for_test()` (with stub and stubtest entry), guarded like every entry point. Verify that the `panic_guard` source-scan test, stubtest and mypy pass.
- [x] 1.6 Add Python tests:
  - OCR of a page with the hook armed returns warnings that include `image: decode failed`, with no `InternalError`.
  - Extend `test_no_output.py` so a caught codec panic in the subprocess leaves fd 1 and fd 2 empty.
  - A later `_panic_for_test()` carries its own message.

  Verify with `pytest tests/python -k "codec or no_output"`.
- [x] 1.7 Add a Rust test that installs a custom panic hook, triggers an armed codec panic, and asserts that the custom hook ran, so the library did not replace it. Verify that it passes, and that it runs in its own test binary so the global hook doesn't leak into other tests.
- [x] 1.8 Write the ADR that partially supersedes 0035, using `adr_index.py --next` and the template: codec-boundary exception, `cfg(fuzzing)` compile-out, and panic-hook ownership. Flip 0035 to record the partial supersession per the ADR workflow. Add an addendum to ADR 0079 pointing at it. Verify that `adr_index.py docs/adr` exits 0.
- [x] 1.9 Update `ARCHITECTURE.md` and the README error-model section (Rust callers no longer receive codec panics; hook ownership), and the `vendor/patches` notes for H18. Verify that `cargo doc` with `RUSTDOCFLAGS=-D warnings` passes.

## 2. loom model checking

- [x] 2.1 Add `[lints.rust] unexpected_cfgs` `check-cfg` for `cfg(loom)` and `cfg(fuzzing)` to the workspace lints. Add `loom` as a `[target.'cfg(loom)'.dependencies]` of core and ocr. Add private `sync` shim modules to core and ocr. Verify that `cargo clippy --workspace -D warnings` passes, and that `cargo deny check` stays clean.
- [x] 2.2 Extract the close protocol into `crates/core/src/close_cell.rs` (`CloseCell<T>`: `new`, `get`, `close`, `is_closed`) using the shim, and make `DocShared` hold `CloseCell<Loaded>`. Give the file a header comment. Verify that all existing close tests pass unchanged: `document_tests.rs`, `tests/concurrency.rs`, and Python `test_document.py` and `test_concurrency.py`.
- [x] 2.3 Move `crates/ocr/src/pool.rs` onto the shim. Make the pid source a `fn() -> u32` field (default `std::process::id`), and keep `FORK_RESETS` as a std atomic. Verify that `pool_tests.rs` and `tests/pool.rs` pass unchanged.
- [x] 2.4 Add `crates/core/tests/loom_close.rs` (`#![cfg(loom)]`) with three cases: close racing an operation, release by the last operation, and double close. Verify with `RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 cargo test -p pdf-extract-core --test loom_close --release`.
- [x] 2.5 Add `crates/ocr/tests/loom_pool.rs` with four cases:
  - 3 callers against `max_instances = 2`, checking the cap and that no wake-up is lost;
  - a failed creation waking a waiter;
  - a return across a simulated fork reset;
  - the `Slot::drop` generation-mismatch path.

  Verify with the same loom command. If loom finds a lost wake-up, fix it in `pool.rs` with a regression case, and record the fix in the task notes.
- [x] 2.6 Add a `loom` job to `ci.yml` on the pinned stable toolchain that runs both loom tests. Verify that `actionlint` (or `shellcheck` on any inline scripts) passes, and the job runs green under `gh workflow run ci.yml --ref <branch>` before landing.
- [x] 2.7 Add an addendum to ADR 0040 (loom coverage, `CloseCell`). Update `ARCHITECTURE.md` (the `sync` shim, `CloseCell`, the pool pid seam, how to run loom). Verify that `adr_index.py` exits 0.

## 3. Fuzz crate and regression replay

- [x] 3.1 Add core's `fuzzing` feature with `#[doc(hidden)] pub mod fuzz_api`. It re-exports the `text/unicode.rs` fallbacks needed by the unicode target, and defines `fuzz_limits()` (the D6 values). Verify that `cargo test -p pdf-extract-core --features fuzzing` passes, and that a unit test asserts every `Limits` field is set by `fuzz_limits()` (a struct-literal exhaustiveness check).
- [x] 3.2 Create the `fuzz/` crate with its own `[workspace]`, `rust-toolchain.toml` (dated nightly, resolved per the design's open question), the root `[patch.crates-io]` hayro entries copied over, and a committed `Cargo.lock`. Add the five targets `open`, `text`, `render`, `cmap` and `unicode`, using `fuzz_limits()`. Verify that `cargo +<pinned> fuzz build` succeeds for all five, and that each runs for 60 s from its seeds locally.
- [x] 3.3 Add `tools/fuzz/check-patch-sync.sh`, which fails if the `fuzz/` and root `[patch.crates-io]` tables differ, with a `--self-test`. Wire it into `ci.yml`'s `vendor-patches` job. Verify that the self-test passes and shellcheck is clean.
- [x] 3.4 Add seed tooling:
  - `tools/fuzz/seed-cmaps.py`, which extracts ToUnicode streams from corpus C plus embedded CMaps;
  - committed `fuzz/seeds/unicode/`;
  - document targets that read corpus B and C directly.

  Verify that each target's seed set is non-empty and the script passes ruff and mypy.
- [x] 3.5 Add `fuzz/regressions/<target>/` with a README, plus `crates/core/tests/fuzz_regressions.rs` and `crates/render/tests/fuzz_regressions.rs`, which replay every regression file through the matching entry point with `fuzz_limits()` on `try_on_small_stack`. Verify that they pass on an empty set and on a hand-placed sample file per target, and run in the per-push `rust` job.
- [x] 3.6 Add `fuzz/src/bin/replay_small_stack.rs`, which replays a directory of inputs through the document-level entry points on 512 KiB threads. Verify that it runs clean over corpus B and C.
- [x] 3.7 Document the fuzz crate in `ARCHITECTURE.md`: layout, nightly pin, how to add a regression, and the `cfg(fuzzing)` compile-out. Verify that the documented commands run as written.

## 4. Local fuzzing script and ledger

- [x] 4.1 Add `tools/fuzz/ledger.py` with `record`, `status` and `reset`, and a committed initial `fuzz/ledger.json` (all targets at 0). Add unit tests in the Python suite for:
  - a clean session adds the time it fuzzed;
  - a finding resets to zero;
  - an interrupted session credits only elapsed time;
  - the exit is met at 72 hours or more for every target.

  Verify that pytest, ruff and mypy pass.
- [x] 4.2 Add `tools/fuzz/run.sh [--minutes N] [targets…]`. It merges seeds into the gitignored `fuzz/corpus/`, runs each target with the D6 flags, runs `cmin`, runs the small-stack replay, and records to the ledger. On a finding it keeps the artifacts and resets. On `SIGINT` it credits elapsed clean time. Verify:
  - shellcheck is clean;
  - a 2-minute run on macOS updates the ledger;
  - a planted panicking input in a scratch target resets its count.
- [x] 4.3 Document the local fuzzing workflow and the ledger policy (resets, committing updates, the 72-hour exit) in `ARCHITECTURE.md`. Verify that the documented commands run as written.

## 5. Dispatch-only hardening workflow and sanitizers

- [x] 5.1 Verify that the pinned `cmake` and `cc` crate versions pass `CFLAGS`/`CXXFLAGS` to the vendored Tesseract and Leptonica builds. Add `rerun-if-env-changed` for both to `crates/tesseract-sys/build.rs`. Verify with a build where `CXXFLAGS=-DPXE_PROBE` causes a compile-time `#error` probe, then remove the probe.
- [x] 5.2 Add `tools/sanitizers/run.sh asan|tsan`, which runs with a per-sanitizer `CARGO_TARGET_DIR`, `-Zbuild-std`, the instrumented C/C++ flags, and the `PXE_SANITIZER` skip of stack-measuring tests (enumerated). Gate those tests on `PXE_SANITIZER`. Verify that `run.sh asan` passes locally on macOS, and that shellcheck is clean.
- [x] 5.3 Add `tools/sanitizers/{lsan,tsan}.supp` and `check-suppressions.sh` (a `# why:` comment is required; workspace crates are rejected), with a `--self-test`. Wire it into `ci.yml`'s `shellcheck` job. Verify that the self-test passes.
- [x] 5.4 Add `.github/workflows/hardening.yml`: `workflow_dispatch` only, with inputs `job` (`fuzz-smoke`, `asan`, `tsan`, `all`) and `fuzz_minutes` (default 10), and no `schedule` or `push`. It has:
  - a fuzz-smoke matrix over the five targets, uploading artifacts on a finding;
  - `asan` and `tsan` jobs calling `run.sh`, with a 120-minute timeout, uploading reports on failure.

  Verify that `actionlint` passes, then dispatch `fuzz-smoke`, `asan` and `tsan` once each from the branch. Record their outcomes and any initial suppressions (with justifications) in the PR.
- [x] 5.5 Run `tools/sanitizers/run.sh tsan` and record the result. File any sanitizer or fuzz-smoke findings as GitHub issues rather than fixing them in this change (per the proposal's out-of-scope note). Verify that the issue links are listed in the PR.
- [x] 5.6 Write the ADR for the cost-capped hardening model: dispatch-only workflow, local fuzzing with a committed ledger, the nightly pin, and sanitizer scope and platforms. Update `ARCHITECTURE.md` (the hardening workflow and how to dispatch it, the sanitizer script). Verify that `adr_index.py` exits 0.

## 6. Integration

- [x] 6.1 Run the full local gate on the final branch: fmt, clippy `-D warnings`, `cargo test --workspace`, `cargo doc` with `-D warnings`, vendor patch checks, shellcheck, the Python suite with ruff, mypy and stubtest, and both loom tests. Verify that everything passes.
- [x] 6.2 Adversarially review the whole change in a fresh context: the codec boundary's scope, `AssertUnwindSafe` soundness, `CloseCell` equivalence, the loom model's coverage, and the ledger logic. Resolve blocking findings. Verify that the review verdict is pass.
- [x] 6.3 After landing and green CI on `main`, run `openspec validate harden-fuzzing-sanitizers-loom --strict`, update the design-spec Phase 3 row to note the cost-capped fuzzing model, and refresh the docs. Verify that validation passes and the docs PR lands.
