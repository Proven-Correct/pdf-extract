---
id: 0029
title: Apply the 50-level nesting cap to Type 3 glyphs, tiling patterns and soft masks
date: 2026-10-01
status: Accepted
summary: H3 adds the 50-level check where upstream stored a depth but never checked it; the text device never runs Type 3 procedures.
---

# 0029. Apply the 50-level nesting cap to Type 3 glyphs, tiling patterns and soft masks

## Context

Decision D-33 (revision 3, found in unit 06). Upstream stores `ctx.nesting_depth() + 1` in Type 3 glyphs, patterns and
soft masks but checks it only for forms, so a self-referential Type 3 font overflowed a 512 KiB stack under a device
that runs glyph procedures (as the renderer does).

## Decision

`Type3::render_glyph`, `TilingPattern::interpret` and `SoftMask::interpret` refuse at depth greater than
`MAX_NESTED_INTERPRETATION_DEPTH` (50), bump `nesting_cap_hits` and return, giving exactly 50 levels as for forms.
The text device never runs Type 3 procedures, so the text path was not exposed.

## Alternatives considered

No alternative is recorded.

## Consequences

The fix matters for the OCR run's renderer. A form that draws a Type 3 glyph or pattern that draws the form again is
stopped only by the nesting cap (the cycle stack lives in the `Context` and nested kinds start fresh contexts), so it can
repeat up to 50 times (residual).
