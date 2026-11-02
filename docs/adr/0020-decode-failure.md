---
id: 0020
title: A stream over the decode limit is a hard decode failure, not a truncation
date: 2026-10-01
status: Accepted
summary: A stream that exceeds max_stream_decoded_bytes fails to decode and is treated as absent, with a warning.
---

# 0020. A stream over the decode limit is a hard decode failure, not a truncation

## Context

Decision D-13, alongside change H2b (per-stream decoded-size cap, default 256 MiB), which defends against flate,
LZW and image bombs.

## Decision

Over-limit decoding yields `DecodeFailure::LimitExceeded`; hayro treats the stream as absent and core adds the
warning `stream: decoded size limit`. Output is not truncated.

## Alternatives considered

Truncation was rejected: truncated fonts and CMaps are garbage.

## Consequences

Absence plus a warning is the honest outcome. The limit also covers xref streams and the object streams decoded by the
repair scan; streams decoded without document context (inline images, `FromBytes`) are not limited, which is not
reachable on the text path. Peak memory per extraction is bounded only by the cap times the live decoded streams
(ADR 0021).
