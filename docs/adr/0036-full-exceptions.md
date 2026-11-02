---
id: 0036
title: Define the full exception hierarchy, OCR classes included, now
date: 2026-10-01
status: Accepted
summary: pdf_extract.errors carries the whole design-spec hierarchy so __all__ and the RuntimeError rule are stable from the first release.
---

# 0036. Define the full exception hierarchy, OCR classes included, now

## Context

Decision D-17. The OCR run adds OCR errors, but the `RuntimeError` tri-state contract (design spec D5) must hold from the start.

## Decision

The hierarchy of design spec 9.2 is defined in full in `python/pdf_extract/errors.py`. Only `OcrUnavailableError` derives
from `RuntimeError`; `DocumentNotFoundError` also derives from `FileNotFoundError`. Exceptions with attributes pickle.

## Alternatives considered

Defining only the classes this run raises was rejected; recorded rationale: stable `__all__`, and the `RuntimeError`
invariant test lands once.

## Consequences

Some exported classes cannot be raised by this run's code.
