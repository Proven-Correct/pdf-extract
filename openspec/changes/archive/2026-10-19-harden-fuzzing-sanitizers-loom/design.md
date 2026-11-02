# Design

## Context

See proposal.md for the motivation. The current state that shapes the approach:

- **Toolchain.**
  - The toolchain is pinned to stable `1.98.0` in `rust-toolchain.toml`, and no nightly is used anywhere.
  - `cargo-fuzz` (libFuzzer and `-Zsanitizer`) and `-Zbuild-std` need nightly.
- **CI.**
  - `ci.yml` is the only workflow. It runs on push to `main` and on `workflow_dispatch` (ADR 0085), with no schedule.
  - The repository is internal, so Actions minutes are billed.
- **Close protocol.**
  - It lives in `crates/core/src/document.rs`: `DocShared.state: RwLock<Option<Arc<Loaded>>>`, `close()`, `DocShared::loaded()`, plus an `AtomicBool` for lazy repair and the `Pages` `AtomicUsize` cursor.
  - It uses std primitives only, and `Loaded` holds a real `hayro_syntax::Pdf`.
- **OCR pool.**
  - It lives in `crates/ocr/src/pool.rs`, as a std `Mutex<State>` plus `Condvar`, a `Reservation` RAII guard, and `check_pid` using `std::process::id()`.
  - It is already generic over `RecognizerFactory`, and `pool_tests.rs` has a fake factory and `pretend_forked`.
- **Codec dispatch.**
  - It happens in `vendor/hayro-syntax/src/filter/{dct,ccitt,jbig2,jpx}.rs`. Each codec module's `decode` returns `Result<FilterResult, DecodeFailure>`.
  - `DecodeFailure` is `Copy`, with no payload. `ImageDecode` already flows to `InterpreterWarning::ImageDecodeFailure` and then to `WarningKind::ImageDecodeFailure` with no change downstream.
  - `jbig2::decode` resolves `JBIG2Globals` (xref and Flate) before calling the codec.
- **Panic hook.**
  - The Python binding installs a silent panic hook at module init (`crates/python/src/guard.rs`). It records into a thread-local `LAST_PANIC`.
  - The library installs no hook for Rust callers.
- **Fuzz entry points.**
  - Targets 1–3 are public: `Document::from_bytes`, `page_count`, `Page::text_page`, `render_gray`.
  - The CMap parser is `hayro_cmap::CMap::parse`, from crates.io, but it uses the vendored `hayro-postscript` through `[patch.crates-io]`.
  - The fallbacks in `crates/core/src/text/unicode.rs` are private.
- **Corpora.** Corpus B (60 PDFs) and corpus C (295 files) are committed. The small-stack runner is `crates/core/tests/common/stack.rs`.

## Goals / Non-Goals

**Goals:**
- One codec panic boundary that covers every image codec, with nothing downstream changed.
- A loom setup that checks the code the library actually ships, not a replica of it.
- Fuzzing and sanitizer infrastructure with near-zero recurring CI cost: long runs are local and resumable, CI runs are manual and time-capped, and a committed ledger makes the 72-hour criterion mechanically checkable.

**Non-Goals:**
- Fixing whatever fuzzing or the sanitizers find. Each finding goes through the ordinary issue workflow as an issue.
- Running Python tests under sanitizers.
- Fuzzing the OCR path.
- Model-checking `extract()`'s scoped-worker cursor or the helper client.
- Changing the stable toolchain pin.

## Decisions

### D1. The codec boundary wraps the codec call, not `Filter::apply`
Each of `dct::decode`, `ccitt::decode`, `jbig2::decode` and `jpx::decode` wraps only its call into the codec crate. That means the zune-jpeg decode, the hayro-ccitt decode, `Image::new_embedded` plus `decode_within_budget`, and `Image::new` plus `decode`. The call uses `std::panic::catch_unwind(AssertUnwindSafe(..))`, and `Err(_)` maps to `DecodeFailure::ImageDecode`.

- **Placement.** Wrapping at `Filter::apply` would also swallow panics in xref resolution, lazy repair and the nested Flate decode of `JBIG2Globals`. Those are parser bugs, not codec bugs, and the spec keeps them propagating.
- **`AssertUnwindSafe` is sound here.** The wrapped closures borrow only the input slice, local decoder state, and copies of settings. No shared state is left half-updated:
  - The thread-local memory and pixel-limit counters are written only after a decode returns.
  - The xref locks already recover from poisoning.
- **`no_std` builds.** The guard is `#[cfg(all(feature = "std", not(fuzzing)))]`. In `no_std` builds there is no unwinding to catch.
- **Shape of the patch.** It is one vendor patch, 0028 (H18), made as its own commit and exported with `tools/vendor/export-patches.sh` (ADR 0080). It touches a single helper function `guard_codec<T>(f) -> Result<T, DecodeFailure>` in `filter/mod.rs` and its four call sites.
- **Alternatives considered:**
  - Catch at core's page boundary. The whole page would fail, which is the behaviour internal #76 wants gone.
  - Audit and patch each site. Rejected in the internal #63 fix pass as a chain that never ends.
  - A new `ImageDecoderPanic` kind. The user chose to reuse `ImageDecodeFailure`.

### D2. Fuzzing sees panics by compiling the catch out under `cfg(fuzzing)`
`cargo fuzz` passes `--cfg fuzzing` to every crate, path dependencies included, so the vendored guard disappears in fuzz builds and a codec panic aborts as a libFuzzer crash. A runtime switch would be unnecessary in production and is easy to leave on by mistake.

The vendored crate's `Cargo.toml` gains `[lints.rust] unexpected_cfgs = { check-cfg = ["cfg(fuzzing)"] }` in the same patch. The workspace lints gain `cfg(fuzzing)` and `cfg(loom)`.

### D3. A test-only codec panic hook behind a feature
- **Why a hook.** The spec scenarios need a codec panic on demand. No known input does this reliably, and fuzzing exists to find and then fix such inputs.
- **The hook.** `hayro-syntax` gains a `codec-panic-hook` feature, off by default. It adds a thread-local `ARM: Cell<bool>` and a public `arm_codec_panic_for_test()`. When armed, the next guarded codec call panics inside the guard and the flag clears.
- **Who enables it.**
  - Core forwards it as a `test-hooks` feature.
  - Core's and render's dev-dependencies enable it.
  - The Python binding enables it unconditionally and exposes `_native._arm_codec_panic_for_test()`, consistent with the existing always-present `_panic_for_test`.
  - The production cost is one thread-local read per image decode.
- **Alternative considered.** A committed fixture that panics a codec today. Fuzzing will make it stale as soon as the underlying bug is fixed.

### D4. Panic-hook ownership: Python silent, Rust caller's hook
- **Rust callers.** The library never calls `set_hook` for them. Their hook is what runs when a codec panic is caught. The default hook prints, and that is the application's choice.
- **Python.** The existing module-init hook already silences output there.
- **Stale panic message.** A caught codec panic leaves a stale `LAST_PANIC` on the thread. `guarded` already takes the slot after each unwind it catches, and a codec catch never reaches `guarded`. So the slot is cleared on entry to every guarded call. This is a one-line change in `guard.rs` that makes the spec's "later unrelated panic carries its own message" hold under every ordering.
- **ADR.** A new ADR partially supersedes ADR 0035. It covers the codec-boundary exception, and that the hook belongs to the application.
- **Alternatives considered:**
  - A library-installed, chained, thread-scoped quiet hook. This seizes process-global state for a library's convenience.
  - Catch only when called from Python. That gives two behaviours for one Rust API.

### D5. loom: a `sync` shim plus a generic close cell
- **The shim.** Core and ocr each get a private `sync` module that re-exports `Arc`, `RwLock`, `Mutex`, `Condvar` and the atomics from `loom::sync` under `cfg(loom)` and from `std::sync` otherwise. `loom` is a `[target.'cfg(loom)'.dependencies]` entry.
- **Core.** The close protocol moves out of `DocShared` into `CloseCell<T>` in `crates/core/src/close_cell.rs`. Its interface is `new(T)`, `get() -> Option<Arc<T>>`, `close() -> Option<Arc<T>>` (the caller drops it outside the lock) and `is_closed()`. `DocShared` holds `CloseCell<Loaded>`.
  - The loom test instantiates `CloseCell<Counter>`, so no hayro `Pdf` (with its std locks) is constructed under loom.
  - This is the one structural change to production code. It is a pure extraction, and the existing close tests must pass unchanged.
- **OCR pool.**
  - `Pool` takes its pid source as a field of type `fn() -> u32`, defaulting to `std::process::id`, so a loom test can simulate fork resets.
  - `FORK_RESETS` stays a std atomic because loom forbids statics. It is test observability only.
  - The loom test uses the existing fake-factory pattern.
  - The first property to check is the generation-mismatch return path in `Slot::drop`, which returns without `notify_one`.
- **Where the tests live.** Loom tests are in `crates/{core,ocr}/tests/loom_*.rs`, gated `#![cfg(loom)]`. They run as `RUSTFLAGS="--cfg loom" cargo test -p … --test loom_* --release` in a new `loom` job in `ci.yml`, on stable, with `LOOM_MAX_PREEMPTIONS=3`.
- **Out of scope.** The `Pages` cursor and the lazy-repair `AtomicBool` stay std. They are single atomic operations already covered by stress tests, and modelling them adds state space without a protocol to check.
- **Alternatives considered:**
  - A test-only replica of the protocol. It checks a copy, not the code we ship.
  - `shuttle` (randomised). It is not exhaustive, while the spec says "every interleaving".

### D6. A separate `fuzz/` workspace on a pinned nightly
- **Layout.** `fuzz/Cargo.toml` declares its own `[workspace]`, so it is not a root member and stays out of `cargo-deny`, the shipped lock and the stable CI. It has its own committed `fuzz/Cargo.lock`.
  - It copies the root `[patch.crates-io]` hayro entries verbatim.
  - A check script, `tools/fuzz/check-patch-sync.sh`, is run in `ci.yml`'s `vendor-patches` job. It fails if the two patch tables diverge, so fuzzing always exercises the forked code.
- **Toolchain.** `fuzz/rust-toolchain.toml` pins a dated nightly that is at least two weeks old. Bumping it is a deliberate commit, like the stable pin.
- **Targets.** There are five `fuzz_targets/*.rs` files: `open`, `text`, `render`, `cmap`, `unicode`.
  - `unicode` splits its input into a selector byte plus payload, and drives the glyph-name, code and PDF text-string fallbacks.
  - Those fallbacks are exposed by core's `fuzzing` feature as `#[doc(hidden)] pub mod fuzz_api`, a thin `pub fn` re-export of the private `text/unicode.rs` functions. `#![forbid(unsafe_code)]` is unaffected.
- **Tightened limits.** These are one `fuzz_limits()` in core's `fuzzing` feature module (`fuzz_api`), shared by the targets and the regression replay (D7):

  | Limit | Value |
  |---|---|
  | `max_file_size` | 4 MiB |
  | `max_stream_decoded_bytes` | 32 MiB |
  | `max_ops_per_page` | 200k |
  | `page_time_budget` | 2 s |
  | `max_chars_per_page` | 200k |
  | `max_image_pixels` | 4M |
  | `max_render_pixels` | 4M |
  | `max_pages` | 1,000 (text target: first 6 pages + last) |

  libFuzzer runs with `-timeout=20 -rss_limit_mb=2560 -max_len=4194304`. That leaves headroom over a page's budget times the pages a target walks: render is one page, and text extracts at most seven pages (the first six and the last) × the 2 s page budget = 14 s, under the 20 s timeout, while documents of hundreds of pages still open so long page trees are fuzzed.
- **Alternatives considered:**
  - Make `fuzz/` a workspace member. That drags nightly-only `libfuzzer-sys` into stable CI and deny.
  - `cargo-afl`. libFuzzer is what §13.6 names and is better supported by `cargo-fuzz`.

### D7. Regression set and small-stack replay
- **The regression set.** `fuzz/regressions/<target>/<sha256-prefix>` holds minimised reproducers, committed with each fix.
- **Replay in the ordinary suite.** `crates/core/tests/fuzz_regressions.rs` and `crates/render/tests/fuzz_regressions.rs` run on stable, without libFuzzer, in the per-push `rust` job. They replay every file through the same entry points with `fuzz_limits()`, on `try_on_small_stack`.
  - So `fuzz_limits()` is defined once in core's `fuzzing` feature module, and `fuzz/` re-uses it.
  - The set starts empty, with a `README`.
- **Replay at the end of a local session.** `tools/fuzz/run.sh` replays the accumulated corpus through `fuzz/src/bin/replay_small_stack.rs`. That binary drives the document-level entry points on `std::thread::Builder::stack_size(512 * 1024)` threads.

### D8. Local fuzzing, committed ledger, dispatch-only CI smoke
The user capped runner cost tightly, because the repository is internal and billed. Long fuzzing therefore runs locally, and CI only smoke-tests.

- **`tools/fuzz/run.sh [--minutes N] [targets…]`** (default 60 minutes per target, run in sequence).
  1. Merges the seeds into the persistent local corpus `fuzz/corpus/<target>/`, which is gitignored and reused across sessions.
  2. Runs `cargo fuzz run <target> -- -max_total_time=…` with the D6 flags.
  3. Minimises the corpus with `cargo fuzz cmin`.
  4. Runs the small-stack replay.
  5. Calls `tools/fuzz/ledger.py record <target> --seconds <fuzzed> --clean|--finding`.

  `trap` handles interruption. On `SIGINT`, the elapsed clean time (the script measures wall time itself, not the requested duration) is still recorded, and the corpus is kept.
- **Ledger.**
  - `fuzz/ledger.json` is committed. It holds `{target: {clean_seconds, sessions, last_reset, last_session, commit}}`.
  - `ledger.py status` prints every target's total and "Phase 3 fuzzing exit: met/not met".
  - The ledger records the `HEAD` commit of each session, so a reviewer can see which code the hours were earned on.
  - The developer commits ledger updates.
  - `ledger.py` is unit-tested in the Python test suite.
  - Policy, documented in the ADR: a target's clean total is reset by a finding, not by unrelated commits. If a later fix changes a target's code path, the reviewer may reset that target by hand with `ledger.py reset <target>`.
- **Seeds.**
  - Corpus B and corpus C feed the document targets.
  - The CMap target gets the embedded predefined CMaps, through `hayro_cmap::load_embedded`, plus the corpus C ToUnicode streams, extracted by `tools/fuzz/seed-cmaps.py`.
  - The unicode target uses a small committed `fuzz/seeds/unicode/`.
- **`.github/workflows/hardening.yml`.**
  - It has only `workflow_dispatch`, with inputs `job` (`fuzz-smoke`, `asan`, `tsan` or `all`; default `fuzz-smoke`) and `fuzz_minutes` (default 10).
  - It has no `schedule` and no `push`.
  - The fuzz smoke job is a matrix over the five targets on ubuntu-24.04. Each target starts from the seeds plus the regression set, with no cache and no ledger update. A finding uploads `fuzz/artifacts/<target>/` and fails the job.
  - At the defaults, the five targets run for about 50 fuzzing minutes plus their builds, about one runner-hour.
- **Platforms.** `run.sh` supports macOS (the developer machine) and Linux. libFuzzer's ASan-based fuzzing works on both.
- **ADR.** A new ADR records the cost-capped model: local long fuzzing with a committed ledger, a dispatch-only hardening workflow, and the nightly toolchain pin. It sits alongside ADR 0085 and leaves it unchanged.
- **Alternatives considered:**
  - Scheduled nightly CI fuzzing with a cache-based ledger. Rejected for cost: 72 hours × 5 targets is 360 runner-hours.
  - Weekly short CI runs. A recurring cost for little gain over the local runs.

### D9. Sanitizer runs
- **Entry points.** `tools/sanitizers/run.sh asan|tsan` does the work. The `asan` and `tsan` jobs in `hardening.yml` call the same script on ubuntu-24.04 with a 120-minute timeout, so local and CI runs cannot drift.
- **AddressSanitizer.**
  - The build uses `RUSTFLAGS="-Zsanitizer=address"` and `CFLAGS`/`CXXFLAGS="-fsanitize=address -fno-omit-frame-pointer"`, then runs `cargo +nightly test --workspace -Zbuild-std --target <host triple>`.
  - It runs with `ASAN_OPTIONS=detect_leaks=1` (Linux) and `LSAN_OPTIONS=suppressions=tools/sanitizers/lsan.supp`.
  - `--target` keeps build scripts and proc-macros uninstrumented.
- **ThreadSanitizer.** The same build with `thread`, `TSAN_OPTIONS=suppressions=tools/sanitizers/tsan.supp`, and `-Zbuild-std`, which TSan needs to avoid false positives from an uninstrumented std.
- **Tesseract build.**
  - The vendored Tesseract and Leptonica builds go through `cmake` and `cc`, which pick up `CFLAGS` and `CXXFLAGS`. This needs to be verified against the pinned crate versions as the first task.
  - `build.rs` gains `rerun-if-env-changed` for `CFLAGS` and `CXXFLAGS`.
  - The script uses a separate `CARGO_TARGET_DIR` per sanitizer, so instrumented and plain builds never mix.
- **What is excluded.**
  - Stack-measuring tests are skipped through a `PXE_SANITIZER` environment check: `stack_size.rs` and the small-stack harnesses, whose 512 KiB assumption doesn't hold under ASan's stack inflation. The script enumerates the skip list.
  - The `system-tesseract` feature is not run.
- **Suppressions.** `tools/sanitizers/check-suppressions.sh` enforces the spec's justification rule. It requires a `# why:` comment on each entry, and rejects entries naming the `pdf_extract` crates or `tesseract_sys` Rust symbols. It also runs in `ci.yml`'s `shellcheck` job, which takes seconds, so a bad entry is caught on push.
- **Platforms.**
  - In CI the runs are Linux only, because LSan on macOS is unsupported.
  - Locally the script also runs on macOS, with ASan but no leak detection, plus TSan.

## Risks / Trade-offs

- **[Trade-off] Fuzzing depth depends on someone running the local script.**
  - Accepted by the user for cost reasons.
  - The ledger makes progress visible, and the Phase 3 exit cannot be claimed without it.
  - CI catches only regressions of committed reproducers and shallow crashes found by a smoke run.
- **[Risk] Local hours are not independently verifiable.** The ledger records commit and session counts, and its updates go through PRs like any other change. It is a progress log, not an attestation.
- **[Risk] The sanitizer build of vendored Tesseract under ASan or TSan is slow, or fails to link.** Each run uses its own target dir with a 120-minute CI timeout. If C++ instrumentation proves unworkable, the fallback is to instrument only the Rust code and record it in the ADR. That would leave the FFI scenario unmet, so it would come back to the user.
- **[Risk] TSan false positives from uninstrumented libstdc++.** Mitigated by suppressions with justification. Our crates are never suppressed.
- **[Risk] The first sessions find many crashes**, especially in JBIG2 (internal #115 and #116 are known). Expected and intended: findings become issues, and the ledger stays at zero for affected targets until they are fixed. The change is complete when the infrastructure works, not when the 72 hours are reached.
- **[Trade-off] Catching codec panics hides codec bugs from production Rust callers.** This is accepted: fuzzing still sees every one (D2), and the warning still records the skipped image.
- **[Trade-off] The `CloseCell` extraction touches code that every operation passes through.** It is a mechanical move. The existing close tests in `document_tests.rs` and `tests/concurrency.rs`, and the Python close tests, must pass unchanged.
- **[Risk] The `fuzz/` patch table drifts from the root.** The D6 sync check in the `vendor-patches` job catches it.
- **[Risk] The nightly toolchain pin ages.** It is bumped deliberately, like the stable pin, and only the dispatch-only workflow and the local scripts depend on it.

## Migration Plan

- **No user-visible API changes**, except that Rust callers no longer receive codec panics. This goes in the README, under the error model.
- **Rollback.** Revert patch 0028, `hardening.yml` and the scripts independently. The loom shim is inert outside `cfg(loom)`.
- **Order.**
  1. The codec boundary, with its spec, ADR and test hook.
  2. The loom shim and tests.
  3. The fuzz crate, the regression replay and the local script with the ledger.
  4. `hardening.yml`.
  5. The sanitizer script.
  6. Docs.

## Open Questions

- **Exact dated nightly** for `fuzz/rust-toolchain.toml` and the sanitizer script. Pick the newest nightly at least two weeks old on which `cargo fuzz` and `-Zbuild-std` both build the workspace.
- **Entries in the initial TSan and LSan suppressions**, if any. They become known only on the first instrumented run.
