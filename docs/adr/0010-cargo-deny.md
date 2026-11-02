---
id: 0010
title: Enforce dependency licenses with cargo-deny from the start
date: 2026-10-01
status: Accepted
summary: cargo-deny checks licenses, advisories, bans and sources on every push, with a permissive-only allow-list.
---

# 0010. Enforce dependency licenses with cargo-deny from the start

## Context

Decision D-2 of the run spec; requirement L1 (permissive-only shipped dependency graph). Original wording: enforcement
from unit 01. Revised on 2026-10-02 (revision 9): the allow-list is unchanged and `[licenses.private] ignore = true`
covers our own unpublished crates (ADR 0009).

## Decision

Run `cargo deny check licenses advisories bans sources` in the `deny` CI job from the first unit. The license
allow-list is permissive-only. cargo-deny 0.18 and later has no `copyleft` or `unlicensed` keys: the allow-list
rejects GPL, LGPL and AGPL dependencies by construction.

## Alternatives considered

Recorded rationale: it is cheap, and it guards every later dependency (L1). No alternative is recorded.

## Consequences

cargo-deny cannot see assets that are not crates; the full `THIRD_PARTY_NOTICES` file (Phase 3) must also cover
hayro-interpret's bundled Foxit `.pfb` fonts (BSD-style) and CC0 ICC profile, and the Tinos test asset. The wheel
ships a stopgap `THIRD_PARTY_NOTICES` with the Adobe Glyph List notice and `README-AFM.txt`. Advisory exceptions
are not accepted (ADR 0011).

## Addendum (2026-10-02)

The Phase 3 notices file in Consequences landed early: `THIRD_PARTY_NOTICES` is now generated from the
extension's dependency graph and the embedded hayro assets, and CI fails on drift (ADR 0054). It covers the
Foxit fonts, the ICC profiles of hayro-interpret and hayro-jpeg2000, and hayro-cmap's CMaps. The Tinos font is
a test asset only and is not in the wheel.

## Addendum (2026-10-03)

cargo-deny sees only crates. The vendored C/C++ trees (Tesseract, Apache-2.0; Leptonica, BSD-2-Clause) are checked
by `tools/license/check-c-licenses.sh`, run in the `deny` CI job, which fails on a copyright holder outside its
allow list or any licence keyword other than Apache-2.0 / BSD in the two trees (spec 0002 section 3.7).
