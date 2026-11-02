---
id: 0098
title: Pull requests run CI automatically
date: 2026-11-02
status: Accepted
supersedes: 0085
summary: Now that the repository is public, ci.yml runs on every pull request into main (on GitHub's merge ref; forks wait for a maintainer's approval) as well as on pushes to main and by dispatch.
---

# 0098. Pull requests run CI automatically

## Context

ADR 0085 removed the `pull_request` trigger. The repository was internal, so Actions minutes were billed, and fixes landed as batches of back-to-back PRs whose individual runs were superseded by the run on `main` before anyone looked at them. A PR was run only when dispatched by hand, and the dispatched run tested the branch tip, not its merge with `main`.

Both premises changed with the public release (2026-11-02, ADR 0096 and 0097):
- Actions minutes are free for a public repository, including the macOS and arm runners `ci.yml` uses.
- Pull requests can now come from contributors outside the project. They cannot dispatch a workflow, and their changes have not been through the project's local check set.

## Decision

`.github/workflows/ci.yml` triggers on:
- `push` to `main`;
- `pull_request` into `main`;
- `workflow_dispatch`.

A pull request runs on GitHub's merge of the branch with `main` (`refs/pull/N/merge`). A pull request from a fork waits for a maintainer's approval before it runs, which is GitHub's default for outside contributors, and it runs with the workflow's read-only `contents: read` token; the workflow uses no secrets. The concurrency group stays keyed on `github.ref` with `cancel-in-progress`, so a new push to a pull request cancels its previous run.

`wheels.yml` (tags and dispatch) and `hardening.yml` (dispatch only) keep their triggers; see ADR 0087's addendum of 2026-11-02.

## Alternatives considered

- **Keep ADR 0085's on-demand runs.** Outside contributors cannot dispatch, so their pull requests would get no CI signal until a maintainer intervened, and on-demand runs test the branch tip rather than the merge.
- **`pull_request_target`.** It would run fork PRs with a write token and the repository's secrets against untrusted code, for no benefit: the workflow needs neither.
- **Require status checks on `main`.** Not decided here; branch protection is a repository setting, and nothing in the workflow depends on it.

## Consequences

- Every pull request gets a full run (about 19 jobs, including one cold build of vendored Tesseract and Leptonica per platform). A batch of back-to-back PRs now starts one run per PR; each is cancelled by its own next push, but not by merges of other PRs.
- A PR's result reflects its merge with `main` at the time it ran. A run that predates later merges is stale, and is re-run before merging when the change interacts with them.
- The known timing flakes in the `coverage` job (public issues #2 and #3) can now fail outside contributors' PRs, which raises their priority.
