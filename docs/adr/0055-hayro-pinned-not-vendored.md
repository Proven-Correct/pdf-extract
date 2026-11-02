---
id: 0055
title: hayro is pinned from crates.io and routed onto the fork, not vendored
date: 2026-10-03
status: Accepted
supersedes: 0037
summary: hayro =0.7.1 is a crates.io pin routed onto the fork by [patch.crates-io]; it is not vendored.
---

# 0055. hayro is pinned from crates.io and routed onto the fork, not vendored

## Context

Spec 0002 D-1. ADR 0037 said hayro 0.7.1, the rasteriser, would be vendored when the render crate arrived, and ADR 0003 vendors only what we patch. hayro compiles and renders unchanged against the patched fork (spec 0002 Appendix B.1).

## Decision

`hayro = "=0.7.1"` is taken from crates.io and resolved onto the existing fork through the existing `[patch.crates-io]`. It is not vendored. This supersedes ADR 0037's "vendored when the render crate arrives" and extends ADR 0003 (vendor only what we patch).

## Alternatives considered

- **Vendor hayro** (ADR 0037's plan) - adds review surface and a provenance row for an unpatched crate, for nothing.

## Consequences

The render crate depends on the registry pin. A lockfile bump of hayro, vello or pic-scale needs the render golden hashes re-blessed (spec 0002 section 4.7).
