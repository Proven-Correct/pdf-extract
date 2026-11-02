#!/usr/bin/env bash
# Local long-running fuzzing (design D8; spec "Long fuzzing runs locally with
# a persistent corpus", "Clean fuzzing hours are counted per target"). Runs
# on the developer machine (macOS or Linux), never in CI, and records each
# target's session in the committed clean-hours ledger, fuzz/ledger.json.
#
# Usage: tools/fuzz/run.sh [--minutes N] [TARGET...]
#
# TARGET is open, text, render, cmap or unicode (default: all five, in that
# order); each fuzzes for N minutes (default 60). Per target:
#
# 1. Merge the seeds into the persistent corpus fuzz/corpus/TARGET/
#    (gitignored, kept between sessions), each named by its SHA-1 so a seed
#    already there is not copied twice. Document targets: corpus B and C;
#    cmap: the CMap streams of corpus B and C (seed-cmaps.py) and the
#    hand-written text CMaps of the fuzz crate's synthetic_cmaps; unicode:
#    fuzz/seeds/unicode/. Every target: its fuzz/regressions/TARGET/.
# 2. Build, then fuzz with the D6 libFuzzer flags. Only the fuzzing itself
#    is timed (wall clock, measured here), so build time is never credited.
# 3. Minimise the corpus (`cargo fuzz cmin`).
# 4. Document targets: replay the corpus through the open, text and render
#    drivers on 512 KiB threads
#    (fuzz/src/bin/replay_small_stack.rs, a plain release build with
#    `--cfg fuzzing`, so codec panics are not caught there either).
# 5. Record the session: clean time added, or the total reset on a finding.
#
# A finding is a new file in fuzz/artifacts/TARGET/ (libFuzzer writes crash-,
# timeout-, oom- and leak- files there; its slow-unit- files, for inputs over
# 10 s but under the 20 s timeout, are not findings: the text target's 7
# sampled pages at the 2 s page budget legitimately take up to 14 s, and the fuzzing spec only asks that the
# library's limits bind before the 20 s ceiling) or a small-stack replay that panicked or died of a signal, whose
# log is saved there. Each crash input is minimised (`cargo fuzz tmin`, kept
# beside it), the target's total is reset, and the session stops with exit
# status 1. A failure that leaves no artifact (a build error, a cargo
# problem) records nothing and exits 2: it says nothing about the code.
#
# SIGINT/SIGTERM: the time fuzzed so far is credited if no finding appeared,
# the corpus is kept, and the session stops without minimising or replaying
# (the next session replays the same corpus). Interrupting before fuzzing
# starts records nothing. Send the signal to the whole process group (as a
# terminal's Ctrl-C does): a signal to this script alone is acted on only
# when the running fuzzer exits, at the end of its -max_total_time.
#
# Environment: FUZZ_STATE_DIR (default fuzz/) holds corpus/, artifacts/ and
# the helper binaries' target/tools/;
# FUZZ_LEDGER (default fuzz/ledger.json) is the ledger; PYTHON (default
# python3) runs the helper scripts. All three exist for the test suite.
set -uo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
state=${FUZZ_STATE_DIR:-$repo/fuzz}
ledger=${FUZZ_LEDGER:-$repo/fuzz/ledger.json}
python=${PYTHON:-python3}
all_targets=(open text render cmap unicode)
# libFuzzer's ceilings sit above every library budget (D6): a page's 2 s
# time budget times the pages a target walks, the 32 MiB stream and 4 M pixel
# caps, and the 4 MiB file cap as the longest input.
fuzz_flags=(-timeout=20 -rss_limit_mb=2560 -max_len=4194304)

usage() {
  echo "usage: $0 [--minutes N] [open|text|render|cmap|unicode ...]" >&2
  exit 2
}

minutes=60
targets=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --minutes)
      [[ $# -ge 2 && "$2" =~ ^[1-9][0-9]*$ ]] || usage
      minutes=$2
      shift 2
      ;;
    open | text | render | cmap | unicode)
      targets+=("$1")
      shift
      ;;
    *) usage ;;
  esac
done
[[ ${#targets[@]} -gt 0 ]] || targets=("${all_targets[@]}")

toolchain=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$repo/fuzz/rust-toolchain.toml")
[[ -n "$toolchain" ]] || { echo "no channel in fuzz/rust-toolchain.toml" >&2; exit 2; }

# The trap only sets a flag: bash runs it once the foreground command (which
# got the same signal from the terminal) has exited, and the script then
# decides what to credit.
interrupted=0
trap 'interrupted=1' INT TERM
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

cd "$repo" || exit 2

# The fuzz crate's helper binaries, built once per session by plain cargo
# (not cargo-fuzz: no sanitizer, no coverage instrumentation) into their own
# target dir. Built up front so a build failure is never mistaken for a
# failed replay.
tools="$state/target/tools/release"
build_tools() {
  RUSTFLAGS="--cfg fuzzing" CARGO_TARGET_DIR="$state/target/tools" \
    cargo "+$toolchain" build --quiet --locked --release --manifest-path "$repo/fuzz/Cargo.toml" \
    --bin replay_small_stack --bin synthetic_cmaps
}

sha1() {
  if command -v sha1sum >/dev/null; then sha1sum <"$1"; else shasum -a 1 <"$1"; fi | cut -d' ' -f1
}

# Copies every file of the given directories into $1, named by content.
merge_into() {
  local dest=$1 dir f
  shift
  for dir in "$@"; do
    [[ -d "$dir" ]] || continue
    for f in "$dir"/*; do
      [[ -f "$f" ]] || continue
      case "$(basename "$f")" in .* | README*) continue ;; esac
      cp "$f" "$dest/$(sha1 "$f")"
    done
  done
}

seed() {
  local target=$1 corpus=$2
  case "$target" in
    open | text | render)
      merge_into "$corpus" tests/fixtures/corpus-b/hayro tests/fixtures/corpus-b/pdfbox \
        tests/fixtures/corpus-c
      ;;
    cmap)
      "$python" tools/fuzz/seed-cmaps.py "$scratch/cmap-seeds" --corpus tests/fixtures/corpus-c \
        --corpus tests/fixtures/corpus-b/hayro --corpus tests/fixtures/corpus-b/pdfbox || return 1
      "$tools/synthetic_cmaps" "$scratch/cmap-seeds" >/dev/null || return 1
      merge_into "$corpus" "$scratch/cmap-seeds"
      ;;
    unicode) merge_into "$corpus" fuzz/seeds/unicode ;;
  esac
  merge_into "$corpus" "fuzz/regressions/$target"
}

record() {
  "$python" tools/fuzz/ledger.py --ledger "$ledger" record "$@"
}

# Artifacts newer than the session marker, i.e. this session's findings.
new_artifacts() {
  find "$1" -type f -newer "$1/.session-start" ! -name '.session-start' ! -name 'minimized-*' \
    ! -name 'replay-*.log' ! -name 'slow-unit-*'
}

# Runs one target's session; returns 0 clean, 1 finding, 2 tool failure,
# 3 interrupted (time already credited).
session() {
  local target=$1 corpus="$state/corpus/$1" artifacts="$state/artifacts/$1"
  local start end fuzzed status found log
  mkdir -p "$corpus" "$artifacts"
  echo "== $target: seeding $corpus"
  if ! seed "$target" "$corpus"; then
    [[ $interrupted -eq 0 ]] || return 3
    echo "seeding $target failed" >&2
    return 2
  fi
  echo "== $target: building"
  cargo "+$toolchain" fuzz build "$target" || { echo "building $target failed" >&2; return 2; }
  [[ $interrupted -eq 0 ]] || return 3

  touch "$artifacts/.session-start"
  echo "== $target: fuzzing for $minutes min"
  start=$(date +%s)
  cargo "+$toolchain" fuzz run "$target" "$corpus" -- "${fuzz_flags[@]}" \
    -max_total_time=$((minutes * 60)) -artifact_prefix="$artifacts/"
  status=$?
  end=$(date +%s)
  fuzzed=$((end - start))

  found=$(new_artifacts "$artifacts")
  if [[ -n "$found" ]]; then
    echo "== $target: FINDING after ${fuzzed}s; artifacts kept in $artifacts" >&2
    local a
    while IFS= read -r a; do
      case "$(basename "$a")" in
        crash-* | timeout-* | oom-* | leak-*)
          # No -max_len: libFuzzer's minimiser sets the length itself and
          # asserts if it was given one. Its output, minimized-from-<sha1>,
          # lands beside the crash (the later -artifact_prefix wins over
          # cargo-fuzz's own).
          cargo "+$toolchain" fuzz tmin "$target" "$a" -- -timeout=20 -rss_limit_mb=2560 \
            -artifact_prefix="$artifacts/" || true
          ;;
      esac
    done <<<"$found"
    record "$target" --seconds "$fuzzed" --finding
    return 1
  fi
  if [[ $interrupted -ne 0 ]]; then
    echo "== $target: interrupted after ${fuzzed}s; crediting the time fuzzed"
    record "$target" --seconds "$fuzzed" --clean
    return 3
  fi
  if [[ $status -ne 0 ]]; then
    echo "cargo fuzz run $target failed (status $status) without an artifact" >&2
    return 2
  fi

  # From here an interrupt kills cmin or the replay; that is the developer
  # stopping, not a finding, so the clean fuzzing time is still credited.
  echo "== $target: minimising the corpus"
  if ! cargo "+$toolchain" fuzz cmin "$target" "$corpus" -- "${fuzz_flags[@]}"; then
    [[ $interrupted -eq 0 ]] || { record "$target" --seconds "$fuzzed" --clean; return 3; }
    echo "cmin $target failed" >&2
    return 2
  fi
  case "$target" in
    open | text | render)
      echo "== $target: small-stack replay of the corpus"
      log="$scratch/replay-$target.log"
      # Every document driver, not just this target's: the error-model spec
      # has each corpus input opened, extracted and rendered on 512 KiB.
      "$tools/replay_small_stack" all "$corpus" >"$log" 2>&1
      status=$?
      if [[ $status -ne 0 ]]; then
        [[ $interrupted -eq 0 ]] || { record "$target" --seconds "$fuzzed" --clean; return 3; }
        # 1: an input panicked; above 128: a signal (a stack overflow
        # aborts). Anything else (2: usage or unreadable input; 126/127:
        # no binary) is the tool failing, which says nothing about the code.
        if [[ $status -ne 1 && $status -le 128 ]]; then
          tail -n 20 "$log" >&2
          echo "the small-stack replay of $target failed to run (status $status)" >&2
          return 2
        fi
        cp "$log" "$artifacts/replay-$(date +%Y%m%dT%H%M%S).log"
        tail -n 20 "$log" >&2
        echo "== $target: FINDING in the small-stack replay; log kept in $artifacts" >&2
        record "$target" --seconds "$fuzzed" --finding
        return 1
      fi
      tail -n 1 "$log"
      ;;
  esac
  record "$target" --seconds "$fuzzed" --clean
}

echo "== building the replay and seed tools"
build_tools || { echo "building the fuzz crate's tools failed" >&2; exit 2; }
[[ $interrupted -eq 0 ]] || exit 0

for target in "${targets[@]}"; do
  session "$target"
  case $? in
    0) ;;
    1) exit 1 ;;
    3) exit 0 ;;
    *) exit 2 ;;
  esac
  if [[ $interrupted -ne 0 ]]; then
    exit 0
  fi
done
