---
id: 0066
title: A timeout is Recognize == -1 and our clock past the deadline
date: 2026-10-03
status: Accepted
summary: Tesseract's -1 return is classified as a timeout only when our own clock is past the deadline.
---

# 0066. A timeout is Recognize == -1 and our clock past the deadline

## Context

Spec 0002 D-11. Tesseract returns -1 for timeout, cancel, uninitialised and layout failures alike (Appendix B.2).

## Decision

A timeout is `Recognize == -1` and our clock past the deadline; any other `-1` is `Recognition`.

## Alternatives considered

No alternative recorded; reading Tesseract internals is the thing avoided.

## Consequences

The clock disambiguates without reading Tesseract internals. Tesseract does not check its deadline during layout analysis, so a timeout can be late by a whole layout pass (unit 06 measurement).
