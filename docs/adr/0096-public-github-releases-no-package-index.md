---
id: 0096
title: Releases are public GitHub Release assets; no package index, crates unpublished
date: 2026-10-22
status: Accepted
summary: pdf-extract is released as wheels, sdist and SHA256SUMS on the public repository's GitHub Releases, installed by URL pinned by hash; nothing goes to PyPI or crates.io, which closes design spec §17.2 Q1 and, for wheels, Q6 (the golden re-bless is a RELEASING.md step).
---

# 0096. Releases are public GitHub Release assets; no package index, crates unpublished

## Context

Phase 4 needs consumers to install pdf-extract without credentials. v0.1.0
was a GitHub Release asset on an internal repository, so installing it took
an authenticated `gh`. On 2026-10-22 the project owner decided to make
pdf-extract a public project, with releases independent of any one consumer
(change `publish-publicly-for-adoption`).

Two open questions in the design spec bear on publishing:

- **§17.2 Q1, the name.** PyPI `pdf-extract` is unregistered, but the
  unrelated `pdfextract` exists. PyPI refuses a new name that collides with an
  existing one once `-`, `_` and `.` are removed, so `pdf-extract` would very
  likely be rejected. This is an inference from PyPI's rule; no registration
  was attempted. On crates.io, `pdf-extract` is taken by an unrelated crate.
- **§17.2 Q6, the renderer's guarantees.** The 512 KiB stack guarantee for
  hostile deep-nesting renders depends on this workspace's
  `[profile.dev.package.*]` opt-levels, which a dependent workspace would not
  inherit. The render golden hashes must also be re-blessed whenever hayro,
  vello or pic-scale change in the lockfile.

## Decision

- Releases are GitHub Release assets of the public repository: six wheels,
  the sdist and `SHA256SUMS`, with notes from `docs/releases/v<version>.md`.
  The distribution name stays `pdf-extract`, and the import name stays
  `pdf_extract`.
- Consumers depend on a wheel by URL, pinned by hash: uv records the hash in
  `uv.lock`, and pip takes `--hash` with `--require-hashes`.
- Nothing is published to PyPI. The Rust crates stay `publish = false`.
- Q1 is closed by the above.
- Q6 is closed for wheels. They are built in this workspace, so its profiles
  apply and the stack guarantee holds as tested. The golden re-bless is a
  step of `RELEASING.md`. Publishing the crates would reopen Q6.

## Alternatives considered

- **PyPI under another name.** This is the most conventional route, but it
  means choosing and defending a new public name now, and adding a publishing
  credential or trusted-publisher setup. The owner chose GitHub Releases.
- **Try PyPI `pdf-extract` first.** It would very likely be refused under the
  collision rule, and a refusal would leave the question open anyway.
- **Keep releases internal.** Consumers would need repository credentials at
  install time, which is what this decision removes.
- **Publish the crates.** The crates.io name is taken, the stack guarantee
  does not carry to dependent workspaces, and no consumer needs the Rust API.

## Consequences

- Every consumer's lockfile names a URL and hash per platform wheel, and an
  upgrade means editing those entries.
- Without an index there is no `pip install pdf-extract` and no resolver
  choosing a platform wheel for you. The README shows the per-platform marker
  form.
- `RELEASING.md` is the release procedure, and the `Wheels` workflow refuses
  a version with no committed release notes.
- A later move to PyPI only adds a channel; nothing here prevents it.

## Addendum (2026-11-02)

v0.1.1, the first public release, was published from the public repository's tag `v0.1.1`:
- `Wheels` run 41522501002 passed every job and attached six wheels, the sdist, `SHA256SUMS` and the committed notes.
- An aarch64 abi3 wheel was installed anonymously by URL plus hash (`pip install --require-hashes`) on a clean `ubuntu:26.04` with its own Python 3.14.

