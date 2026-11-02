## MODIFIED Requirements

### Requirement: Wheels install on the production OS image
For each Linux architecture, the release build SHALL install the abi3 wheel into a stock Ubuntu 24.04 image with the distribution's `tesseract-ocr-eng` and a uv-managed CPython 3.13 virtual environment. It SHALL also install it into a stock Ubuntu 26.04 image with the distribution's `tesseract-ocr-eng` and a virtual environment on the distribution's own Python 3.14. In each image there SHALL be no compiler and no Python package besides the wheel (only what creating the environment needs), and the release build SHALL run the smoke test there.

#### Scenario: Ubuntu 24.04 install
- **WHEN** the x86_64 and aarch64 abi3 wheels are installed in `ubuntu:24.04` containers on their native architectures
- **THEN** installation succeeds without building anything, and the smoke test passes using the distribution model under `/usr/share/tesseract-ocr/5/tessdata` [§16 Phase 3 exit criterion]

#### Scenario: Ubuntu 26.04 with system Python 3.14
- **WHEN** the x86_64 and aarch64 abi3 wheels are installed into a virtual environment created by the distribution's `python3` (3.14) in `ubuntu:26.04` containers on their native architectures
- **THEN** installation succeeds without building anything, and the smoke test passes using the distribution's `tesseract-ocr-eng` model

### Requirement: Release workflow triggers and outputs
The wheel release workflow SHALL run only on pushes of `v*` tags and on manual dispatch; it SHALL NOT run on a schedule, on branch pushes or on pull requests. On a tag whose version equals the package version, it SHALL attach the verified wheels, the sdist and a `SHA256SUMS` file to a GitHub Release for that tag in the public repository. The release SHALL carry notes that name the changes since the previous release. On manual dispatch, on any ref, it SHALL upload the same files, `SHA256SUMS` included, as workflow artifacts and create no release. It SHALL NOT publish to any package index.

#### Scenario: Tag release
- **WHEN** tag `v0.2.0` is pushed and `pyproject.toml` declares version `0.2.0`
- **THEN** a GitHub Release `v0.2.0` carries six wheels, one sdist and `SHA256SUMS`, whose digests match the files [§16]

#### Scenario: Tag and version disagree
- **WHEN** a `v*` tag is pushed whose version differs from the package version
- **THEN** the workflow fails before building, and no release is created

#### Scenario: Dispatch builds without releasing
- **WHEN** the workflow is dispatched manually on a branch
- **THEN** the files are uploaded as workflow artifacts and no release or tag is created [ADR-0087]

#### Scenario: Release notes present
- **WHEN** a tag release is created
- **THEN** its notes come from the committed release-notes file for that version and are not empty
