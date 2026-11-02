---
id: 0035
title: The Rust API does not catch panics; the Python boundary does
date: 2026-10-01
status: Accepted
summary: Panics propagate as panics to Rust callers and become InternalError at every exported Python function.
---

# 0035. The Rust API does not catch panics; the Python boundary does

## Context

Decision D-16 with requirement T45 and R1. Panics from any crate (hayro, skrifa) must never crash a Python process or
surface as `pyo3.PanicException`, a `BaseException`.

## Decision

Rust callers of `pdf-extract-core` receive panics as panics (so `PageError::Internal` of the design spec is not in the
Rust enum). Every exported native body runs in `guard::guarded` (`catch_unwind`) and a panic becomes `InternalError(message)`;
`panic = "unwind"` is set in every profile.

## Alternatives considered

Catching panics at each Rust call was rejected; recorded rationale: idiomatic Rust, it would hide bugs from Rust users and
costs nothing to add later.

## Consequences

Do not switch to `panic = "abort"`. A new `#[pyfunction]` or `#[pymethods]` body (getters included) without the guard would
let `PanicException` escape. The guard's panic hook is replaced at module init to keep stderr silent (T48), and error
classes are cached at init because a failed lookup at raise time would panic outside any unwinding frame. A panic inside the
`#[pymodule]` init itself is unguarded (residual).

## Addendum (2026-10-06)

Spec 0003 D-7 (documented, not changed): a panic inside the first `default_ocr_engine()` construction propagates under this rule and leaves its `OnceLock` empty, so the failed construction is not cached and the next call constructs again. An `Err` from that construction, by contrast, is stored and returned forever.

## Addendum (2026-10-19)

Partially superseded by ADR 0086, for image-codec decodes only. The vendored hayro-syntax now catches a panic inside a call into an image codec crate (H18) and skips that image with an `image: decode failed` warning, so Rust callers no longer receive codec panics. Fuzzing builds (`cfg(fuzzing)`) still see them. Every other panic still propagates as this record decides. The status stays Accepted because the decision log has no partial-supersession status. ADR 0086 also records that the library never installs a panic hook for Rust callers, and that `guard::guarded` clears the recorded panic message on entry.
