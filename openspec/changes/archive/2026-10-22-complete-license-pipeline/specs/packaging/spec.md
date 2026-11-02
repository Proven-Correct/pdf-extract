## MODIFIED Requirements

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
