#!/usr/bin/env bash
# Regenerate vendor/patches/hayro/ from the commits that change the vendored
# crates (spec §3.2; run 0003 §3.2 for the two ranges).
#
# Usage: tools/vendor/export-patches.sh [BASE1 [BASE2]]
#
# The vendored hayro crates were imported in two steps, so the series has two
# ranges, numbered continuously:
#   1. BASE1..BASE2^ over hayro-syntax, hayro-interpret and hayro-postscript.
#      BASE1 defaults to the tag vendor/hayro-pristine-1 (their pristine
#      import).
#   2. BASE2..HEAD over those three plus hayro-jbig2 and hayro-jpeg2000.
#      BASE2 defaults to the tag vendor/hayro-pristine-2, the commit that
#      imported the two codec crates pristine; it is the boundary of both
#      ranges and is itself never exported.
# Every commit after BASE1 is either an ancestor of BASE2^ or not an ancestor
# of BASE2 (the import commit has one parent), so the two ranges together cover
# each later commit exactly once. A single range from BASE1 cannot work: it
# would export the import commit itself as a patch.
#
# Every non-merge commit in a range that touches one of its crates becomes one
# patch, numbered in commit order. --relative=vendor strips the vendor/ prefix
# so paths read hayro-syntax/src/..., as in the upstream monorepo, and drops the
# parts of a commit outside vendor/ (our tests in crates/core). The whole
# series is rewritten each time so that dropped or reordered commits leave no
# stale files; check-patches.sh then verifies the series reproduces vendor/.
#
# lib-format-patch.sh pins the format-patch/diff settings the caller's git
# config could otherwise change (its header lists the residuals it deliberately
# leaves unpinned), and --zero-commit and --no-signature keep the files stable
# across rebases and git versions, so re-exporting an unchanged stack produces
# no diff. Each range is its own format-patch run, so a "[PATCH n/N]" header counts within its range:
# range 1's files say n/22 however many range 2 adds, which is what keeps them
# byte-identical as later patches are added. --numbered makes a one-patch range
# carry its number too.
set -euo pipefail

base1=${1:-vendor/hayro-pristine-1}
base2=${2:-vendor/hayro-pristine-2}
repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"
# shellcheck source=tools/vendor/lib-format-patch.sh
. "$repo/tools/vendor/lib-format-patch.sh"

for base in "$base1" "$base2"; do
  if ! git rev-parse --verify --quiet "$base^{commit}" >/dev/null; then
    echo "error: base $base not found (tags are local; pass the pristine commit)" >&2
    exit 1
  fi
done
if ! git merge-base --is-ancestor "$base1" "$base2"; then
  echo "error: $base1 is not an ancestor of $base2" >&2
  exit 1
fi
# BASE2..HEAD is empty, or names someone else's commits, unless BASE2 is in
# HEAD's history: a squash-merged or rebased vendor import leaves the tag on a
# commit outside HEAD, and exporting from it would silently write the wrong
# series and exit 0.
if ! git merge-base --is-ancestor "$base2" HEAD; then
  echo "error: $base2 is not an ancestor of HEAD; the vendor import must land by merge commit" >&2
  echo "  (a squash-merged or rebased import strands the tag; see ARCHITECTURE.md, 'Vendor imports must land by merge commit')" >&2
  exit 1
fi
# The partition below needs BASE2 to have one parent: BASE2^ names only the
# first, so a second parent's commits would fall out of both ranges.
if git rev-parse --verify --quiet "$base2^2" >/dev/null; then
  echo "error: $base2 is a merge; the pristine import must be a plain commit" >&2
  exit 1
fi

# The crates each range exports. Range 2 must keep range 1's crates: later
# commits go on patching them.
paths1=(vendor/hayro-syntax vendor/hayro-interpret vendor/hayro-postscript)
paths2=("${paths1[@]}" vendor/hayro-jbig2 vendor/hayro-jpeg2000)

out=vendor/patches/hayro
mkdir -p "$out"
find "$out" -maxdepth 1 -name '*.patch' -delete
count() { find "$out" -maxdepth 1 -name '*.patch' | wc -l | tr -d ' '; }

format_patch_pinned --numbered \
  --relative=vendor -o "$out" "$base1..$base2^" -- "${paths1[@]}"
n1=$(count)
format_patch_pinned --numbered \
  --relative=vendor --start-number "$((n1 + 1))" -o "$out" \
  "$base2..HEAD" -- "${paths2[@]}"
n=$(count)
echo "exported $n patch(es) to $out ($n1 before $base2, $((n - n1)) after)"
