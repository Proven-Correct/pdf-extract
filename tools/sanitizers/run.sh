#!/usr/bin/env bash
# The Rust workspace test suite under AddressSanitizer or ThreadSanitizer,
# with the vendored Leptonica and Tesseract and the C++ shim instrumented too
# (sanitizer-checks spec; design D9). The one entry point for both local runs
# and the dispatch-only `asan`/`tsan` jobs of .github/workflows/hardening.yml,
# so the two cannot drift.
#
# Usage: tools/sanitizers/run.sh asan|tsan [extra cargo test args...]
#
# Exit status is cargo's: non-zero if any included test fails or the
# sanitizer reports an error (a report aborts the test binary). The whole
# output is also written to $PXE_SANITIZER_LOG_DIR/<sanitizer>.log (default
# target/sanitizers/), which the workflow uploads on failure.
#
# Platforms: Linux (CI) and macOS (local). LeakSanitizer runs only on Linux;
# macOS does not support it, so a macOS asan run checks memory errors only.
#
# How the build is put together, and why:
# - Toolchain: the dated nightly NIGHTLY below, the same pin as
#   fuzz/rust-toolchain.toml (`-Zsanitizer` and `-Zbuild-std` are
#   nightly-only). Install it with the rust-src component.
# - `-Zbuild-std --target <host>`: std is rebuilt with the sanitizer (TSan
#   needs it to avoid false positives; `-Zsanitizer` is also a target
#   modifier, so instrumented crates refuse to link against the prebuilt
#   std), and an explicit --target keeps build scripts and proc-macros
#   uninstrumented.
# - CFLAGS/CXXFLAGS carry -fsanitize to the cmake and cc builds of
#   Leptonica, Tesseract and csrc/shim.cpp (verified against the pinned
#   cmake 0.1.58 / cc 1.5.1; build.rs reruns when they change). On Linux the
#   C/C++ compiler defaults to clang, so the instrumentation matches the
#   LLVM sanitizer runtime rustc links (gcc's libasan/libtsan is a different
#   runtime). Apple clang emits an ASan version check symbol of its own
#   (`__asan_version_mismatch_check_apple_clang_*`) that rustc's runtime
#   does not define, so the check is turned off for the C/C++ objects.
# - CARGO_TARGET_DIR is per sanitizer (target/sanitizer-<name>), so
#   instrumented and plain artefacts never mix and switching between the
#   two never forces a rebuild of the other.
# - `--tests`: unit and integration test targets only. Doctests are left
#   out: rustdoc would need the same build-std and sanitizer flags, and they
#   exercise documentation examples the ordinary suite already runs.
# - pdf-extract-python is excluded: it has no Rust tests (`test = false`; the
#   binding is exercised by pytest, and Python under sanitizers is a
#   non-goal), and building its extension-module cdylib would only cost time.
# - The `system-tesseract` feature is not run: the point is the vendored
#   code, built instrumented.
#
# Skipped under PXE_SANITIZER (stack-measuring tests; sanitizer
# instrumentation inflates stack frames several times over, so the
# "fits in 512 KiB" claims they make do not hold and are not what a
# sanitizer run checks):
# - crates/core/tests/stack_size.rs, all four tests, skipped by name below:
#   they test the 512 KiB small-stack runner's own measurement.
# - Every user of the small-stack runner (crates/core/tests/common/stack.rs:
#   the core integration tests that use `on_small_stack`/
#   `try_on_small_stack`/`run_on_small_stack`, render's hostile.rs and
#   silence.rs, and parity gate C) is NOT skipped: under PXE_SANITIZER the
#   runner runs the body on a 64 MiB stack without measuring, so the hostile
#   inputs still go through the instrumented code; only the stack bound
#   itself is unchecked.
# - core's text unit tests include the same runner (crates/core/src/text/
#   tests/mod.rs, ADR 0089); its users there are likewise not skipped, but
#   the_unit_tests_small_stack_is_measured, which tests the measurement, is.
#
# Also skipped, because the claim they check is changed by the
# instrumentation itself rather than by our code (each still runs in the
# ordinary suite):
# - a_4mb_named_property_referenced_2000_times_is_fast (core text unit
#   test): a wall-clock bound (< 2 s) that a several-times-slower
#   instrumented build misses (2.1 s measured on macOS arm64).
# - helpers_start_with_only_the_standard_descriptors (ocr tests/process.rs):
#   the instrumented helper reports one descriptor more (fd 3) than 0-2.
#   It is opened inside the helper (attributed to the sanitizer runtime's
#   start-up; seen on macOS ASan), not inherited: the fd the test plants
#   keeps its own, higher number across exec and does not appear.
# - a_hanging_helper_is_killed_at_the_deadline (ocr tests/process.rs): see
#   the TSan entry below. ASan's smaller slowdown puts the instrumented
#   render right at the 500 ms deadline: on Linux CI it passed in one run
#   and failed in the next, so it is skipped under both.
# TSan only (its slowdown, roughly an order of magnitude, breaks
# deadline arithmetic that ASan's does not):
# - the_time_budget_is_a_limit_error_within_a_second (render tests/render.rs):
#   a wall-clock bound (1.37 s measured).
# - a_hanging_helper_is_killed_at_the_deadline (skipped under ASan too, see
#   above) and
#   the_deadline_counts_from_the_start_of_ocr_not_from_the_pipe_write (ocr
#   tests/process.rs): with a 500 ms page timeout the instrumented render
#   inside ocr() already uses up the deadline, so the engine times the page
#   out before asking the helper (engine.rs, the check after rendering), so
#   the idle helper the first test expects killed at the deadline is only
#   reaped when the engine drops, after the test's check (a timing artefact
#   of the slowdown, not a leak); the
#   second's calibrated render-vs-helper timing no longer holds (29 s).
# - runtime_sizes_open_in_hayro (testkit tests/generators_hayro.rs): the
#   first CI run killed it (SIGTERM, exit 143) on the Linux runner about a
#   minute in; it builds every runtime-size hostile document at once (a
#   50 MiB flate bomb, a 2M-operator stream, a 100k-page tree), most likely
#   too much under TSan's shadow memory. It passed under TSan on macOS
#   arm64, and the same documents still run under ASan.
#
# LeakSanitizer (Linux ASan) and leaks by design: the OCR pool abandons an
# instance it cannot safely drop after fork() by leaking it on purpose
# (Recognizer::abandon, mem::forget; concurrency spec on fork()), and the pool
# unit tests listed in LEAK_BY_DESIGN pretend a fork to check exactly that.
# LSan rightly reports those leaks (each test strands its fake factory's two
# shared counters: 10 x 24 bytes in the first CI run). Suppressing them
# would mean naming our own crate, which check-suppressions.sh forbids, so
# they are skipped in the main run and then run in a second pass with ASan
# still on and only leak detection off.
#
# RUST_BACKTRACE is deliberately left unset: capturing and symbolising a
# backtrace for every caught panic in an instrumented build is slow enough
# to make a_failing_worker_stops_the_others (core) see all workers run
# before the panic lands. The sanitizer's own reports carry their stacks.
set -euo pipefail

NIGHTLY=nightly-2026-10-01

# See the header for why each is skipped.
SKIP=(
    # stack_size.rs: the small-stack runner's own measurement.
    the_runner_hands_out_a_stack_of_the_size_it_promises
    a_big_stack_does_not_pass_for_a_small_one
    the_runner_survives_a_cache_of_bigger_stacks
    a_stack_that_cannot_be_had_at_the_right_size_fails_the_test
    # core's text unit tests: the same measurement, through that runner.
    the_unit_tests_small_stack_is_measured
    # Changed by the instrumentation, not by our code.
    a_4mb_named_property_referenced_2000_times_is_fast
    helpers_start_with_only_the_standard_descriptors
    a_hanging_helper_is_killed_at_the_deadline
)
LEAK_BY_DESIGN=(
    pool::tests::a_fork_reset_leaks_the_old_instances
    pool::tests::an_instance_returned_across_a_reset_is_leaked
    pool::tests::a_creation_across_a_reset_does_not_corrupt_the_count
    pool::tests::dropping_a_pool_in_a_forked_child_leaks
    pool::tests::a_reset_by_a_returning_slot_wakes_a_waiter
)
SKIP_TSAN=(
    runtime_sizes_open_in_hayro
    the_time_budget_is_a_limit_error_within_a_second
    the_deadline_counts_from_the_start_of_ocr_not_from_the_pipe_write
)

usage() {
    echo "usage: $0 asan|tsan [extra cargo test args...]" >&2
    exit 2
}

[ "$#" -ge 1 ] || usage
case $1 in
    asan) san=address ;;
    tsan) san=thread ;;
    *) usage ;;
esac
name=$1
shift

repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"

# `cargo +toolchain` needs rustup's proxies; put them first when present so a
# distribution cargo elsewhere on PATH does not shadow them.
if [ -d "$HOME/.cargo/bin" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
fi

# A suppression file that breaks the rules fails the run before any build.
tools/sanitizers/check-suppressions.sh

host=$(rustc +"$NIGHTLY" -vV | sed -n 's/^host: //p')
[ -n "$host" ] || { echo "cannot determine the host triple from rustc +$NIGHTLY" >&2; exit 1; }
os=$(uname -s)

cflags="-fsanitize=$san -fno-omit-frame-pointer -g"
if [ "$os" = Darwin ] && [ "$san" = address ]; then
    cflags="$cflags -mllvm -asan-guard-against-version-mismatch=0"
fi
if [ "$os" = Linux ]; then
    export CC="${CC:-clang}" CXX="${CXX:-clang++}"
    # The sanitizers symbolise their reports with llvm-symbolizer found on
    # PATH; Ubuntu ships it only as a versioned binary under /usr/lib/llvm-N.
    if ! command -v llvm-symbolizer >/dev/null; then
        for d in /usr/lib/llvm-*/bin; do
            if [ -x "$d/llvm-symbolizer" ]; then symbolizer_dir=$d; fi
        done
        if [ -n "${symbolizer_dir:-}" ]; then export PATH="$PATH:$symbolizer_dir"; fi
    fi
fi
export CFLAGS="$cflags" CXXFLAGS="$cflags"
export RUSTFLAGS="-Zsanitizer=$san -Cforce-frame-pointers=yes"
export CARGO_TARGET_DIR="$repo/target/sanitizer-$name"
export PXE_SANITIZER="$name"

supp="$repo/tools/sanitizers"
case $name in
    asan)
        if [ "$os" = Linux ]; then
            export ASAN_OPTIONS="detect_leaks=1"
            export LSAN_OPTIONS="suppressions=$supp/lsan.supp:print_suppressions=0"
        else
            export ASAN_OPTIONS="detect_leaks=0"
        fi
        ;;
    tsan)
        export TSAN_OPTIONS="suppressions=$supp/tsan.supp:halt_on_error=1"
        ;;
esac

log_dir="${PXE_SANITIZER_LOG_DIR:-$repo/target/sanitizers}"
mkdir -p "$log_dir"
log="$log_dir/$name.log"

if [ "$name" = tsan ]; then
    SKIP+=("${SKIP_TSAN[@]}")
fi
leak_pass=0
if [ "$name" = asan ] && [ "$os" = Linux ]; then
    SKIP+=("${LEAK_BY_DESIGN[@]}")
    leak_pass=1
fi
skip_args=()
for t in "${SKIP[@]}"; do
    skip_args+=(--skip "$t")
done

echo "sanitizer run: $name on $host ($os); log: $log"
set -o pipefail
# errexit off: a failing run's status is collected and returned at the end,
# after the leak-by-design pass has run too.
set +e
cargo +"$NIGHTLY" test -Zbuild-std --target "$host" --locked \
    --workspace --exclude pdf-extract-python --tests --no-fail-fast "$@" \
    -- "${skip_args[@]}" 2>&1 | tee "$log"
status=${PIPESTATUS[0]}

# The leak-by-design pool tests (see the header): ASan on, leak detection
# off, appended to the same log.
if [ "$leak_pass" = 1 ]; then
    echo "== leak-by-design pass (ASan without leak detection)" | tee -a "$log"
    ASAN_OPTIONS="detect_leaks=0" cargo +"$NIGHTLY" test -Zbuild-std --target "$host" --locked \
        -p pdf-extract-ocr --lib -- --exact "${LEAK_BY_DESIGN[@]}" 2>&1 | tee -a "$log"
    leak_status=${PIPESTATUS[0]}
    if [ "$status" = 0 ]; then status=$leak_status; fi
fi
exit "$status"
