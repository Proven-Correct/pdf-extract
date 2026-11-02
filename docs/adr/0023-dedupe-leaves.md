---
id: 0023
title: The page index visits each node once and skips cycles; the oracle differs
date: 2026-10-01
status: Accepted
summary: A node reached a second time is skipped with a warning; each leaf is yielded once and iteration never raises (KP10).
---

# 0023. The page index visits each node once and skips cycles; the oracle differs

## Context

Decision D-14, compat point KP10. The oracle counts a duplicated leaf twice and raises `cycle in page tree` when the page
is loaded.

## Decision

A node (by object identifier) reached a second time is skipped with the warning `page tree: node revisited`, whether
through a cycle or a duplicate `/Kids` entry. A kid that is a direct dictionary cannot be deduplicated.

## Alternatives considered

Matching the oracle (count duplicates, raise on cycles) was rejected; recorded rationale: a duplicated leaf is a
malformed file, and the index needs bounded memory and no exceptions during iteration.

## Consequences

`page_count` differs from the oracle for such files; pinned by the `kp10_*` divergence fixtures.
