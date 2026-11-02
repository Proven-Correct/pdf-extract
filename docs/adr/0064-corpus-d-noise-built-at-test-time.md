---
id: 0064
title: Corpus D noise variants are built at test time, not committed
date: 2026-10-03
status: Accepted
summary: The 12 noise variants are built by the test kit from the committed clean files, with SHA-256 pins in manifest.toml [[built]].
---

# 0064. Corpus D noise variants are built at test time, not committed

## Context

Spec 0002 D-9 as revised in revision (5), a user decision. The signed-off pipeline produced all 52 files at 77.5 MB against Q4's 25 MB cap; Gaussian noise at sigma 12 carries 3-4 bits per pixel, so the twelve noise files alone were 55.1 MB and Flate could not help.

## Decision

The 12 noise variants are built at test time by `testkit::corpus_d` from the committed clean files' decoded pixels and boxes, with deterministic bytes pinned by SHA-256 in `manifest.toml` `[[built]]`, the arrangement `hostile/` already uses. The noise parameters (sigma 12, seed 20261003) and every gate of O34 are unchanged.

## Alternatives considered

- **Commit all 52 files** - 77.5 MB against the 25 MB cap.
- **Commit them Flate-compressed** - 55.1 MB for the noise alone; Flate saves under 2x on noise.

## Consequences

The built PDFs are about 105 MB unfiltered in target/corpus-d/. The noise realisation is the test kit's, not numpy's. A pin mismatch fails quality.rs naming the file and both hashes.
