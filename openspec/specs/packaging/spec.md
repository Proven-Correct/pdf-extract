# packaging Specification

## Purpose
Defines the distributed artifacts of the `pdf-extract` Python package: which wheels are built and for which platforms, what every wheel must contain (the extension, the OCR helper, the license files and CycloneDX SBOMs), how each wheel is verified before release, and how releases are produced within the project's CI cost cap.

## Requirements

### Requirement: Wheel matrix
The release build SHALL produce, for each of manylinux_2_28 x86_64, manylinux_2_28 aarch64 and macOS arm64, one stable-ABI wheel tagged `cp313-abi3` and one free-threaded wheel tagged `cp314t`. No other platform SHALL be built. Each Linux wheel SHALL pass manylinux_2_28 audit without grafting any shared library.

#### Scenario: Six wheels per release
- **WHEN** the release workflow completes for a tag
- **THEN** exactly six wheels exist, one per platform and ABI pair, and each filename carries the expected platform and ABI tags [§14.2]

#### Scenario: No grafted libraries
- **WHEN** a Linux wheel is audited
- **THEN** it meets manylinux_2_28, and the wheel contains no `pdf_extract.libs` (or other grafted-library) directory, because Tesseract and Leptonica are linked statically [§14.2]

#### Scenario: abi3 wheel loads on several interpreters
- **WHEN** the abi3 wheel for a platform is installed into CPython 3.13 and into CPython 3.14
- **THEN** `import pdf_extract` succeeds and text extraction works on both [§14.2]

### Requirement: Supported Python versions
The package SHALL declare `requires-python >= 3.13`, and SHALL NOT be installable on an earlier CPython.

#### Scenario: Older interpreter refused
- **WHEN** a user runs `pip install` of any built wheel or of the sdist on CPython 3.12
- **THEN** the installer refuses it because the Python version is not supported [§14.2]

### Requirement: Source distribution
The release build SHALL produce one sdist from which a wheel can be built with the vendored C/C++ sources and no network access beyond the Rust crate registry.

#### Scenario: sdist contents
- **WHEN** the sdist is listed
- **THEN** it contains the workspace crates, the vendored hayro crates and the vendored Tesseract and Leptonica sources, `LICENSE`, `THIRD_PARTY_NOTICES`, and the helper-staging script [§14.2; ADR-0076]

### Requirement: Every wheel ships the OCR helper
Every built wheel SHALL contain `pdf_extract/pdf-extract-ocr-helper`, built from the same commit and with the same Tesseract linkage as the extension, and installed with its executable permission.

#### Scenario: Helper present and executable after install
- **WHEN** any built wheel is installed into a fresh virtual environment
- **THEN** `pdf-extract-ocr-helper` exists beside `_native`, is executable, and passes the engine's version handshake at construction [internal #53; ADR-0076]

#### Scenario: Missing helper fails the build
- **WHEN** a wheel is built without the helper staged
- **THEN** wheel verification fails, and the wheel is neither uploaded nor released [internal #53; ADR-0076]

### Requirement: Every wheel ships its license files
Every built wheel and the sdist SHALL contain `LICENSE`, `THIRD_PARTY_NOTICES`, `README-AFM.txt` and the Rust standard library copyright document in their license-file location, byte-identical to the committed files.

#### Scenario: License files in the wheel
- **WHEN** a built wheel's `.dist-info/licenses/` is compared with the repository
- **THEN** the four files are present and identical to the committed ones [§14.3; ADR-0054]

### Requirement: Every wheel carries a CycloneDX SBOM
Every built wheel SHALL carry, under `.dist-info/sboms/`, CycloneDX JSON documents that together list every Rust crate linked into the extension (name, version, license expression, package URL), including the crates of the pinned toolchain's Rust standard library that are linked statically, and the vendored native components, Tesseract and Leptonica (version, license, the upstream archive's SHA-256 as pinned in `vendor/`), and the OCR helper as a component of the package.

#### Scenario: SBOM present and valid
- **WHEN** a built wheel is unpacked
- **THEN** `.dist-info/sboms/` holds at least one document for the Rust graph and one for the native components, and each validates against the CycloneDX 1.5 (or later) JSON schema [§14.3]

#### Scenario: Rust graph covered
- **WHEN** the crates of the extension's normal dependency graph for that wheel's target are compared with the SBOM components
- **THEN** every crate appears with its locked version, and no dev-only or build-only crate appears [§14.3]

#### Scenario: No build-machine paths
- **WHEN** any SBOM document of a built wheel is searched for absolute paths or `file:` URLs
- **THEN** none is found; path crates are recorded relative to the repository [§14.3]

#### Scenario: Native components match provenance
- **WHEN** the Tesseract and Leptonica entries are compared with the pinned versions and archive SHA-256s recorded under `vendor/`
- **THEN** they agree, and CI fails on any disagreement [§14.3; ADR-0080]

#### Scenario: Standard library covered
- **WHEN** the standard library crates of the pinned toolchain that are linked for that wheel's target are compared with the SBOM components
- **THEN** every one appears with its version and license expression, is marked as coming from the Rust toolchain rather than the project's lockfile, and carries no build-machine path [§14.3]

### Requirement: Every built wheel is verified before release
The release build SHALL verify each wheel by installing it into a fresh environment, outside the source tree, and running a smoke test: open a generated PDF and extract its text, then recognise one rendered page with the default (process-isolated) OCR engine. A wheel that fails verification SHALL NOT be released.

#### Scenario: Smoke test on each wheel
- **WHEN** a wheel is installed on its own platform with the matching interpreter (CPython 3.13 for abi3, 3.14t for free-threaded) and the English model available
- **THEN** text extraction returns the expected text, `Page.ocr()` returns the expected words, and `engine.info().isolation == "process"` [§16; internal #53]

#### Scenario: Free-threaded wheel keeps the GIL off
- **WHEN** the cp314t wheel is imported under `PYTHON_GIL=0`
- **THEN** `sys._is_gil_enabled()` is false [§14.2; C1]

### Requirement: Wheels install on the production OS image
For each Linux architecture, the release build SHALL install the abi3 wheel into a stock Ubuntu 24.04 image with the distribution's `tesseract-ocr-eng` and a uv-managed CPython 3.13 virtual environment, with no compiler and no Python package besides the wheel (only what installing uv needs), and SHALL run the smoke test there.

#### Scenario: Ubuntu 24.04 install
- **WHEN** the x86_64 and aarch64 abi3 wheels are installed in `ubuntu:24.04` containers on their native architectures
- **THEN** installation succeeds without building anything, and the smoke test passes using the distribution model under `/usr/share/tesseract-ocr/5/tessdata` [§16 Phase 3 exit criterion]

### Requirement: Release workflow triggers and outputs
The wheel release workflow SHALL run only on pushes of `v*` tags and on manual dispatch; it SHALL NOT run on a schedule, on branch pushes or on pull requests. On a tag whose version equals the package version, it SHALL attach the verified wheels, the sdist and a `SHA256SUMS` file to a GitHub Release for that tag. On manual dispatch, on any ref, it SHALL upload the same files, `SHA256SUMS` included, as workflow artifacts and create no release. It SHALL NOT publish to any package index.

#### Scenario: Tag release
- **WHEN** tag `v0.2.0` is pushed and `pyproject.toml` declares version `0.2.0`
- **THEN** a GitHub Release `v0.2.0` carries six wheels, one sdist and `SHA256SUMS`, whose digests match the files [§16]

#### Scenario: Tag and version disagree
- **WHEN** a `v*` tag is pushed whose version differs from the package version
- **THEN** the workflow fails before building, and no release is created

#### Scenario: Dispatch builds without releasing
- **WHEN** the workflow is dispatched manually on a branch
- **THEN** the files are uploaded as workflow artifacts and no release or tag is created [ADR-0087]

### Requirement: Per-push wheel check
The per-push CI pipeline SHALL build one manylinux_2_28 x86_64 abi3 wheel using the same build script as the release workflow, and SHALL fail when that wheel lacks the helper, the SBOM documents or the license files, or when process-isolated OCR fails from an installed copy of it.

#### Scenario: A change drops the helper from the build
- **WHEN** a commit on `main` changes the build so that the helper is not staged
- **THEN** the CI `wheel` job fails on that push [internal #53; ADR-0076]
