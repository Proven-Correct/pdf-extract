---
id: 0084
title: OpenSpec capability specs are the living behavioural contract
date: 2026-10-12
status: Accepted
summary: openspec/specs/ holds the current behavioural contract as per-capability requirements; new work arrives as OpenSpec changes against it, while docs/specs and docs/adr keep mechanism, evidence and rationale.
---

# 0084. OpenSpec capability specs are the living behavioural contract

## Context

The contract was spread across `docs/design-spec.md`, three run specs (`docs/specs/0001`–`0003`) that refine and revise it row by row, and 83 ADRs. Answering "what does the system promise today" meant reading a design-spec requirement, then every later "Revised" note on the run-spec rows that refine it, then the ADRs. Phase 3 would add a fourth run spec on top. Each run spec also mixes contract with mechanism (hayro mappings, H-patch details, probe evidence), so none of them could serve as the current contract.

## Decision

We adopt OpenSpec (schema `spec-driven`). `openspec/specs/<capability>/spec.md` states the current behavioural contract: requirements in SHALL form with testable scenarios, and every scenario cites its source (design-spec ID, run-spec row such as `0001 T9`, or ADR).

The baseline (internal PR #73, archived as `openspec/changes/archive/2026-10-12-baseline-specs`) covers only what has been decided **and built**: design-spec Phases 0–2 plus run 0003, in 13 capabilities (document-open, page-index, text-extraction, resource-limits, concurrency, error-model, test-kit, output-parity, licensing, ocr-rendering, ocr-engine, ocr-batch-extract, ocr-process-isolation). Each behaviour has one owning capability; the others refer to it by name.

Rules:

- Where the code falls short of a requirement, the requirement keeps its full strength and the gap is tracked as an issue (for example internal #63 against R1).
- New behaviour, starting with Phase 3, arrives as an OpenSpec change whose delta specs modify the baseline. It does not arrive as edits to `docs/specs/`.
- `docs/specs/0001`–`0003` become the historical record of how each run reached its contract. They and `docs/adr/` keep the mechanism, evidence and rationale, which the capability specs deliberately leave out.

## Alternatives considered

- **Keep extending run specs** - every new run adds another layer of revisions, and the current contract can still only be reconstructed by reading all of them.
- **Backfill the whole design-spec intent, including unbuilt Phase 3/4** - `openspec/specs/` would then claim behaviour the code does not have. Unbuilt work belongs in changes.
- **Copy the run-spec rows verbatim** - this would duplicate the mechanism and revision history, leaving two copies to keep in step.
- **Write main specs directly instead of through a baseline change** - a baseline change made of ADDED-only deltas goes through the same validation and archive path as later changes, and leaves a record of the backfill.

## Consequences

- One place answers what the system promises, and Phase 3 changes can be reviewed as deltas.
- Requirements and code can drift apart. Unlike the parity and contract tests, nothing checks the specs against the code automatically, so a change that alters behaviour must carry a delta spec.
- The backfill surfaced gaps that are not yet fixed: three code gaps, about ten requirements without a direct test, and stale run-spec or design-spec text. They are listed in internal PR #73.
- Performance targets P2–P4 are absent because nothing verifies them. P1 is stated only as a target, because its benchmark is report-only.
- Some rules have no capability, among them repository hygiene (ADR-0017) and "no runtime Python dependencies". They can be added when packaging gets its own capability.
