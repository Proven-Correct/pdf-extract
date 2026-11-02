---
id: 0024
title: max_page_tree_depth defaults to 256
date: 2026-10-01
status: Accepted
summary: The page-tree depth limit is 256; deeper nodes are skipped with a warning.
---

# 0024. max_page_tree_depth defaults to 256

## Context

Decision D-26 (revision 1). The first draft used 64. Re-probing showed the oracle has no depth cap: a 20,000-deep chain
extracts normally.

## Decision

`max_page_tree_depth` defaults to 256. A node deeper than the limit is skipped with a `PageTreeDepth` warning.

## Alternatives considered

Depth 64 (the earlier default) was replaced. Recorded rationale: 256 exceeds any real tree and bounds the walk.

## Consequences

Pages deeper than the limit are a documented divergence (KP23, pinned by `deep_tree_3000`). A depth-limited tree does not
trigger the brute-force scan, otherwise the scan would resurrect the pages the cap refused.
