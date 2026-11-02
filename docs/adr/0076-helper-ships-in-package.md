---
id: 0076
title: The helper binary ships inside the Python package directory
date: 2026-10-03
status: Accepted
summary: A script stages the helper into python/pdf_extract/ and maturin includes it.
---

# 0076. The helper binary ships inside the Python package directory

## Context

Spec 0002 D-21. maturin has no second-artefact mechanism for mixed layouts.

## Decision

The helper binary ships inside the Python package directory, staged by a script into `python/pdf_extract/` and included by maturin.

## Alternatives considered

- **A second maturin artefact** - not available for mixed layouts.

## Consequences

The package directory is the one location the binding can find without configuration. The helper must be staged before `maturin develop`.
