---
id: 0034
title: The source buffer is Arc<Vec<u8>>
date: 2026-10-01
status: Accepted
summary: Document sources are held as Arc<Vec<u8>> because Arc<[u8]> does not compile against hayro's PdfData.
---

# 0034. The source buffer is Arc<Vec<u8>>

## Context

Decision D-15. The design spec §5.1 says `Arc<[u8]>`.

## Decision

Use `Arc<Vec<u8>>`.

## Alternatives considered

`Arc<[u8]>` was verified not to compile: hayro's `PdfData: From<Arc<T>>` requires `T: Sized`.

## Consequences

One level of indirection more than the design spec drew; no behaviour consequence.
