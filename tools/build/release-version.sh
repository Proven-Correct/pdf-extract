#!/usr/bin/env bash
# The release preconditions the `version` job of wheels.yml checks (packaging
# spec "Release workflow triggers and outputs"; design D5 of
# publish-publicly-for-adoption):
#
#   tools/build/release-version.sh [TAG]
#
# - the release notes for the workspace version, docs/releases/v<version>.md,
#   exist and are not blank, on every run (a dispatch rehearsal catches a
#   missing file before the tag does);
# - with TAG (a tag push), TAG is exactly v<version>.
#
# Prints the notes path, which the `release` job passes to
# `gh release create --notes-file`. The version is read from Cargo.toml's
# [workspace.package], the single source the Python package follows too.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
if [ $# -gt 1 ]; then
    echo "usage: $0 [TAG]" >&2
    exit 2
fi
# The `version = "..."` line of the [workspace.package] table. awk rather
# than Python's tomllib, so the script runs on any host (macOS ships a
# python3 too old for tomllib); the workspace manifest is ours and keeps
# this plain form.
version="$(awk '
    /^\[/ { section = $0; next }
    section == "[workspace.package]" && /^version[[:space:]]*=/ {
        gsub(/^version[[:space:]]*=[[:space:]]*"|".*$/, ""); print; exit
    }' "$repo/Cargo.toml")"
if [ -z "$version" ]; then
    echo "release-version: no version in Cargo.toml's [workspace.package]" >&2
    exit 1
fi
if [ $# -eq 1 ] && [ "$1" != "v$version" ]; then
    echo "release-version: tag $1 does not match the workspace version $version" >&2
    exit 1
fi
notes="docs/releases/v$version.md"
if [ ! -f "$repo/$notes" ]; then
    echo "release-version: no release notes for $version: write $notes" >&2
    exit 1
fi
if ! grep -q '[^[:space:]]' "$repo/$notes"; then
    echo "release-version: $notes is blank" >&2
    exit 1
fi
echo "$notes"
