#!/usr/bin/env bash
# Builds one release wheel of pdf-extract (design D4 of the
# ship-wheels-sbom-process-default change). The same script runs locally and
# in CI (`ci.yml` job `wheel`, `wheels.yml`), so a wheel is always built the
# same way: the OCR helper is staged first, by construction, rather than by a
# step someone has to remember (ADR 0076).
#
#     tools/build/build-wheel.sh [--out DIR] [--docker x86_64|aarch64] abi3|ft
#
#   abi3  the stable-ABI wheel (cp313-abi3): `--features abi3`, built with
#         CPython 3.13;
#   ft    the free-threaded wheel (cp314t): no abi3 (free-threaded CPython
#         has no stable ABI), built with CPython 3.14t.
#
# Platforms:
#   Linux  must run inside the pinned manylinux_2_28 image (`--docker` starts
#          it): the extension and the helper are both linked against its
#          glibc 2.28, and the wheel is tagged manylinux_2_28.
#   macOS  runs natively, with MACOSX_DEPLOYMENT_TARGET (default 11.0)
#          exported to both cargo invocations, so the helper and the extension
#          share one minimum OS version. Interpreters come from uv.
#
# `--docker <arch>` re-runs this script in the manylinux image for <arch>
# (native architecture only in CI; anything else is emulated and slow). The
# checkout is mounted at /io; rustup, the cargo home and the build tools live
# in target/docker-<arch>/ and the build output in target/manylinux-<arch>/,
# all on the host so CI can cache them. The host's staged helper (a macOS or
# glibc-host binary used by `maturin develop`) is saved before the container
# overwrites it and restored afterwards.
#
# Environment:
#   PDF_EXTRACT_WHEEL_PYTHON  build interpreter, overriding the default above
#                             (native runs only: a host path means nothing in
#                             the container, so --docker does not pass it on)
#   MACOSX_DEPLOYMENT_TARGET  macOS minimum (default 11.0)
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# The manylinux_2_28 images, pinned by digest: a new image can bring a new
# compiler or CMake, so moving the pin is a deliberate change.
image_x86_64="quay.io/pypa/manylinux_2_28_x86_64@sha256:b74a5b6f1705ecff17be8bd792589bef9b3b039766d12205720ecd2a9e919192"
image_aarch64="quay.io/pypa/manylinux_2_28_aarch64@sha256:0333f44c4eb759ec8f47b27bc9dc27b1c929932a267cf13ce0cff1a4b758cccb"

usage() {
    echo "usage: $0 [--out DIR] [--docker x86_64|aarch64] abi3|ft" >&2
    exit 2
}

out="$repo/target/wheels"
docker_arch=""
abi=""
while [ $# -gt 0 ]; do
    case "$1" in
    --out)
        [ $# -ge 2 ] || usage
        out="$2"
        shift 2
        ;;
    --out=*)
        out="${1#--out=}"
        shift
        ;;
    --docker)
        [ $# -ge 2 ] || usage
        docker_arch="$2"
        shift 2
        ;;
    --docker=*)
        docker_arch="${1#--docker=}"
        shift
        ;;
    abi3 | ft)
        [ -z "$abi" ] || usage
        abi="$1"
        shift
        ;;
    *)
        echo "build-wheel: unknown argument: $1" >&2
        usage
        ;;
    esac
done
[ -n "$abi" ] || usage
# Relative to the caller's directory, before any cd.
mkdir -p "$out"
out="$(cd "$out" && pwd)"

if [ -n "$docker_arch" ]; then
    case "$docker_arch" in
    x86_64) image="$image_x86_64" platform=linux/amd64 ;;
    aarch64) image="$image_aarch64" platform=linux/arm64 ;;
    *)
        echo "build-wheel: --docker takes x86_64 or aarch64, not $docker_arch" >&2
        exit 2
        ;;
    esac
    case "$out" in
    "$repo"/*) ;;
    *)
        echo "build-wheel: with --docker, --out must be inside the checkout ($repo)" >&2
        exit 2
        ;;
    esac
    cache="$repo/target/docker-$docker_arch"
    mkdir -p "$cache"
    helper="$repo/python/pdf_extract/pdf-extract-ocr-helper"
    saved="$(mktemp -d)"
    # Restore the host's helper (or its absence) whatever happens inside.
    # Called only by the EXIT trap, which shellcheck cannot see.
    # shellcheck disable=SC2329
    restore() {
        if [ -e "$saved/helper" ]; then
            mv -f "$saved/helper" "$helper"
        else
            rm -f "$helper"
        fi
        rm -rf "$saved"
    }
    trap restore EXIT
    if [ -e "$helper" ]; then cp -p "$helper" "$saved/helper"; fi
    docker run --rm --platform "$platform" \
        --user "$(id -u):$(id -g)" \
        -v "$repo:/io" -w /io \
        -e HOME="/io/target/docker-$docker_arch" \
        -e CARGO_HOME="/io/target/docker-$docker_arch/cargo" \
        -e RUSTUP_HOME="/io/target/docker-$docker_arch/rustup" \
        -e CARGO_TARGET_DIR="/io/target/manylinux-$docker_arch" \
        "$image" \
        bash /io/tools/build/build-wheel.sh --out "/io/${out#"$repo"/}" "$abi"
    exit 0
fi

cd "$repo"

case "$(uname -s)" in
Linux)
    if [ ! -d /opt/python ]; then
        echo "build-wheel: Linux wheels are built in the manylinux image; use --docker $(uname -m)" >&2
        exit 2
    fi
    case "$abi" in
    abi3) py="${PDF_EXTRACT_WHEEL_PYTHON:-/opt/python/cp313-cp313/bin/python3}" ;;
    ft) py="${PDF_EXTRACT_WHEEL_PYTHON:-/opt/python/cp314-cp314t/bin/python3}" ;;
    esac
    compat=(--compatibility manylinux_2_28)
    # The image has no Rust: rustup goes into the mounted (cached)
    # CARGO_HOME/RUSTUP_HOME, then installs the rust-toolchain.toml pin
    # explicitly (rustup no longer installs a missing toolchain on first use).
    if ! command -v rustup >/dev/null 2>&1 && [ ! -x "${CARGO_HOME:-$HOME/.cargo}/bin/rustup" ]; then
        # Three attempts: a dropped TLS connection (curl exit 35) is not
        # among the failures curl's own --retry repeats.
        for attempt in 1 2 3; do
            if curl --proto '=https' --tlsv1.2 -sSf --retry 3 https://sh.rustup.rs |
                sh -s -- -y --no-modify-path --profile minimal --default-toolchain none; then
                break
            fi
            [ "$attempt" = 3 ] && exit 1
            sleep 10
        done
    fi
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
    rustup toolchain install
    ;;
Darwin)
    case "$abi" in
    abi3) want=3.13 ;;
    ft) want=3.14t ;;
    esac
    if [ -z "${PDF_EXTRACT_WHEEL_PYTHON:-}" ]; then
        uv python install "$want" >/dev/null
        py="$(uv python find "$want")"
    else
        py="$PDF_EXTRACT_WHEEL_PYTHON"
    fi
    # uv may also know an x86_64 (Rosetta) CPython; an arm64 wheel needs an
    # arm64 interpreter (maturin tags the wheel after it).
    if [ "$("$py" -I -c 'import platform; print(platform.machine())')" != arm64 ]; then
        echo "build-wheel: $py is not an arm64 interpreter; set PDF_EXTRACT_WHEEL_PYTHON" >&2
        exit 2
    fi
    compat=()
    export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
    ;;
*)
    echo "build-wheel: unsupported platform $(uname -s)" >&2
    exit 2
    ;;
esac

features=()
[ "$abi" = abi3 ] && features=(--features abi3)

# maturin at the version uv.lock pins for development, in a venv of its own
# (the build interpreter's site-packages stay untouched).
maturin_version="$("$py" -I -c '
import re, sys
text = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r"\[\[package\]\]\nname = \"maturin\"\nversion = \"([^\"]+)\"", text)
sys.exit("build-wheel: no maturin pin in uv.lock") if m is None else print(m.group(1))
' "$repo/uv.lock")"
tools="${CARGO_TARGET_DIR:-$repo/target}/wheel-tools/maturin-$maturin_version-$(basename "$(dirname "$(dirname "$py")")")"
# Reused only if it actually runs: a CI cache of target/ (rust-cache prunes
# files it does not recognise) can restore a half-deleted venv, so anything
# else is rebuilt from scratch (--clear).
if ! "$tools/bin/maturin" --version >/dev/null 2>&1; then
    "$py" -m venv --clear "$tools"
    "$tools/bin/python" -m pip install --quiet --disable-pip-version-check "maturin==$maturin_version"
fi

# The helper first, with the wheel's profile (release) and linkage (the
# default `vendored`). stage-helper.sh parses cargo's JSON with python3.
# Scripts are run through bash, not their execute bit: maturin's sdist stores
# every file 0644, and a release wheel must be buildable from the sdist.
PATH="$(dirname "$py"):$PATH" bash tools/build/stage-helper.sh

# The two SBOMs (tools/license/sbom.py): the Rust graph for this target and
# features, and the native components plus the helper just staged. Written
# per ABI so an abi3 and an ft build never share (or race on) the files.
triple="$(rustc -vV | sed -n 's/^host: //p')"
sbom_dir="${CARGO_TARGET_DIR:-$repo/target}/sbom-$abi"
sbom_features=""
[ "$abi" = abi3 ] && sbom_features=abi3
"$py" -I tools/license/sbom.py --target "$triple" --features "$sbom_features" \
    --helper python/pdf_extract/pdf-extract-ocr-helper --out-dir "$sbom_dir" >/dev/null

"$tools/bin/maturin" build --release --locked \
    --interpreter "$py" ${compat[@]+"${compat[@]}"} ${features[@]+"${features[@]}"} \
    --sbom-include "$sbom_dir/rust.cdx.json" --sbom-include "$sbom_dir/native.cdx.json" \
    --out "$out"
