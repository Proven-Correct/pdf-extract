---
id: 0002
title: Carry a patched fork of hayro until the changes are upstream
date: 2026-09-25
status: Accepted
summary: A patched fork of the hayro crates (changes H1-H10) is used through [patch.crates-io] until upstream takes the changes.
---

# 0002. Carry a patched fork of hayro until the changes are upstream

## Context

Probing `hayro-syntax` 0.7.2 showed that `Pdf::new` builds an object for every page while opening a file: 8.9 s and
1.49 GB for a 1M-page, 87 MB PDF (0.17 s and 149 MB for 100k pages), where PyMuPDF took 6 ms and 64 ms because it
trusts `/Count`. That defeats the page-count guard and is a denial-of-service vector. Further gaps found against
the requirements: no cap on object nesting (verified stack overflow at 200,000-deep nesting, even on 8 MiB stacks),
no decoded-size cap on streams, no work budget or cancellation, no marked-content properties for `/ActualText`,
no public character code or per-glyph font facts, no thread-safe lazy xref repair. Source: design spec D2 and
section 3.3; run spec sections 3.2 and Appendix B.

## Decision

Keep a patched fork of hayro until the changes are merged upstream. The workspace uses it through
`[patch.crates-io]` (mechanics: ADR 0003); versions are pinned exactly. Each change is its own commit with
an exported patch and a test in `crates/core`. The changes landed in this run are: H1 lazy pages, H2a/H2b nesting
and decode limits, H3/H3b interpreter budget and form-cycle stack, H4 marked-content properties, H5 glyph facts,
H6 CID Unicode fallbacks, H8 render-mode 7 and annotation filtering, H9 thread-safe xref repair, H10a-h device
and lexer signals. If upstream declines a blocking change, the fork becomes permanent, rebased on each hayro
release adopted, with the corpus gates run on every rebase.

## Alternatives considered

Not recorded in the spec beyond the above. Two related points are recorded: opening upstream pull requests is out
of scope for this run (patches are exported, not sent), and H7 (a shared font cache) was left out (ADR 0012).

## Consequences

The fork's diff must stay reviewable (ADR 0003) and a `vendor-patches` CI job fails on drift. Every hayro
upgrade is a rebase of a 19-patch series. The text path depends on fork-only APIs (`Page::from_dict`,
`Context::stats()`, `Glyph::to_unicode()`, the device hooks), so it cannot be built against stock hayro.
