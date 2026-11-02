#!/usr/bin/env bash
# Tests for build-wheel.sh's argument handling and its --docker wrapper
# (CI: the `rust` job, beside test-stage-helper.sh).
#
# The script is copied into a throwaway tree (it locates the repository from
# its own path, so the real checkout is never touched) and run with a fake
# `docker` on PATH that records its arguments and, like the real container
# build, overwrites the staged helper. Nothing is compiled. Checks:
#
#   1. a missing, unknown or repeated ABI is refused with status 2;
#   2. `--docker` refuses an unknown architecture, and an --out outside the
#      checkout (the container only sees the checkout);
#   3. `--docker aarch64 abi3` runs the pinned aarch64 image (by digest),
#      mounts the checkout at /io, re-runs the script inside with --out
#      translated to /io/..., and keeps cargo state under target/;
#   4. the host's staged helper is restored after the container overwrote it,
#      and stays absent when there was none.

# The assertion helpers are called only through `check`, which shellcheck
# cannot see.
# shellcheck disable=SC2329
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/build-wheel.sh"
root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT

mkdir -p "$root/tools/build" "$root/python/pdf_extract" "$root/bin"
cp "$script" "$root/tools/build/build-wheel.sh"
helper="$root/python/pdf_extract/pdf-extract-ocr-helper"

cat >"$root/bin/docker" <<FAKE
#!/usr/bin/env bash
printf '%s\n' "\$@" >"$root/docker-args"
printf 'linux-helper\n' >"$helper"
FAKE
chmod +x "$root/bin/docker"

fail=0
status=0
args=""
# run <args...>: run the copy with the fake docker; status and the docker
# arguments land in $status and $args.
run() {
    rm -f "$root/docker-args"
    status=0
    PATH="$root/bin:$PATH" "$root/tools/build/build-wheel.sh" "$@" >/dev/null 2>&1 || status=$?
    args="$(cat "$root/docker-args" 2>/dev/null || true)"
}
check() {
    local what="$1"
    shift
    if "$@"; then echo "ok   $what"; else echo "FAIL $what"; fail=1; fi
}
status_is() { [ "$status" = "$1" ]; }
has() { printf '%s\n' "$args" | grep -qxF -- "$1"; }
has_match() { printf '%s\n' "$args" | grep -qE -- "$1"; }
no_docker() { [ -z "$args" ]; }
helper_is() { [ "$(cat "$helper")" = "$1" ]; }
no_helper() { [ ! -e "$helper" ]; }

out="$root/target/out"

run --out "$out"
check "no ABI: usage error" status_is 2
run --out "$out" abi4
check "unknown ABI refused" status_is 2
run --out "$out" abi3 ft
check "two ABIs refused" status_is 2
run --out "$out" --bogus abi3
check "unknown option refused" status_is 2
run --out "$out" --docker sparc abi3
check "--docker sparc refused" status_is 2
check "--docker sparc: docker not run" no_docker
outside="$(mktemp -d)"
run --out "$outside" --docker aarch64 abi3
rmdir "$outside"
check "--docker with --out outside the checkout refused" status_is 2

printf 'host-helper\n' >"$helper"
run --out "$out" --docker aarch64 abi3
check "--docker aarch64 abi3 succeeds" status_is 0
check "pinned aarch64 image by digest" has_match '^quay\.io/pypa/manylinux_2_28_aarch64@sha256:[0-9a-f]{64}$'
check "arm64 platform" has linux/arm64
check "checkout mounted at /io" has "$root:/io"
check "cargo target under target/manylinux-aarch64" has "CARGO_TARGET_DIR=/io/target/manylinux-aarch64"
check "inner script run through bash (sdists drop execute bits)" has bash
check "inner run gets /io --out" has /io/target/out
check "inner run gets the ABI" has abi3
check "host helper restored" helper_is host-helper

rm -f "$helper"
run --out="$out" --docker=x86_64 ft
check "--docker=x86_64 ft succeeds" status_is 0
check "pinned x86_64 image by digest" has_match '^quay\.io/pypa/manylinux_2_28_x86_64@sha256:[0-9a-f]{64}$'
check "absent host helper stays absent" no_helper

exit "$fail"
