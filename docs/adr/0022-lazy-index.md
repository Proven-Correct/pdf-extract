---
id: 0022
title: Build the page index lazily with an iterative walk; page_count is honest and bounded
date: 2026-09-25
status: Accepted
summary: hayro's page list is made lazy (H1) and core indexes pages with its own iterative walk, ignoring /Count and honouring max_pages.
---

# 0022. Build the page index lazily with an iterative walk; page_count is honest and bounded

## Context

Design spec D7, change H1 and requirement T9. `Pdf::new` built a `Page` for every leaf (ADR 0002), and the
recursive `resolve_pages` overflows a 512 KiB stack on a 3,000-deep tree. A guard that trusts `/Count` can be lied
to; the design spec measured PyMuPDF doing exactly that.

## Decision

H1 makes `Pdf::new` stop walking the page tree (a private `OnceLock<CachedPages>`, `pages_root()`, `pages_built()`,
`xref_was_repaired()`, `InheritedAttributes` and `Page::from_dict`). Core builds a 16-byte
`PageRef { obj, inherit, kid }` per leaf with an iterative walk that resolves one dictionary per node and never touches
`/Contents` or `/Resources` values. `/Count` is ignored entirely, not even as a capacity hint. With `max_pages = Some(n)`
the walk stops at leaf n+1 (`TooManyPages`). If the walk yields no leaf and no subtree was cut at the depth limit, a
brute-force scan of `/Type /Page` objects runs. A hayro `Page` is rebuilt per operation from the `PageRef` and never
cached (T17).

## Alternatives considered

Trusting `/Count` (PyMuPDF's behaviour) was rejected as unsafe. Keeping upstream's eager and recursive walk was
rejected for the cost and the stack overflow above.

## Consequences

`Pdf::new` on a 100k-page tree retains about 33 B and peaks about 49 B per xref entry (hayro's `XrefMap`). Because xref
repair is lazy, `xref_was_repaired()` can become true after open (ADR 0030). Direct-dictionary `/Kids` are
re-found by position and only the first 4096 positions are indexed. The P1 timing targets are report-only
(ADR 0013).
