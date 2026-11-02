# Proposal

## Why

Phase 4 is adoption. The first consumer is a separate application that currently extracts with PyMuPDF. It rolls out in four steps: shadow, switch, improve, remove (spec §15.3). That work happens in the consumer's own repository, after the consumer's in-flight deployment change lands. That change moves it to Ubuntu 26.04, system Python 3.14 and uWSGI. It also needs things pdf-extract does not yet have:

1. **A way to install pdf-extract without credentials.**
   - The repository is internal, and v0.1.0 exists only as a GitHub Release asset on it.
   - The project owner decided (2026-10-22) to make pdf-extract a public project with releases independent of any consumer. Consumers install from those public releases.
2. **A public tree that names no client and holds no private history.**
   - The current history contains a client's name in 47 tracked files, a work email address on hundreds of commits, and one commit authored by a tool account.
   - The owner chose to publish a fresh public repository starting from one squashed, scrubbed commit. The internal repository becomes a read-only archive, and all further work happens in the public one.
3. **A way to measure parity without shipping PyMuPDF.**
   - The original rollout ran PyMuPDF beside pdf-extract in production (shadow mode). That keeps the AGPL dependency in every release until the switch, which is the licensing problem pdf-extract exists to solve. Its quadratic comparison would also compete with real extractions for their deadlines.
   - The owner decided (2026-10-22) that parity is measured offline instead, on a snapshot of the consumer's stored PDFs, with the project's private-corpus run. Then one cut-over release replaces PyMuPDF.
   - The §13.2 metrics lived only inside `crates/core/tests/`. They become a core library module, so the gates have one definition.
4. **Proof that the wheels install on the consumer's target platform.**
   - The production-image test covers Ubuntu 24.04 with CPython 3.13. The consumer is moving to Ubuntu 26.04 with system Python 3.14.

The owner chose that this change stays planning-only on the consumer side. pdf-extract delivers the public release and a generic migration kit. The consumer-specific Phase 4 plan is written into the consumer's repository, to be applied there once its deployment change lands.

## What Changes

- **Parity module (Rust only).** `pdf_extract_core::parity` compares a reference extraction with pdf-extract's page by page, returning the §13.2 metrics without retaining either text, and scores pages against the §13.2 gates. The parity tests and the offline private-corpus run use it. The owner decided there is no Python binding.
- **PyMuPDF migration guide.** A generic guide (`docs/migrating-from-pymupdf.md`) replaces the consumer-specific §15. It covers the call mapping, the behaviour changes, failure containment (§8.5) and the rollout: measure offline, cut over once, improve. There is no production shadow.
- **Tree scrub, enforced by a test.**
  - Client names, client-internal identifiers and internal-only notes are removed or neutralised.
  - The wordlist test gains those terms, so they cannot come back.
  - Authorship lines in exported patch files use the public project identity.
  - An ADR records this one-time redaction. The archived originals stay in the internal repository, and the public ADR bodies differ from them only by the documented redaction mapping.
- **Open questions closed.**
  - Spec §17.2 Q1 (name): no package index. Distribution `pdf-extract`, import `pdf_extract`, released as public GitHub Release assets. Rust crates stay `publish = false`.
  - Q6 (renderer guarantees): resolved for a wheels-only release. Wheels are built in this workspace, so its profile settings hold. The golden re-bless goes on a new `RELEASING.md` checklist.
- **Production-image test.** It also covers Ubuntu 26.04 with the distribution's Python 3.14 (the abi3 wheel) and `tesseract-ocr-eng`, the consumer's target platform.
- **BREAKING (repository):** a new public repository is created from a squashed, scrubbed snapshot. The internal repository is renamed and archived, read-only. The existing v0.1.0 release stays there.
- **First public release, v0.1.1.**
  - Built by `wheels.yml` in the public repository: the full run deferred from the last change, including the `license` gate.
  - Its release notes call out that v0.1.0 lacked the Rust standard library notices.
  - Install instructions use a uv direct-URL source pinned by hash.
- **Consumer handoff.** A planning document for the consumer's Phase 4 is written into the consumer's repository as an OpenSpec change proposal, not implemented. It covers:
  - all PDF extraction in a killable child process per document, with a wall-clock limit, capped retries and quarantine;
  - an offline parity run on a snapshot of its stored PDFs (gate A), plus a downstream check against the PyMuPDF output it already stores;
  - the dependency, a public release URL plus hash in `uv.lock`;
  - sequencing after its deployment change;
  - one cut-over release that replaces PyMuPDF and drops it, then the improve step.

## Capabilities

### New Capabilities

- `public-distribution`:
  - the public repository's provenance (a scrubbed snapshot) and the enforced absence of client or internal references;
  - where releases are published and how consumers install them;
  - the release checklist.
- `migration-parity`: the core page-comparison and gate-scoring module, the offline corpus run built on it, and the migration guide's offline rollout.

### Modified Capabilities

- `packaging`:
  - the production-image requirement adds Ubuntu 26.04 with system Python 3.14;
  - the release workflow requirement states that releases come from the public repository, with release notes.

## Impact

- **Code:**
  - a new `parity` module in `crates/core`, used by the parity tests;
  - `strsim` moves from a dev-dependency to a normal dependency; it is MIT and already in `Cargo.lock`, so THIRD_PARTY_NOTICES is regenerated;
  - Rust unit tests, and a Python test that runs the guide's examples.
- **CI:** `tools/build/prod-image-test.sh` and the `wheels.yml` `prod-image` matrix (a 26.04 entry per architecture), plus the wordlist test.
- **Docs:**
  - a new migration guide and `RELEASING.md`;
  - README (install from public releases, no client names);
  - a scrub across `docs/`, ADRs, OpenSpec archives, THEORY and ARCHITECTURE;
  - a new ADR for the public snapshot and the redaction, and one for Q1/Q6.
- **Repositories (outward-facing; each step needs explicit owner confirmation at the time):**
  - creating the public repository;
  - pushing the snapshot;
  - archiving and renaming the internal repository;
  - the public release;
  - writing into the consumer's repository.
- **No change** to the extraction API or behaviour.
- **Out of scope:**
  - implementing the consumer's Phase 4;
  - publishing to PyPI or crates.io;
  - revisiting the CI cost caps now that public repositories get free Actions minutes (noted as follow-up);
  - the consumer's own licence.
