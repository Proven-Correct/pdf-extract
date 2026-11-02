---
id: 0040
title: Document state is an RwLock<Option<Arc<Loaded>>>; operations clone and release
date: 2026-09-25
status: Accepted
summary: Each operation clones the Arc under a read lock and works lock-free; close() drops the document's own reference.
---

# 0040. Document state is an RwLock<Option<Arc<Loaded>>>; operations clone and release

## Context

Design spec 8.2 and requirements T39, T40 (C4). One `Document` is shared between threads, and a long extraction must neither
block `close()` nor be blocked by it.

## Decision

`DocShared { state: RwLock<Option<Arc<Loaded>>>, page_count }`. An operation takes the read lock, clones the `Arc<Loaded>`
and releases the lock at once, then works on its clone. `close()` takes the write lock and sets the state to `None`;
in-flight operations finish, memory is freed when the last clone drops, a second `close()` is harmless, later operations
return `PageError::Closed`, and `page_count` stays readable. A hayro `InterpreterCache` is created per operation and never
shared (it is `!Send`).

## Alternatives considered

Not recorded in the spec beyond the design spec's note that a `loom` model-checking test was planned (later run).

## Consequences

Never hold the lock across parsing or interpretation. The Python test kit holds its builder in a `Mutex` taken only inside
`py.detach`, so no Python code runs under it. Python tests run 500 rounds of close-during-use.

## Addendum (2026-10-19)

The planned `loom` model checking landed (change harden-fuzzing-sanitizers-loom, design D5). The protocol itself is
unchanged; its code moved out of `DocShared` into `CloseCell<T>` (`crates/core/src/close_cell.rs`: `new`, `get`, `close`,
`is_closed`), which `DocShared` holds as `state: CloseCell<Loaded>`. `close()` returns the removed `Arc` so that
`Document::close` drops it after the write lock is released, as before. The extraction exists so that the model checker
runs the shipped code rather than a replica.

`CloseCell` takes `Arc` and `RwLock` from a private `sync` shim (`crates/core/src/sync.rs`): `std::sync` normally, loom's
types under `RUSTFLAGS="--cfg loom"`, a build in which `CloseCell` is also re-exported (`#[doc(hidden)]`) for
`crates/core/tests/loom_close.rs`. That test instantiates `CloseCell<Counter>`, so no hayro type is built under loom,
and checks three scenarios in every interleaving within `LOOM_MAX_PREEMPTIONS=3`: an operation racing `close()`, two
in-flight operations releasing the value after the close, and a double close around an operation. Each asserts the value
is freed exactly once and only after every close and operation is done with it. The `Pages` cursor and the lazy-repair
`AtomicBool` stay `std`: single atomic operations with no protocol to check.

The same pattern covers the OCR instance pool (`crates/ocr/src/sync.rs`, `crates/ocr/tests/loom_pool.rs`; the pool takes
its pid source as a `fn() -> u32` field so a test can stage a fork). Its first run found a lost wake-up: a fork reset
performed by a returning pre-fork slot freed every slot without notifying the condvar, leaving a caller already waiting on
the full pool asleep for good. The reset now calls `notify_all`. Both loom tests run in the `loom` job of `ci.yml`.
