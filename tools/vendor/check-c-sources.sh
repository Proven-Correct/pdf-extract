#!/usr/bin/env bash
# Prove vendor/tesseract and vendor/leptonica equal their checksummed, pruned
# upstream archives plus the exported patch series (spec §3.4; CI job
# `vendor-patches`). The C/C++ counterpart of check-patches.sh.
#
# Usage: tools/vendor/check-c-sources.sh
#
# Imports fresh pristine trees with import-c.sh into a temp dir (which verifies
# each archive's SHA-256 and applies the prune list), applies
# vendor/patches/<source>/*.patch in name order to each, and diffs each result
# against vendor/<source>. Any drift fails: an edit that was never exported, a
# patch that no longer applies, a stale patch, or a prune list that no longer
# matches the committed tree. A missing or empty patch directory is an empty
# series, and then the tree must be pristine. Needs network access to
# github.com (via import-c.sh).
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

"$repo/tools/vendor/import-c.sh" "$work/vendor"

# As in check-patches.sh: stop git from discovering a repository above the
# temp dir (TMPDIR may sit inside one), which would make `git apply` resolve
# paths against that repository and silently skip every file.
export GIT_CEILING_DIRECTORIES
GIT_CEILING_DIRECTORIES=$(dirname "$work")

status=0
for name in tesseract leptonica; do
  shopt -s nullglob
  patches=("$repo/vendor/patches/$name"/*.patch)
  shopt -u nullglob
  # Patches are exported with --relative=vendor/<source>, so their paths start
  # at the tree root (src/...), as in the upstream repository.
  for patch in ${patches[@]+"${patches[@]}"}; do
    echo "applying ${patch#"$repo"/}"
    numstat=$(git -C "$work/vendor/$name" apply --numstat "$patch" 2>&1 || true)
    if [[ -z $numstat ]]; then
      echo "error: patch did not match any files: ${patch#"$repo"/}" >&2
      exit 1
    fi
    git -C "$work/vendor/$name" apply --whitespace=nowarn "$patch"
  done
  if diff -r "$work/vendor/$name" "$repo/vendor/$name"; then
    echo "vendor/$name matches pristine + ${#patches[@]} patch(es)"
  else
    echo "error: vendor/$name differs from pristine + ${#patches[@]} patch(es)." >&2
    echo "Run tools/vendor/export-c-patches.sh and commit the result." >&2
    status=1
  fi
done
exit "$status"
