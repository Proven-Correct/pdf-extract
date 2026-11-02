---
id: 0004
title: Write our own text layer that reproduces PyMuPDF's text shape
date: 2026-09-25
status: Accepted
summary: A structured page, block, line, char model is filled by a hayro Device and a deterministic line assembler rebuilds PyMuPDF's get_text('text') shape.
---

# 0004. Write our own text layer that reproduces PyMuPDF's text shape

## Context

The downstream paragraph heuristics in the first consumer depend on the exact shape of PyMuPDF's `'text'` output (internal
survey section 2.5). hayro provides glyphs, fonts and positions through its `Device` trait but no text layer.
Source: design spec D3.

## Decision

Write our own text layer. A hayro `Device` (`TextDevice`) records characters in content order; a deterministic line
assembler (`LineAssembler`) groups them into lines and blocks and serialises the PyMuPDF `'text'` shape (every
line ends in `\n`, an empty page is `""`). The Unicode mapping chain is owned by core, not by hayro's
`as_unicode()`. Thresholds and their derivation: ADR 0047.

## Alternatives considered

The design spec records no alternative for this decision beyond the foundation choice (ADR 0001).

## Consequences

Parity with PyMuPDF is approximate and measured, not exact: the differences are listed as residuals (run spec
section 2.5, ARCHITECTURE.md). The `blocks()` model is `#[doc(hidden)]` in this run. Behaviour differences from the
oracle are either configurable back to parity or listed as deliberate differences (K2).
