---
id: 0095
title: A weekly scheduled cargo-deny advisories run, the only schedule trigger
date: 2026-10-22
status: Accepted
summary: advisories.yml runs only `cargo deny check advisories`, weekly and on dispatch, so a RustSec advisory published after a release is seen within a week; it is the repository's only schedule trigger, approved as cheap, and no precedent for scheduling heavy workflows.
---

# 0095. A weekly scheduled cargo-deny advisories run, the only schedule trigger

## Context

Advisories were checked by the `deny` CI job only, which runs on main pushes
and dispatch. An advisory published after a release went unseen until
someone next pushed. Actions minutes are billed, and ADR 0087 keeps heavy
workflows (fuzzing, sanitizers) off `schedule` and `push`.

## Decision

`.github/workflows/advisories.yml` runs `cargo deny check advisories` every
Monday at 06:17 UTC, and on `workflow_dispatch`. It is one `ubuntu-24.04` job
with a 10-minute timeout and no build. The action's `cargo metadata` may
install the pinned toolchain, which is most of its run time. The project
owner approved it on 2026-10-22 as the repository's only schedule trigger,
because it costs a few runner-minutes a week. The release workflow also reruns
every licence check, advisories included, on the commit it releases (the
reusable `license.yml`).

## Alternatives considered

- **Dispatch only.** This depends on someone remembering to run it, which is
  the gap being closed.
- **Leave advisories to push and release runs.** Weeks can pass between
  pushes on a stable library.
- **Schedule the whole `deny` job.** It also installs cargo-about and
  regenerates the notices, which costs more and adds nothing to advisory
  detection.

## Consequences

- A failing run notifies the repository's watchers by GitHub's default.
- GitHub disables scheduled workflows after 60 days with no repository
  activity, without warning, and the check stops until someone re-enables
  it. The release workflow's own advisories check still covers releases.
- This is not precedent: ADR 0087's cap on scheduling heavy workflows is
  unchanged.
