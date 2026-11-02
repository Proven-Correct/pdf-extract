---
id: 0003
title: Vendor only the three patched hayro crates, with exported patches and a drift check
date: 2026-10-01
status: Superseded
superseded_by: 0080
summary: hayro-syntax, hayro-interpret and hayro-postscript live under vendor/ as a pristine import plus an exported patch series that CI re-applies.
---

# 0003. Vendor only the three patched hayro crates, with exported patches and a drift check

## Context

Decision D-1 of the run spec. The fork (ADR 0002) touches three crates: `hayro-syntax` 0.7.2,
`hayro-interpret` 0.7.0 and `hayro-postscript` 0.1.0. The remaining hayro crates are not patched.

## Decision

Vendor only the three patched crates under `vendor/`. The unpatched leaf crates (`hayro-cmap =0.1.0`,
`hayro-jbig2 =0.3.0`, `hayro-jpeg2000 =0.3.5` as hayro-syntax 0.7.2 requires, `hayro-ccitt =0.3.0`) are pinned
exactly and reproducible from `Cargo.lock`. `hayro` 0.7.1 (the rasteriser) is not a dependency until the render
crate exists. Mechanics: the pristine import is a single commit tagged `vendor/hayro-pristine-1`;
`.cargo-checksum.json` is deleted because cargo would verify per-file hashes and refuse patched files; the package
checksum is recorded in `vendor/PROVENANCE.md`; the crates are wired in by `[patch.crates-io]` and are not
workspace members (`exclude = ["vendor"]`), so fmt and clippy skip them and their upstream tests (which read
`../hayro-tests/`) are not run. Patches are exported with `git format-patch --relative=vendor` and
`tools/vendor/check-patches.sh` re-applies them on a fresh pristine import and diffs against `vendor/`.

## Alternatives considered

Recorded rationale is one line: a reviewable diff, and the unpatched crates are reproducible from the lockfile.
No rejected alternative is recorded in the spec.

## Consequences

Any edit under `vendor/hayro-*` must be its own commit followed by a patch export, or the `vendor-patches` job
fails (it needs network access to crates.io). The pristine tag is local; in a fresh clone the pristine commit is
passed as the base. Fork behaviour is tested from `crates/core/tests/`, not from the vendored crates.

## Addendum (2026-10-03)

Spec 0002 adds a second kind of vendored tree: pruned, committed copies of Tesseract 5.5.3 (`vendor/tesseract/`)
and Leptonica 1.87.0 (`vendor/leptonica/`). They have their own provenance table in `vendor/PROVENANCE.md`
(archive URL, archive SHA-256 and commit id), are imported by `tools/vendor/import-c.sh` from a committed prune
list, and are checked by `tools/vendor/check-c-sources.sh`. Two local pristine tags,
`vendor/tesseract-pristine-1` and `vendor/leptonica-pristine-1`, mark the unpatched imports; any change would be an
exported patch under `vendor/patches/{tesseract,leptonica}/` (the series starts empty). Like the hayro tags, the
tags were local; they have since been pushed to origin. `hayro` itself is not vendored (ADR 0055).
