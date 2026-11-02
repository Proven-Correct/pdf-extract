#!/usr/bin/env bash
# Builds the OCR helper binary and stages it into the Python package
# directory (run 0002 spec §3.7, D-21), where `isolation="process"` finds it
# beside the extension module and maturin's `include` ships it in the wheel.
#
# Run before `maturin develop` or `maturin build`:
#
#     tools/build/stage-helper.sh [extra cargo args, e.g. --features ...]
#
# Extra arguments go to `cargo build` unchanged, so a `system-tesseract`
# source build stages a helper linked the same way as its extension:
#     tools/build/stage-helper.sh --no-default-features --features system-tesseract
# `--target`, `--target-dir` and the target environment variables work too,
# and so does `--profile <name>` (or `--profile=<name>`): it replaces the
# default `--release`, since cargo rejects the pair. The staged binary is
# the one cargo reports, so a profile's own output directory needs no care.
#
# The helper is built in release mode unless a profile is named: it is a
# separate process whose speed does not depend on how the extension was
# built, and a debug Tesseract is several times slower. The staged file is
# git-ignored. A missing helper
# is not a build failure; it is `OcrUnavailableError(reason="engine_not_built")`
# for `isolation="process"` only (O28).
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo"

# `--release` is the default profile only; cargo errors on `--release` with
# `--profile`. (The `+` form below keeps an empty array legal under `set -u`
# on macOS's bash 3.2.)
profile_args=(--release)
for arg in "$@"; do
    case "$arg" in
    --profile | --profile=*) profile_args=() ;;
    esac
done

# The binary's location comes from cargo's own report, not from a guessed
# `<target>/release/` path: `--target`, `CARGO_BUILD_TARGET`, `--target-dir`,
# `CARGO_TARGET_DIR`, `build.target-dir` and `--profile` each move it, and a
# guessed path then misses the file or, worse, stages a stale binary from an
# earlier build. `json-render-diagnostics` keeps compiler diagnostics on
# stderr as usual and puts one JSON message per artifact on stdout. A fresh
# (cached) build still reports its artifacts. python3 does the parsing so
# that no further tool is needed (the Python build already requires it).
messages="$(cargo build ${profile_args[@]+"${profile_args[@]}"} --locked -p pdf-extract-ocr \
    --bin pdf-extract-ocr-helper --message-format=json-render-diagnostics "$@")"
src="$(python3 -I -c '
import json, sys
exe = None
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    t = m.get("target", {})
    if (m.get("reason") == "compiler-artifact" and t.get("name") == "pdf-extract-ocr-helper"
            and "bin" in t.get("kind", []) and m.get("executable")):
        exe = m["executable"]
if exe is None:
    sys.exit("stage-helper: cargo reported no executable for pdf-extract-ocr-helper")
print(exe)
' <<<"$messages")"
dest="$repo/python/pdf_extract/pdf-extract-ocr-helper"

# Copy to a temporary name and rename, so a concurrently starting engine
# never sees a half-written binary.
tmp="$dest.tmp.$$"
# A failed cp/chmod/mv must not leave the temporary file in the package dir,
# where maturin's `include` could ship it.
trap 'rm -f "$tmp"' EXIT
cp "$src" "$tmp"
chmod 0755 "$tmp"
mv -f "$tmp" "$dest"
echo "staged $dest"
