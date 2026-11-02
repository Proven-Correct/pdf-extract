---
id: 0013
title: Performance numbers on shared CI are report-only
date: 2026-10-01
status: Accepted
summary: Benchmarks annotate but never gate; correctness assertions (no pages built at open, bounded node resolutions, allocation bound) gate P1.
---

# 0013. Performance numbers on shared CI are report-only

## Context

Decision D-22. Shared CI runners are too noisy to gate on raw timing.

## Decision

Benchmark steps run with `continue-on-error` and only report; the `bench` job uploads `bench.json` and annotates the
comparison with `crates/core/benches/baseline.json`. The gating checks for P1 are correctness assertions: hayro's
`Pdf::new` builds no pages (`Pdf::pages_built()`), retained memory is bounded per xref entry, and the page walk
resolves at most leaves plus interior nodes (counted with a counting `XRef` wrapper in tests).

## Alternatives considered

Gating on timings was rejected because shared runners are too noisy (recorded as the rationale; no further
alternative recorded).

## Consequences

The committed baseline was recorded on a local arm64 Mac (`crates/core/benches/baseline.json`: 100k pages open 32.6 ms / 22 MB; 1M pages 450 ms / about
127 MB over the file; `max_pages=10` on 1M 47.7 ms; the spec's 32.5 / 443 / 47.5 ms were an earlier local run) and is to be replaced by a CI `bench.json` artefact. CI runs on
both `push` and `pull_request`, so branch pushes run twice.

## Addendum (2026-10-14)

The final sentence of Consequences no longer holds: since ADR 0085, CI runs
automatically only on pushes to `main`; pull requests run on demand.
