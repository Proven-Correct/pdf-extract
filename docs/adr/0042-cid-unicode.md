---
id: 0042
title: Add Unicode fallbacks for CID fonts in the fork (H6)
date: 2026-10-01
status: Accepted
summary: CID fonts without ToUnicode map through the ordering's UCS2 CMap, or through the reversed cmap of an embedded program.
---

# 0042. Add Unicode fallbacks for CID fonts in the fork (H6)

## Context

Decision D-6. hayro's `as_unicode()` for CID fonts uses only ToUnicode and calls itself "somewhat experimental"; the
ordering's UCS2 CMap was loaded only for non-embedded (fallback) fonts.

## Decision

H6a loads the ordering's `*-UCS2` CMap (Adobe-GB1/CNS1/Japan1/Korea1) whenever there is no ToUnicode, embedded or not.
H6b, for an embedded OpenType/TrueType program with no map, reverses the font `cmap` once into a
`OnceLock<HashMap<GlyphId, char>>` and looks up the glyph for the CID; it is never used for a substituted fallback font
(whose glyph order is unrelated to the CIDs).

## Alternatives considered

Recorded rationale: small, localised in `cid.rs`, and it shrinks the CJK residual before corpus B. The design spec noted
the layer could do it instead if H5 landed.

## Consequences

The reverse-cmap build (up to about 1.1M entries) happens once per font per context inside one operator and is not
budgeted. skrifa's `charmap()` ignores (3,0) symbolic and Macintosh cmaps, so symbol-encoded or Mac-only embedded CID
fonts stay unmapped (residual). CJK text where the UCS2 CMap and MuPDF's mapping disagree remains a residual.
