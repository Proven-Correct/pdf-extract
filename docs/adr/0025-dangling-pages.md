---
id: 0025
title: A page tree that cannot be built opens with zero pages
date: 2026-10-01
status: Accepted
summary: A dangling /Pages reference, or a root without /Kids, opens with page_count 0 plus warnings; a missing /Pages key stays Corrupt.
---

# 0025. A page tree that cannot be built opens with zero pages

## Context

Decision D-31. H1 removed hayro's failure for an unbuildable tree. The oracle is lenient about missing page trees: it
opens a catalog without `/Pages` with 0 pages.

## Decision

Core resolves `pages_root()` itself. A `/Pages` reference that does not resolve, or a root that is a non-dictionary or
lacks `/Kids`, opens with `page_count == 0` and the warnings `PageTreeNodeUnreadable` and `PageTreeBruteForce` when the
brute-force scan finds nothing. A catalog without a `/Pages` key remains `Corrupt` (hayro's `XRef::new` requires the
reference).

## Alternatives considered

No alternative recorded beyond the rationale.

## Consequences

The missing-key case diverges from the oracle (pinned by `catalog_no_pages`). A catalog whose `/Pages` is a direct
dictionary is also `Corrupt` for us (open question Q7: the oracle probably opens it, unprobed; accepted for this run).

## Addendum (2026-10-03)

Open question Q7 is resolved by change H11 (ADR 0075): a catalog whose `/Pages` is a direct dictionary now opens
and its pages are walked from the inline root. The Consequences above, which describe that case as `Corrupt`, no
longer hold. A catalog without a `/Pages` key still does.
