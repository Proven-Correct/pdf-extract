#!/usr/bin/env bash
# Tests for stage-helper.sh's argument handling (internal #97 B6; CI: the `rust` job).
#
# The script is copied into a throwaway tree (it locates the repository from
# its own path, so the real python/pdf_extract/ is never touched) and run
# against a fake `cargo` that records its arguments and answers with the
# JSON artifact message the script parses. This checks what the script hands
# to cargo and that it stages the binary cargo reports, not cargo itself.
#
#   1. no arguments: `--release` is passed (the documented default);
#   2. `--profile dev` / `--profile=dev`: `--release` is omitted, because
#      cargo rejects the pair, and the profile is passed through;
#   3. other extra arguments keep `--release`;
#   4. the binary named by cargo's report (here under a custom profile
#      directory) is the one staged.

# The assertion helpers are called only through `check`, which shellcheck
# cannot see.
# shellcheck disable=SC2329
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/stage-helper.sh"
root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT

mkdir -p "$root/tools/build" "$root/python/pdf_extract" "$root/bin" "$root/target/custom"
cp "$script" "$root/tools/build/stage-helper.sh"
printf 'staged-binary\n' >"$root/target/custom/pdf-extract-ocr-helper"

cat >"$root/bin/cargo" <<FAKE
#!/usr/bin/env bash
printf '%s\n' "\$@" >"$root/cargo-args"
printf '{"reason":"compiler-artifact","target":{"name":"pdf-extract-ocr-helper","kind":["bin"]},"executable":"$root/target/custom/pdf-extract-ocr-helper"}\n'
FAKE
chmod +x "$root/bin/cargo"

fail=0
args=""
# run <args...>: stage with the fake cargo; the arguments it saw land in $args.
run() {
    rm -f "$root/cargo-args" "$root/python/pdf_extract/pdf-extract-ocr-helper"
    PATH="$root/bin:$PATH" "$root/tools/build/stage-helper.sh" "$@" >/dev/null
    args="$(cat "$root/cargo-args")"
}
# check <description> <command...>: runs the assertion; a failure is recorded
# and reported rather than ending the script (errexit would hide the rest).
check() {
    local what="$1"
    shift
    if "$@"; then echo "ok   $what"; else echo "FAIL $what"; fail=1; fi
}
has() { printf '%s\n' "$args" | grep -qxF -- "$1"; }
lacks() { ! has "$1"; }
staged_is() { [ "$(cat "$root/python/pdf_extract/pdf-extract-ocr-helper")" = "$1" ]; }

run
check "no arguments: --release passed" has --release

run --profile dev
check "--profile dev: --release omitted" lacks --release
check "--profile dev: --profile passed through" has --profile
check "--profile dev: the profile name passed through" has dev
check "--profile dev: the binary cargo reports is staged" staged_is staged-binary

run --profile=dev
check "--profile=dev: --release omitted" lacks --release
check "--profile=dev: passed through" has --profile=dev
check "--profile=dev: the binary cargo reports is staged" staged_is staged-binary

run --features foo
check "other arguments: --release kept" has --release
check "other arguments: passed through" has foo
check "other arguments: the binary cargo reports is staged" staged_is staged-binary

exit "$fail"
