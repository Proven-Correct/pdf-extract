---
id: 0071
title: The helper is found by absolute path or beside the module; PATH is never searched
date: 2026-10-03
status: Accepted
summary: PDF_EXTRACT_OCR_HELPER (absolute), then beside the extension module, then beside the executable; a missing helper fails at construction.
---

# 0071. The helper is found by absolute path or beside the module; PATH is never searched

## Context

Spec 0002 D-16. Fail fast (O7) and no ambient-authority search, consistent with design-spec section 7.3's no whereis.

## Decision

The helper is found at an absolute `PDF_EXTRACT_OCR_HELPER`, else beside the extension module (`helper_dir`), else beside the executable; `PATH` is never searched; a missing helper fails at engine construction.

## Alternatives considered

- **Search PATH** - ambient authority.

## Consequences

See the helper packaging record for where the binary ships.
