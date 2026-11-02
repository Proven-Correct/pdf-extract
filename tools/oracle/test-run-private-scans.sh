#!/usr/bin/env bash
# Tests for run_private_scans.sh's housekeeping (internal #56 B26), run by hand and by the CI oracle-tests job:
#
#   tools/oracle/test-run-private-scans.sh [<script under test>]
#
# The script is copied into a throwaway "repository" (it locates the repo from
# its own path) and run with a stub `uv` standing in for gen.py, so no PyMuPDF,
# network or tessdata is needed. Checked:
#
#   1. an out directory inside the repository is refused (exit 2) and every
#      directory the run created on the way is removed, not just the leaf;
#   2. a pre-existing parent of a refused out directory is left alone;
#   3. a run that aborts before the cargo step (here: the stub gen.py fails,
#      as it does on duplicate files) leaves no report.json from an earlier run.

# The assertion helpers are called only through `check`, which shellcheck
# cannot see.
# shellcheck disable=SC2329
set -euo pipefail

script="${1:-$(cd "$(dirname "$0")" && pwd)/run_private_scans.sh}"
root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT

repo="$root/repo"
mkdir -p "$repo/tools/oracle" "$root/bin" "$root/scans" "$root/outside"
cp "$script" "$repo/tools/oracle/run_private_scans.sh"
chmod +x "$repo/tools/oracle/run_private_scans.sh"
printf '#!/usr/bin/env bash\nexit 1\n' >"$root/bin/uv"
chmod +x "$root/bin/uv"

fail=0
# check <description> <command...>: a failure is recorded and reported rather
# than ending the script (errexit would hide the rest).
check() {
    local what="$1"
    shift
    if "$@"; then echo "ok   $what"; else echo "FAIL $what"; fail=1; fi
}
# run <out-dir>: the script's exit status lands in $status.
status=0
run() {
    status=0
    PATH="$root/bin:$PATH" "$repo/tools/oracle/run_private_scans.sh" "$root/scans" "$1" \
        >/dev/null 2>&1 || status=$?
}
is() { [ "$status" -eq "$1" ]; }
absent() { [ ! -e "$1" ]; }
present() { [ -e "$1" ]; }

run "$repo/a/b/c"
check "out inside the repository: exit 2" is 2
check "nested refused out path: first created ancestor removed" absent "$repo/a"

mkdir -p "$repo/existing"
run "$repo/existing/new/deeper"
check "refused out under an existing directory: new levels removed" absent "$repo/existing/new"
check "the existing parent is kept" present "$repo/existing"

mkdir -p "$root/outside/out"
printf '{}\n' >"$root/outside/out/report.json"
run "$root/outside/out"
check "aborted run: exit 1 from the failing gen.py" is 1
check "aborted run: stale report.json removed" absent "$root/outside/out/report.json"

exit "$fail"
