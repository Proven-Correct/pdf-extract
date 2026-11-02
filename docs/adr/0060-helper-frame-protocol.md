---
id: 0060
title: Process isolation uses a bespoke length-prefixed frame protocol over pipes
date: 2026-10-03
status: Accepted
summary: The helper protocol is length-prefixed frames over stdin/stdout with poll(2) deadlines and no IPC or serialisation crate.
---

# 0060. Process isolation uses a bespoke length-prefixed frame protocol over pipes

## Context

Spec 0002 D-6. OCR needs crash containment for ASSERT_HOST and abort() inside Tesseract (spec 0002 sections 1.5, 2.3).

## Decision

Length-prefixed frames over the child's stdin/stdout pipes, with `poll(2)` deadlines and no IPC or serialisation crate. One reviewable file is compiled into both sides.

## Alternatives considered

No alternative recorded beyond the rationale (no new dependency, no filesystem endpoint, identical semantics on Linux and macOS).

## Consequences

The protocol authenticates nothing; the helper contains crashes, not malice (spec 0002 section 2.2).
