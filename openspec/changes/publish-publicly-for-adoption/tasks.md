# Tasks

## 1. Parity API (D1)

- [x] 1.1 Add `pdf_extract_core::parity` with `compare_page`, `PageComparison`, `ParityAccumulator` (`add`, `merge`, `report`) and `ParityReport` with gate verdicts, plus a header comment. The similarity histogram has 1e-5 resolution and bounded memory. Move `strsim` to a normal dependency of core, and regenerate THIRD_PARTY_NOTICES with `gen-notices.sh`. Add unit tests for every `migration-parity` scenario: identical pages, the similarity definition, routing disagreement, both pages empty, gates passing, the routing gate failing, merging, and an empty accumulator. Verify with `cargo test -p pdf-extract-core`, clippy, the `deny` drift check, and coverage at 100% for the module.
- [x] 1.2 Switch `crates/core/tests/parity.rs::page_metrics` and `gate_b` to `compare_page` and `ParityAccumulator`, keeping the existing p10 gate and adding p1 to the report. Verify that the parity tests give the same pass/fail and the same per-page similarities as before on corpus B and C (compare the old and new `parity-report*.json`).
- [x] 1.3 No Python binding (owner decision, 2026-10-22, when production shadowing was dropped). A binding was built and tested, then removed. Verify that `pdf_extract` exports none of the parity names (stubtest and the package's `__all__` are unchanged from `main`), and that the parity module's only callers are the Rust parity tests and the gate-A run.

## 2. Migration guide (D1, spec "Migration guide uses the parity API")

- [x] 2.1 Write `docs/migrating-from-pymupdf.md`. It names no consumer and covers the call mapping, the behaviour changes with their parity settings, containment (§8.5) and the offline rollout: measure with `run_private_corpus.sh` plus a downstream check, cut over once, improve. There is no production shadow. Make spec §15 point to it with a short generic summary. Add a pytest that extracts and runs every Python example in the guide. Verify that the test passes and the guide renders.

## 3. Production-image test on Ubuntu 26.04 (D4)

- [x] 3.1 Extend `tools/build/prod-image-test.sh` with `--image 24.04|26.04`. Each image is pinned by digest, and the 26.04 path uses apt `python3`, `python3-venv` and `tesseract-ocr-eng` with a `--no-index` install. Update its header comment and the `wheels.yml` `prod-image` matrix to {24.04, 26.04} × {x86_64, aarch64}. Verify locally on aarch64 for both images with a locally built Linux abi3 wheel (`build-wheel.sh --docker aarch64 abi3`). Also verify with shellcheck and actionlint.
- [x] 3.2 Update ARCHITECTURE (release workflow, prod-image) and the packaging-related text in spec §14. Verify by reading both against `wheels.yml`.

## 4. Release plumbing (D5, D7)

- [x] 4.1 Make `wheels.yml`'s `version` job fail a tag with no `docs/releases/v<version>.md`, and make `release` use that file as its notes. Write `docs/releases/v0.1.1.md`, noting the Rust standard library notice gap in v0.1.0. Bump the workspace version to 0.1.1, which carries through to `Cargo.lock` and to pyproject via dynamic version. Verify with actionlint, a dispatch dry run of the `version` logic in a local script test, and the existing version tests.
- [x] 4.2 Write `RELEASING.md`: the version bump, golden re-bless and review when the hayro, vello or pic-scale lock entries changed, notices regeneration, release notes, tag, and `SHA256SUMS` check. Add the README section "Installing a release": a uv direct URL plus the hash from `SHA256SUMS`, with no credentials. Close §17.2 Q1 and Q6 in the design spec, and record the decision in a new ADR. Verify that `adr_index.py docs/adr` exits 0, and that a test asserts every workspace member is still `publish = false` (existing license test) and that `RELEASING.md` names the golden re-bless.

## 5. Scrub and enforcement (D2)

- [x] 5.1 Build the redaction mapping and record it in a new ADR, which is also the one-time exception to the no-rewrite rule for ADR bodies. The mapping covers:
  - the client system and client-internal identifiers;
  - the survey's section labels, rewritten to "internal survey §n": the PyMuPDF usage survey (`docs/reference/pymupdf-api-surface.md`) describes the consumer's code and is removed from the tree (owner decision, 2026-11-01);
  - the work address in prose;
  - internal `#NNN` issue and PR references and commit SHAs, rewritten to a non-linking "internal #NNN" or "internal commit SHA" form, including `fuzz/ledger.json`'s session commits.

  Apply it across the tracked tree: docs, ADRs, OpenSpec specs and archives, README, THEORY, ARCHITECTURE, code comments and tests. Do not edit `vendor/patches/**` (D2: the replay in 6.0 and 6.1 regenerates their `From:` lines). Verify that `tools/vendor/test-check-patches.sh` and the `vendor-patches` CI job still pass, and that `git grep -i` for each source term, outside `vendor/patches/` and the wordlist test, finds nothing.
- [x] 5.2 Add the blocked terms to `crates/testkit/tests/wordlist.rs`, with a test that planting a term fails. Run the full local check set and CI (dispatch on the branch). Then send the scrubbed tree to an adversarial reviewer instructed to find any remaining client, internal or tool reference, including in binary-adjacent text and fixture metadata. Verify that CI is green and every review finding is resolved.
- [x] 5.3 After the PRs for groups 1–5 merge, dispatch `wheels.yml` on internal `main` as the full rehearsal, a billed run that the owner confirms first. Verify that every job passes, including `license` and the four `prod-image` entries.

## 6. Publication (D3, D5) — tasks 6.1–6.3 each need the owner's explicit confirmation when they run

- [x] 6.0 Write `tools/release/build-public-history.sh SRC DEST`, with a header comment. It builds D3's history in DEST from the internal repository SRC: the pristine imports with their four tags, the range-1 patches replayed with `git am` (their commit messages' internal issue references qualified, ADR 0097), the codec import, the range-2 patches, `export-patches.sh` to regenerate `From:`, then the snapshot commit. Everything is authored by the public identity. The script refuses to finish unless:
  - DEST's head tree equals SRC's `main` tree, ignoring in the patch files the `From:` lines and the internal-reference qualifiers ADR 0097 adds to the patches' commit messages;
  - the regenerated series equals the committed one except for those lines;
  - `test-check-patches.sh` passes in DEST.

  Rehearse it into a scratch directory, and add a test of its refusal paths (shellcheck plus a scripted check). Verify by running it against the internal `main`, which must pass all three checks, and by listing DEST's history: four tags, one patch commit per committed patch (35 as of 2026-11-01), three import commits, one snapshot, one author.
- [x] 6.1 Export: run `build-public-history.sh` against the scrubbed internal `main` into a fresh repository. Before pushing, grep the result once more for every source term, the work domain and tool names, in both the tree (including `vendor/`, which the wordlist test skips) and every commit message, grepping a saved copy of `git log --all -p` rather than a pipe into `grep -q`. Verify that the script's three checks pass, and that `git log --format='%ae' | sort -u` shows only the public identity.
- [x] 6.2 Rename the internal repository to `pdf-extract-internal` and archive it. Create the public `Proven-Correct/pdf-extract`, push `main` and the four vendor tags (they are lightweight, so name them: `git push origin main 'refs/tags/vendor/*'`), and enable Actions. Verify that the repository is public, `main` holds the snapshot, and CI dispatched on it is green.
- [x] 6.3 Tag `v0.1.1` in the public repository. Verify that `wheels.yml` passes all jobs and the GitHub Release carries six wheels, the sdist, `SHA256SUMS` and the committed notes. Then install a wheel anonymously by URL plus hash on a clean `ubuntu:26.04` container. This also discharges the deferred task 7.3 of `complete-license-pipeline`.

## 7. Consumer handoff (D6) — needs the owner's confirmation

- [ ] 7.1 Write the Phase 4 planning proposal into the consumer's repository as a new OpenSpec change (proposal, design and tasks; no code). It covers:
  - the offline parity run on a snapshot of the stored PDFs (gate A via `run_private_corpus.sh`) and the downstream check against the PyMuPDF output already stored, neither running PyMuPDF inside the application;
  - one cut-over release: pdf-extract replaces PyMuPDF, all PDF extraction runs in a child process per document under a wall-clock deadline with capped retries and quarantine, and `pymupdf` is dropped;
  - the dependency pinned by public URL plus hash;
  - sequencing after that repository's deployment change;
  - the improve step.

  Verify with that repository's `openspec validate --strict`. Leave it uncommitted for its owner unless the owner says otherwise.

## 8. Close-out

- [ ] 8.1 Update ARCHITECTURE, THEORY and README for the public state (repository locations, release source, the parity API) in the public repository, and update the ADR index. Verify that the docs match the code and `adr_index.py` exits 0.
