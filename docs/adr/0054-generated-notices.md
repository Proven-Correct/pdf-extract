---
id: 0054
title: Generate THIRD_PARTY_NOTICES from the extension's dependency graph and embedded assets
date: 2026-10-02
status: Accepted
summary: tools/license/gen-notices.sh writes THIRD_PARTY_NOTICES (cargo-about 0.9.2 + AGL block + verbatim asset notices) and CI fails on any diff.
---

# 0054. Generate THIRD_PARTY_NOTICES from the extension's dependency graph and embedded assets

## Context

ADR 0010 left the full notices file to Phase 3; the wheel shipped a hand-written stopgap carrying only the
Adobe Glyph List notice. The wheel statically links every crate in `crates/python`'s normal dependency graph
and embeds data files that no crate-level licence tool sees: hayro-interpret's Foxit fonts and ICC profiles,
hayro-jpeg2000's ICC profiles and hayro-cmap's Adobe CMaps (internal issues #19, #20, #21). A release could not ship
without those notices.

## Decision

`THIRD_PARTY_NOTICES` is a generated file, never edited by hand. `tools/license/gen-notices.sh` writes it from
three parts:

1. cargo-about 0.9.2 (pinned; its output format is part of the file) over `crates/python`'s graph, configured by
   `about.toml`: the accepted list equals `deny.toml`'s allow-list in the same order (so dual-licensed crates ship
   their MIT text), dev and build dependencies are excluded, our private crates are ignored. Crates whose package
   lacks a file for the licence cargo-about would pick are clarified in `about.toml` to a real, checksum-pinned
   licence file (brotli-decompressor, alloc-stdlib).
2. The Adobe Glyph List block (`tools/license/agl-notice.txt`, verbatim).
3. The embedded assets' notices and each vendored hayro crate's LICENSE, copied verbatim from the crate sources
   at generation time.

The CI `deny` job reruns the script and fails on any diff. `crates/testkit/tests/license.rs` checks by exact text
that each vendored hayro LICENSE section and each asset notice is present, that no unfilled SPDX template
placeholder remains, and that `about.toml`'s accepted list equals `deny.toml`'s allow-list.

hayro's embedding features (`embed-fonts`, `embed-cmaps`) stay on; the assets ship with their notices.

## Alternatives considered

- **Keep a hand-maintained file** — the stopgap showed it goes stale silently; nothing ties it to `Cargo.lock`.
- **Turn off hayro's embedding features to avoid shipping the assets** — rejected; the embedded fonts and CMaps
  are kept, so their notices ship instead.
- **A separate CI job for the notices check** — the check runs as steps in the existing `deny` job, next to the
  licence policy it complements.

## Consequences

- Any dependency change that alters the linked graph requires rerunning `gen-notices.sh` and committing the
  result; CI enforces it.
- The alloc-stdlib clarification reads the sibling `alloc-no-stdlib-2.0.4` licence file by path, so a lockfile
  bump of that crate breaks generation (loudly, via the checksum) until `about.toml` is updated.
- cargo-about includes crates reached only through proc-macros (never linked into the wheel); the file
  over-includes rather than under-includes.
- MIT files that carry no copyright line upstream ship without one; the crate's own licence file is reproduced
  as-is.
- The wordlist test (ADR 0017) scans the generated file, so new dependency licence text can require an ALLOW
  entry.

## Addendum (2026-10-03)

`gen-notices.sh` gains Part 4, "Vendored C/C++ libraries": the verbatim Tesseract and Leptonica licence texts with a
heading naming the version and the copyright holders found by `check-c-licenses.sh`. The Apache text's section 4(d)
sentence needed no new wordlist `ALLOW` row: the existing `THIRD_PARTY_NOTICES` row already matches both the Part 1 and
the Part 4 copy (spec 0002 section 4.5).

## Addendum (2026-10-22)

`gen-notices.sh` gains Part 5, "Rust standard library" (ADR 0094). cargo-about sees only our own graph, so the
statically linked `std`, `core`, `alloc`, `compiler_builtins` and std's crates.io dependencies were acknowledged
nowhere, v0.1.0 included. `tools/license/rust_std.py` derives the crate list from the pinned toolchain's `rust-src`
and copies each crates.io crate's licence in the same accepted order as Part 1. The Rust project's own notice for
the in-tree crates, the toolchain's `COPYRIGHT-library.html`, is copied verbatim to
`tools/license/rust-std-COPYRIGHT.html`, a fourth `license-files` entry. The `deny` drift check covers both files,
so a toolchain bump now needs a regeneration as well as a `Cargo.lock` change.
