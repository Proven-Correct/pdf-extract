---
id: 0085
title: Pull requests run CI on demand; main runs on every push
date: 2026-10-14
status: Superseded
superseded_by: 0098
summary: The CI workflow triggers automatically only on pushes to main (cancel-in-progress keeps the newest); pull requests run only when dispatched by hand.
---

# 0085. Pull requests run CI on demand; main runs on every push

## Context

CI triggered on `push` to `main`, on every `pull_request`, and on
`workflow_dispatch`. Fix batches land as a series of small PRs, each opened
only after the previous one has merged, and merged by merge commit without
waiting on checks (`main` is not protected). A batch of twelve PRs started
twelve full runs (each builds vendored Tesseract and Leptonica), and every
one of them was superseded by the run on `main` before anyone looked at it.
Each fix is checked locally before it is committed, and the whole batch is
judged by the run on `main` after it lands.

## Decision

`.github/workflows/ci.yml` triggers on `push` to `main` and on
`workflow_dispatch` only. A pull request is run on demand with
`gh workflow run ci.yml --ref <branch>`. The concurrency group stays keyed on
`github.ref` with `cancel-in-progress`, so a burst of merges to `main` runs once
for the newest commit.

## Alternatives considered

- **Keep `pull_request` and cancel each PR's run after merging** — still starts
  every run and depends on a manual step being remembered.
- **Keep `pull_request` and let the runs finish** — costs a full run per PR
  whose result nobody waits for.
- **Require status checks on `main`** — makes each merge wait for a full run,
  serialising a batch over many hours, while the batch is judged by the final
  run on `main` anyway.

## Consequences

- A red `main` is now the first CI signal for a merged change. Fixing it is
  part of the batch that caused it, through a new PR.
- A contributor PR from outside the batch workflow gets no automatic checks;
  whoever reviews it dispatches the run.
- The first batch landed this way (internal #80–#91) went red on `main` from a
  `cargo doc -D warnings` failure that no local check covered. internal #92 fixed it.
  A local pre-push check list should include `cargo doc` with
  `RUSTDOCFLAGS="-D warnings"`.
- ADR 0013's note that branch pushes run twice no longer holds (see its
  addendum).
