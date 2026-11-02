# licensing Specification

## Purpose

Keeps every third-party component that pdf-extract builds or ships permissively licensed, pinned to verifiable upstream sources, and acknowledged in the shipped notices, while the project's own code is GPL-3.0-or-later and no MuPDF or PyMuPDF code ever enters the project.

## Requirements

### Requirement: Project's own license
The system SHALL license the project's own code, meaning every Cargo workspace member and the `pdf_extract` Python package, under GPL-3.0-or-later. The repository `LICENSE` file SHALL be the canonical GPL-3.0 text, and every workspace member SHALL be unpublishable (`publish = false`).

#### Scenario: Workspace members declare GPL-3.0-or-later
- **WHEN** the workspace metadata is listed without dependencies
- **THEN** each of the six workspace members declares `license = "GPL-3.0-or-later"` and `publish = false` [0001 §4.12b; ADR-0009]
- **AND** `LICENSE` is byte-identical to the FSF `gpl-3.0.txt`, which a test checks by SHA-256 [0001 §4.12b]

#### Scenario: Python package declares GPL-3.0-or-later
- **WHEN** `pyproject.toml` is read
- **THEN** it declares `license = "GPL-3.0-or-later"` and the GPLv3+ classifier [0001 §4.12b; ADR-0009]

#### Scenario: No stale MIT claim for our own code
- **WHEN** `LICENSE`, `README.md`, `pyproject.toml` and the workspace `Cargo.toml` files are scanned
- **THEN** none of them claims MIT for the project's own code (vendored notices are excluded) [0001 §4.12b; ADR-0008; ADR-0009]

### Requirement: Permissive-only Rust dependency graph
The system SHALL reject, in CI, any Rust dependency, direct or transitive and with all features enabled, whose license is not on the allow-list: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, Unlicense, 0BSD, BSL-1.0, OFL-1.1. GPL, LGPL and AGPL dependencies, and all other copyleft or unlicensed crates, SHALL fail the build.

#### Scenario: Copyleft dependency rejected
- **WHEN** a crate licensed GPL-3.0, LGPL-2.1 or AGPL-3.0 enters the dependency graph
- **THEN** the `deny` CI job fails on the license check [L1; §14.3; 0001 §4.1; ADR-0010]

#### Scenario: Unlisted license rejected
- **WHEN** a dependency declares any license not on the allow-list, or declares no license
- **THEN** it is rejected because the policy is a pure allow-list [L1; 0001 §4.1; ADR-0010]

#### Scenario: Permissive dependency accepted
- **WHEN** every dependency's license expression is satisfied by the allow-list
- **THEN** the license check passes [§14.3; 0001 §4.1]

### Requirement: Own-crate exemption does not weaken the dependency policy
The system SHALL exempt from the dependency allow-list only the project's own unpublished workspace crates. CI SHALL prove on every run, against the repository's real policy file, that a copyleft dependency of such a crate is still rejected and a permissive one is still accepted.

#### Scenario: Negative proof runs in CI
- **WHEN** the `deny` job builds a throwaway GPL-3.0-or-later `publish = false` crate whose dependency is licensed GPL-3.0-only, LGPL-2.1(-only) or AGPL-3.0(-only)
- **THEN** each case fails the license check and an MIT control passes [L1; 0001 §4.12b; ADR-0009]

#### Scenario: Allow-list unchanged by the relicensing
- **WHEN** the project's own license moved to GPL-3.0-or-later
- **THEN** the dependency allow-list was not widened to admit GPL [0001 §4.12b; ADR-0009]

### Requirement: Dependency sources restricted to crates.io
The system SHALL fail CI when any Rust dependency comes from a registry other than crates.io or from a git repository.

#### Scenario: Git dependency refused
- **WHEN** a dependency is declared from a git URL
- **THEN** the `deny` sources check fails, since no git source is allowed [0001 §4.1; ADR-0080]

#### Scenario: Unknown registry refused
- **WHEN** a dependency comes from a registry other than the crates.io index
- **THEN** the `deny` sources check fails [0001 §4.1; ADR-0010]

### Requirement: Advisory and duplicate-version checks
The system SHALL fail CI when a Rust dependency has a known security advisory or is a yanked release. Duplicate versions of a crate SHALL be reported as warnings without failing the build. The advisory check SHALL also run weekly on a schedule, independently of any push, so that an advisory published after a release is noticed.

#### Scenario: Yanked crate fails
- **WHEN** `Cargo.lock` pins a yanked crate version
- **THEN** the `deny` advisories check fails [§14.3; ADR-0010]

#### Scenario: Duplicate versions warn only
- **WHEN** two versions of the same crate are in the graph
- **THEN** cargo-deny reports a warning and the job still passes [§14.3; 0001 §4.1]

#### Scenario: Advisory published after a release
- **WHEN** a RustSec advisory against a locked dependency is published and nobody pushes
- **THEN** the weekly scheduled advisory run fails within a week, on one runner, without building the project [§14.3]

#### Scenario: Scheduled run stays advisories-only
- **WHEN** the scheduled workflow runs
- **THEN** it runs only the cargo-deny advisories check, so its cost stays a few minutes of one runner per week, and it can also be started by hand [§14.3]

### Requirement: Vendored C/C++ license check
The system SHALL check in CI that the vendored Tesseract and Leptonica trees, which cargo-deny cannot see, name no license other than Apache-2.0 or BSD. Each copyright notice SHALL name an expected holder, and the set of holders found SHALL equal the expected set exactly. The one accepted exception is Leptonica's CC0-derived red-black tree files.

#### Scenario: Foreign license keyword fails
- **WHEN** a line in either vendored C/C++ tree names GPL, LGPL, AGPL, MPL, a Creative Commons license, MIT or another non-Apache/BSD license outside the named exceptions
- **THEN** the license check fails [L1; §14.3; 0002 §4.5]

#### Scenario: Holder set drift fails
- **WHEN** a notice names a copyright holder not expected for its tree, or an expected holder no longer appears
- **THEN** the license check fails [0002 §3.7; 0002 §4.5]

#### Scenario: Stale exception fails
- **WHEN** a named exception no longer matches any line in its file
- **THEN** the license check fails [0002 §4.5]

#### Scenario: Self-test proves the check bites
- **WHEN** the check's self-test plants a GPL string, a foreign holder and a dual SPDX tag in a scratch copy, and removes a holder
- **THEN** each plant is caught, the clean copy passes, and CI runs this self-test [0002 §4.5]

### Requirement: Vendored C/C++ sources pinned by checksum
The system SHALL build Tesseract 5.5.3 (Apache-2.0) and Leptonica 1.87.0 (BSD-2-Clause) from sources committed under `vendor/`. Each committed tree SHALL equal its upstream archive, verified against a recorded SHA-256 and pruned by a committed prune list, plus its exported patch series, and CI SHALL prove this.

#### Scenario: Reproducibility proof in CI
- **WHEN** the `vendor-patches` job downloads both archives, verifies their SHA-256, prunes them, applies `vendor/patches/{tesseract,leptonica}/` and diffs the result against `vendor/`
- **THEN** any difference, such as an unexported edit, a stale patch or a prune-list mismatch, fails the job [D4; 0002 §3.4; 0002 §4.5; ADR-0080]

#### Scenario: Checksum mismatch
- **WHEN** a downloaded archive's SHA-256 differs from the provenance row
- **THEN** the import fails before extraction [0002 §3.4]

#### Scenario: Tesseract pin carries its commit
- **WHEN** the Tesseract tag archive is fetched
- **THEN** its embedded commit id must equal the recorded commit [0002 §3.4; 0002 Q2]
- **AND** the pristine import is tagged `vendor/tesseract-pristine-1` and `vendor/leptonica-pristine-1` [0002 §3.4; ADR-0080]

### Requirement: Vendored hayro crates reproducible from crates.io
The system SHALL vendor exactly the hayro crates it patches (hayro-syntax 0.7.2, hayro-interpret 0.7.0, hayro-postscript 0.1.0, hayro-jbig2 0.3.0, hayro-jpeg2000 0.3.5). Each SHALL equal its crates.io tarball, identified by package SHA-256, plus one exported patch series, and CI SHALL prove this.

#### Scenario: Drift check in CI
- **WHEN** the `vendor-patches` job imports every provenance row fresh from crates.io, applies the patch series in name order and diffs against `vendor/`
- **THEN** any difference fails the job [D2; 0001 §3.2; 0003 §3.2; ADR-0080]

#### Scenario: Pristine tags mark the imports
- **WHEN** the vendored hayro trees are traced to their imports
- **THEN** `vendor/hayro-pristine-1` marks the import of the first three crates and `vendor/hayro-pristine-2` marks the import of the two image codecs [0001 §4.2; 0003 §4.3; ADR-0080]

#### Scenario: Unpatched hayro crates stay crates.io pins
- **WHEN** a hayro crate is used but not patched
- **THEN** it is pinned from crates.io and not vendored [ADR-0055; ADR-0080]

### Requirement: Vendored code keeps its upstream license
The system SHALL leave every vendored crate under its upstream license, `Apache-2.0 OR MIT` for the hayro crates, and SHALL NOT relicense vendored code under the project's own license.

#### Scenario: Vendored manifests unchanged by relicensing
- **WHEN** the license of each crate under `vendor/` is read
- **THEN** it contains MIT or Apache and does not contain GPL [0001 §4.12b; D1]

### Requirement: Generated third-party notices
The system SHALL ship a `THIRD_PARTY_NOTICES` file generated from the dependency graph of the Python extension module. It SHALL cover every linked Rust crate's license, including the crates of the pinned toolchain's Rust standard library linked into the shipped binaries, the Adobe Glyph List notice, the notices of data files that vendored crates embed, and the Tesseract and Leptonica license texts verbatim. CI SHALL fail when the committed file differs from a fresh generation.

#### Scenario: Drift fails CI
- **WHEN** the `deny` job regenerates `THIRD_PARTY_NOTICES` with the pinned generator
- **THEN** any difference from the committed file fails the job [§14.3; ADR-0054; 0002 §3.7; 0003 §3.6]

#### Scenario: Required sections present
- **WHEN** the notices file is checked by the license test
- **THEN** it carries each vendored hayro crate's license, each embedded asset notice, and both C/C++ license texts verbatim under headings that name the version and copyright holders [ADR-0054; 0002 §3.7; 0003 §4.3]

#### Scenario: No template placeholders
- **WHEN** the notices file contains an unfilled license-template copyright line
- **THEN** the license test fails [ADR-0054]

#### Scenario: Notices and policy agree
- **WHEN** the notices generator's accepted licenses are compared with the dependency allow-list
- **THEN** the two lists are identical, in the same order [ADR-0054]

#### Scenario: Build-only and dev-only crates excluded
- **WHEN** a crate is only a build dependency or a dev dependency
- **THEN** it does not appear in the notices because it is not shipped [ADR-0054; 0002 §3.7]

#### Scenario: Standard library acknowledged
- **WHEN** the notices file is checked by the license test
- **THEN** it names the pinned toolchain version, lists every standard library crate (with version and license expression) linked into the extension or the helper for each shipped target, and carries each listed crates.io crate's license text verbatim, all taken from that toolchain's own sources, and it points to the Rust project's standard library copyright document, which ships beside it [§14.3]

#### Scenario: Rust standard library copyright document ships verbatim
- **WHEN** the package's license files are compared with the pinned toolchain's own standard library copyright document
- **THEN** a committed copy of it is byte-identical to the toolchain's and is one of the package's license files [§14.3]

#### Scenario: Toolchain bump regenerates the standard library section
- **WHEN** `rust-toolchain.toml` pins a different Rust version and the notices are not regenerated
- **THEN** the `deny` drift check fails [§14.3; ADR-0054]

#### Scenario: Standard library licenses on the allow-list
- **WHEN** any standard library crate in the notices declares a license expression that the dependency allow-list does not satisfy
- **THEN** the notices generation fails [L1; §14.3]

#### Scenario: System runtimes stated, not shipped
- **WHEN** the notices file is read
- **THEN** it states that libc and the C++ runtime (libstdc++ on Linux, libc++ on macOS) are linked from the host system and not shipped in the wheel [§14.3; L1]

### Requirement: Package declares its license files
The system SHALL declare `LICENSE`, `THIRD_PARTY_NOTICES`, the Adobe AFM notice and the Rust standard library copyright document as the Python package's license files, so that any built distribution carries them.

#### Scenario: License files in package metadata
- **WHEN** `pyproject.toml` is read
- **THEN** `license-files` lists `LICENSE`, `THIRD_PARTY_NOTICES`, `crates/testkit/data/README-AFM.txt` and the committed Rust standard library copyright document [§14.3; 0001 §2.1; 0002 §3.7]

### Requirement: Clean-room rule
The system SHALL NOT contain MuPDF or PyMuPDF code, whether read, copied or translated. Behaviour SHALL be matched to PyMuPDF only by black-box observation of its outputs.

#### Scenario: Parity derived from observed output only
- **WHEN** a behaviour is aligned with PyMuPDF
- **THEN** the alignment comes from comparing outputs on documents, never from MuPDF or PyMuPDF source [L2; D9; ADR-0005]

#### Scenario: MuPDF excluded as a dependency
- **WHEN** the PDF engine is chosen
- **THEN** no AGPL or GPL engine such as MuPDF or Poppler is a dependency; the core is built on permissively licensed hayro crates [D1; L1]

### Requirement: Oracle kept out of the library, its tests and CI
The system SHALL NOT depend on PyMuPDF from the library, its tests or CI. Oracle expectations SHALL be generated locally in a separate development-only project and committed, and CI SHALL compare against those committed files without installing PyMuPDF.

#### Scenario: CI parity runs without PyMuPDF
- **WHEN** the parity and OCR quality gates run in CI
- **THEN** they read committed expectations and never import or install PyMuPDF [L2; 0001 §4.13; 0002 O34]

#### Scenario: PyMuPDF absent from the project lockfile
- **WHEN** the project's `uv.lock` is inspected
- **THEN** PyMuPDF does not appear; it is locked only in the separate `tools/oracle` project [L2; ADR-0005]

### Requirement: Imported fixtures carry provenance and license
The system SHALL record the origin and SHA-256 of every test fixture that the test kit did not generate. Every imported corpus-B fixture SHALL also record its license, and a test SHALL enforce both rules.

#### Scenario: Corpus-B entry missing license metadata
- **WHEN** a corpus-B manifest entry lacks `origin`, `license` or `license_note`
- **THEN** the fixtures manifest test fails [0001 §3.6; 0001 §4.14]

#### Scenario: Non-generated fixture without provenance
- **WHEN** a fixture not produced by the test kit has no provenance row, or its row records a different hash
- **THEN** the fixtures manifest test fails [0001 §3.6]

### Requirement: Notices cover every shipped crate
The system SHALL refuse to release a wheel when a crate in that wheel's Rust SBOM is not acknowledged in the wheel's `THIRD_PARTY_NOTICES`. Only the project's own workspace crates, covered by `LICENSE`, are exempt.

#### Scenario: Crate missing from the notices
- **WHEN** a wheel's Rust SBOM lists a non-workspace crate at a version that the wheel's `THIRD_PARTY_NOTICES` does not name
- **THEN** wheel verification fails and the release does not happen [§14.3; L1]

#### Scenario: Own crates exempt
- **WHEN** the SBOM lists a `pdf-extract-*` workspace crate
- **THEN** verification does not require a notices entry for it, because `LICENSE` covers it [§14.3; ADR-0009]

### Requirement: License checks rerun on release
The system SHALL run the full license check set on the exact commit being released, and SHALL NOT publish a release unless every check passes. The set is the cargo-deny checks, the copyleft negative proof, the vendored C/C++ license check and its self-test, and the notices drift check.

#### Scenario: Stale notices block a release
- **WHEN** a version tag is pushed on a commit whose committed `THIRD_PARTY_NOTICES` differs from a fresh generation
- **THEN** the release workflow's license job fails and no GitHub Release is created [§14.3]

#### Scenario: Copyleft dependency blocks a release
- **WHEN** a version tag is pushed on a commit whose dependency graph contains a crate licensed outside the allow-list
- **THEN** the release workflow's license job fails and no GitHub Release is created [L1; §14.3]

#### Scenario: Dispatch runs the checks too
- **WHEN** the release workflow is started by hand
- **THEN** the license job runs and gates the collected artefacts, as on a tag [§14.3]
