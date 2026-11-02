---
id: 0037
title: No render crate stub in this run
date: 2026-10-01
status: Superseded
superseded_by: 0055
summary: The render crate is not created until the OCR run.
---

# 0037. No render crate stub in this run

## Context

Decision D-18.

## Decision

No `pdf-extract-render` crate stub is added. `hayro` 0.7.1 (the rasteriser) is vendored when the render crate arrives.

## Alternatives considered

An empty stub was rejected. Recorded rationale: nothing to verify, and one more matrix cell (a workspace member, a CI
cell and a `[patch]` entry for `hayro`).

## Consequences

The OCR run starts the render crate from nothing.
