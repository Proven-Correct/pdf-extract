---
id: 0091
title: Wheels are built by one script, verified before use, and released from tags only
date: 2026-10-22
status: Accepted
summary: tools/build/build-wheel.sh builds every wheel (helper staged first, generated SBOMs, abi3 by opt-in feature, Linux in the digest-pinned manylinux_2_28 image); tools/build/verify_wheel.py and smoke.py gate each one; ci.yml builds one wheel per push and wheels.yml builds the matrix on tags and dispatch, releasing to GitHub only.
---

# 0091. Wheels are built by one script, verified before use, and released from tags only

## Context

Phase 3 of the design spec (§16) ends with "wheels install on the
production OS image". Until this change no wheel was ever built: CI ran
`maturin develop`, and a wheel built without first running
`tools/build/stage-helper.sh` silently lacked the OCR helper (ADR 0076). The
helper is a second binary that maturin neither builds nor audits. internal #53, the
Python process-isolation default, turns a missing helper into a failure for
every Python OCR user, so the build had to make the helper impossible to
forget and CI had to prove it shipped.

Further constraints:

- §14.3 promises a CycloneDX SBOM in every wheel.
- The repository is INTERNAL, so Actions minutes are billed and recurring
  cost is capped (ADR 0087).
- The package name is undecided (§17.2 Q1), so nothing may be published to
  PyPI.
- The supported floor is CPython 3.13. Free-threaded builds start at 3.14t
  (PyO3 0.29).
- Production runs Ubuntu 24.04 with a uv-managed CPython 3.13.

## Decision

1. **One build script.** `tools/build/build-wheel.sh <abi3|ft>` stages the
   helper, generates the SBOMs, and runs `maturin build` with the version
   that `uv.lock` pins, in a venv of its own.
   - **Linux** runs inside `quay.io/pypa/manylinux_2_28_{x86_64,aarch64}`,
     pinned by digest. `--docker <arch>` starts the container with the
     checkout mounted. rustup, the cargo home and the build output live
     under `target/` so CI can cache them. The host's staged helper is
     saved and restored around the run.
   - **macOS** runs natively with `MACOSX_DEPLOYMENT_TARGET=11.0`, so the
     extension and the helper share one floor. The script refuses an
     interpreter that is not arm64.
2. **abi3 is an opt-in crate feature.** `crates/python` gains
   `abi3 = ["pyo3/abi3-py313"]`. The release turns it on for the
   `cp313-abi3` wheels and leaves it off for `cp314t`, because free-threaded
   CPython has no stable ABI. Development builds keep the full API.
3. **Single version source.** `pyproject.toml` declares the version dynamic,
   so the package, the extension and the helper handshake all carry the
   workspace version.
4. **Our own script writes the SBOMs.** `tools/license/sbom.py` writes
   two documents, which maturin includes with `--sbom-include`:
   - `rust.cdx.json`: the `cargo tree -e normal,no-proc-macro --target
     <triple>` crates of the extension and of the helper.
   - `native.cdx.json`: Tesseract and Leptonica from `vendor/PROVENANCE.md`,
     plus the helper with its hash.

   maturin's own Rust SBOM is turned off with `[tool.maturin.sbom] rust =
   false`, for three reasons:
   - it resolves all targets, so `loom` and other platforms' crates appear
     as required;
   - it embeds the build machine's absolute paths;
   - it lists build-only crates.
5. **Every wheel is verified before use.** `tools/build/verify_wheel.py`
   checks:
   - the filename tags, and that there is exactly one extension and no
     grafted libraries;
   - that the helper is present with mode 0755 and is portable. For a
     Linux helper that means its `DT_NEEDED` and symbol versions are within
     auditwheel's manylinux_2_28 policy, which maturin applies only to the
     extension. For a macOS helper it means the minimum OS is within the
     tag and only system libraries are linked;
   - that the licence files are byte-identical to the repository's;
   - that the SBOMs are valid CycloneDX 1.5, list exactly the crates of a
     fresh `cargo tree`, agree with `PROVENANCE.md` and the helper's hash,
     and contain no build paths.

   `tools/build/smoke.py` then installs the wheel into a fresh venv outside
   the checkout and runs text extraction and default-engine OCR.
6. **CI shape under the cost cap.**
   - **Per push.** `ci.yml` job `wheel` builds the manylinux x86_64 abi3
     wheel, verifies and smoke-tests it, and runs the whole pytest suite
     against it.
   - **Tag or dispatch.** `wheels.yml` builds the full matrix: linux
     x86_64 and aarch64 on native runners and macOS arm64, each × {abi3,
     ft}, plus the sdist. It runs only on `v*` tags and manual dispatch,
     never on a schedule or a branch push. Each wheel is verified and
     smoke-tested on its own platform. A production-image job installs
     each Linux abi3 wheel into a digest-pinned `ubuntu:24.04` container
     with a uv-managed CPython 3.13 and the distribution
     `tesseract-ocr-eng`.
   - **Release.** On a tag, the files and `SHA256SUMS` go to a GitHub
     Release. Nothing goes to any package index.

## Alternatives considered

- **`PyO3/maturin-action` with a `before-script-linux` staging step.** It
  cannot be run locally the same way, and it hides the container that the
  helper also needs.
- **Cross-compiling aarch64 from x86_64.** Cross-building the vendored C++
  through cmake is the riskiest part of the build. Native arm64 runners
  remove that risk.
- **maturin's built-in Rust SBOM.** This was the plan until a real wheel
  was inspected: its contents were wrong in the three ways listed above.
- **cargo-cyclonedx run separately.** It is the same generator with the
  same all-targets behaviour, plus a second tool pin.
- **abi3 always on.** That would rely on PyO3 ignoring the feature for
  free-threaded interpreters, and would change every development build.
- **The full matrix on every push.** That means about 3 to 5 runner-hours
  per push, against a cost cap. One wheel per push catches the
  helper-dropping class of regression, and the rest waits for a tag or a
  dispatch.
- **Publishing to PyPI or a private index.** Blocked on the package name
  (§17.2 Q1). GitHub Releases on the internal repository need no name
  decision.

## Consequences

- A wheel without the helper, with an over-new glibc symbol in the helper,
  with stale licence files or with a wrong SBOM fails CI on the push that
  causes it. That is true for linux x86_64 abi3 on every push, and for the
  other entries at release or dispatch.
- Each release costs about 3 to 5 runner-hours, including the macOS
  multiplier. Each push adds one cached container build (about 10 minutes
  cold) and a pytest run.
- The two image digests and the `ubuntu:24.04` digest are pins. Bumping one
  is a deliberate change, like the toolchain pin.
- The free-threaded and aarch64 builds are not exercised per push. A
  regression that is specific to them is found at release or on a manual
  dispatch. `python-freethreaded` still covers the free-threaded code paths
  with a development build.
- Installing needs repository access, because releases live on the internal
  repository, until §17.2 Q1 is decided.
- Supersedes nothing. ADR 0076 (the helper lives in the package directory)
  stands, and this record makes it enforced rather than remembered.
