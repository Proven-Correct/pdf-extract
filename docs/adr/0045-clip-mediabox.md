---
id: 0045
title: Clip text to the MediaBox
date: 2026-10-01
status: Superseded
superseded_by: 0046
summary: The first sign-off clipped glyphs against the MediaBox, based on a probe in crop-relative coordinates.
---

# 0045. Clip text to the MediaBox

## Context

Compat point KP7 as first resolved. The first-pass probe used `page.insert_text` after `set_cropbox`, whose coordinates are
relative to the CropBox, and concluded that the CropBox does not clip.

## Decision

The clip box is the MediaBox, tested per glyph; clip paths filter text by bounding box.

## Alternatives considered

Not recorded. The design spec's term "visible box" (section 1.3) already meant the CropBox clipped to the MediaBox, and its compat point KP7 was left as only partly resolved.

## Consequences

Superseded by ADR 0046 after unit 03's generators contradicted it and the probe was redone with absolute coordinates.
