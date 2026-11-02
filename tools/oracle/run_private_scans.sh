#!/usr/bin/env bash
# OCR agreement on a private directory of real scans (run 0002 spec 3.8,
# "Private scans"). Real scans have no ground truth, so the measure is how
# closely our 72- and 300-dpi OCR agrees with the oracle's 72-dpi OCR
# (median and p10 character similarity), plus the routing agreement at
# min_chars = 50. Every number is report-only in this run.
# Developer-only: needs PyMuPDF (this uv project) and never runs on CI.
#
#   tools/oracle/run_private_scans.sh <scans-dir> [<out-dir>]
#
# Expectations and the report go to <out-dir> (default
# ${XDG_CACHE_HOME:-~/.cache}/pdf-extract-private-scans/<dir name>), which
# must lie outside the repository: the scans may be confidential, so nothing
# derived from them may land in a tracked tree. The report holds lengths,
# similarities and flags only, never text. Both sides use the pinned
# tessdata_fast model (mkcorpusd.py --fetch-model). Exit status: 0 when every
# page was scored, 1 when a file or page could not be, 2 on a usage error.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
if [[ $# -lt 1 || $# -gt 2 || ! -d $1 ]]; then
    echo "usage: $0 <scans-dir> [<out-dir>]" >&2
    exit 2
fi
scans=$(cd "$1" && pwd -P)
out=${2:-${XDG_CACHE_HOME:-$HOME/.cache}/pdf-extract-private-scans/$(basename "$scans")}
# `mkdir -p` may create several levels; remember the topmost new one so a
# refusal below removes all of them, not just the leaf. Everything under it
# was created here, so only empty directories can be there to remove.
created=
probe=$out
while [[ ! -e $probe ]]; do
    created=$probe
    probe=$(dirname "$probe")
done
mkdir -p "$out"
out=$(cd "$out" && pwd -P)
for path in "$scans" "$out"; do
    if [[ $path/ == "$repo"/* ]]; then
        echo "$0: $path is inside the repository; keep private scans private" >&2
        [[ -z $created ]] || find "$created" -depth -type d -exec rmdir {} + 2>/dev/null || true
        exit 2
    fi
done

# A report from an earlier run must not outlive a run that aborts before the
# test writes a new one (gen.py refuses duplicate files, for example), or the
# stale numbers would read as this run's.
rm -f "$out/report.json"

# The pinned model, then the oracle's 72-dpi OCR of every PDF, opened from
# disk (L2: black box only). --prune drops expectations of removed files.
model=$repo/tools/oracle/.cache/tessdata_fast
(cd "$repo/tools/oracle" &&
    uv run python mkcorpusd.py --fetch-model >/dev/null &&
    uv run python gen.py --ocr --tessdata "$model" --out "$out/expectations" \
        --root "$scans" --prune "$scans")

# Our side and the metrics, in release (real scans are many pages).
status=0
(cd "$repo" &&
    TESSDATA_PREFIX=$model PRIVATE_SCANS_DIR=$scans \
        PRIVATE_SCANS_EXPECTATIONS=$out/expectations PRIVATE_SCANS_REPORT=$out/report.json \
        cargo test --release --locked -q -p pdf-extract-ocr --test quality -- \
        --ignored --exact private::private_scans --nocapture) >"$out/cargo.log" 2>&1 || status=1
if [[ ! -f $out/report.json ]]; then
    cat "$out/cargo.log" >&2
    exit 1
fi
grep -E '^private scans:' "$out/cargo.log" || true
echo "report: $out/report.json (log: $out/cargo.log)"
exit "$status"
