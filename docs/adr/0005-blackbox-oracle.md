---
id: 0005
title: Measure parity black-box against PyMuPDF 1.28.2; no MuPDF code is read
date: 2026-09-25
status: Accepted
summary: PyMuPDF 1.28.2 is used only as a black box in a separate dev-only project; no MuPDF or PyMuPDF source is read, copied or translated.
---

# 0005. Measure parity black-box against PyMuPDF 1.28.2; no MuPDF code is read

## Context

The reason for replacing PyMuPDF is its license; the replacement must not bring AGPL code back in (design spec D9).
Requirement L2 forbids reading, copying or translating MuPDF or PyMuPDF code. Parity still has to be established
against PyMuPDF's observable behaviour.

## Decision

Behaviour is matched only by black-box observation of PyMuPDF 1.28.2 (MuPDF 1.28.2). The oracle lives in
`tools/oracle/`, a separate uv project that pins `pymupdf==1.28.2`; it is used through `open`, `new_page`,
`insert_text`, `get_text('text')`, raw PDF bytes and `save(encryption=...)`, never imports our package, and is never
a dependency of the library, its tests or CI. The oracle version has not changed during the run, so there is no
earlier version decision to supersede.

## Alternatives considered

No alternative is recorded in the spec; the license motive excludes reading the source.

## Consequences

Where black-box probing cannot determine a rule the difference is documented rather than guessed (for example the
`/ActualText` distribution rule, ADR 0041). Probe methodology matters: an unsaved in-memory PyMuPDF document
behaves differently from the same document saved and reopened for optional content (ADR 0044), and the oracle's own
f32 glyph placement decides hair-under threshold cases (ADR 0047). Oracle outputs are committed as
expectations (ADR 0006).
