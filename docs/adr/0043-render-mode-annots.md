---
id: 0043
title: Emit render-mode 7 text and filter annotations by subtype and NoView (H8)
date: 2026-10-01
status: Accepted
summary: Clip-mode text reaches the device as an invisible glyph; annotations that are NoView, Popup or Link are skipped.
---

# 0043. Emit render-mode 7 text and filter annotations by subtype and NoView (H8)

## Context

Decision D-4 (revised 2026-10-01: the earlier `ShowAll` optional-content switch, H8a, was dropped; ADR 0044). The oracle
includes Tr 7 text and excludes Popup and Link annotations and those flagged NoView. hayro skipped only Hidden annotations
and never delivered Tr 7 glyphs to a device.

## Decision

`show_glyph` emits `draw_glyph(..., GlyphDrawMode::Invisible)` for text render mode 7 after collecting the clip (the
renderer already ignores `Invisible`); `interpret_page` skips annotations with flag bit 6 (NoView, value 32) and subtypes
`Popup` and `Link`. Annotation `/AP /N` streams of the remaining annotations follow page content in output order.

## Alternatives considered

No alternative is recorded.

## Consequences

The oracle synthesises appearances hayro does not (FreeText without `/AP`, check boxes with a state dictionary, signature
fields), so widgets are a residual. Tr 4-6 handling is ADR 0053.
