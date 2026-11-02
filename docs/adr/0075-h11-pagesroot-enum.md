---
id: 0075
title: H11 introduces PagesRoot { Ref, Inline }
date: 2026-10-03
status: Accepted
summary: pages_root() returns a PagesRoot enum so a direct /Pages dictionary is a valid shape distinct from a missing /Pages.
---

# 0075. H11 introduces PagesRoot { Ref, Inline }

## Context

Spec 0002 D-20. ADR 0025 recorded a direct-dictionary /Pages as Corrupt (open question Q7); this run resolves Q7 by opening it.

## Decision

H11 introduces `PagesRoot { Ref, Inline }` rather than making `pages_root()` return `Option`. An inline root has no identifier and no dedup.

## Alternatives considered

- **`Option`** - would conflate an inline root with no `/Pages`, which stays `Corrupt`.

## Consequences

A direct /Pages dictionary now opens. See the addendum on ADR 0025.

## Addendum (2026-10-21)

A departure that this decision's `Inline` fallback leaves open, recorded as found (internal issue #56 B21); it is characterization, not endorsement.

`pages_root()` re-reads an inline root from the catalog on each call, and falls back to an empty inline dictionary (no `/Kids`) when a lazy xref repair has replaced the catalog since open (`vendor/hayro-syntax/src/pdf.rs`, `unwrap_or_default`). Core's page index also keeps an inline root as an index entry and re-reads it from the catalog for each page operation (`index.rs`, `inline_root` and `node_dict`). When the re-read fails (the replacement catalog has no inline `/Pages`), `inherited_attributes` skips that node, so the root's inherited `/MediaBox` (and `/CropBox`, `/Rotate`) and `/Resources` are dropped. The page is then read with defaults: the default page box, and a font that only the root's `/Resources` named is missing, so hayro silently substitutes Helvetica. No warning is raised for any of it.

The consequence is an order dependence. Whether a page reads normally or degraded depends on whether an earlier operation on the same document happened to trigger the repair; the same page read first is normal. Spec 0001 T39 (concurrent use gives the results of sequential use) is only indirectly relevant: the tests' module doc cites it, but it does not cover an order dependence in sequential use.

Pinned, not fixed, by `crates/core/tests/repair_after_open.rs` (`pages_root_is_an_empty_inline_dictionary_once_the_catalog_is_replaced`, `a_catalog_replaced_by_a_repair_after_open_drops_the_inline_roots_attributes`) and the render test `a_catalog_replaced_by_a_repair_after_open_drops_the_inherited_media_box` in `crates/render/tests/render.rs`. No openspec scenario was added: the page-index requirement "Inherited page attributes" keeps its full strength (ADR 0084), and this behaviour is a gap against it, not a contract.
