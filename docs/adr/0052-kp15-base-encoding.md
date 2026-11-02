---
id: 0052
title: Normalise only the three base-encoding no-break positions
date: 2026-10-01
status: Accepted
summary: WinAnsi 0xA0/0xAD and MacRoman 0xCA on Type 1 and TrueType fonts without ToUnicode become space and hyphen; nothing else is normalised.
---

# 0052. Normalise only the three base-encoding no-break positions

## Context

Compat point KP15, re-probed in revision 6. The oracle normalises only these positions: `/Differences` names, Type 3 names and
ToUnicode keep U+00A0 and U+00AD; StandardEncoding 0xA0 is unmapped.

## Decision

Core normalises WinAnsi 0xA0/`nbspace` to U+0020, WinAnsi 0xAD/`sfthyphen` to U+002D and MacRoman 0xCA/`nbspace` to U+0020, from the
raw `glyph_name()`, for Type 1/TrueType fonts without ToUnicode.

## Alternatives considered

No alternative is recorded.

## Consequences

hayro does not say whether a name came from `/Differences`, so a `/Differences` entry restating 0xA0 as `nbspace` is read as a space
where the oracle keeps U+00A0 (divergence `kp15_differences_nbspace`).
