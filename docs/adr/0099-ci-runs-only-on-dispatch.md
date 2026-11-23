---
id: 0099
title: CI runs only on dispatch
date: 2026-11-23
status: Accepted
supersedes: 0098
summary: ci.yml drops its push and pull_request triggers and runs only on workflow_dispatch; the owner dispatches a run at the end of a block of work.
---

# 0099. CI runs only on dispatch

## Context

ADR 0098 made `ci.yml` run on every push to `main` and every pull request into
`main`, so that the newly public repository tested contributions automatically.
In practice the owner lands work as a serial stack of small pull requests, each
merged with a merge commit and followed by a merge to `main`, so a two-PR batch
starts about four full runs of a pipeline that builds vendored Tesseract and runs
coverage, while only the final run on `main` is acted on. The owner's working
rule is to run CI once at the end of a block of work, not on every commit.

## Decision

`.github/workflows/ci.yml` triggers on `workflow_dispatch` only. Pushes and pull
requests, including pull requests from forks, start no run. A maintainer
dispatches `gh workflow run ci.yml --ref <ref>` at the end of a block of work,
normally on `main` after the block has landed, and before merging an outside
contribution. The concurrency group stays keyed on `github.ref` with
`cancel-in-progress`. `wheels.yml`, `hardening.yml` and `advisories.yml` keep
their triggers.

## Alternatives considered

- **Keep ADR 0098** — every stacked PR and every merge runs the full pipeline;
  the cost is runs nobody reads.
- **Cancel unwanted runs by hand during a batch** — leaves the trigger in place
  and depends on remembering to cancel; rejected by the owner.
- **Pull requests only, not pushes to `main`** — still one run per PR of a stack.

## Consequences

- A pull request shows no check unless someone dispatched one; reviewers of
  outside contributions must dispatch before merging. A dispatched run tests the
  branch tip, not its merge with `main`.
- A merge to `main` is not tested until the end-of-block dispatch, so a red
  `main` can persist for the length of a block; it is fixed by the change that
  caused it.
- Reverses ADR 0098's intent that outside contributors get automatic feedback.
