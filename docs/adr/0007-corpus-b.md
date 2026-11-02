---
id: 0007
title: Corpus B is hayro-tests custom files and PDFBox test resources only
date: 2026-10-01
status: Accepted
summary: The real-world corpus is imported from hayro-tests and Apache PDFBox test resources; pdf.js files are excluded.
---

# 0007. Corpus B is hayro-tests custom files and PDFBox test resources only

## Context

Decision D-25 and open question Q2 of the run spec. Corpus B needs real-world files with a redistributable license.

## Decision

Corpus B is hayro's `hayro-tests/pdfs/custom/*.pdf` files that contain text (Apache-2.0 OR MIT) plus PDFBox test
resources with text (Apache-2.0), each with origin, license and SHA-256 in `tests/fixtures/PROVENANCE.md` and
`manifest.toml`. As built: 60 files (20 from hayro-tests at `febaff5`, 40 from PDFBox at `c4d556a`).

## Alternatives considered

**pdf.js test files** excluded by default: they carry no per-file license. They could be added after per-file license
checks (Q2); the default was signed off.

## Consequences

Corpus B is a sample of what those two projects test, not of the first consumer's uploads; the private corpus-A run
covers that. Residuals found by B are listed in ARCHITECTURE.md and run spec section 2.5.
