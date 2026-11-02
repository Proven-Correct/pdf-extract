---
id: 0077
title: Test-time noise deviates are integer Irwin-Hall with unfiltered image streams
date: 2026-10-03
status: Accepted
summary: Noise is 12 x 20-bit uniforms from SplitMix64 (seed 20261003 per file), not Box-Muller, and built image streams are unfiltered.
---

# 0077. Test-time noise deviates are integer Irwin-Hall with unfiltered image streams

## Context

Spec 0002 D-22 (revision 5). A pinned hash needs byte identity across hosts; libm ln/cos differ in their last bits between platforms, and deflate output differs between miniz_oxide versions.

## Decision

The test-time noise deviates are integer Irwin-Hall (12 x 20-bit uniforms from SplitMix64, seed 20261003 per file), not Box-Muller, and the built image streams are unfiltered.

## Alternatives considered

- **Box-Muller** - platform-dependent last bits.
- **Flate-compressed built streams** - encoder output varies with the library version, and Flate would save under 2x on noise.

## Consequences

The approximation's sigma is exact and its density error is below 1e-3.
