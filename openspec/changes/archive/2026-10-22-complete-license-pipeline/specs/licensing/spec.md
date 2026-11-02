## MODIFIED Requirements

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

## ADDED Requirements

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
