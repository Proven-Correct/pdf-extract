---
id: 0047
title: Line assembly uses four strict thresholds in ems of the incoming glyph
date: 2026-10-02
status: Accepted
summary: SPACE_GAP 0.149, GAP_BREAK 0.798, PERP_BREAK 0.809 and BACK_BREAK 0.795, each strictly inside its probed bracket.
---

# 0047. Line assembly uses four strict thresholds in ems of the incoming glyph

## Context

Decision D-9 with compat points KP1-KP3, revised twice on 2026-10-02 (revisions 7 and 7b). The first draft used one
`SPACE_GAP` of 0.15 em and one `LINE_BREAK` of 0.8 em. The recorded oracle data (Appendix A.1 of the run spec: sweeps at 11
and 22 pt, mixed sizes, superscripts, backward moves) does not fit either: under the strict `>` rule the space bracket is
[0.1477, 0.15), not "(0.148, 0.159]" (the 22 pt join at 3.25 pt is the floor; the 8 to 20 pt space at exactly 3 pt = 0.15 em
excludes 0.15), and a single 0.8 em sits on two data points with opposite outcomes (11 pt raised 8.8 pt joins, 8 to 20 pt
forward gap of 16 pt breaks) and flips with page position.

## Decision

Thresholds are in units of the incoming glyph's size (`em = c.size`, not `max(c.size, L.size)`), compare only with the
current line, and are all strict `>`: `SPACE_GAP` 0.149 in [0.1477, 0.15); `GAP_BREAK` 0.798 in [0.7955, 0.8000)
(forward along-line gap); `PERP_BREAK` 0.809 in [0.8000, 0.8182) (perpendicular offset); `BACK_BREAK` 0.795 in
[0.7727, 0.8182) (backward movement joins without a space up to it). Along and perpendicular tests are independent (not a
Euclidean distance); the direction-change test is 2 degrees; blocks use 1.5 em across, 2 em left of the block, direction
change and form or annotation boundaries. Values are `LayoutParams` fields.

## Alternatives considered

`max(c, L)` sizing, a single `LINE_BREAK`, and `SPACE_GAP` 0.15 were rejected because they do not reproduce the probe data.

## Consequences

Each default sits strictly inside its bracket, so no corpus-C row is on a boundary. The oracle itself compares with `>=` at
exactly 0.15 and 0.8 em and its f32 placement decides hair-under cases (a 3 pt gap at 20 pt is a space at x = 72 and a join at
x = 10); our strict `>` with in-bracket values reproduces every exactly-on-boundary row and is position-independent, so only
noise-level gaps (about 1e-6 em) can differ. The along and perpendicular brackets abut at 0.8 em, so a forward gap and a rise of
the same size around 0.8 em are classified oppositely by design. Anisotropic scaling puts one corpus-B case outside every
bracket (`CombTest`). Tuning, if any, stays inside the brackets and requires gates B and C to be re-run; no tuning was needed
on corpus B. `LayoutParams` is public in Rust, not Python-visible.
