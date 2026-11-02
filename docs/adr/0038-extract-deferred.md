---
id: 0038
title: Defer extract() to the OCR run
date: 2026-10-01
status: Accepted
summary: The batch extraction helper, Extraction and PageResult are not in this run.
---

# 0038. Defer extract() to the OCR run

## Context

Decision D-19. `extract()` encodes the OCR policy (extract text, OCR pages with too little).

## Decision

`extract()`, `Extraction` and `PageResult` (design spec 12.4) wait for the OCR run.

## Alternatives considered

A text-only version was rejected; recorded rationale: it would be a second API to deprecate.

## Consequences

Callers loop over `doc.pages()` and apply their own routing in the meantime.
