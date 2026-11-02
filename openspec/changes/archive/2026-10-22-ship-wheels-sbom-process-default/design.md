# Design

## Context

See proposal.md, "Why". This section covers only the state that shapes the approach.

- **Build.**
  - maturin 1.15 builds a mixed layout: `python/` plus `crates/python`, giving `pdf_extract._native`.
  - `tools/build/stage-helper.sh` builds `pdf-extract-ocr-helper` with cargo and copies it into `python/pdf_extract/`. maturin's `include` then ships it (ADR 0076).
  - CI only ever runs `maturin develop`, so nothing builds or inspects a real wheel.
- **Vendored C++.** The default `vendored` feature compiles Tesseract 5.5.3 and Leptonica 1.87.0 through the `cmake` crate and links them statically into both the extension and the helper. A cold build is about 10 minutes per architecture.
- **Python API.**
  - PyO3 is pinned at 0.29.3 with no `abi3` feature, so today's builds are version-specific.
  - The binding uses `PyBuffer`, which needs the limited API of 3.11 or later.
  - `requires-python = ">=3.11"` today. This change raises it to 3.13 (D11).
- **Default engine.** Python's `default_ocr_engine()` and `Page.ocr()` with no engine both call the Rust `pdf_extract_ocr::default_ocr_engine()`. That engine is built from `OcrConfig::default()` with `helper_dir: None`. Under process isolation it would look for the helper beside the *interpreter* (internal #53, fix pass comment).
- **Raster cap.** `OcrConfig::validate` already caps `max_render_pixels` at `MAX_PROCESS_RENDER_PIXELS` (66,977,776) under `Process`, and the Python binding raises `ValueError` for it. So internal #56 B10 is handled; only the message needs the remedy.
- **Provenance.** `vendor/PROVENANCE.md` already records each C/C++ archive's URL, SHA-256 and commit.
- **maturin SBOM support.** maturin 1.15 embeds cargo-cyclonedx and writes `.dist-info/sboms/`. It has a `[tool.maturin.sbom]` table and `--sbom-include`, and its auditwheel repair step writes `sboms/auditwheel.cdx.json`.
- **Cost cap.** The repository is INTERNAL and Actions minutes are billed. ADR 0085 sets per-push CI on `main` only. ADR 0087 sets no scheduled workflows.
- **Versions.** The workspace version (`0.1.0`) and `pyproject.toml`'s version are declared separately. The helper's version handshake compares crate versions.

## Goals / Non-Goals

**Goals:**
- One build script produces every wheel the same way locally and in CI, with the helper staged by construction rather than by a remembered step.
- A wheel that would fail a user (no helper, wrong glibc floor, missing SBOM or licences) never reaches a release.
- Python OCR is crash-contained by default, with no behaviour change for Rust callers.

**Non-Goals:**
- Reproducible (bit-identical) wheels.
- Signing wheels or attestations beyond GitHub's own release integrity.
- Making the CI pipeline's other jobs build from wheels.
- Changing helper discovery for Rust binaries.

## Decisions

### D1. Python gets its own default engine; Rust's is untouched

`crates/python/src/ocr.rs` gains a binding-local `static DEFAULT: OnceLock<Result<OcrEngine, OcrUnavailable>>`. It is built from `OcrConfig { isolation: Process, helper_dir: package_dir(py), ..OcrConfig::default() }`. `default_engine` (used by `default_ocr_engine()` and by `page.rs`) switches to it.

- **Construction.** `package_dir` needs the interpreter, so it is computed before `py.detach`. Construction then runs detached, as today (O21).
- **Same semantics as the Rust default (O12).** An `Err` is stored. A panic is not stored, because it propagates out of `get_or_init` and leaves the cell empty.
- **Fork list.** The new cell joins the fork-hazard lock list in the `pdf-extract-ocr` crate docs, the `OcrEngine` and `default_ocr_engine` docstrings, and the README.

*Alternatives:*
- **Flip `OcrConfig::default()`.** Rejected (user decision). Every Rust caller, including tests and examples, would need the helper beside its executable.
- **A Rust `default_ocr_engine_with(config)` keyed by config.** Rejected. It is more API for one caller.

### D2. No automatic fallback to in-process

When the helper is missing, Python's default engine fails with `engine_not_built`, and that failure is stored.

Falling back silently would turn a broken install into an uncontained one, which is the exposure internal #53 removes. Source installs must run `stage-helper.sh`, as they already must for `isolation="process"`. The error message names the script. The explicit `isolation="in_process"` remains the opt-out.

### D3. abi3 through an opt-in crate feature, free-threaded as version-specific

`crates/python` gets a feature `abi3 = ["pyo3/abi3-py313"]`. The build script turns it on for the abi3 wheels and leaves it off for cp314t.

- Free-threaded CPython has no stable ABI, so it needs the non-abi3 build.
- Keeping abi3 off by default keeps `maturin develop` and the existing CI jobs on full-API builds.
- The per-push `wheel` job runs the whole pytest suite against the installed abi3 wheel, so any divergence of the limited API shows up on every push. Coverage is not gated there; the `python` job keeps that gate.

*Alternative:* `abi3-py313` always on, relying on PyO3 to ignore it for free-threaded interpreters. Rejected: it depends on PyO3's fallback behaviour and changes every dev build.

### D4. One build script, run inside the manylinux image on Linux

`tools/build/build-wheel.sh <abi3|ft> [--out DIR]` does three things:

1. It stages the helper with the same cargo target, profile and features as the wheel.
2. It runs `maturin build --release --locked` with `--compatibility manylinux_2_28` on Linux, `--features abi3` for abi3, and the matching interpreter (`/opt/python/cp313-cp313` or `cp314-cp314t` in manylinux, `uv python find 3.13 / 3.14t` on macOS).
3. It writes the native SBOM (D5) and passes it with `--sbom-include`.

On Linux, CI and developers invoke the script inside `quay.io/pypa/manylinux_2_28_{x86_64,aarch64}` (digest-pinned) through `docker run`. They mount the repository, a cargo home and a target directory from the host so `actions/cache` can keep them. rustup installs the `rust-toolchain.toml` pin inside the container. Both the helper and the extension are compiled against the image's glibc 2.28.

- **macOS.** The script runs natively on `macos-14` with `MACOSX_DEPLOYMENT_TARGET=11.0`, exported for both cargo invocations so the helper and the extension share a floor.
- **aarch64.** It builds on native `ubuntu-24.04-arm` runners, not cross-compiled. Cross-compiling the vendored C++ through cmake is the riskiest part of the build, and the native runner removes it.

*Alternative:* `PyO3/maturin-action` with `before-script-linux` to stage the helper. Rejected: it cannot be run locally the same way, and it hides the container invocation that the helper also needs.

### D5. SBOM: two generated documents; maturin's own Rust SBOM is off

*Revised during apply (task 2.1).* maturin 1.15's built-in cargo-cyclonedx document (`[tool.maturin.sbom] rust`, on by default) was inspected on a real wheel. It is unfit for three reasons:

- It resolves dependencies for all targets, so `loom` (a `cfg(loom)` dependency) and other platforms' crates are scoped `required`. On aarch64-apple-darwin that is 115 components against the 92 crates of the real graph.
- Its `bom-ref`s embed the build machine's absolute paths.
- It lists build-only crates, scoped `excluded`.

So `[tool.maturin.sbom] rust = false`, and `tools/license/gen-sbom.py` writes both documents at build time. `build-wheel.sh` passes them with `--sbom-include`. They cannot be committed, because they carry the helper's hash and depend on the target.

- **Rust document (`rust.cdx.json`).**
  - Contents: the crates in `cargo tree -e normal --target <triple>` of `pdf-extract-python` (with the wheel's features) and of the helper (`pdf-extract-ocr`, default features), deduplicated. Each entry has its version, licence expression and `pkg:cargo` purl.
  - The vendored hayro crates (path sources) carry a `pedigree` note naming `vendor/patches/hayro/`, so a reader can see they are not the crates.io bytes.
  - Our own crates are listed with `GPL-3.0-or-later`.
  - Absolute paths never appear: path sources are recorded relative to the repository.
- **Native document (`native.cdx.json`).**
  - Tesseract and Leptonica as `library` components, with version, SPDX licence, download URL and `hashes` (SHA-256), read from `vendor/PROVENANCE.md`. Each has a `pedigree.patches` list naming the files under `vendor/patches/{tesseract,leptonica}/`.
  - The helper as an `application` component, with the SHA-256 of the staged file and `dependsOn` edges to both libraries and to the Rust root.
- **auditwheel.** maturin's `auditwheel` SBOM stays at its default. Nothing should be grafted, and the verifier refuses a grafted-libs directory anyway.
- **Checks.**
  - `tools/build/verify-wheel.py` validates every document under `sboms/` against the CycloneDX schema, using `cyclonedx-python-lib` (a dev-group dependency, never a runtime one).
  - It compares the Rust components with a fresh `cargo tree` for the wheel's target.
  - It compares the native components with `PROVENANCE.md`.
  - It refuses absolute paths in any document.
  - The generator has its own pytest tests.

*Alternatives:*
- **maturin's built-in Rust SBOM.** Rejected for the reasons above. This was the plan before inspection.
- **cargo-cyclonedx run separately.** Rejected. It is the same generator as maturin's, with the same all-targets behaviour, plus a second tool pin.
- **One merged document.** Rejected. PEP 770 allows several documents, and two keep the cargo-derived and provenance-derived halves independently testable.

### D6. Wheel verification is a script, run in three places

`tools/build/verify-wheel.py WHEEL` runs two kinds of checks.

**Static checks:**
- the filename tags against the expected matrix entry;
- the helper is present, with mode `0755` in the zip's external attributes;
- for the Linux helper, its `NEEDED` libraries are within the manylinux_2_28 allow-list and its highest `GLIBC_`/`GLIBCXX_` symbol versions are within the policy. maturin's auditwheel step audits only the extension, so the script checks the helper itself, parsing the ELF in pure Python;
- for the macOS helper, `minos` is no newer than the wheel's platform tag;
- there is no `.libs` directory;
- the license files are byte-equal to the repository's;
- the SBOM checks from D5.

**Dynamic checks:** `tools/build/smoke.py` runs in a fresh venv outside the checkout.

- It builds a two-line PDF with `pdf_extract.testing.PdfBuilder`, which ships in the wheel.
- It asserts `page.text()`.
- It runs `page.ocr()` with no engine and asserts the words and `isolation == "process"`.

The smoke script and the verifier run in the release build jobs, in the per-push `wheel` job, and, for the smoke script only, inside `ubuntu:24.04` for the production-image test (D7).

### D7. Production-image test in a stock container

For each Linux architecture, a job on the matching native runner runs `docker run ubuntu:24.04`. Inside the container it does the following:

- installs `tesseract-ocr-eng` and `ca-certificates` from apt;
- installs a pinned uv release;
- runs `uv venv --python 3.13`, then `uv pip install --no-index <wheel>`;
- runs `smoke.py` with no `TESSDATA_PREFIX`, so the engine finds `/usr/share/tesseract-ocr/5/tessdata` through its known-directory search (O9).

`--no-index` proves there are no runtime dependencies (§14.2).

Ubuntu 24.04's own `python3` is 3.12, below the floor. Production runs a uv-managed CPython 3.13 on that image, so the test does the same; the distribution Python is not used. The uv release and the CPython version uv resolves are pinned.

The container image is pinned by digest in the workflow. Bumping the digest is a deliberate change.

### D8. Workflows and cost

**`wheels.yml`** runs on `push: tags: ['v*']` and `workflow_dispatch` only.

- `version` (Ubuntu, seconds): on a tag, it fails unless `v$(project version)` equals the tag.
- `sdist`: `maturin sdist`. A check asserts that the vendored C/C++ trees and `stage-helper.sh` are inside the archive.
- `build` matrix: {linux-x86_64, linux-aarch64, macos-arm64} × {abi3, ft}. Each job runs `build-wheel.sh`, then `verify-wheel.py`, then `smoke.py`, using the tessdata action's pinned model, and uploads its wheel.
- `prod-image`: {x86_64, aarch64}. It needs `build` and runs D7.
- `release`: tag only. It needs everything above, writes `SHA256SUMS`, and runs `gh release create v… --verify-tag` with the files. On dispatch, the files remain as workflow artifacts.

**Release cost.** One tag costs about 6 builds × (10–25 min) + 2 × 5 min + small jobs, which is roughly 3 to 5 runner-hours. The macOS minutes are billed at the macOS multiplier.

**`ci.yml` gains one job, `wheel`.** It runs the linux-x86_64 abi3 build through the same script, the verifier, the smoke test, and then the full pytest suite against the installed wheel (D3). Its cache key covers the vendored C/C++ like the other jobs.

ADR 0085's trigger policy is unchanged. A new ADR records the wheel release model and its cost envelope next to ADR 0087.

### D9. Single version source

`pyproject.toml` declares `dynamic = ["version"]`, and maturin takes the version from `crates/python` (the workspace version). The Python package version and the helper handshake version can then no longer drift. Releasing is: bump `[workspace.package] version`, merge, then tag `v<version>`.

### D10. Spec and ADR bookkeeping

- **New ADR: the Python process-isolation default** (internal #53). It partly supersedes ADR 0061 for Python only, and records D1 and D2. ADR 0061's status is updated per the supersede rule.
- **New ADR: wheel build, verification and release model.** It covers D4–D8 and cites ADR 0076, which it does not supersede: the helper still lives in the package directory.
- **Design-spec amendments:**
  - §7.6 (default isolation per language)
  - §14.2 (abi3 ≥ 3.13; cp314t only; Windows not built; Ubuntu 24.04 as the production image)
  - §14.3 (SBOM location and contents)
  - §16 (exit criterion met by the `prod-image` job)
  - §17.2 Q1 (names still open; releases go to GitHub only)

### D11. Python floor 3.13

The supported floor rises from 3.11 to 3.13 (user decision). The abi3 wheel is `cp313-abi3`.

- **Changes that move with it:**
  - `requires-python`;
  - ruff `target-version`;
  - mypy `python_version`;
  - the classifiers;
  - the `ci.yml` `python` matrix, where the 3.11 entry is dropped and 3.13 and 3.14 remain.
- **Code that may simplify:** Python-side code written for 3.11 compatibility may be simplified, but that is not required by this change.
- **Free-threaded builds:** they stay at 3.14t, because PyO3 0.29 supports nothing earlier.

## Risks / Trade-offs

- **[Risk]** The manylinux_2_28 image's gcc-toolset may not build vendored Tesseract unchanged. → The first task builds both architectures in the container before any workflow is written. Any vendored change goes through the normal patch series (`vendor/patches/`) and its CI drift check.
- **[Risk]** The limited API breaks a binding call. `PyBuffer` is fine from 3.11, but others may not be. → Task 1.3 builds with `--features abi3` and runs the full suite first. Any non-limited call is replaced or gated, and the per-push `wheel` job keeps it honest.
- **[Risk]** A future maturin turns its Rust SBOM back on, or renames the `sbom` keys. → The keys come from maturin 1.15's `maturin.schema.json`. The verifier requires exactly our two documents (plus an optional auditwheel one), so it fails closed on an unexpected document.
- **[Risk]** Every OCR call from Python now pays IPC and the helper's memory. Measured overhead is 0.3–1.6% (internal #53). macOS helper spawn costs about 130 ms with a high fd limit (internal #56 B12), once per pool slot. → This is documented in the README and THEORY, and `isolation="in_process"` remains the opt-out.
- **[Risk]** Python 3.11 and 3.12 users can no longer install the package. → This is accepted (user decision). The first consumer is the only one, and its production image runs a uv-managed 3.13.
- **[Risk]** Python callers who raised `max_render_pixels` above 66,977,776 start getting `ValueError`. → The message names the opt-out, and the README and changelog call it out as BREAKING.
- **[Risk]** Source and dev installs without a staged helper lose default OCR. → The error names `tools/build/stage-helper.sh`. The README's source-install section already requires it. CI stages it in every Python job.
- **[Trade-off]** There is no per-push free-threaded or aarch64 wheel check. A regression specific to those builds is caught only at release or on a manual dispatch. This is accepted under the cost cap. `python-freethreaded` already covers the free-threaded code paths with `maturin develop`.
- **[Trade-off]** Wheels go to GitHub Releases on an INTERNAL repository, so consumers need repository access (a token for `pip install` from the release URL). This is accepted until §17.2 Q1 is decided.

## Migration Plan

1. Land the build script, verifier, SBOM and abi3 feature, and the `wheel` CI job, with the default unchanged. The per-push job is green.
2. Land the Python default flip (internal #53), with tests and docs.
3. Land `wheels.yml`. Dispatch it on the branch, and confirm six verified wheels and a green `prod-image`.
4. Bump the version if needed, tag `v0.1.0`, and confirm the release assets.

**Rollback.** The default flip is one keyword default and one cell in `crates/python`. Reverting that PR restores in-process OCR without touching packaging. Release workflows can be disabled with no effect on per-push CI.

## Open Questions

- Which interpreter the macOS smoke test uses for abi3 (3.13 from uv, or the runner's). This does not affect the specs, since either satisfies them.
