---
id: 0067
title: Render determinism is exact per host and tolerance-gated across hosts
date: 2026-10-03
status: Accepted
summary: Rendering is byte-exact per host; across hosts at least 99.9 % of pixels must be identical with max difference 8.
---

# 0067. Render determinism is exact per host and tolerance-gated across hosts

## Context

Spec 0002 D-12. vello_cpu and pic-scale dispatch SIMD at runtime; OCR text, not pixels, is what the product promises.

## Decision

Render determinism is exact per host and tolerance-gated across hosts (at least 99.9 % identical pixels, max |delta| at most 8).

## Alternatives considered

No alternative recorded.

## Consequences

Golden hashes are per OS and must be re-blessed on a hayro, vello or pic-scale lockfile bump. The render-cross-os CI job checks the tolerance.
