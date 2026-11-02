---
id: 0016
title: Hostile fixtures are gated on outcome, warning kinds and our own pinned text
date: 2026-10-01
status: Accepted
summary: Hostile fixtures are never compared with the oracle's text; they gate on open/extract outcome, the expected warning-kind set and our snapshot.
---

# 0016. Hostile fixtures are gated on outcome, warning kinds and our own pinned text

## Context

Decision D-27 (revision 1, clarified in revision 10). Probing showed the oracle's output for over-deep input depends
on where the nesting sits: empty text, partial text, or a raised error.

## Decision

Hostile fixtures are gated on outcome class, the expected `warnings` kind set and our own pinned text
(`snapshots/<sha>.txt`), never on the oracle's text. The earlier wording "where it matters" became "always" in unit 13
and is kept (stricter than the residual wording). Lazy xref repair adds `XrefRepaired` to the expected sets of
`deep_obj_20k` and `deep_resources_20k`.

## Alternatives considered

Matching the oracle's text on hostile input was rejected as a goal: it depends on nesting placement and sometimes
raises.

## Consequences

The 200,000-deep variants are generated at test time; the committed variants nest 20,000 deep to keep files small.
