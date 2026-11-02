---
id: 0008
title: Initial project license: MIT
date: 2026-10-01
status: Superseded
superseded_by: 0009
summary: The first sign-off placed the repository's own code, LICENSE and manifests under MIT terms.
---

# 0008. Initial project license: MIT

## Context

Open question Q1 of the run spec asked for the project license, needed for `Cargo.toml`, `pyproject.toml` and the
LICENSE files. It was resolved as MIT on 2026-10-01 at sign-off. Rationale not recorded in the spec.

## Decision

Our own code is MIT-licensed. Dependencies stay permissive-only (L1).

## Alternatives considered

Not recorded in the spec.

## Consequences

Units 01-12 (internal PRs #1-#13) were merged with MIT, so history carries MIT until unit 12b. Superseded by ADR 0009.
