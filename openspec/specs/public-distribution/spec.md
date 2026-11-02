# public-distribution Specification

## Purpose

Defines how pdf-extract is published as a public project: the public repository starts from a scrubbed, squashed snapshot of the internal one; the tree carries no client or internal references; releases are public GitHub Release assets that any consumer can install without credentials; and each release follows a written checklist.

## Requirements

### Requirement: Public tree names no client or internal context
The tracked tree SHALL contain no name of a client or client system, no client-internal identifier, and no work email address other than the public project contact. The wordlist test SHALL enforce the blocked terms, case-insensitively, on every tracked text file. The only exceptions are exact (path, line substring) allow-list entries for third-party text.

#### Scenario: Planted client name fails
- **WHEN** a tracked text file contains a blocked client or internal term
- **THEN** the wordlist test fails and names the file and line

#### Scenario: Exported patches carry the public identity
- **WHEN** the `From:` lines of the public repository's exported vendor patch series are read
- **THEN** each names the public project identity, because they are regenerated from the replayed commits, and the `vendor-patches` reproducibility check still passes

#### Scenario: Internal references do not dangle
- **WHEN** the public tree is searched for references to the internal repository's issues, pull requests or commits
- **THEN** each is written in a non-linking form that names the internal archive (for example "internal #171"), so no reference resolves to an unrelated public issue

### Requirement: Public repository starts from a reconstructed vendor history and a squashed snapshot
The public repository's history SHALL consist only of:
- the pristine vendor imports, tagged `vendor/hayro-pristine-1`, `vendor/hayro-pristine-2`, `vendor/tesseract-pristine-1` and `vendor/leptonica-pristine-1`, in the internal repository's order;
- one commit per committed hayro patch, replayed from the patch files themselves, in series order between the same tags as internally;
- one squashed snapshot commit that brings the tree to the scrubbed internal tree.

Every commit SHALL be authored by the public project identity. The internal repository SHALL then be archived read-only with its full history, and all further development SHALL happen in the public repository. The vendor patch tooling and its regeneration check SHALL work unchanged in the public repository.

#### Scenario: Snapshot equals the scrubbed tree
- **WHEN** the public repository's head tree is compared with the internal commit it was exported from
- **THEN** they are identical except, in `vendor/patches/hayro/*`, for the `From:` lines, which name the public identity, and for the internal issue references in the patches' commit messages, which read "internal #N"

#### Scenario: No private history
- **WHEN** the public repository's history is listed
- **THEN** it holds only the pristine imports, the replayed patch commits and the snapshot commit, and no commit is authored by a work address or a tool account

#### Scenario: Patch tooling works in the public repository
- **WHEN** `tools/vendor/test-check-patches.sh` runs in a full clone of the public repository
- **THEN** it passes: the four pristine tags resolve, and `export-patches.sh` regenerates `vendor/patches/hayro/` byte-identically from the replayed commits

### Requirement: Releases are public and installable without credentials
Releases SHALL be published as GitHub Release assets of the public repository, and to no package index. The distribution name SHALL be `pdf-extract` and the import name `pdf_extract`. The Rust crates SHALL stay unpublished. The README SHALL show how to depend on a release by direct URL pinned by SHA-256, as uv and pip support.

#### Scenario: Anonymous install
- **WHEN** a host with no GitHub credentials installs a release wheel by its public URL and the hash from `SHA256SUMS`
- **THEN** the installation succeeds

#### Scenario: Crates stay unpublished
- **WHEN** the workspace manifests are read
- **THEN** every member still declares `publish = false`

### Requirement: Release checklist
The repository SHALL carry a release checklist (`RELEASING.md`) that a release follows. It covers:
- the version bump;
- the render golden hashes, re-blessed and reviewed after any change to the hayro, vello or pic-scale lockfile entries;
- regenerating the notices after any toolchain or lockfile change;
- the release notes;
- the tag and workflow run;
- checking the published `SHA256SUMS`.

#### Scenario: Golden re-bless is on the checklist
- **WHEN** `RELEASING.md` is read
- **THEN** it requires the render golden hashes to be re-blessed and reviewed whenever the hayro, vello or pic-scale lockfile entries changed since the last release [§17.2 Q6]
