---
id: 0051
title: The glyph-name table is Adobe's single-code-point list only
date: 2026-10-01
status: Accepted
summary: No TeX glyph names; core owns every name mapping through raw glyph_name() and Glyph::to_unicode().
---

# 0051. The glyph-name table is Adobe's single-code-point list only

## Context

Decision D-39, user decision in revision 6. About 255 TeX glyph names had been added to `crates/core/src/text/agl.rs`; their
source list is the GPL-2 lcdf-typetools, and requirement L1 bans copyleft-derived data.

## Decision

`agl.rs` holds Adobe AGL single-code-point entries only (BSD-3 notice retained, to be reproduced in binary distributions). Core's
chain bypasses hayro's own name table for simple fonts without ToUnicode, reading the raw `glyph_name()`; with a ToUnicode map
present, H10h's `Glyph::to_unicode()` consults the map alone so hayro's table never decides.

## Alternatives considered

Keeping the TeX names was rejected on provenance. Recorded rationale: the oracle leaves most TeX names raw anyway.

## Consequences

TeX names such as `parenleftbig` and `summationtext` stay unmapped (raw code) for fonts without ToUnicode, a parity residual; the
oracle maps some of them, notably `square` to U+25A1.
