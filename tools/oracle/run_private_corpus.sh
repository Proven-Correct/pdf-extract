#!/usr/bin/env bash
# Corpus-A parity run (spec §4.14, design spec §13.2): scores pdf-extract
# against the oracle on a private directory of PDFs with the corpus-A gates
# (100% open, routing >= 99.9%, similarity median >= 0.999, p1 >= 0.98).
# Developer-only: needs PyMuPDF (this uv project) and never runs on CI.
#
#   tools/oracle/run_private_corpus.sh <corpus-dir> [<out-dir>]
#
# Expectations and the report go to <out-dir> (default
# ${XDG_CACHE_HOME:-~/.cache}/pdf-extract-private/<corpus name>), which must
# lie outside the repository: corpus A may be confidential, so nothing
# derived from it may land in a tracked tree. The metrics are those of gate B
# (`gate_a_private` in crates/core/tests/parity/gate_b.rs) and the report
# holds lengths and similarities only, never page text. Exit status: 0 when
# every gate passes, 1 when one fails, 2 on a usage error.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
if [[ $# -lt 1 || $# -gt 2 || ! -d $1 ]]; then
    echo "usage: $0 <corpus-dir> [<out-dir>]" >&2
    exit 2
fi
corpus=$(cd "$1" && pwd -P)
out=${2:-${XDG_CACHE_HOME:-$HOME/.cache}/pdf-extract-private/$(basename "$corpus")}
[[ -d $out ]] && created= || created=$out
mkdir -p "$out"
out=$(cd "$out" && pwd -P)
for path in "$corpus" "$out"; do
    if [[ $path/ == "$repo"/* ]]; then
        echo "$0: $path is inside the repository; keep corpus A private" >&2
        [[ -z $created ]] || rmdir "$created" 2>/dev/null || true
        exit 2
    fi
done

# Expectations from the oracle, opened from disk (L2: black box only).
# --prune drops the expectations of files no longer in the corpus.
(cd "$repo/tools/oracle" &&
    uv run python gen.py --out "$out/expectations" --root "$corpus" --prune "$corpus")

# Our side and the metrics, in release (corpus A is large).
status=0
rm -f "$out/report.json"
(cd "$repo" &&
    PARITY_PRIVATE_DIR=$corpus PARITY_PRIVATE_EXPECTATIONS=$out/expectations \
        PARITY_PRIVATE_REPORT=$out/report.json \
        cargo test --release --locked -q -p pdf-extract-core --test parity -- \
        --ignored --exact gate_b::gate_a_private) >"$out/cargo.log" 2>&1 || status=1
if [[ ! -f $out/report.json ]]; then
    cat "$out/cargo.log" >&2
    exit 1
fi
python3 "$repo/tools/oracle/score.py" --gate-report "$out/report.json"
echo "report: $out/report.json (log: $out/cargo.log)"
exit "$status"
