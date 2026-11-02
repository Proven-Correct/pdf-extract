#!/usr/bin/env bash
# Reproduce the pristine vendored hayro tree (spec §3.2) into DEST.
#
# Usage: tools/vendor/import.sh DEST
#
# vendor/PROVENANCE.md is the manifest: each table row names a crate, its
# version and the crates.io package checksum. The crates are fetched with
# `cargo vendor` from a throwaway manifest, the package checksum cargo records
# is compared with PROVENANCE.md (a mismatch aborts), and each crate lands in
# DEST/<name>/ minus .cargo-checksum.json. That file is removed because cargo
# checks every vendored file against it and would refuse to build once a patch
# touches one; the package checksum it held is what PROVENANCE.md keeps.
# DEST must not exist yet. Needs network access to crates.io.
set -euo pipefail

[[ $# -eq 1 ]] || { echo "usage: $0 DEST" >&2; exit 2; }
dest=$1
[[ ! -e $dest ]] || { echo "error: $dest already exists" >&2; exit 1; }
repo=$(cd "$(dirname "$0")/../.." && pwd)

# Rows look like "| hayro-syntax | 0.7.2 | <sha256> | <date> |". A read loop
# rather than mapfile keeps this working on macOS's bash 3.2.
rows=()
while IFS= read -r row; do rows+=("$row"); done < <(
  awk -F' *[|] *' '/^[|] hayro-/ { print $2, $3, $4 }' "$repo/vendor/PROVENANCE.md")
[[ ${#rows[@]} -gt 0 ]] || { echo "error: no crates in PROVENANCE.md" >&2; exit 1; }

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir "$scratch/src"
: >"$scratch/src/lib.rs"
{
  printf '[package]\nname = "vendor-import"\nversion = "0.0.0"\nedition = "2021"\n'
  printf '[workspace]\n[dependencies]\n'
  for row in "${rows[@]}"; do
    read -r name version _ <<<"$row"
    printf '%s = "=%s"\n' "$name" "$version"
  done
} >"$scratch/Cargo.toml"

# Run from the repo so rust-toolchain.toml selects the pinned cargo.
(cd "$repo" && cargo vendor --quiet --versioned-dirs \
  --manifest-path "$scratch/Cargo.toml" "$scratch/vendor" >/dev/null)

mkdir -p "$dest"
for row in "${rows[@]}"; do
  read -r name version want <<<"$row"
  src="$scratch/vendor/$name-$version"
  got=$(sed -n 's/.*"package":"\([0-9a-f]\{64\}\)".*/\1/p' "$src/.cargo-checksum.json")
  if [[ $got != "$want" ]]; then
    echo "error: $name $version checksum $got, PROVENANCE.md says $want" >&2
    exit 1
  fi
  cp -R "$src" "$dest/$name"
  rm "$dest/$name/.cargo-checksum.json"
done
