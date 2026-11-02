#!/usr/bin/env bash
# The production-image test (packaging spec "Wheels install on the production
# OS image"; design D7 of ship-wheels-sbom-process-default, D4 of
# publish-publicly-for-adoption; ADR 0091): installs one built Linux abi3
# wheel into a stock Ubuntu container and runs tools/build/smoke.py there.
#
#     tools/build/prod-image-test.sh [--image 24.04|26.04] WHEEL
#
# Two images, each the platform a deployment uses:
#   24.04  the Phase 3 exit criterion: Ubuntu 24.04 with a pinned uv and a
#          uv-managed CPython 3.13 (24.04's own python3 is 3.12, below the
#          package's floor). The default.
#   26.04  the first consumer's target: Ubuntu 26.04 with the distribution's
#          own python3 (3.14) and python3-venv, the venv made by
#          `python3 -m venv` and the wheel installed with pip.
# The image matches the wheel's architecture (x86_64 or aarch64, from the
# file name); run it on a host of that architecture (emulation works but is
# slow). Inside the container, as production does:
#   - the distribution's tesseract-ocr-eng supplies the model (no
#     TESSDATA_PREFIX: the engine finds /usr/share/tesseract-ocr/5/tessdata);
#   - the wheel is installed with --no-index, which proves it needs nothing
#     else (no runtime dependencies, nothing built at install time);
#   - smoke.py runs from outside any checkout.
# Nothing else is installed: no compiler and no Python package besides the
# wheel. Only what creating the environment needs is added (curl and
# ca-certificates to fetch uv on 24.04; python3 and python3-venv on 26.04).
set -euo pipefail

# Pins. Each is a deliberate change: each image by its multi-arch index
# digest, uv by release, CPython by patch release.
UBUNTU_2404="ubuntu:24.04@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac"
UBUNTU_2604="ubuntu:26.04@sha256:c098726dd9b9733f3e885f8de0ef375206a311b0a317eadc40d347b1115fb944"
UV_VERSION="0.12.3"
PYTHON_VERSION="3.13.15"

usage() {
    echo "usage: $0 [--image 24.04|26.04] WHEEL" >&2
    exit 2
}
image=24.04
if [ "${1:-}" = --image ]; then
    [ $# -ge 2 ] || usage
    image=$2
    shift 2
fi
[ $# -eq 1 ] || usage
case "$image" in
24.04) ubuntu=$UBUNTU_2404 ;;
26.04) ubuntu=$UBUNTU_2604 ;;
*)
    echo "prod-image-test: unknown image: $image (24.04 or 26.04)" >&2
    exit 2
    ;;
esac
wheel="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
[ -f "$wheel" ] || {
    echo "prod-image-test: no such wheel: $1" >&2
    exit 2
}
case "$(basename "$wheel")" in
*-abi3-manylinux_2_28_x86_64.whl) platform=linux/amd64 ;;
*-abi3-manylinux_2_28_aarch64.whl) platform=linux/arm64 ;;
*)
    echo "prod-image-test: not an abi3 manylinux_2_28 x86_64/aarch64 wheel: $1" >&2
    exit 2
    ;;
esac
smoke="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/smoke.py"

docker run --rm --platform "$platform" \
    -v "$wheel:/w/$(basename "$wheel"):ro" \
    -v "$smoke:/s/smoke.py:ro" \
    -e UV_VERSION="$UV_VERSION" -e PYTHON_VERSION="$PYTHON_VERSION" \
    -e WHEEL="/w/$(basename "$wheel")" -e IMAGE="$image" \
    "$ubuntu" bash -euo pipefail -c '
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq
        if [ "$IMAGE" = 24.04 ]; then
            apt-get install -y -qq --no-install-recommends tesseract-ocr-eng ca-certificates curl >/dev/null
            curl -LsSf "https://astral.sh/uv/$UV_VERSION/install.sh" | env UV_NO_MODIFY_PATH=1 sh >/dev/null
            export PATH="$HOME/.local/bin:$PATH"
            uv venv --quiet --python "$PYTHON_VERSION" /venv
            VIRTUAL_ENV=/venv uv pip install --quiet --no-index "$WHEEL"
        else
            apt-get install -y -qq --no-install-recommends tesseract-ocr-eng python3 python3-venv >/dev/null
            python3 -c "import sys; assert sys.version_info[:2] == (3, 14), sys.version"
            python3 -m venv /venv
            /venv/bin/pip install --quiet --no-index --disable-pip-version-check "$WHEEL"
        fi
        cd /
        /venv/bin/python -I /s/smoke.py
    '
