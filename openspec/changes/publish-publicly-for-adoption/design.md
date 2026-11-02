# Design

## Context

The motivation and the owner's decisions are in proposal.md, and the requirements are in `specs/`. The facts below shape the approach and were checked on 2026-10-22.

- **Repositories.**
  - `Proven-Correct/pdf-extract` is INTERNAL. v0.1.0 is a GitHub Release asset there.
  - The consumer's repository is a separate, private one with its own OpenSpec root. It has an in-progress deployment change that moves it to Ubuntu 26.04 and its system Python 3.14.
  - The consumer currently extracts in threads inside its web workers, with no timeout.
- **The scrub surface in pdf-extract.**
  - The client's name appears in 47 tracked files. The heaviest are `docs/design-spec.md` (18 lines), `docs/reference/pymupdf-api-surface.md`, `docs/specs/*`, ADRs, OpenSpec archives, README, THEORY and ARCHITECTURE.
  - A work address appears as the `From:` of the exported hayro patches.
  - Git history: hundreds of commits by the work address, and one commit authored by a tool account.
  - Corpus D is generated, not client data. The private corpus A was never committed (`tools/oracle/run_private_corpus.sh` reads it from outside the repository).
- **The PyPI name.** `pdf-extract` is unregistered, but `pdfextract` exists, and PyPI refuses names that collide once separators are removed. The owner chose public GitHub Releases only.
- **Ubuntu 26.04.1** (`ubuntu:26.04`, multi-arch index digest `sha256:c0987…b944`) has no Python preinstalled. apt provides `python3` and `python3-venv` 3.14.3, and `tesseract-ocr-eng` installs `/usr/share/tesseract-ocr/5/tessdata`.
- **Metric definitions in use.** `crates/core/tests/parity.rs::page_metrics` computes similarity as `1 − strsim::levenshtein(a, b) / max(chars(a), chars(b))`, with equal texts short-circuited to 1.0. `strsim` is a dev-dependency today.

## Goals / Non-Goals

**Goals:**

- One definition of the §13.2 metrics, used by the project's parity tests on the committed corpus and by the offline private-corpus run that consumers use on their own documents.
- No release of any consumer runs PyMuPDF beside pdf-extract. Parity is measured offline, and the cut-over release is the one that removes PyMuPDF.
- The public repository is clean by construction: a wordlist test keeps it clean, so the scrub is not a one-off that rots.
- Every irreversible or outward-facing step is a separate task that needs the owner's explicit confirmation when it runs.

**Non-Goals:**

- Implementing anything in the consumer's repository beyond writing its planning proposal.
- Publishing to PyPI or crates.io, or changing the import name.
- Rewriting the internal repository's history. It is archived as it is.
- Changing CI cost policy for the public repository (a follow-up).

## Decisions

### D1. The parity metrics are library code in core, used by the parity test too

A new public module, `pdf_extract_core::parity`, provides the definitions below. It is Rust-only; there is no Python binding (owner decision, 2026-10-22):

- `compare_page(reference: &str, candidate: &str) -> PageComparison`, with:
  - `routing_agrees`: the 50-character threshold after `trim`;
  - `similarity`: f64;
  - `exact`: equal after stripping;
  - `line_delta`: i64, candidate lines minus reference lines, counted as `'\n'`-separated parts of the stripped text.
  - Stripping is Python's `str.strip()` (Unicode whitespace plus U+001C–U+001F), as the existing gate code already defined it.
- `ParityAccumulator`, with `add`, `merge` and `report() -> ParityReport`. The report holds the page count, the routing rate, the median, p1 and p10 similarity, the exact rate, a line-delta histogram bucketed at |Δ| = 0, 1, 2 and >2, and gate verdicts.

Design points:

- **Memory is bounded.** The accumulator keeps a sparse similarity histogram of at most 100,001 buckets at 1e-5 resolution, so a large corpus does not grow it. Every gate threshold is a multiple of 1e-5, so verdicts are exact; reported percentiles are bucket floors, at most 1e-5 below the sample. Measured on corpus B: same pages, same verdicts, and p10 0.96790 against 0.9679012.
- **Why no Python binding.** A binding was built (with pickling, so per-process results could merge), and then removed when production shadowing was dropped. The only remaining consumer of the metrics is the offline run, which is Rust (`gate_a_private`).
- **One definition.** `parity.rs` switches to `compare_page`, so the test and the API cannot drift. The existing p10 gate in the corpus-B test stays as it is; p1 is added to the report.
- **Dependency.** `strsim` moves to a normal dependency of core. It is MIT, already in `Cargo.lock` and allowed by `deny.toml`. THIRD_PARTY_NOTICES is regenerated.
- **Alternatives considered:**
  - *Production shadow mode with a Python API:* this keeps PyMuPDF, an AGPL dependency, shipping in the consumer's releases until the switch, which is the problem the project exists to solve. Its O(nm) comparison would also compete with real extractions for their deadlines. Rejected by the owner on 2026-10-22.
  - *Leave the metrics to the consumer:* the metrics would drift from the gates.

### D2. The scrub is a mapping, applied once and then enforced

- **The mapping is recorded in the new ADR.** The client system becomes "the consuming application" or "the first consumer". Client-internal identifiers (function names quoted from its code, its spec's prefixed section labels) become neutral descriptions. The work address becomes the public project identity.
- **Order of work.** The scrub is applied in the internal repository on a branch, reviewed, merged and verified green. Then the snapshot is exported.
- **Patch `From:` lines are not edited internally.** `test-check-patches.sh` section 3 regenerates `vendor/patches/hayro/` from the real commits and compares the files byte for byte, so hand-edited authors would turn internal CI red. The public identity comes from the replay in D3 instead, which regenerates the files.
- **Internal references.** About 100 `#NNN` issue and PR numbers in the docs, plus commit SHAs such as those in `fuzz/ledger.json`, would dangle in the public repository, or later resolve to unrelated public issues. They are rewritten to a non-linking form ("internal #171", "internal commit 1ae9afd"). The ledger's per-session `commit` fields become the snapshot's provenance note, and the ledger's format is unchanged.
- **The PyMuPDF usage survey is withheld (owner decision, 2026-11-01).** `docs/reference/pymupdf-api-surface.md` described the first consumer's own code (module, function and setting names, deployment units, internal invariants), so renaming the consumer would still leave a readable map of its system. It is removed from the public tree and stays in the internal archive. Its section-label citations become non-linking "internal survey §n" references, like the issue numbers below, and the wordlist also blocks the consumer's code identifiers and their fragments.
- **Enforcement.** `crates/testkit/tests/wordlist.rs` gains the blocked terms. *Revised 2026-11-01 (review finding):* listing them in the test would have published them in that file, so the test holds each client term only as a SHA-256 digest with a 12-bit rolling-hash prefilter. The public tree spells none of them anywhere.
- **Fork comments and patch messages (2026-11-01).** The fork's own comments under `vendor/` cited internal issues by bare number; an ordinary fork commit qualifies them, exported as patch `0035`. The patches' commit messages (`Fixes #N`, `Refs #N`) cannot change internally without breaking the byte-identical regeneration check, so the export qualifies them when it replays the patches (D3).
- **ADR redaction.** The project rule is never to rewrite an Accepted ADR's body. A new ADR records the redaction as a one-time exception made for publication. The archived internal repository keeps the original bodies, and the public bodies differ only by the recorded mapping. No decision text changes meaning.
- **Alternatives considered:**
  - *Supersede each affected ADR:* dozens of records whose only change is a name.
  - *Leave the ADRs unredacted:* contradicts the owner's decision that the public tree names no client.

### D3. The export: a reconstructed vendor history, then the snapshot

A single squashed commit is not enough. The vendor tooling depends on history:
- `tools/vendor/test-check-patches.sh` section 3, which CI's `rust` job runs with `fetch-depth: 0`, needs the pristine tags;
- the same section regenerates the 35-patch series (as of 2026-11-01) with `git format-patch` over the commit ranges `hayro-pristine-1..hayro-pristine-2^` (22 patches) and `hayro-pristine-2..HEAD` (13 patches);
- `export-patches.sh` and `export-c-patches.sh`, which are how any future fork patch is made, need the same tags and commits;
- the licensing spec requires the four tags.

Every other section of that script builds its own throwaway repository.

So the public repository is built in this order, all authored by the public identity:

1. **Pristine imports.**
   - Commit A: the three hayro crates exactly as `vendor/hayro-pristine-1` holds them (`git archive` of that tag's `vendor/hayro-{syntax,interpret,postscript}`). Tag it `vendor/hayro-pristine-1`.
   - Commit B: Tesseract and Leptonica as `vendor/tesseract-pristine-1` holds them. Their current trees equal that import, and there are no C patches. Tag it with both C tags. It comes after A, as internally.
2. **Range-1 patches.** `git am --directory=vendor` of patches 0001–0022 from the committed files. Each replay keeps its subject, body and author date, with the author set to the public identity, and with internal issue references in the message qualified as ADR 0097 maps them.
3. **Commit C:** the two codec crates as `vendor/hayro-pristine-2` holds them. Tag it `vendor/hayro-pristine-2`. It has one parent, as the exporter requires.
4. **Range-2 patches.** `git am` of patches 0023 onward (0023–0035 as of 2026-11-01).
5. **The snapshot commit.** Everything else, so the head tree equals the scrubbed internal `main` exactly. The vendored trees already match, because `vendor-patches` proves vendor = upstream + patches. The commit message names the internal commit.

Then run `tools/vendor/export-patches.sh` once in the new repository. It changes only the `From:` lines of `vendor/patches/hayro/*`, to the public identity, and the internal-reference qualifiers ADR 0097 adds to the patches' commit messages, and that change is folded into the snapshot commit. Finally, `test-check-patches.sh` must pass there before anything is pushed.

The internal `v0.1.0` tag is not carried over. A rehearsal of this whole build in a scratch directory (task 6.0) comes before the real export.

- **Export target.** Push the result to the new public repository's `main`, and tag nothing beyond the four vendor tags until the first release.
- **Repository names.** The internal repository is renamed to `pdf-extract-internal` and archived. The public repository takes `Proven-Correct/pdf-extract`, so that the name and the README links stay stable.
  - GitHub's redirect from the old internal name stops once a new repository takes that name. This is acceptable: the internal repository is archived, and its v0.1.0 release stays reachable under the new name.
- **Repository settings.** Enable Actions on the public repository. The workflows need no secrets: `GITHUB_TOKEN` is enough for releases.
- **Alternative considered:** a public mirror of an internal canonical repository. The owner rejected it, because it needs a repeatable scrub-and-export at every release.

### D4. Production-image test gains Ubuntu 26.04 with system Python 3.14

- **Arguments.** `prod-image-test.sh` takes `--image 24.04|26.04`, and each image has its own pinned digest.
- **The 26.04 path:**
  - `apt-get install python3 python3-venv tesseract-ocr-eng`, which are what creating the environment needs;
  - `python3 -m venv`;
  - `pip install --no-index` of the wheel;
  - smoke from outside any checkout.
- **Matrix.** The `wheels.yml` `prod-image` job runs {24.04, 26.04} × {x86_64, aarch64}.
- **Why the abi3 wheel:** the consumer runs the distribution's Python 3.14, not 3.14t.

### D5. Releases: notes from a committed file; v0.1.1 first

- **Release notes.** `wheels.yml`'s `release` job reads `docs/releases/v<version>.md`. The `version` job fails a tag that has no such file. This replaces the fixed `--notes` text.
- **First public release.** v0.1.1. There is no API change, so it is a patch release. Its notes state that v0.1.0 (internal) lacked the Rust standard library notices.
- **Deferred check.** The tag run is the full `wheels.yml` run deferred as task 7.3 of `complete-license-pipeline`.
- **README.** It shows a `uv add "pdf-extract @ https://github.com/Proven-Correct/pdf-extract/releases/download/v0.1.1/<wheel>"` form, and how hashes come from `SHA256SUMS` into `uv.lock`.

### D6. The consumer handoff is a planning proposal in the consumer's repository

- **What is written.** One OpenSpec change directory (proposal, design, tasks; no implementation) in the consumer's repository, written after the owner confirms. It covers:
  - an offline parity run: a snapshot of the stored PDFs, scored on an internal machine with `tools/oracle/run_private_corpus.sh` (gate A); and a downstream check that re-extracts stored documents with pdf-extract (`dpi=72`, default output options), applies the application's own post-processing, and compares with the PyMuPDF output it already stores. Neither runs PyMuPDF inside the application.
  - the cut-over release: pdf-extract replaces PyMuPDF; all PDF extraction runs in one child process per document, where the parent enforces a wall-clock deadline, kills on expiry, caps retries and quarantines; and `pymupdf` is dropped.
  - the dependency, a public release URL pinned by hash.
  - sequencing: it starts after that repository's deployment change finishes.
  - the gates for cut-over and for the improve step, quoting the migration guide.
- **Why only a proposal:** the owner chose plan-only on the consumer side. That repository has its own active change and its own workflow.
- **No client terms in pdf-extract.** pdf-extract's public tree never names the consumer, so this task's artefact lives only in the consumer's repository.

### D7. Spec §17.2

- **Q1** is closed: no index, GitHub Releases, distribution `pdf-extract` and import `pdf_extract`.
- **Q6** is closed for wheels: they are built in this workspace, so its profile settings hold. Crates stay `publish = false`, and a crates.io publish would reopen Q6. The golden re-bless goes on `RELEASING.md`.
- An ADR records both.

## Risks / Trade-offs

- **The scrub misses a reference, and it becomes public.**
  - Mitigation: the wordlist terms are enforced before export.
  - A final review before pushing greps the exported tree for every mapping source term, the work domain and tool names.
  - Binary files are checked by listing. Fixtures are PDFs that the test kit builds or that were imported with provenance, and none comes from the consumer.
- **The squashed history loses `git blame` for public readers.** The internal archive keeps it, and ADRs and OpenSpec archives preserve the reasoning.
- **Making the repository public cannot be undone in practice.** Mitigation: it is the last step before the release, it is gated on the owner's confirmation, and the review comes first.
- **`strsim::levenshtein` is O(nm).** Very long pages, such as 50k-character data tables, cost about a second each.
  - Mitigation: the migration guide tells consumers to run comparisons inside the contained child, off the request path.
  - The API documents its cost. A future banded variant can be added without changing the result type.
- **The internal v0.1.0 release URL changes with the rename.** Nothing depends on it in production.
- **The replay could diverge from the internal series.** For example, `git am` might not reproduce a patch's exact context, or the head tree might differ from internal `main`.
  - Mitigation: the rehearsal (task 6.0) checks the regenerated series against the committed files, ignoring only the `From:` lines and the internal-reference qualifiers ADR 0097 adds to the patches' commit messages, and checks tree equality against internal `main`. Any difference stops the export.
- **Two repositories have changes in flight.** The handoff is only a new change directory in the consumer's repository. It does not touch that repository's in-flight change.

## Migration Plan

1. Implement the parity API, the guide, the 26.04 prod-image test, the release-notes plumbing and `RELEASING.md` in the internal repository. Use normal PRs, with CI green.
2. Scrub on a branch, add the wordlist enforcement and record the ADRs. Merge with CI green.
3. Dispatch `wheels.yml` on the internal `main` once, as a full rehearsal including the 26.04 prod-image test. The owner confirms the billed run.
4. Rehearse the export (reconstructed vendor history plus snapshot) in a scratch directory, and verify the tree, the patch series and `test-check-patches.sh`.
5. **Owner confirms:** export the snapshot and create the public repository.
6. **Owner confirms:** rename and archive the internal repository.
7. **Owner confirms:** tag v0.1.1 in the public repository, which releases it.
8. **Owner confirms:** write the handoff proposal into the consumer's repository.

Rollback: before step 4, ordinary reverts. After step 4, the public repository can be deleted only before anyone forks or clones it, so treat step 4 as irreversible.
