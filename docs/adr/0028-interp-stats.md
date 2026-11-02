---
id: 0028
title: Interpreter events are counted in the shared budget state and read through Context::stats()
date: 2026-10-01
status: Accepted
summary: Nesting-cap and form-cycle events are counted in always-allocated shared state and exposed as InterpretStats.
---

# 0028. Interpreter events are counted in the shared budget state and read through Context::stats()

## Context

Decision D-29 (revision 2). hayro's `warn!` macro expands to nothing without the `logging` feature, so events were
invisible.

## Decision

`BudgetState` counts nesting-cap hits and form-cycle hits next to the operator count; `Context::stats()` returns
`InterpretStats { ops, exhausted, nesting_cap_hits, form_cycle_hits }`; core maps the counters to warnings.

## Alternatives considered

Recorded rationale only: the budget state already crosses nested contexts and is always allocated, so it is the one
place every interpreter event can report to.

## Consequences

Signals that cannot live in the shared state (font substitutions, lexer damage, decode-limit hits) use thread-locals
(ADR 0019, ADR 0032). `exhausted` is `Option<BudgetExhaustion { Ops, Deadline, Cancelled }>`.
