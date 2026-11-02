---
id: 0019
title: Object-depth hits are a fixed 256 constant reported through a thread-local counter
date: 2026-10-01
status: Accepted
summary: The nesting cap is a constant in the fork and is reported by a thread-local hit counter that core samples around each synchronous operation.
---

# 0019. Object-depth hits are a fixed 256 constant reported through a thread-local counter

## Context

Decision D-12, amended in revision 2. An over-deep read returning `None` is indistinguishable from any parse failure,
and content operands are read through a bare `Reader` with no context at all, so neither a per-document counter nor a
`Limits` field can see them.

## Decision

The cap is a fixed constant (256) in the fork, not a `Limits` field in this run. It reports through a thread-local
hit counter in hayro-syntax (`object_depth_limit_hits()`, `std` only) that core samples before and after each
synchronous operation; hayro-postscript hits are not counted (it is `no_std`; a failed CMap surfaces as a font
without ToUnicode). The later fork signals (content damage, decode-limit hits, font substitutions) reuse the pattern.

## Alternatives considered

A process-wide knob was rejected because it would let documents interfere; a per-document counter or a `Limits` field
cannot see content operands. The thread-local costs nothing on the hot path.

## Consequences

Everything between the two samples must stay synchronous on the calling thread, and core never crosses threads inside
an operation (`open.rs`, `text/extract.rs`). Concurrent operations on one document never charge each other's
warnings. `Pdf::decode_stats()` is a document-wide total for diagnostics only. Making the cap configurable is deferred.
