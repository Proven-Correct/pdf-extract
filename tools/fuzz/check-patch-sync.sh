#!/usr/bin/env bash
# Checks that fuzz/Cargo.toml's [patch.crates-io] table routes exactly the
# crates the root Cargo.toml's does to the same vendored directories (design
# D6; CI job `vendor-patches`). fuzz/ is its own workspace, so the root table
# does not apply there; a crate missing from its copy would be fuzzed as the
# unpatched crates.io release, not the fork the library ships.
#
# Usage: tools/fuzz/check-patch-sync.sh [ROOT_MANIFEST FUZZ_MANIFEST]
#        tools/fuzz/check-patch-sync.sh --self-test
#
# Each table must hold only `name = { path = "..." }` entries (comments and
# blank lines aside); anything else is an error, because a form this script
# cannot read is a form it cannot compare. Fuzz paths are relative to fuzz/,
# so one leading `../` is removed before comparing. --self-test runs the
# check on scratch manifests: an identical pair must pass, and a missing
# entry, an extra entry, a different path and an unreadable entry must each
# fail.
set -euo pipefail
export LC_ALL=C

repo=$(cd "$(dirname "$0")/../.." && pwd)

# Prints the [patch.crates-io] entries of manifest $1 as sorted "name path"
# lines, with prefix $2 removed from each path. Fails on an unreadable entry
# or a missing table.
entries() {
  awk -v prefix="$2" -v file="$1" '
    /^\[/ { in_table = ($0 == "[patch.crates-io]"); if (in_table) seen = 1; next }
    !in_table { next }
    /^[[:space:]]*(#|$)/ { next }
    {
      line = $0
      if (match(line, /^[A-Za-z0-9_-]+[[:space:]]*=[[:space:]]*\{[[:space:]]*path[[:space:]]*=[[:space:]]*"[^"]*"[[:space:]]*\}[[:space:]]*$/) == 0) {
        printf "%s: unreadable [patch.crates-io] entry: %s\n", file, line > "/dev/stderr"
        bad = 1
        next
      }
      name = line; sub(/[[:space:]]*=.*/, "", name)
      path = line; sub(/^[^"]*"/, "", path); sub(/".*/, "", path)
      if (prefix != "") {
        if (substr(path, 1, length(prefix)) != prefix) {
          printf "%s: path of %s does not start with %s\n", file, name, prefix > "/dev/stderr"
          bad = 1
          next
        }
        path = substr(path, length(prefix) + 1)
      }
      print name, path
    }
    END {
      if (!seen) { printf "%s: no [patch.crates-io] table\n", file > "/dev/stderr"; exit 1 }
      exit bad
    }
  ' "$1" | sort
  # awk's status, not sort's: an unreadable entry must fail the check.
  return "${PIPESTATUS[0]}"
}

check() {
  local root_manifest=$1 fuzz_manifest=$2 root_entries fuzz_entries
  root_entries=$(entries "$root_manifest" "") || return 1
  fuzz_entries=$(entries "$fuzz_manifest" "../") || return 1
  if [[ "$root_entries" != "$fuzz_entries" ]]; then
    echo "[patch.crates-io] differs between $root_manifest and $fuzz_manifest (paths relative to the root):" >&2
    diff <(echo "$root_entries") <(echo "$fuzz_entries") >&2 || true
    return 1
  fi
  echo "patch tables in sync: $(echo "$root_entries" | wc -l | tr -d ' ') entries"
}

# The first line of the check's complaint is echoed, so a plant caught for
# the wrong reason (a botched plant, not the drift it stands for) shows.
expect_failure() {
  local out
  if out=$(check "$scratch/root.toml" "$scratch/fuzz.toml" 2>&1); then
    echo "self-test FAILED: $1 was not caught" >&2
    exit 1
  fi
  echo "self-test: $1 caught: $(head -n 1 <<<"$out")"
}

self_test() {
  # Global, not local: the EXIT trap runs after this function has returned.
  scratch=$(mktemp -d)
  trap 'rm -rf "$scratch"' EXIT
  cat >"$scratch/root.toml" <<'EOF'
[workspace]
members = ["crates/*"]

[patch.crates-io]
# a comment
hayro-syntax = { path = "vendor/hayro-syntax" }
hayro-interpret = { path = "vendor/hayro-interpret" }
EOF
  local good='[package]
name = "x"

[patch.crates-io]
hayro-interpret = { path = "../vendor/hayro-interpret" }

hayro-syntax = { path = "../vendor/hayro-syntax" }

[[bin]]
name = "open"'
  echo "$good" >"$scratch/fuzz.toml"
  check "$scratch/root.toml" "$scratch/fuzz.toml" >/dev/null ||
    { echo "self-test FAILED: an identical table does not pass" >&2; exit 1; }
  echo "self-test: identical tables pass"

  # Replacements come from variables: escapes inside ${var/pat/repl} are
  # kept literally by bash 3.2 (macOS), which would botch the plants.
  local syntax='hayro-syntax = { path = "../vendor/hayro-syntax" }'
  local extra=$syntax$'\n''hayro-jbig2 = { path = "../vendor/hayro-jbig2" }'
  local old='../vendor/hayro-syntax-old' rootrel='vendor/hayro-syntax'
  local git='hayro-syntax = { git = "https://example.invalid/hayro" }'
  echo "${good/"$syntax"/}" >"$scratch/fuzz.toml"
  expect_failure "a missing entry"
  echo "${good/"$syntax"/$extra}" >"$scratch/fuzz.toml"
  expect_failure "an extra entry"
  echo "${good/..\/vendor\/hayro-syntax/$old}" >"$scratch/fuzz.toml"
  expect_failure "a different path"
  echo "${good/..\/vendor\/hayro-syntax/$rootrel}" >"$scratch/fuzz.toml"
  expect_failure "a path not relative to fuzz/"
  echo "${good/"$syntax"/$git}" >"$scratch/fuzz.toml"
  expect_failure "an unreadable entry"
  echo "${good/\[patch.crates-io\]/[dependencies]}" >"$scratch/fuzz.toml"
  expect_failure "a missing table"
  echo "self-test passed"
}

case "${1:-}" in
  --self-test) self_test ;;
  "") check "$repo/Cargo.toml" "$repo/fuzz/Cargo.toml" ;;
  *)
    [[ $# -eq 2 ]] || { echo "usage: $0 [ROOT_MANIFEST FUZZ_MANIFEST] | --self-test" >&2; exit 2; }
    check "$1" "$2"
    ;;
esac
