---
id: 0080
title: Vendor only what we patch: five hayro crates, two pristine tags, two-range patch export
date: 2026-10-05
status: Accepted
supersedes: 0003
summary: The rule of ADR 0003 is restated for five patched hayro crates (adding hayro-jbig2 and hayro-jpeg2000) with a second pristine tag, a two-range patch export and merge-commit-only vendor imports.
---

# 0080. Vendor only what we patch: five hayro crates, two pristine tags, two-range patch export

## Context

ADR 0003 vendored only the three patched hayro crates and pinned the rest from `Cargo.lock`; its title says "three". Run 0003 patches `hayro-jbig2` 0.3.0 (H14) and `hayro-jpeg2000` 0.3.5 (H15) (ADR 0079), so the rule's own logic puts them under `vendor/` and the title is no longer true. A git dependency is not an option (`deny.toml` `allow-git = []`). ADR 0003's body is frozen, so it is superseded; its Tesseract/Leptonica addendum is carried over below.

## Decision

The rule stands: vendor only the crates we patch. The vendored hayro crates are now five: `hayro-syntax` 0.7.2, `hayro-interpret` 0.7.0, `hayro-postscript` 0.1.0, `hayro-jbig2` 0.3.0 and `hayro-jpeg2000` 0.3.5. `hayro-cmap` and `hayro-ccitt` stay pinned from the lockfile; `hayro` (the rasteriser) is not vendored (ADR 0055). Mechanics of ADR 0003 carry over unchanged (`.cargo-checksum.json` deleted, checksum in `vendor/PROVENANCE.md`, `[patch.crates-io]`, not workspace members, `check-patches.sh` re-applies and diffs). New:

- A second pristine tag, `vendor/hayro-pristine-2`, on the import commit of the two new crates (the three older ones are already patched there). Tag 1 stays.
- A two-range export: `export-patches.sh` emits 0001-0022 from `pristine-1..pristine-2^` over the three old crates, then 0023 onward from `pristine-2..HEAD` over all five, numbering continuous.
- One-time normalisation of patch headers: the committed series had mixed `[PATCH n/N]` subject totals from incremental exports, so "byte-identical headers" was never true. Unit 03 normalised them once; the invariant is patch bodies byte-identical, headers normalised once, regeneration byte-identical thereafter.
- Vendor imports land by merge commit only, so the pristine tag stays on a reachable import commit.
- Carried over from ADR 0003's addendum: pruned, committed copies of Tesseract 5.5.3 (`vendor/tesseract/`) and Leptonica 1.87.0 (`vendor/leptonica/`), with their own provenance table, `tools/vendor/import-c.sh` and `check-c-sources.sh`, local tags `vendor/tesseract-pristine-1` and `vendor/leptonica-pristine-1` (pushed to origin), and patch series under `vendor/patches/{tesseract,leptonica}/`.

## Alternatives considered

- **Addendum to ADR 0003** - leaves a frozen title that says "three" and a body that contradicts the tree; a supersession is the honest record.
- **Git dependency on a fork** - refused by `deny.toml`.
- **Leave the codecs pinned and wrap them** - cannot reach internal allocations.
- **Re-baseline to a single pristine tag** - would invalidate the committed patch series 0001-0022.

## Consequences

Any edit under `vendor/hayro-*` is its own commit followed by a patch export, or `vendor-patches` fails. `cargo-about` orders path crates before registry crates, so `THIRD_PARTY_NOTICES` was regenerated once and must stay clean. Two more crates carry upstream-merge cost. Fork behaviour is still tested from `crates/core/tests/`.
