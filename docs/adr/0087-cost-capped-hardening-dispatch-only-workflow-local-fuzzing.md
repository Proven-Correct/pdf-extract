---
id: 0087
title: Cost-capped hardening - dispatch-only workflow, local fuzzing with a committed ledger
date: 2026-10-19
status: Accepted
summary: Fuzzing and sanitizer runs never start on push or schedule; long fuzzing runs locally and is counted in the committed fuzz/ledger.json, and .github/workflows/hardening.yml offers dispatch-only fuzz smoke, ASan and TSan runs on the pinned nightly-2026-10-01.
---

# 0087. Cost-capped hardening - dispatch-only workflow, local fuzzing with a committed ledger

## Context

Phase 3 of the design spec (§16) requires fuzzing, sanitizers and loom, and
its exit criterion is 72 hours of clean fuzzing. The repository is internal,
so GitHub Actions minutes are billed, and the user capped recurring runner
cost tightly. 72 hours across five fuzz targets in CI would be 360
runner-hours; a nightly sanitizer schedule would be a recurring bill for
results that change only when the code does. ADR 0085 already limits the
per-push pipeline (`ci.yml`) to pushes to `main` plus manual dispatch.

Fuzzing (`cargo fuzz`, libFuzzer), `-Zsanitizer` and `-Zbuild-std` all
need a nightly toolchain, while the repository pins stable 1.98.0.

## Decision

1. **Long fuzzing is local.** `tools/fuzz/run.sh` runs the five targets
   (`open`, `text`, `render`, `cmap`, `unicode`) on the developer's machine
   (macOS or Linux), resumably, and records each session in the committed
   `fuzz/ledger.json` (clean seconds per target, sessions, last reset, last
   commit). A finding resets that target to zero; unrelated commits do not.
   The Phase 3 fuzzing exit is met when every target has at least 72 clean
   hours in the ledger. The developer commits ledger updates through
   ordinary PRs.
2. **CI hardening is dispatch-only.** `.github/workflows/hardening.yml` has
   only `workflow_dispatch` (no `schedule`, no `push`), with inputs `job`
   (`fuzz-smoke`, `asan`, `tsan`, `all`; default `fuzz-smoke`) and
   `fuzz_minutes` (default 10, 1-60). The fuzz smoke run is a matrix over
   the five targets from the seeds plus the regression set, with no cache
   and no ledger update, and uploads `fuzz/artifacts/<target>/` on a
   finding. The `asan` and `tsan` jobs call `tools/sanitizers/run.sh`, with
   a 120-minute timeout, and upload the run's log on failure. `ci.yml` does
   not depend on any of it; ADR 0085 is unchanged.
3. **One dated nightly**, `nightly-2026-10-01`, pinned in
   `fuzz/rust-toolchain.toml`, in `tools/sanitizers/run.sh` (`NIGHTLY`)
   and in `hardening.yml` (`env.NIGHTLY`). Bumping it is a deliberate
   commit that changes all three, like the stable pin. The stable pin is
   unchanged; only the fuzz workspace, the sanitizer script and the
   dispatch-only workflow use nightly.
4. **Sanitizer scope and platforms.** `tools/sanitizers/run.sh asan|tsan`
   is the single entry point for local and CI runs. It runs
   `cargo +nightly test -Zbuild-std --target <host> --workspace --tests`
   with `-Zsanitizer=address|thread`, and passes `-fsanitize=…` through
   `CFLAGS`/`CXXFLAGS` to the cmake and cc builds of the vendored Leptonica
   and Tesseract and the C++ shim (verified to be forwarded by the pinned
   cmake 0.1.58 and cc 1.5.1; `tesseract-sys/build.rs` reruns when the
   flags change). Each sanitizer has its own `CARGO_TARGET_DIR`. On Linux
   the C/C++ compiler is clang, so the instrumentation matches the LLVM
   runtime rustc links. CI runs on ubuntu-24.04 only, with LeakSanitizer;
   locally the script also runs on macOS, with ASan but no leak detection
   (unsupported there) and with TSan. Excluded: doctests, the Python
   binding crate (no Rust tests; Python under sanitizers is a non-goal),
   the `system-tesseract` feature, the four tests of the small-stack
   runner's own measurement, and tests whose claim the instrumentation
   itself changes (wall-clock and page-deadline bounds, and the OCR
   helper's descriptor count), all skipped by name in run.sh with the
   reason; each still runs in the ordinary suite. Under
   `PXE_SANITIZER` the small-stack runners run their bodies on a 64 MiB
   stack without measuring it, because instrumentation inflates frames and
   the 512 KiB bound is not what a sanitizer run checks.
5. **Suppressions** live in `tools/sanitizers/{lsan,tsan}.supp`; every
   entry needs a `# why:` comment, and none may name this workspace's
   crates, the `tesseract_sys` symbols or the shim's `pxt_` functions.
   `tools/sanitizers/check-suppressions.sh` enforces this before each
   sanitizer run and in `ci.yml`'s `shellcheck` job on every push.

## Alternatives considered

- **Scheduled nightly CI fuzzing with a cache-based ledger** — rejected for
  cost: reaching 72 hours per target is 360 runner-hours, and a cache is
  evicted silently, losing the count.
- **Weekly short scheduled runs** (fuzz or sanitizers) — a recurring cost
  for little gain over local runs and on-demand dispatch.
- **Sanitizer jobs in `ci.yml`** — would either run per push (cost, and an
  hour-long instrumented Tesseract build on every merge) or need a second
  trigger policy inside a workflow ADR 0085 keeps simple.
- **Instrumenting only the Rust code** — cheaper and simpler, but leaves the
  FFI path into Tesseract unchecked, which is the part the sanitizer spec
  most wants covered. Kept only as the fallback if the C++ build proved
  unworkable; it did not.
- **gcc for the C/C++ side on Linux** — gcc's libasan/libtsan is a
  different runtime from the LLVM one rustc links; mixing them is
  unsupported.

## Consequences

- Recurring Actions cost for hardening is zero; a dispatched smoke run
  costs about one runner-hour at the defaults and each sanitizer run one to
  two.
- Fuzzing depth depends on someone running the local script. The ledger
  makes progress visible, but its hours are a progress log, not an
  attestation: they are as trustworthy as the PRs that commit them.
- CI catches only regressions of committed reproducers and shallow crashes
  a 10-minute smoke run finds.
- The nightly pin ages; it must be bumped deliberately in three places.
- On macOS, Apple clang's ASan objects reference a version-check symbol of
  their own (`__asan_version_mismatch_check_apple_clang_*`) that rustc's
  runtime does not define, so run.sh disables that check for C/C++ with
  `-mllvm -asan-guard-against-version-mismatch=0`; the C/C++
  instrumentation and the Rust runtime then come from different LLVM
  releases, which is the usual compatible case but not a guarantee.
- Findings from either kind of run become GitHub issues and go through
  the ordinary issue workflow; they are not fixed in the hardening change, and our crates are
  never suppressed.

## Addendum (2026-10-22)

Partially superseded by ADR 0093. The 72 clean hours per target are no longer
the Phase 3 fuzzing exit or a release gate. The ledger keeps counting them as
a depth goal, and its status line says "Fuzzing depth goal". Instead, callers
on untrusted input contain crashes and hangs. Everything else in this record
stands: the local script, the committed ledger and the dispatch-only
workflow. The status stays Accepted because the decision log has no
partial-supersession status, the same pattern as ADR 0035, 0061 and 0086.

## Addendum (2026-11-02)

The repository became public on 2026-11-02, so Actions minutes are no longer billed, and the cost reason above no longer holds. The decision stands for its other reasons:
- **The workflow stays dispatch-only.** Sanitizer and fuzz-smoke results change only when the code does, so a schedule would mostly rerun unchanged code and produce noise.
- **Long fuzzing stays local.** It needs a resumable, committed ledger (`fuzz/ledger.json`), which a cache-evicting CI runner cannot provide.

Pull requests now run `ci.yml` automatically (ADR 0098), which does not depend on this workflow.

