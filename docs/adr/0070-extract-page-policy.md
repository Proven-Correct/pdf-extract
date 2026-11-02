---
id: 0070
title: extract() policy: sparse rule, page-order OCR selection, status precedence
date: 2026-10-03
status: Accepted
summary: extract() treats a page as sparse below min_chars after Unicode trim, picks OCR candidates in page order, and ranks BudgetExhausted above Partial.
---

# 0070. extract() policy: sparse rule, page-order OCR selection, status precedence

## Context

Spec 0002 D-15. It mirrors `len(text.strip()) < 50` in the caller and must be deterministic under parallelism (C6).

## Decision

Sparse = Unicode-trimmed code-point count `< min_chars`; OCR candidates are chosen in page order; `BudgetExhausted` outranks `Partial`; a text-pass limit yields `""` plus a document warning.

## Alternatives considered

No alternative recorded.

## Consequences

The caller sees every page row. Unit 10 review wrote back: status precedence Unavailable > BudgetExhausted > Partial > Ok; a limit-hit page is not OCR'd; with workers > 1 which in-flight pages end as Ocr after Unavailable is timing-dependent.

## Addendum (2026-10-06)

Spec 0003 D-5: `Extraction.warnings` is document warnings, then page-limit warnings, then per-page OCR notes in page order, each `Warning` carrying a detail of the form `page N: ocr: <text>` (`N` 0-based). Today these are the noise pre-check notes (ADR 0081); without them an `extract()` caller cannot tell a skipped page from a blank one.
