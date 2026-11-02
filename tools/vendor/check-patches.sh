#!/usr/bin/env bash
# Prove vendor/ equals pristine upstream plus the exported patch series
# (spec §3.2; CI job `vendor-patches`), for the hayro crates. The C/C++ trees
# are checked by check-c-sources.sh.
#
# Usage: tools/vendor/check-patches.sh
#
# Imports a fresh pristine tree with import.sh into a temp dir, applies
# vendor/patches/hayro/*.patch in name order (their numeric prefixes give the
# series order), and diffs the result against vendor/. Any drift fails: an
# edit under vendor/ that was never exported, a patch that no longer applies,
# or a stale patch left behind. An empty series is valid and means vendor/
# must still be pristine. Needs network access to crates.io (via import.sh).
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

"$repo/tools/vendor/import.sh" "$work/vendor"

# Prevent git from escaping the temp directory if TMPDIR is inside a git repo.
# Without this, git would resolve patch paths against the parent repo and
# silently skip files while exiting 0. That escape is what older git did; on
# git 2.56 it no longer reproduces, so this export has no observable effect
# there and no test covers it (test-check-patches.sh's section 1 could not
# fail without it). It stays as defence for older git. The empty-numstat check
# below is the tested backstop: it catches the escape whatever its cause.
ceiling_dir=$(dirname "$work")
export GIT_CEILING_DIRECTORIES="$ceiling_dir"

shopt -s nullglob
patches=("$repo"/vendor/patches/hayro/*.patch)
shopt -u nullglob
# Patches are exported with --relative=vendor, so their paths start at the
# crate directory (hayro-syntax/src/...), as in the upstream monorepo. git
# apply works on a plain directory; it needs no repository.
for patch in ${patches[@]+"${patches[@]}"}; do
  echo "applying ${patch#"$repo"/}"
  # Validate that the patch will actually apply to files before applying.
  # git apply --numstat shows which files would be affected; an empty result
  # means the patch failed to match any files (likely due to git resolving
  # paths against the wrong repository).
  numstat=$(git -C "$work/vendor" apply --numstat "$patch" 2>&1 || true)
  if [ -z "$numstat" ]; then
    echo "error: patch did not match any files: ${patch#"$repo"/}" >&2
    exit 1
  fi
  git -C "$work/vendor" apply --whitespace=nowarn "$patch"
done

# PROVENANCE.md and patches/ are our files, not part of the upstream tree,
# and the vendored C/C++ trees (tesseract/, leptonica/) are not hayro: their
# own pristine-plus-patches check is check-c-sources.sh. Copying them in
# (rather than `diff -x`, which matches a basename at any depth and could hide
# an upstream file of the same name) keeps the diff exact for the hayro crates.
cp "$repo/vendor/PROVENANCE.md" "$work/vendor/"
for dir in patches tesseract leptonica; do
  if [[ -d $repo/vendor/$dir ]]; then cp -R "$repo/vendor/$dir" "$work/vendor/"; fi
done
if ! diff -r "$work/vendor" "$repo/vendor"; then
  echo "error: vendor/ differs from pristine + ${#patches[@]} patch(es)." >&2
  echo "Run tools/vendor/export-patches.sh and commit the result." >&2
  exit 1
fi
echo "vendor/ matches pristine + ${#patches[@]} patch(es)"
