---
id: 0021
title: Drop max_document_decoded_bytes from this run
date: 2026-10-01
status: Accepted
summary: No cumulative decoded-bytes budget exists in this run; only the per-stream cap does.
---

# 0021. Drop max_document_decoded_bytes from this run

## Context

Decision D-11 and open question Q5. hayro does not cache decoded streams.

## Decision

`max_document_decoded_bytes` is dropped. The Q5 default was signed off: drop it and revisit when image decoding (the
OCR run) needs a memory budget.

## Alternatives considered

A cumulative counter was rejected. Recorded rationale: it would measure work, not memory, and would reject large
legitimate documents; a live-bytes budget needs decode caching upstream first.

## Consequences

Theoretical peak memory per extraction is 50 nested forms times 256 MiB; in practice the operator budget ends such
pages first (run spec section 2.4).
