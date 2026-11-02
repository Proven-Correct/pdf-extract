---
id: 0015
title: Hayro-level units assert hayro observables only
date: 2026-10-01
status: Accepted
summary: Units 04-07 assert only what hayro exposes; Warning, page_count and Page::text assertions belong to units 08-10.
---

# 0015. Hayro-level units assert hayro observables only

## Context

Decision D-30 (run spec revision 2). Unit 04 depends on unit 03 only, yet its oracle initially asserted core-level
observables that do not exist until unit 08.

## Decision

The hayro-level units (04-07) assert only what hayro itself exposes; every `WarningKind`, `page_count` and
extracted-text assertion moved to the unit 08-10 oracles.

## Alternatives considered

Not recorded beyond the rationale below.

## Consequences

Recorded rationale: a unit's oracle must be checkable with the code that exists when it lands.
