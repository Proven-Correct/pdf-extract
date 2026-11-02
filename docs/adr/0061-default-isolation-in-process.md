---
id: 0061
title: Default isolation is InProcess; Process is opt-in
date: 2026-10-03
status: Accepted
summary: OcrEngine defaults to isolation=InProcess as design-spec section 7.6 chose; Process is opt-in.
---

# 0061. Default isolation is InProcess; Process is opt-in

## Context

Spec 0002 D-7. The default matches the design spec and PyMuPDF's exposure. The cost of Process (spawn, an 8.7 MB pipe copy per A4 page at 300 dpi) was unmeasured when decided; spec Q1 asks whether to flip the default once numbers exist.

## Decision

Default `isolation = InProcess`; `Process` is opt-in.

## Alternatives considered

- **Default to Process** - left to spec Q1 once measured.

## Consequences

An abort() inside Tesseract ends the process in the default mode (spec 0002 section 2.3 item 5). Unit 12 later measured process overhead at 1.6 % on clean pages at 300 dpi.

## Addendum (2026-10-22)

Partially superseded by ADR 0092, for the Python binding only. From Python,
`OcrEngine()`, `default_ocr_engine()` and `Page.ocr()` without an engine now
use process isolation. Rust's `OcrConfig::default()` and
`default_ocr_engine()` stay `InProcess`, as this record decided. The status
stays Accepted because the decision log has no partial-supersession status,
the same pattern as ADR 0035 and ADR 0086.
