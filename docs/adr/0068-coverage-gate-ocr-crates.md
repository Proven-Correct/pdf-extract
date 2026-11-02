---
id: 0068
title: Coverage gate covers core, render and ocr; the C++ shim is covered functionally
date: 2026-10-03
status: Accepted
summary: The 95 % line gate covers crates/{core,render,ocr}; tesseract-sys is reported and the shim is covered by tests and a symbol-reference test.
---

# 0068. Coverage gate covers core, render and ocr; the C++ shim is covered functionally

## Context

Spec 0002 D-13. cargo-llvm-cov instruments Rust only; the shim is about 200 lines whose every entry point unit 06 calls. Builds on ADR 0014.

## Decision

The 95 % line gate covers `crates/{core,render,ocr}`; `tesseract-sys` is reported; the C++ shim is covered functionally and by a symbol-reference test.

## Alternatives considered

No alternative recorded.

## Consequences

The shim has no line-coverage measurement.
