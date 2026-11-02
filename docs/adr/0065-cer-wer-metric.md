---
id: 0065
title: CER and WER are NFC, whitespace-collapsed Levenshtein means; gates per O34
date: 2026-10-03
status: Accepted
summary: Error rates are Levenshtein over NFC, whitespace-collapsed, case-preserving text, aggregated as corpus means; latency and RSS are report-only.
---

# 0065. CER and WER are NFC, whitespace-collapsed Levenshtein means; gates per O34

## Context

Spec 0002 D-10. A single stated normalisation makes the oracle and our output comparable. ADR 0013 forbids timing gates on shared runners.

## Decision

CER/WER are Levenshtein over NFC, whitespace-collapsed, case-preserving text (code points / tokens), aggregated as corpus means; gates per O34; latency and RSS report-only. NFC is `unicode-normalization` (MIT OR Apache-2.0), a dev-dependency of `pdf-extract-ocr` only.

## Alternatives considered

- **ASCII-only ground truth** - OCR output is not ASCII, and the private-scans metric compares two OCR outputs, so it would not remove the need for NFC.

## Consequences

Means over a 50-page corpus are stable enough to gate at a 1-point margin.
