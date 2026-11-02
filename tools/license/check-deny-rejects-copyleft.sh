#!/usr/bin/env bash
# Prove that the cargo-deny policy still rejects copyleft *dependencies* now
# that our own GPL-3.0-or-later crates are exempted via
# `[licenses.private] ignore = true` (spec 4.12b, D-40, L1).
#
# Usage: tools/license/check-deny-rejects-copyleft.sh
#
# Builds a throwaway workspace in a temp dir: a private (`publish = false`)
# GPL-3.0-or-later crate, like ours, that depends on a local path crate whose
# `license` is varied. A permissive control (MIT) must pass; GPL-3.0-only,
# LGPL-2.1 and AGPL-3.0 must each fail `cargo deny check licenses`. Uses the
# repo's real deny.toml and needs no network (path crates only). Requires
# cargo-deny on PATH (the `deny` CI job installs it for its own use).
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# probe: stands in for our own crates. The dependency is deliberately not
# `publish = false`, so the `private` exemption cannot hide it.
make_workspace() {
    local license=$1
    rm -rf "${work:?}/ws"
    mkdir -p "$work/ws/probe/src" "$work/ws/dep/src"
    cp "$repo/deny.toml" "$work/ws/deny.toml"
    cat >"$work/ws/Cargo.toml" <<TOML
[workspace]
members = ["probe", "dep"]
resolver = "3"
TOML
    cat >"$work/ws/probe/Cargo.toml" <<TOML
[package]
name = "probe"
version = "0.0.0"
edition = "2024"
license = "GPL-3.0-or-later"
publish = false
[dependencies]
dep = { path = "../dep" }
TOML
    cat >"$work/ws/dep/Cargo.toml" <<TOML
[package]
name = "dep"
version = "0.0.0"
edition = "2024"
license = "$license"
TOML
    echo "// probe" >"$work/ws/probe/src/lib.rs"
    echo "// dep" >"$work/ws/dep/src/lib.rs"
}

run_deny() {
    (cd "$work/ws" && cargo deny --offline check licenses >"$work/out.txt" 2>&1)
}

fail=0

make_workspace "MIT"
if run_deny; then
    echo "ok   control (MIT dependency) passes"
else
    echo "FAIL control (MIT dependency) was rejected; harness is broken" >&2
    cat "$work/out.txt" >&2
    fail=1
fi

for lic in "GPL-3.0-only" "LGPL-2.1-only" "AGPL-3.0-only" "LGPL-2.1" "AGPL-3.0"; do
    make_workspace "$lic"
    if run_deny; then
        echo "FAIL $lic dependency was accepted" >&2
        fail=1
    elif grep -q "rejected" "$work/out.txt"; then
        echo "ok   $lic dependency is rejected"
    else
        echo "FAIL $lic: deny failed for an unrelated reason" >&2
        cat "$work/out.txt" >&2
        fail=1
    fi
done

exit "$fail"
