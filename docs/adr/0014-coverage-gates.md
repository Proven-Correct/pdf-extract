---
id: 0014
title: Coverage gates: core and the Python package at 95% lines
date: 2026-10-01
status: Accepted
summary: pdf-extract-core must reach 95% line coverage and the pdf_extract package 95%; vendored crates are excluded.
---

# 0014. Coverage gates: core and the Python package at 95% lines

## Context

Decision D-23. The standing project goal is test coverage as close to 100% as practical.

## Decision

`cargo llvm-cov` gates `crates/core` at 95% lines (the `coverage` CI job, report filtered to core's own sources);
`pytest --cov=pdf_extract --cov-fail-under=95` gates the Python package. Vendored crates are excluded. Coverage of
`crates/python` and the testkit is reported only.

## Alternatives considered

Not recorded beyond the rationale: as close to 100% as practical, and vendored upstream code is tested by its own
suite.

## Consequences

Vendored fork changes are covered by tests in `crates/core/tests/` rather than by vendored-crate coverage.
