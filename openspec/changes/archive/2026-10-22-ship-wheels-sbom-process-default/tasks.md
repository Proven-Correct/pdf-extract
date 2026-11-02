# Tasks

## 1. Build foundations: version source, abi3, manylinux build

- [x] 1.1 Make `pyproject.toml` take its version from Cargo (`dynamic = ["version"]`, remove the literal), per design D9. Verify that `maturin build --sdist` writes `PKG-INFO` with `Version: 0.1.0`, and that a new pytest asserts `importlib.metadata.version("pdf-extract")` equals the helper handshake version (the workspace version).
- [x] 1.2 Raise the Python floor to 3.13 (D11).
  - `pyproject.toml`: `requires-python = ">=3.13"`, ruff `target-version = "py313"`, mypy `python_version = "3.13"`.
  - `ci.yml`: drop the 3.11 entry from the `python` matrix.
  - README: the supported-versions statement.

  Run `ruff check` (applying any new pyupgrade fixes) and `mypy`. Verify that both are clean and that the pytest suite passes on 3.13 and 3.14.
- [x] 1.3 Add the `abi3 = ["pyo3/abi3-py313"]` feature to `crates/python/Cargo.toml`, with a comment explaining why it is opt-in (D3). Build with `maturin develop --release --features abi3` and run the full pytest suite plus stubtest. Replace or gate any binding call outside the limited API. Verify that the suite passes on both the abi3 and the default build.
- [x] 1.4 Build the vendored Tesseract, the extension and the helper inside digest-pinned `quay.io/pypa/manylinux_2_28_x86_64` and `_aarch64` images (the latter on an arm64 host or runner). Fix any build issue through the vendor patch series. Verify that both builds complete, and that `tools/vendor/check-c-sources.sh` and `tools/vendor/test-check-patches.sh` still pass.
- [x] 1.5 Write `tools/build/build-wheel.sh <abi3|ft> [--out DIR]` (D4), with a header comment.
  - It stages the helper with the wheel's target, profile and features, and generates the native SBOM (group 2).
  - It then runs `maturin build --release --locked` with the platform's compatibility, interpreter, `--features abi3` when building abi3, and `--sbom-include`.
  - A wrapper mode, `--docker <arch>`, runs it inside the pinned manylinux image with the cargo home and target directory mounted.

  Add `tools/build/test-build-wheel.sh`, covering argument parsing and the refusal of unknown ABIs, in the style of `test-stage-helper.sh`. Verify that the script produces `pdf_extract-0.1.0-cp313-abi3-manylinux_2_28_x86_64.whl` locally, and that shellcheck and the test script pass.

## 2. SBOM

- [x] 2.1 Confirm maturin 1.15's `[tool.maturin.sbom]` keys from its schema. Set `rust = false`, with a comment citing the keys' source and why maturin's document is unfit (design D5, revised). Verify that a built wheel without `--sbom-include` has no `sboms/` Rust document.
- [x] 2.2 Write `tools/license/sbom.py` (D5; named to be importable by its tests, like `tools/fuzz/ledger.py`), with a header comment.
  - It writes `rust.cdx.json`: the `cargo tree -e normal --target <triple>` crates of the extension (with the wheel's features) and of the helper, with licence, `pkg:cargo` purl, and no absolute paths.
  - It reads `vendor/PROVENANCE.md` and the staged helper path, and writes `native.cdx.json` (CycloneDX 1.5).
  - The native document has components for Tesseract and Leptonica (version, SPDX licence, URL, archive SHA-256 and `pedigree.patches` from `vendor/patches/{tesseract,leptonica}/`), pedigree notes for the vendored hayro crates, and the helper as an `application` with its SHA-256 and `dependsOn` edges.
  - It fails loudly on a provenance row it cannot parse.

  Add `cyclonedx-python-lib` (with JSON validation) to the dev dependency group. Add `tools/license` to the ruff, mypy and CI lint paths, as `tools/fuzz` is.
- [x] 2.3 Add `tests/python/test_sbom.py`. It covers schema validity of both documents, the Rust crate set against `cargo tree` (including that `loom` and build-only crates are absent), agreement with `PROVENANCE.md` (planting a changed SHA-256 must fail the agreement check), the patch lists, the helper hash, and the refusal of a malformed provenance row. Verify that the tests pass and that the new module is fully covered.
- [x] 2.4 Write the SBOM half of `tools/build/verify_wheel.py`.
  - Validate every document in `sboms/`.
  - Require one Rust document and one native document.
  - Compare the Rust components with `cargo metadata --filter-platform <triple>` normal dependencies of `pdf-extract-python`: no missing crate, and no dev-only or build-only crate.
  - Compare the native components with `PROVENANCE.md`.

  Add tests using synthetic wheels: a missing document, an extra dev crate, and a wrong archive hash must each fail. Verify that the tests pass.

## 3. Wheel verification and the per-push `wheel` job

- [x] 3.1 Complete `tools/build/verify_wheel.py` (D6) with the static checks.
  - Wheel tags against the expected matrix entry.
  - The helper is present with mode `0755` in its zip external attributes.
  - Linux helper: an ELF parse checks that `NEEDED` is within the manylinux_2_28 allow-list and that the highest `GLIBC_`/`GLIBCXX_` symbol versions are within the policy.
  - macOS helper: `minos` is no newer than the wheel's platform tag.
  - There is no grafted-libs directory.
  - `LICENSE`, `THIRD_PARTY_NOTICES` and `README-AFM.txt` in `.dist-info/licenses/` are byte-equal to the repository's.

  Add tests with synthetic wheels for each failure: no helper, a non-executable helper, an over-new glibc symbol, a missing licence file, and a wrong tag. Verify that the tests pass.
- [x] 3.2 Write `tools/build/smoke.py`. It must not import from the repository.
  - It builds a PDF with `pdf_extract.testing.PdfBuilder`.
  - It asserts `page.text()`.
  - It runs `page.ocr()` with no engine and asserts the expected words and `isolation`.
  - It takes `--expect-isolation` so it can run before the default flip in group 4. Group 4 changes the default argument to `process`.
  - With `--gil-off-check`, it asserts that the GIL stays disabled.

  Verify that it passes against a locally installed wheel in a fresh venv outside the checkout.
- [x] 3.3 Add the `wheel` job to `ci.yml`, with a comment block in the file's style.
  - It runs `build-wheel.sh --docker x86_64 abi3`, then `verify-wheel.py`, then `smoke.py` in a fresh venv, and then the full pytest suite against the installed wheel, without a coverage gate.
  - It uses the tessdata action and the vendored-C++ cache key.

  Verify that the job runs green via `gh workflow run ci.yml --ref <branch>` before landing.
- [x] 3.4 Write the wheel build, verification and release model ADR (`adr_index.py --next`, template), covering D3–D8 and citing ADR 0076 and ADR 0087. Update `ARCHITECTURE.md` (the build-wheel and verify-wheel scripts, the `wheel` job, the abi3 feature, the single version source) and the README's build-from-source section. Verify that `adr_index.py docs/adr` exits 0.

## 4. Python process-isolation default (internal #53)

- [x] 4.1 In `crates/python/src/ocr.rs`, add the binding-local default engine (D1). It is a `OnceLock<Result<OcrEngine, OcrUnavailable>>`, built with `Isolation::Process` and `helper_dir = package_dir(py)`, the directory computed before `py.detach`. `default_engine` (and thus `page.rs`) switches to it. Change the `OcrEngine` `isolation` keyword default to `"process"`. Extend the `render_pixels` error to name `isolation='in_process'`, and the `engine_not_built` detail for the package directory to name `tools/build/stage-helper.sh`. Verify that `cargo clippy` passes with `-D warnings`.
- [x] 4.2 Update `python/pdf_extract/_native.pyi` (the `isolation` default) and the `OcrEngine` and `default_ocr_engine` docstrings: the default, the opt-out, and the new lock in the fork hazard. Add the lock to the `pdf-extract-ocr` crate's `Forking` section. Verify that stubtest, mypy and `cargo doc -D warnings` pass.
- [x] 4.3 Update the Python tests that assumed the in-process default.
  - Pin `isolation="in_process"` where the test is about in-process behaviour, for example `test_ocr_fork.py::test_fork_in_process` and the silence records in `test_no_output.py`.
  - Assert the new default where the test is about defaults.

  Verify that `pytest tests/python` passes with the 95% coverage gate.
- [x] 4.4 Add Python tests for the new requirements.
  - `OcrEngine().isolation == "process"`.
  - `default_ocr_engine().info().isolation == "process"`, using the helper beside `_native`.
  - The helper aborting under `Page.ocr()` with no engine raises `OcrPageError`, and the interpreter survives.
  - With the helper hidden in a child interpreter, `default_ocr_engine()` raises `OcrUnavailableError(reason="engine_not_built")`. A second call returns the stored error without a new search.
  - `OcrEngine(max_render_pixels=80_000_000)` raises `ValueError` containing `66977776` and `isolation='in_process'`, and the same value with `isolation="in_process"` is accepted.

  Verify that the tests pass in the `python` and `python-freethreaded` jobs.
- [x] 4.5 Add a Rust test asserting that `pdf_extract_ocr::default_ocr_engine()` still reports `Isolation::InProcess`. Verify with `cargo test -p pdf-extract-ocr --test default_engine`.
- [x] 4.6 Switch `smoke.py`'s default expectation to `process`, and drop the explicit `--expect-isolation` from the `wheel` job. Verify that the `wheel` job is green on dispatch.
- [x] 4.7 Write the ADR for the Python process-isolation default. It partly supersedes ADR 0061 for Python, recording D1 and D2, and follows the ADR 0086/0035 pattern (0061 stays Accepted, with an addendum pointing at the new record).
  - Update the README OCR section, including a BREAKING note on `max_render_pixels` and the source-install helper requirement.
  - Update THEORY (why Python and Rust differ).
  - Amend design-spec §7.6 and §17.1's Tesseract-abort row.

  Verify that `adr_index.py docs/adr` exits 0, and close internal #53 from the PR body with a closing keyword on its own line.

## 5. Release workflow and production-image test

- [x] 5.1 Write `.github/workflows/wheels.yml` (D8), triggered only by `push: tags: ['v*']` and `workflow_dispatch`, with comment blocks in `ci.yml`'s style.
  - `version`: the tag must equal `v` plus the project version.
  - `sdist`: an archive check confirms that the vendored C/C++ trees and `stage-helper.sh` are present.
  - `build`: {linux-x86_64 on `ubuntu-24.04`, linux-aarch64 on `ubuntu-24.04-arm`, macos-arm64 on `macos-14`} × {abi3, ft}, each running build, verify, then smoke. The free-threaded jobs run smoke with `PYTHON_GIL=0 --gil-off-check`.
  - `release`: tag only. It writes `SHA256SUMS` and runs `gh release create --verify-tag`.

  Verify that actionlint and shellcheck pass on the workflow.
- [x] 5.2 Add the `prod-image` job (D7). On each Linux architecture's native runner, a digest-pinned `ubuntu:24.04` container installs `tesseract-ocr-eng` and a pinned uv, creates a `uv venv --python 3.13`, runs `uv pip install --no-index` of the abi3 wheel, and runs `smoke.py` with no `TESSDATA_PREFIX`. Verify that the job is green in the dispatch run (task 5.3).
- [x] 5.3 Dispatch `wheels.yml` on the branch. Verify that the run produces six wheels and one sdist as artifacts, that all `build` and `prod-image` jobs are green, and that no release or tag was created.
- [x] 5.4 Build a wheel from the dispatched sdist in a clean container (no repository checkout). Verify that the result passes `verify-wheel.py` and `smoke.py`, which satisfies the sdist requirement.
- [x] 5.5 Amend the design spec:
  - §14.2: abi3 ≥ 3.13, cp314t only, Windows and macOS x86_64 not built, Ubuntu 24.04 as the production image.
  - §14.3: the SBOM location and documents.
  - §16: the exit criterion is met by `prod-image`.
  - §17.2 Q1: GitHub Releases only; the name is still open.

  Add a README "Installing a release" section, covering pip from the release asset URL with a token and the tessdata requirement, plus the release procedure (bump the workspace version, merge, tag). Verify that the README's commands match the workflow.

## 6. Integration and release

- [x] 6.1 Run the full local pre-push checks: fmt, clippy `-D warnings`, `cargo test --workspace`, the pytest suite with coverage, ruff, mypy, stubtest, shellcheck and `cargo deny`. Then run the complete `ci.yml` on `main` after the last PR lands. Verify that everything is green, including the new `wheel` job.
- [x] 6.2 Ask the user before pushing the release tag, because a tag publishes a release. On approval, tag `v0.1.0` (or the agreed version). Verify that the GitHub Release carries six wheels, the sdist and `SHA256SUMS` whose digests match, and that its `prod-image` jobs are green. This step meets the Phase 3 "wheels install on the production OS image" exit criterion.
