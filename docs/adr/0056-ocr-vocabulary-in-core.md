---
id: 0056
title: OCR vocabulary lives in core; engine and Tesseract live in pdf-extract-ocr
date: 2026-10-03
status: Accepted
summary: The OCR error and result types and extract() are in core; the engine, pool and Tesseract are in pdf-extract-ocr, and Page.ocr is an extension trait.
---

# 0056. OCR vocabulary lives in core; engine and Tesseract live in pdf-extract-ocr

## Context

Spec 0002 D-2. Core is forbid(unsafe_code) and cargo test -p pdf-extract-core must not compile Tesseract. The Python surface must not change.

## Decision

`OcrError`, `TextSource::Ocr`, `PageRecognizer` and `extract` live in core. The engine, pool and Tesseract live in `pdf-extract-ocr`. `Page::ocr` is the extension trait `PageOcr` rather than an inherent method, a deviation from design-spec section 11.

## Alternatives considered

- **Inherent `Page::ocr`** (design-spec section 11) - would put the engine, and so the C++ build, behind core.

## Consequences

Rust callers must import `PageOcr` to call `ocr`; the Python surface is unchanged.
