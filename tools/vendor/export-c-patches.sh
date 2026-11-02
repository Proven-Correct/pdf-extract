#!/usr/bin/env bash
# Regenerate vendor/patches/{tesseract,leptonica}/ from the commits that change
# the vendored C/C++ trees (spec §3.4). The C/C++ counterpart of
# export-patches.sh.
#
# Usage: tools/vendor/export-c-patches.sh [SOURCE [BASE]]
#
# SOURCE is tesseract or leptonica; without it both are exported, each from
# its own pristine tag. BASE defaults to vendor/<SOURCE>-pristine-1. Every
# non-merge commit in BASE..HEAD that touches vendor/<SOURCE> becomes one
# patch, numbered in commit order. --relative=vendor/<SOURCE> strips the prefix
# so paths read src/..., as in the upstream repository, and drops the parts of
# a commit outside the tree. The whole series is rewritten each time so that
# dropped or reordered commits leave no stale files; check-c-sources.sh then
# verifies the series reproduces the tree. An empty series leaves an empty
# directory, which git does not track: that is the same as no series.
#
# lib-format-patch.sh pins the format-patch/diff settings the caller's git
# config could otherwise change (its header lists the residuals it deliberately
# leaves unpinned), and --zero-commit and --no-signature keep the files stable
# across rebases and git versions, so re-exporting an unchanged stack produces
# no diff.
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
cd "$repo"
# shellcheck source=tools/vendor/lib-format-patch.sh
. "$repo/tools/vendor/lib-format-patch.sh"

# export_one SOURCE BASE
export_one() {
  local name=$1 base=$2 out
  if ! git rev-parse --verify --quiet "$base^{commit}" >/dev/null; then
    echo "error: base $base not found (tags are local; pass the pristine commit)" >&2
    exit 1
  fi
  # BASE..HEAD is only the series if BASE is in HEAD's history: a
  # squash-merged or rebased import leaves the tag stranded, and exporting
  # from it would silently write the wrong series and exit 0.
  if ! git merge-base --is-ancestor "$base" HEAD; then
    echo "error: base $base is not an ancestor of HEAD; the vendor import must land by merge commit" >&2
    echo "  (a squash-merged or rebased import strands the tag; see ARCHITECTURE.md, 'Vendor imports must land by merge commit')" >&2
    exit 1
  fi
  out=vendor/patches/$name
  mkdir -p "$out"
  find "$out" -maxdepth 1 -name '*.patch' -delete
  format_patch_pinned --relative="vendor/$name" \
    -o "$out" "$base..HEAD" -- "vendor/$name"
  echo "exported $(find "$out" -maxdepth 1 -name '*.patch' | wc -l | tr -d ' ') patch(es) to $out"
}

case $# in
  0)
    export_one tesseract vendor/tesseract-pristine-1
    export_one leptonica vendor/leptonica-pristine-1
    ;;
  1 | 2)
    case $1 in
      tesseract | leptonica) export_one "$1" "${2:-vendor/$1-pristine-1}" ;;
      *) echo "error: SOURCE must be tesseract or leptonica, not '$1'" >&2; exit 2 ;;
    esac
    ;;
  *) echo "usage: $0 [SOURCE [BASE]]" >&2; exit 2 ;;
esac
