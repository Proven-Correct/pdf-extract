---
id: 0027
title: Bound interpretation with a cooperative budget; the deadline, not the operator cap, bounds time
date: 2026-10-01
status: Accepted
summary: hayro gets an operator and deadline budget (H3) checked every 4,096 operators; page_time_budget is the wall-clock guarantee.
---

# 0027. Bound interpretation with a cooperative budget; the deadline, not the operator cap, bounds time

## Context

Design spec H3 and decision D-32 (revision 3). hayro had no cancellation or work-budget hook. Measurement showed hayro
interprets about 0.9M operators/s in release and 60k/s in debug on an M3 Max, so 10M operators take about 11 s release
and about 165 s debug.

## Decision

H3 adds `InterpretBudget { max_ops, deadline, check_interval }` to `InterpreterSettings` (always present; default
unlimited) with a shared `Arc<BudgetState>` so nested contexts (Type 3 glyphs, patterns, soft masks) share one state.
`interpret()` ticks per operator; every `check_interval` (4,096) operators it checks the deadline; on exhaustion it sets
a flag and stops. Defaults: `max_ops_per_page` 10,000,000, `page_time_budget` 30 s. The wall-clock guarantee is the
deadline; tests assert op-cap exhaustion functionally with small caps and bound time only through
`page_time_budget`.

## Alternatives considered

The earlier claim "fan-out fails within 5 s on CI" was withdrawn. Recorded rationale: any seconds-per-operator claim is
platform- and profile-dependent.

## Consequences

A page can overrun its deadline by the time of 4,096 operators plus one non-interruptible operator (a 256 MiB stream
decode, a large font parse). The vendored and font crates build with `opt-level = 2` in the dev profile (core at 1) so
debug-build tests are usable. Mapping: `Ops` becomes `LimitKind::OpsPerPage`, `Deadline` becomes `PageTimeBudget`.

## Addendum (2026-10-19)

"One non-interruptible operator" is wider than a large decode: a decoder loop that never terminates is a non-interruptible operator that never returns, and the deadline cannot stop it (internal #94). ADR 0088 records the forward-progress guards on the two JBIG2 symbol-dictionary loops that did this, and ADR 0079's addendum of the same date records the JBIG2 instance-placement work allowance. Outside them, CPU time inside one codec call is still bounded only by the codec's own termination and the memory budget's charges, not by `page_time_budget`.
