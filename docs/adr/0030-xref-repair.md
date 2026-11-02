---
id: 0030
title: Make hayro's lazy xref repair thread-safe and idempotent in the fork (H9)
date: 2026-10-01
status: Accepted
summary: Repair takes blocking locks, scans outside the lock and re-checks under the write lock instead of core forcing an eager repair.
---

# 0030. Make hayro's lazy xref repair thread-safe and idempotent in the fork (H9)

## Context

Decision D-34 (revision 5, blocking finding for requirement C2). Upstream `XRef::get_with` did `try_read().unwrap()`,
`repair()` did `try_write().unwrap()` plus `assert!(!repaired)`, and the whole-file scan ran under the write lock, so
concurrent lookups during a repair panicked. Any later object lookup can trigger a repair.

## Decision

H9: `get_with` takes a blocking `read()`; `repair()` runs the fallback scan on the immutable bytes outside any lock, then
takes a blocking `write()`, re-checks `repaired`, swaps the map and sets the flag. As built, `get_with` reads the
`repaired` flag under the same read guard as the entry and retries through `repair()` on a stale entry (it drops the guard
first: holding it across `repair()` would deadlock).

## Alternatives considered

**Forcing an eager repair in core** was rejected: core cannot prove a repair never happens after open, so the fix belongs
where the lock is.

## Consequences

Concurrent lookups block only for the swap. When several threads hit a broken entry at once, each may run its own full-file
scan before the first swap lands (13-16 scans per round at 16 threads in the unit 08b test), so peak memory is N times one
scan; a scan-once guard is a follow-on. `Document::warnings()` reports one `XrefRepaired` from the moment any operation
observes the repair, while the page's `TextPage` never carries it. Tests: `h9_xref_repair.rs`, `h9_pre_fix.rs` (rebuilds
pre-H9 hayro by reverse-applying patch 0010 to show it aborts).
