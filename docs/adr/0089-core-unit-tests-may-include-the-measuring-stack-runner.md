---
id: 0089
title: Core's unit-test build denies rather than forbids unsafe, so its tests can use the measuring small-stack runner
date: 2026-10-21
status: Accepted
summary: pdf-extract-core stays forbid(unsafe_code) in every build except its own unit tests, where it is deny(unsafe_code) and one explicitly allowed test module includes the pthread-measuring small-stack runner (tests/common/stack.rs); this amends spec 0001 §4.16.
---

# 0089. Core's unit-test build denies rather than forbids unsafe, so its tests can use the measuring small-stack runner

## Context

Spec 0001 §4.16 makes `pdf-extract-core` `#![forbid(unsafe_code)]`, and specs 0002 §4.15 and 0003 §4.7 restate the rule. ADR 0056 relies on it. `forbid` cannot be overridden by an inner `allow`, so the rule also covers the crate's unit tests.

The "fits in 512 KiB" tests run their deep inputs on a thread that asks for a 512 KiB stack. On Linux that request is only a request. glibc serves a new thread from its cache of exited threads' stacks and accepts a block up to four times larger, so after a 2 MiB thread exits, a 512 KiB request can get 2 MiB (internal issue #70). The integration tests guard against this with `crates/core/tests/common/stack.rs`. That runner reads each thread's real stack size through pthread FFI (`pthread_getattr_np`, `pthread_get_stacksize_np`) and parks threads that are too big. The FFI is `unsafe`. Core's three unit-test sites in `src/text/tests/limits.rs` therefore used an unmeasured `std::thread::Builder` and proved nothing whenever glibc reused a bigger stack (internal issue #118).

## Decision

In `crates/core/src/lib.rs`, `#![forbid(unsafe_code)]` becomes `#![cfg_attr(not(test), forbid(unsafe_code))]` and `#![cfg_attr(test, deny(unsafe_code))]`. The library still forbids `unsafe` as built for other crates, for doctests and for the integration tests, because none of those builds sets `cfg(test)` on it. In the unit-test build, `src/text/tests/mod.rs` includes the integration tests' runner with `#[allow(unsafe_code)] #[path = "../../../tests/common/stack.rs"] mod stack;`, and the limits tests use its `on_small_stack`. Every other module in that build is still denied `unsafe`. The `allow` is the single, visible exception.

This amends spec 0001 §4.16 and its restatements. The rule for the shipped library is unchanged; the exception is only for the crate's own test code.

## Alternatives considered

- **A dev-only crate that allows `unsafe`** (for example `crates/stack-runner`, `publish = false`, holding the runner as a dev-dependency of core and render) - keeps the spec rule literal. It costs a seventh workspace member: the member count in the licence test changes, the crate needs a row in the ARCHITECTURE crate table, the sanitizer skip names change, the coverage gate counts the crate, and Cargo.lock changes. All of that is for code that never ships.
- **A safe measurement on Linux through `/proc/self/maps`** (find the mapping that holds a local's address) - needs no `unsafe`. It would be a second measuring method that differs from the integration tests' runner. The guard-page accounting is fiddly, and it does not work on macOS.
- **Keep the unmeasured threads and document the gap** - the status quo. The 512 KiB claims of the unit tests then hold only when glibc happens not to reuse a cached stack.

## Consequences

- The unit tests and the integration tests now share one runner, so a deep input in either is checked on a stack measured at 512 KiB. The unit test `text::tests::the_unit_tests_small_stack_is_measured` pins this, and the sanitizer run skips it by name, as it does the tests in `stack_size.rs`.
- "Core is `forbid(unsafe_code)`" now means "core's library code is". A reviewer must not read the `cfg(test)` relaxation as permission for `unsafe` in non-test modules. `deny` still rejects such code in the unit-test build, and the non-test build still forbids it.
- Any further `#[allow(unsafe_code)]` under `cfg(test)` would widen the exception. It needs the same justification: test-only infrastructure that cannot be written safely.
