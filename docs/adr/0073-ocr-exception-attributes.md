---
id: 0073
title: OcrPageError gains limit_name and value; OcrTimeoutError gains timeout
date: 2026-10-03
status: Accepted
summary: The Python OCR exception classes carry the data their Rust variants carry.
---

# 0073. OcrPageError gains limit_name and value; OcrTimeoutError gains timeout

## Context

Spec 0002 D-18. Dropping it at the boundary would force message parsing.

## Decision

`OcrPageError` gains `limit_name`/`value`; `OcrTimeoutError` gains `timeout`; constructors and `__reduce__` follow the existing attribute-class pattern.

## Alternatives considered

No alternative recorded beyond message parsing.

## Consequences

Extends ADR 0036's exception hierarchy.
