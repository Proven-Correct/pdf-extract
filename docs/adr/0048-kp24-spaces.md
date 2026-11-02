---
id: 0048
title: Synthetic-space suppression follows the oracle's two code point sets
date: 2026-10-02
status: Accepted
summary: A synthetic space is suppressed only by an incoming U+0020 or a line ending in U+0020, U+1680, U+3000 or U+FEFF.
---

# 0048. Synthetic-space suppression follows the oracle's two code point sets

## Context

Compat point KP24, user sign-off of recommendation (b) on 2026-10-02 (revision 10). The oracle was probed over the whole Unicode
whitespace set, each character before and after a 0.3 em gap. Option (a) was `char::is_whitespace` on both sides.

## Decision

T30: the incoming glyph suppresses the synthetic U+0020 only when it is U+0020; the line's last character suppresses it when it is
U+0020, U+1680, U+3000 or U+FEFF. Nothing else counts: TAB, LF, VT, FF, CR, U+0085, U+00A0, U+2000-U+200A, U+2028, U+2029, U+202F
and U+205F suppress nothing on either side.

## Alternatives considered

Option (a), `char::is_whitespace`, was rejected: we would emit no space next to the whitespace characters the oracle does not treat as
suppressing, where it inserts one, making eleven KP24 fixtures divergences instead of exact rows.

## Consequences

The assembler deliberately does not use `char::is_whitespace`. Leading and trailing whitespace glyphs are kept verbatim (KP5), and
a space glyph before a gap is not doubled. Pinned by the `kp24_*` fixtures, all exact.
