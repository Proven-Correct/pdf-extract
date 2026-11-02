---
id: 0053
title: Text in render modes 4-6 is emitted once
date: 2026-10-01
status: Accepted
summary: The oracle emits Tr 4-6 text twice; we emit it once.
---

# 0053. Text in render modes 4-6 is emitted once

## Context

Open question Q4 and compat point KP14, default signed off. The oracle emits text in modes 4, 5 and 6 twice (`Mode4\nMode4\n`).

## Decision

Emit once. hayro calls `draw_glyph` twice for modes 2 and 6 (`Fill` then `Stroke`); the device records a glyph once per show by
skipping a `Stroke` that immediately follows a `Fill` with the same glyph and transforms. A later identical glyph from a separate show
(fake bold) is kept (KP4).

## Alternatives considered

Matching the oracle's duplication was rejected: a real document with clip-mode headings would get doubled text.

## Consequences

Deliberate difference, pinned by the `kp13_tr4`, `kp13_tr5` and `kp13_tr6` divergence fixtures and confirmed on corpus B (`text_rendering_stroking_clipping`).
