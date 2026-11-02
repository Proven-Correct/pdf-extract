# Proposal

## Why

Phase 3 of the design spec (§16) also requires wheels and an SBOM. Its exit criterion is "wheels install on the production OS image". Today CI never builds a wheel. The only wheel is whatever `maturin develop` installs locally, and a build that skips `tools/build/stage-helper.sh` silently ships no OCR helper (ADR 0076). Nothing generates the CycloneDX SBOM that §14.3 promises.

Internal issue #53 asks to make process isolation the default for Python OCR. The measured overhead (1.6% and 0.3%) is well under the 10% bar in §7 Q1, and the flip contains Tesseract aborts (R1). The flip is blocked on exactly the gaps above, as the fix-pass comment on internal #53 records:

1. Every wheel must ship the helper, and CI must prove it.
2. Python's default engine must be able to find the helper.
3. Callers who raised `max_render_pixels` must get a clear error.

This is the second of the three Phase 3 changes. The license pipeline follows it.

## What Changes

- **Release wheels.** A new workflow, `.github/workflows/wheels.yml`, builds the §14.2 matrix. It runs on `v*` tags and on `workflow_dispatch` only, never on a schedule or on a push (cost cap, ADR 0087).
  - Linux: manylinux_2_28 x86_64 and aarch64, on native runners.
  - macOS: arm64.
  - Python: one abi3 wheel per platform for CPython ≥ 3.13, plus a cp314t wheel for free-threaded CPython.
  - The workflow also builds an sdist.
  - Each wheel is built with the helper staged first, then verified in a fresh environment:
    - the helper is present and executable;
    - the SBOM and the license files are present;
    - the wheel's tags are the expected ones;
    - a smoke test runs text extraction and process-isolated OCR.
  - On a tag, the verified wheels, the sdist and their SHA-256 sums are attached to a GitHub Release. Nothing is published to PyPI, so the package-name question (§17.2 Q1) stays open.
  - Windows is not built. It is "optional; no current consumer" in §14.2, and the helper's pipe and process code is Unix-only.
- **abi3 and the Python floor.** The extension is built against the stable ABI (`abi3-py313`).
  - **BREAKING:** the supported floor rises from 3.11 to 3.13. `requires-python`, the lint and type-check targets, and the CI interpreter matrix move with it, and §14.2's "abi3 ≥ 3.10" is amended to 3.13.
  - `PyBuffer` needs the 3.11 limited API, which 3.13 covers. Free-threaded CPython has no stable ABI, so it gets its own wheel per version (cp314t now). cp313t is not built, because PyO3 0.29 supports free-threading from 3.14t.
- **Production-image install test.** For each Linux architecture, the release workflow installs the built wheel into a stock `ubuntu:24.04` container, then runs the smoke test. The container has the distribution `tesseract-ocr-eng` and a uv-managed CPython 3.13, because Ubuntu 24.04's own `python3` is 3.12 and production runs a uv-managed 3.13. That test is the §16 exit criterion.
- **Per-push wheel check.** `ci.yml` gains one `wheel` job (Linux x86_64 only). It builds a manylinux wheel the release way, then asserts that the helper and the SBOM are in it and that process-isolated OCR works from an installed copy. This is internal #53's "a wheel job that asserts the helper is present should precede the flip".
- **SBOM.** Each wheel carries CycloneDX SBOMs in `.dist-info/sboms/` (PEP 770).
  - A committed generator, `tools/license/gen-sbom.py`, writes two documents at build time. The first is the Rust dependency graph of the extension and the helper for the wheel's target. maturin's own Rust SBOM is turned off, because it lists every target's crates and embeds build paths.
  - The second document covers the components that cargo cannot see: vendored Tesseract 5.5.3 and Leptonica 1.87.0, with their pinned archive SHA-256s, and the helper binary.
  - maturin's auditwheel step adds its own SBOM of any grafted libraries. With static linking there should be none.
  - CI checks that the native SBOM agrees with `vendor/` provenance.
- **BREAKING (Python): process isolation is the Python default (internal #53).**
  - `OcrEngine()` defaults to `isolation="process"`, and so does the engine behind `default_ocr_engine()` and `Page.ocr()` with no engine.
  - Python's default engine is its own process-wide engine, built from the default configuration with `helper_dir` set to the package directory.
  - Rust is unchanged. `OcrConfig::default()` and Rust's `default_ocr_engine()` stay `InProcess`, because a Rust binary has no package directory in which to find the helper.
  - A Python caller who passes `max_render_pixels` above 66,977,776 without `isolation="in_process"` now gets `ValueError`. The message names `isolation="in_process"` as the remedy.
  - A source build that did not stage the helper now gets `OcrUnavailableError(reason="engine_not_built")` from default OCR, instead of silently running in process. There is no automatic fallback to in-process OCR.

## Capabilities

### New Capabilities
- `packaging`: the distributed artifacts. This covers the wheel matrix and tags, the helper and license files inside each wheel, the SBOM documents and what they must list, the verification of each built wheel, the production-image install test, the per-push wheel check, the release workflow's triggers and outputs, and the sdist.

### Modified Capabilities
- `ocr-process-isolation`:
  - "In-process is the default; process isolation is opt-in" becomes language-specific. Python defaults to process; Rust defaults to in-process.
  - "The Python package ships and finds its helper" extends to the default engine and to every wheel.
- `ocr-engine`:
  - "Engine configuration defaults": the Python `isolation` default differs from Rust's.
  - "Process-wide default engine": Python's default engine is built with process isolation and the package's helper directory.

`concurrency` is not modified. Its fork-hazard requirement already covers a fork during any engine's construction. The Rust crate docs' list of locks gains Python's default-engine lock as an implementation-level doc update. `licensing` is not modified either. The checks that a built wheel carries the license files and an accurate SBOM are `packaging` requirements.

## Impact

- **Python binding:**
  - `crates/python/src/ocr.rs` gets a Python-side default engine (its own `OnceLock`) and the new `isolation` default. The `max_render_pixels` error message changes.
  - `crates/python/src/page.rs` switches to the new default engine.
  - `crates/python/Cargo.toml` enables PyO3's `abi3-py313` feature for the release build only, and keeps free-threaded builds possible.
  - `pyproject.toml`: `requires-python = ">=3.13"`, ruff `target-version = "py313"`, mypy `python_version = "3.13"`.
  - The `ci.yml` `python` matrix drops 3.11.
  - `python/pdf_extract/_native.pyi` and the docstrings change.
- **Packaging:**
  - `pyproject.toml` gains `[tool.maturin]` SBOM configuration and the compatibility and target settings.
  - New files: `tools/build/build-wheel.sh` (stage plus build, used by both workflows) and `tools/build/verify-wheel.py`.
- **License tooling:** new `tools/license/gen-sbom.py` and its check.
- **CI:** new `.github/workflows/wheels.yml`; `ci.yml` gains a `wheel` job. Release cost is roughly 3 to 5 runner-hours per tag. Per push, cost rises by one cached wheel build.
- **Tests:**
  - Python tests that assumed the in-process default are pinned to an explicit `isolation`, or updated.
  - New tests cover the Python default engine, the SBOM generator and the wheel verifier.
- **Docs:**
  - design-spec §7.6, §14.2, §16 and §17.2
  - README (install from a release, the isolation default)
  - ARCHITECTURE and THEORY
  - a new ADR (the Python process-isolation default, partly superseding ADR 0061 for Python)
  - a new ADR (the wheel build and release model, alongside ADR 0076 and 0087)
- **Issues:** closes internal #53. internal #56 B10 is already handled by the configure-time cap; this change only improves its Python message.
- **Out of scope:**
  - The license pipeline, which is Phase 3 change 3.
  - PyPI publishing and choosing the package name.
  - Windows and macOS x86_64 wheels.
  - musllinux wheels.
  - Code signing or notarisation of the macOS helper beyond the linker's ad-hoc signature.
