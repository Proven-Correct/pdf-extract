---
id: 0032
title: Bundle the missing device and lexer signals into one fork unit (H10)
date: 2026-10-01
status: Accepted
summary: Eight small hayro changes that unit 09 found missing are delivered together as H10a-h.
---

# 0032. Bundle the missing device and lexer signals into one fork unit (H10)

## Context

Decision D-37 (revision 6). Unit 09 found gaps that are each a few lines in hayro and unobservable from core: lexer
abandonment at a stray byte, a leaked `BDC`, per-`BDC` re-parse of a property list, invisible form/annotation
boundaries, uncounted font failures, misattributed decode events, no cancel.

## Decision

One bundled fork unit (09b, about 450 lines, exported as patches 0011-0019): (a) content-lexer resync with a
damage counter, (b) auto-close of unbalanced marked content at the end of each content stream, (c) a per-context cache
of named property lists, (d) `Device` hooks `begin_form`/`end_form`/`begin_annotation`/`end_annotation`, (e) a font
substitution counter, (f) a decode-limit hit counter, (g) a cooperative cancel flag on the budget, (h)
`Glyph::to_unicode()` (the ToUnicode lookup alone).

## Alternatives considered

Residuals or several separate units were the alternatives; not chosen. Recorded rationale: one patch keeps the fork diff
reviewable.

## Consequences

Patch 0019 keeps the inline-image `EI` end probe strict so image data is never resynchronised as content. Lexer resync is
limited to bytes that can start no token and malformed `#` name runs; a failed operand still ends the stream (retrying at
each following byte would be quadratic, and H2a pins that case). Signals use thread-locals sampled by core
(ADR 0019).
