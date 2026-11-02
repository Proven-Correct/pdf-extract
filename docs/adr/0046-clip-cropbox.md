---
id: 0046
title: The visible box is CropBox intersected with MediaBox, tested per glyph
date: 2026-10-01
status: Accepted
supersedes: 0045
summary: Glyphs entirely outside CropBox ∩ MediaBox are dropped; a glyph partly inside is kept.
---

# 0046. The visible box is CropBox intersected with MediaBox, tested per glyph

## Context

Supersedes ADR 0045 (decision D-8, revision 1, KP7). Probing with text placed in absolute PDF coordinates: with
CropBox `[100 100 400 700]` on a 595x842 page, text below, right of and above the crop was dropped and text inside or
straddling the edge was kept; with CropBox `[100 -100 400 1000]` (beyond the MediaBox) y=50 was kept and y=850 dropped. So
the clip is CropBox ∩ MediaBox, which matches the design spec's definition of the visible box (section 1.3). Inherited MediaBox is honoured.

## Decision

The clip box is the visible box CropBox ∩ MediaBox (hayro `Page::intersected_crop_box()`), mapped through
`initial_transform(false)` like the glyphs. The test is per glyph, not per line (`x590` at x=590 yields `x`). The glyph box
is `[origin, origin + advance]` by `[descent, ascent]` from the font (FontDescriptor, Type 3 `FontBBox x FontMatrix`,
standard-14 AFM; otherwise 0.72 and -0.22 em; a descriptor with positive descent or non-positive ascent is rejected as a
whole). Active clip paths shrink the box to their bounding box (KP19); non-rectangular clips are approximated by it, as the
oracle does.

## Alternatives considered

The MediaBox-only rule (ADR 0045) failed unit 03's `builder_crop_rotate`, which gives `''` for text outside the crop.

## Consequences

The oracle keeps an 11 pt glyph whose baseline is 3 pt below the page edge and drops one 8 pt below, bracketing Helvetica's
ascent at (0.27, 0.73] em (AFM says 0.718). Glyphs within about 0.1 em of an edge may be classified differently because
MuPDF derives ascent and descent from its substitute fonts (residual).
