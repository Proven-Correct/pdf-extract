---
id: 0059
title: Bind only our C++ shim, not Tesseract's capi.h
date: 2026-10-03
status: Accepted
summary: The Rust side binds a small C++ shim that catches exceptions; capi.h is not used.
---

# 0059. Bind only our C++ shim, not Tesseract's capi.h

## Context

Spec 0002 D-5. capi.h has no try/catch, so a C++ exception would unwind into Rust, which is undefined behaviour.

## Decision

The Rust side binds only our C++ shim. It is the one place C++ exceptions are caught and converted to error codes. Tesseract's `capi.h` is not used.

## Alternatives considered

- **capi.h** - no try/catch.

## Consequences

The shim is a maintained C++ file (at most 300 lines, spec 0002 section 4.15) covered functionally, not by instrumentation (see the coverage record).
