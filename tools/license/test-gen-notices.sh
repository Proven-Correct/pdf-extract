#!/usr/bin/env bash
# Test for gen-notices.sh's sort_used_by (internal #151, #149; CI: license.yml). The
# only other guard on its duplicate-line collapse is the notices drift check,
# which sees it only for the crates that happen to list a licence twice today.
#
# gen-notices.sh is sourced, not run: its source guard stops it after the
# function definitions, so no cargo, jq or repository is needed. The fixture
# has, in order:
#   1. a list that is unsorted and has non-adjacent duplicates, ended by a
#      blank line (the mid-stream flush);
#   2. a bullet outside any "Used by:" list, which must stay where it is;
#   3. a second list, at end of input with no line after it (the END flush),
#      also unsorted and duplicated.
# Mutation check, done by hand when this test changes: delete the
# `run[i] != run[i - 1]` condition in sort_used_by and this test must fail.

set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(mktemp -d)"
trap 'rm -rf "$root"' EXIT

# shellcheck source=tools/license/gen-notices.sh
source "$here/gen-notices.sh"

cat >"$root/input" <<'EOF'
Licence text
Used by:
    - zeta 1.0.0
    - alpha 2.0.0
    - zeta 1.0.0
    - alpha 1.0.0
    - alpha 2.0.0

    - not a list entry
Used by:
    - b 1
    - a 1
    - b 1
EOF

cat >"$root/expected" <<'EOF'
Licence text
Used by:
    - alpha 1.0.0
    - alpha 2.0.0
    - zeta 1.0.0

    - not a list entry
Used by:
    - a 1
    - b 1
EOF

echo "Test: sort_used_by sorts each list, drops duplicates, and flushes a list at EOF"
sort_used_by <"$root/input" >"$root/actual"
if ! diff -u "$root/expected" "$root/actual"; then
    echo "  ✗ FAILED: sort_used_by output differs (diff above)" >&2
    exit 1
fi
echo "  ✓ sorted, deduplicated, END flush"
