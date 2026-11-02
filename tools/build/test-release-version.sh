#!/usr/bin/env bash
# Tests for release-version.sh (CI: the `rust` job). The script is copied into
# a throwaway tree with its own Cargo.toml and docs/releases/, since it finds
# the repository from its own path:
#
#   1. notes present, no tag: prints the notes path;
#   2. notes present, matching tag: prints the notes path;
#   3. a tag that is not v<version>: refused;
#   4. no notes file, or a blank one: refused, with or without a tag;
#   5. more than one argument: usage error;
#   6. a manifest without [workspace.package] version: refused, and only that
#      table's version line counts.
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/release-version.sh"
root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT
mkdir -p "$root/tools/build" "$root/docs/releases"
cp "$script" "$root/tools/build/release-version.sh"
run() { bash "$root/tools/build/release-version.sh" "$@"; }
fail() {
    echo "FAIL: $*" >&2
    exit 1
}
expect_error() {
    local want=$1 out
    shift
    if out=$(run "$@" 2>&1); then fail "accepted: $* ($out)"; fi
    [[ $out == *"$want"* ]] || fail "wrong error for $*: $out"
}

printf '[workspace]\nmembers = []\n' >"$root/Cargo.toml"
expect_error "no version in Cargo.toml"
# Another table's version line must not be taken for the workspace's.
printf '[package]\nversion = "9.9.9"\n\n[workspace.package]\nversion = "1.2.3"\nedition = "2024"\n' >"$root/Cargo.toml"

expect_error "no release notes for 1.2.3"
expect_error "no release notes for 1.2.3" v1.2.3
printf '  \n\t\n' >"$root/docs/releases/v1.2.3.md"
expect_error "is blank" v1.2.3
printf '# pdf-extract 1.2.3\n' >"$root/docs/releases/v1.2.3.md"
[ "$(run)" = docs/releases/v1.2.3.md ] || fail "no-tag run did not print the notes path"
[ "$(run v1.2.3)" = docs/releases/v1.2.3.md ] || fail "tag run did not print the notes path"
expect_error "does not match the workspace version 1.2.3" v1.2.4
expect_error "does not match" 1.2.3
expect_error "usage" v1.2.3 extra
echo "test-release-version: all checks passed"
