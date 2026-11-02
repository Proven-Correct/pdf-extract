---
id: 0009
title: Our own code is GPL-3.0-or-later; dependencies stay permissive-only
date: 2026-10-02
status: Accepted
supersedes: 0008
summary: The workspace crates, the Python package and LICENSE move to GPL-3.0-or-later while L1 keeps third-party dependencies permissive.
---

# 0009. Our own code is GPL-3.0-or-later; dependencies stay permissive-only

## Context

Supersedes ADR 0008. On 2026-10-02 the user decided to relicense the project's own code (decision D-40,
run spec revision 9). The shipped dependency graph is a separate matter: L1 still bans GPL, LGPL and AGPL code there.

## Decision

Our own code (workspace crates, Python package, `LICENSE`) is `GPL-3.0-or-later`; the shipped dependency graph stays
permissive-only. Delivered as unit 12b: `LICENSE` is the canonical GPL-3.0 text (its SHA-256 is pinned in
`crates/testkit/tests/license.rs`), `license = "GPL-3.0-or-later"` in every workspace `Cargo.toml` and
`pyproject.toml` (plus the GPLv3+ classifier), vendored crates keep their upstream `Apache-2.0 OR MIT`. In
`deny.toml` the workspace crates are `publish = false` and `[licenses.private] ignore = true` keeps them out of the
license check, with the dependency allow-list unchanged.

## Alternatives considered

**A scoped cargo-deny exception** for our own crates was rejected: it would have to be edited for every new crate,
whereas `private` covers all unpublished workspace members by construction. No other alternative is recorded; the
rationale for the license itself is the user's licensing choice for the project.

## Consequences

Everything shipped alongside our code is GPLv3-compatible: hayro and PyO3 are `Apache-2.0 OR MIT`, the Adobe AFM/AGL
data is BSD-3-style, Tinos is `Apache-2.0`, and the rest of the graph is MIT/Apache-2.0/BSD/ISC/Zlib/Unicode-3.0, so
the combined wheel can be distributed under GPL-3.0-or-later. Consumer note: a GPL-3.0-or-later wheel used by
a consumer places that consumer's distribution, if any, under the GPL's source-availability obligations for the combined
work; use inside one organisation without distribution carries none. Every workspace crate must stay
`publish = false`. A negative proof (`tools/license/check-deny-rejects-copyleft.sh`, run by the `deny` CI job) shows
that GPL, LGPL and AGPL dependencies are still rejected. PEP 639 deprecates license classifiers; revisit when
packaging for PyPI.
