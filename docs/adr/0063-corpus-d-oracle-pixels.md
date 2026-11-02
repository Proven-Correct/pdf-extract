---
id: 0063
title: Corpus D pixels come from the oracle; clean, skew, blur, low-resolution and JPEG files are committed
date: 2026-10-03
status: Accepted
summary: Corpus D is rasterised by PyMuPDF and the 40 non-noise files are committed as generated, with the builder's input text as ground truth.
---

# 0063. Corpus D pixels come from the oracle; clean, skew, blur, low-resolution and JPEG files are committed

## Context

Spec 0002 D-9. The scan must not come from the rasteriser under test, and PyMuPDF randomises /ID on save, so the files are not byte-reproducible (as for the encrypted fixtures, 0001 T51).

## Decision

Corpus D's pixels are the oracle's. The 40 clean, skew, blur, low-resolution and JPEG files are committed as generated. Ground truth is the builder's input text.

## Alternatives considered

- **Rasterise with our renderer** - a Flate decode of our own output is not an independent scan.

## Consequences

The committed corpus is 22.2 MB, under the 25 MB cap of spec Q4.
