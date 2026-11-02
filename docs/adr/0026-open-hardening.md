---
id: 0026
title: Bound the open path: regular files only, header window, size and count rules
date: 2026-10-01
status: Accepted
summary: Open rejects non-regular paths, requires the %PDF- header wholly within 1024 bytes, enforces max_file_size for buffers, ignores /Count and coalesces repeated warnings.
---

# 0026. Bound the open path: regular files only, header window, size and count rules

## Context

Decision D-36 (revision 5) and open question Q6, accepting unit 08's interpretations. Each closes a blocking or
unbounded path or removes an inconsistency between requirement rows and the unit's oracle.

## Decision

Non-regular paths (FIFO, directory, device) are rejected from `stat` before opening, so a FIFO never blocks. The
`%PDF-` header must lie wholly within the first 1024 bytes (F2 as written; the oracle accepts offset 1500, a documented
difference, Q6 default). `from_bytes` and `open_bytes` enforce `max_file_size`. `/Count` is ignored entirely. The
brute-force scan is skipped after a depth cut. Repeated warnings are coalesced into one with a count. File type
detection is by content only; the name is never consulted.

## Alternatives considered

Raising the header window to match the oracle was the Q6 alternative; the default (keep 1024) was signed off.

## Consequences

A path swapped for a FIFO between the check and the read can still block or hit the size bound (TOCTOU at open, not
defended).
