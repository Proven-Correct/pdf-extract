#!/usr/bin/env bash
# Reproduce the pristine vendored C/C++ trees, Tesseract and Leptonica
# (spec §3.4), into DEST.
#
# Usage: tools/vendor/import-c.sh DEST
#
# vendor/PROVENANCE.md is the manifest: each "| tesseract" / "| leptonica" row
# of its C/C++ table names the source, version, archive URL, the archive's
# SHA-256 and the upstream commit; each "| prune |" row names one path to
# delete from that source's tree. For every source this downloads the archive,
# aborts on a checksum mismatch, checks the commit id the archive embeds (tag
# archives from GitHub carry one; release tarballs do not, and then the column
# is informational), extracts the single top-level directory to DEST/<source>/
# and deletes the prune paths. A prune path that does not exist is an error, so
# the list can only ever describe the archive it was written for.
#
# The checksum covers the archive bytes, so nothing is trusted from the
# network before it is verified: extraction happens only after the match.
# DEST must not exist yet. Needs network access to github.com, curl and git.
set -euo pipefail

[[ $# -eq 1 ]] || { echo "usage: $0 DEST" >&2; exit 2; }
dest=$1
[[ ! -e $dest ]] || { echo "error: $dest already exists" >&2; exit 1; }
repo=$(cd "$(dirname "$0")/../.." && pwd)
provenance=$repo/vendor/PROVENANCE.md

# sha256 FILE: the hex digest, with whichever tool the host has (GNU coreutils
# on Linux, shasum on macOS).
sha256() {
  if command -v sha256sum >/dev/null; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Rows look like "| tesseract | 5.5.3 | <url> | <sha256> | <commit> | <date> |".
# A read loop rather than mapfile keeps this working on macOS's bash 3.2.
sources=()
while IFS= read -r row; do sources+=("$row"); done < <(
  awk -F' *[|] *' '/^[|] (tesseract|leptonica) [|]/ { print $2, $3, $4, $5, $6 }' "$provenance")
[[ ${#sources[@]} -eq 2 ]] || {
  echo "error: expected the tesseract and leptonica rows in PROVENANCE.md, found ${#sources[@]}" >&2
  exit 1
}

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir -p "$dest"

for row in "${sources[@]}"; do
  read -r name version url want commit <<<"$row"
  archive=$scratch/$name-$version.tar.gz
  echo "fetching $name $version"
  curl -fsSL --retry 3 -o "$archive" "$url"
  got=$(sha256 "$archive")
  if [[ $got != "$want" ]]; then
    echo "error: $name $version archive sha256 $got, PROVENANCE.md says $want" >&2
    exit 1
  fi
  # A GitHub tag archive records its commit in a pax header; a release tarball
  # made by `make dist` has none, and get-tar-commit-id then exits non-zero.
  # Not a pipeline: get-tar-commit-id reads only the first header, so gzip
  # would die of SIGPIPE and, under pipefail, every archive would look like
  # one without an id and the check would never run.
  if embedded=$(git get-tar-commit-id < <(gzip -dc "$archive")); then
    if [[ $embedded != "$commit" ]]; then
      echo "error: $name archive embeds commit $embedded, PROVENANCE.md says $commit" >&2
      exit 1
    fi
    echo "$name archive embeds commit $embedded, as recorded"
  fi

  # Extract into an empty directory and insist on exactly one top-level
  # directory, so a tarball with stray top-level entries cannot spill them.
  unpack=$scratch/unpack-$name
  mkdir "$unpack"
  tar -xzf "$archive" -C "$unpack"
  tops=()
  while IFS= read -r top; do tops+=("$top"); done < <(
    find "$unpack" -mindepth 1 -maxdepth 1)
  if [[ ${#tops[@]} -ne 1 || ! -d ${tops[0]} ]]; then
    echo "error: $name archive does not have exactly one top-level directory" >&2
    exit 1
  fi
  tree=${tops[0]}

  prunes=()
  while IFS= read -r path; do prunes+=("$path"); done < <(
    awk -F' *[|] *' -v n="$name" '$2 == "prune" && $3 == n { print $4 }' "$provenance")
  [[ ${#prunes[@]} -gt 0 ]] || { echo "error: no prune rows for $name" >&2; exit 1; }
  for path in "${prunes[@]}"; do
    # Reject anything that could step outside the tree.
    case /$path/ in
      */../* | */./* | //*) echo "error: bad prune path '$path' for $name" >&2; exit 1 ;;
    esac
    if [[ ! -e $tree/$path && ! -L $tree/$path ]]; then
      echo "error: prune path $name/$path is not in the archive" >&2
      exit 1
    fi
    rm -rf "${tree:?}/$path"
  done

  mv "$tree" "$dest/$name"
done
