---
id: 0050
title: Unmapped glyphs emit their raw code; all emitted text is sanitised
date: 2026-10-01
status: Accepted
summary: Unmapped glyphs emit the character code as a code point, and every emitted string has NUL, C0 controls other than TAB/LF/CR, U+FFFE/U+FFFF and surrogates replaced by U+FFFD.
---

# 0050. Unmapped glyphs emit their raw code; all emitted text is sanitised

## Context

Decision D-10 and compat point KP6 (revision 6 extended sanitising from the unmapped path to every source). The oracle emits the raw
code for unmapped glyphs (Type 3 code 65 with name `/g65` gives `A`; code 200 gives `È`; code 0 gives NUL) from any source.
PostgreSQL `text` rejects NUL regardless of where it came from.

## Decision

`Unmapped::Cid` (compat default) emits the raw code. All emitted text (unmapped codes, ToUnicode, glyph names, ActualText) has NUL,
C0 controls other than TAB/LF/CR, U+FFFE, U+FFFF and unpaired surrogates replaced by U+FFFD; C1 controls are kept; LF, CR and TAB
from any source are kept because the oracle emits them. Sanitising runs after ADR 0049's rules. `Replacement` and
`Skip` exist as the other `Unmapped` policies.

## Alternatives considered

Sanitising only the unmapped path was the first form; extended because PostgreSQL rejects NUL whatever its source. Emitting NUL
verbatim, as the oracle does, was rejected.

## Consequences

Deliberate difference from the oracle for NUL and C0 (pinned by KP6 divergence fixtures for codes 0 and 1). The TeX-name residual is
ADR 0051. `square` stays raw where the oracle gives U+25A1 (residual, `kp06_type3_square`).
