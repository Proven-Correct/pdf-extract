---
id: 0097
title: The public snapshot carries a one-time redaction, enforced by the wordlist test
date: 2026-11-01
status: Accepted
summary: Before the internal repository is exported as the public snapshot, the tracked tree is redacted once by a fixed mapping (the consumer's name and code identifiers, the usage survey and its section labels, the work address, internal issue/PR/commit references); this is the single sanctioned exception to never rewriting an Accepted ADR's body, and wordlist.rs keeps the source terms out afterwards.
---

# 0097. The public snapshot carries a one-time redaction, enforced by the wordlist test

## Context

The owner decided (2026-10-22) to publish pdf-extract as a fresh public
repository built from a squashed snapshot of the internal one (openspec change
`publish-publicly-for-adoption`, D2 and D3). The internal tree was written for
one consumer and an internal issue tracker, so it:

- named the first consumer, and quoted its module, function and setting names,
  its deployment units and its internal invariants;
- carried `docs/reference/pymupdf-api-surface.md`, a survey of that consumer's
  PyMuPDF usage that is mostly a description of the consumer's own code, cited
  throughout by a prefixed section label;
- referred to about 300 internal issues and pull requests as `#NNN`, and to a
  few internal commits by SHA. In a public repository these would dangle, or
  later resolve to unrelated public issues;
- named a work address as the author of the exported vendor patches.

Many of these sit in Accepted ADR bodies. The project rule is that an Accepted
ADR's body is never rewritten, only superseded or extended by an addendum.

## Decision

The tracked tree is redacted once, on an internal branch, by this mapping, and
the result is the tree that the public snapshot exports.

| Source (internal) | Public form |
|---|---|
| The first consumer's name | "the first consumer", "the consumer", or "a consumer" where the statement is general |
| The consumer's code identifiers (module, function, setting and test-file names, invariant numbers, deployment units) | a neutral description ("the consumer's paragraph splitter", "its web workers"), or dropped |
| The usage survey `docs/reference/pymupdf-api-surface.md` | removed from the tree (owner decision, 2026-11-01); it stays in the internal archive |
| The survey's section label for §n | "internal survey §n" |
| `#NNN`, `PR #NNN`, `issue #NNN` | "internal #NNN", "internal PR #NNN", "internal issue #NNN"; in a list or range, the first reference carries the qualifier |
| An internal commit SHA | "internal commit `<sha>`" |
| Wording that points at the consumer's domain (the document type it handles, in prose and examples) | generic "documents" and example file names (owner decision, 2026-11-02); generated fixture text is unchanged |
| Names of the development tooling and workflows (the ADR and docs workflows, the issue-fixing workflow, a local knowledge-graph tool) | a description of the work ("the ADR workflow", "a bug-fix pass", "the ordinary issue workflow"); the tool's ignore entry moves to a local exclude (owner decision, 2026-11-02) |
| The work address as patch author | the public project identity, by regenerating `vendor/patches/hayro/` from the replayed commits during the export, never by hand-editing the patch files |

The mapping changes names and references only. No decision, requirement or
measurement changes meaning. The redaction of the ADR bodies under
`docs/adr/` (0001, 0004, 0007, 0008, 0009, 0027, 0054, 0062, 0074, 0075,
0079, 0082–0093) is the one sanctioned exception to the
no-rewrite rule, made for publication. The internal archive keeps the
original bodies, which differ from the public ones only by this mapping.

`crates/testkit/tests/wordlist.rs` blocks the source terms (the consumer's
name, its code identifiers and their distinctive fragments, the survey
label, and the work domain), case-insensitively, on every tracked text file
outside `vendor/`. The test holds each term only as its length, a 12-bit
rolling-hash prefilter and its SHA-256, so the public tree spells none of
them anywhere, the test included. A digest can confirm a guess but cannot
be read. This record therefore describes the source terms by kind and does
not list them.

Two places outside the scanned tree carry internal references too, and are
handled where they are made:

- **The fork's own comments under `vendor/`.** They are qualified by an
  ordinary fork commit, exported as patch `0035` (comments only), so
  vendor = upstream + patches stays true.
- **The commit messages of the fork patches** (`Fixes #N`, `Refs #N` in
  `vendor/patches/hayro/*`). The internal series must stay byte-identical
  to the internal commits, so these are rewritten to the "internal #N" form
  only when the export replays them (task 6.0). The public series is then
  regenerated from the replayed commits, so it differs from the internal one
  in its `From:` lines and in those qualifiers. A bare `Fixes #N` in a
  public commit would otherwise close an unrelated public issue.

Internal references are not enforced by the test. Once the repository is
public, a bare `#NNN` is a legitimate public reference again.

## Alternatives considered

- **Supersede every affected ADR.** About twenty new records whose only
  change is a name or a reference. That buries the decision log in noise and
  leaves the originals, still unredacted, in the public tree.
- **Leave the ADRs unredacted.** This contradicts the owner's decision that
  the public tree names no client.
- **Keep the survey, with the consumer renamed.** It would still be a
  readable map of the consumer's private system. The owner chose to withhold
  it (2026-11-01).
- **Rewrite `#NNN` to bare text, with no qualifier.** The reader would lose
  the information that a reference exists and where it lives.

## Consequences

- The public decision log reads as one project's history, with no client
  context. Readers who need the original wording, the survey or the internal
  issues need access to the internal archive.
- "internal survey §n" citations in the design spec and run specs point to
  a document readers cannot open. The specs state each behaviour they rely
  on themselves, so the citation records only where a requirement came from.
- A new mention of a blocked term, or of the consumer's code, fails CI.
- The no-rewrite rule is unchanged for every later edit. This record is
  not a precedent.

## Addendum (2026-11-02)

The export ran once:
- `tools/release/build-public-history.sh` built the public history from internal commit `a7c8ecf`: 39 commits (three pristine imports, 22 + 13 replayed patches, one snapshot), the four vendor tags, and one author and committer identity.
- All three of its checks passed.
- A final search of the tree (including `vendor/` and binary files) and of every commit message found none of the source terms. The only hits were the plain terms that `wordlist.rs` must spell out, and chance two-byte matches inside the compressed image streams of three test PDFs.

The public repository's `main` is that snapshot (`dbcc847`). The internal repository was renamed and archived.

