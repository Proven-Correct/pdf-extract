---
id: 0049
title: ToUnicode entries that map to control characters follow the oracle
date: 2026-10-02
status: Accepted
summary: A single-code-point entry in U+0008-U+000D reads as a space, other single control targets are ignored, multi-code-point entries are verbatim.
---

# 0049. ToUnicode entries that map to control characters follow the oracle

## Context

Revision 10b (user sign-off of recommendation (b)), from a black-box probe over every C0 code point, U+007F and every C1 code
point for a Type 1 Helvetica, a non-embedded TrueType and a Type 0 Identity-H font, plus multi-code-point entries; results were
identical for all three types. Type 3 was probed later and matches.

## Decision

T26 step 1: a ToUnicode entry whose value is a single code point in U+0008-U+000D (BS, TAB, LF, VT, FF, CR) yields U+0020; one whose
single code point is any other control (U+0000-U+0007, U+000E-U+001F, U+007F, U+0080-U+009F) is ignored and the chain continues
at the glyph-name and CID steps; U+00A0 and U+2028 are honoured; multi-code-point entries are verbatim (`<0041 000A 0042>`
gives `A\nB`) and then sanitised (ADR 0050). In a composite font an ignored entry leaves the glyph unmapped (raw CID), as the
oracle does, not the Macintosh-order fallback of step 4b (a unit 14 correction); simple fonts fall to the glyph name.

## Alternatives considered

Keeping control characters from single-code-point entries (LF, TAB) was rejected in favour of parity: a single-code-point mapping
can no longer emit LF, TAB or any control.

## Consequences

User-visible parity improvement; the twelve `kp24_w_*` rows became exact. Unprobed: whether the oracle would use UCS2 or a reversed
cmap for an ignored entry in a font with an embedded program or known collection; an ignored entry at a KP15 position; multi-code-point
entries holding C1 or U+007F beyond the probed shapes. The oracle also ignores an entry mapping to U+FFFD (we take it as written).
