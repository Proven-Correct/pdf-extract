# Proposal

## Why

This is the third and last Phase 3 change. Most of the license pipeline in spec §14.3 already exists and runs in CI:

- `cargo-deny` checks licenses, advisories, bans and sources.
- A negative proof shows that copyleft licenses really are refused.
- The vendored C/C++ trees have their own license check.
- `THIRD_PARTY_NOTICES` is generated and CI checks it for drift.
- Every wheel carries SBOMs, and `verify_wheel.py` checks that the license files are present.

An audit of the released v0.1.0 wheels against §14.3 still found four gaps:

1. **The Rust standard library is missing from the notices and the SBOM.**
   - `std`, `core`, `alloc` and `compiler_builtins` are linked statically into `_native` and the OCR helper. So are std's own crates.io dependencies: `hashbrown`, `libc`, `cfg-if`, `rustc-demangle`, and the backtrace crates `addr2line`, `gimli`, `object`, `miniz_oxide` and `adler2`.
   - Their MIT notices ship nowhere. cargo-about and `cargo tree` see only our own graph, never the sysroot.
2. **Nothing checks that the notices cover what ships.**
   - `THIRD_PARTY_NOTICES` is generated from `crates/python`'s graph. The SBOM is generated per target from the graphs of the extension and the helper.
   - Today they agree, apart from our own GPL crates, which `LICENSE` covers. But no check would notice if, say, the helper gained a dependency that the extension lacks.
3. **The release workflow doesn't rerun the license checks.**
   - `wheels.yml` compares the shipped notices with the repository file. It never checks that the repository file is current, and never runs `cargo-deny`, on the tagged commit.
   - A tag pushed on a commit whose `deny` job never ran or failed would still release.
4. **New advisories are seen only on push.**
   - A RustSec advisory published after a release goes unnoticed until someone next pushes.

There is also drift in the documents:

- §14.3 names a `vendor/LICENSES.md` that doesn't exist. Provenance lives in `vendor/PROVENANCE.md` and the check in `tools/license/check-c-licenses.sh`.
- The Phase 3 exit criterion "`cargo-deny` clean" is not recorded as met.

## What Changes

- **Standard library notices.**
  - `gen-notices.sh` gains a fifth part. It lists the crates of the pinned toolchain's standard library for each shipped target, taken from that toolchain's `rust-src` component. It also copies the Rust project's license texts and copyright notice verbatim.
  - `rust-toolchain.toml` adds the `rust-src` component.
  - The `deny` drift check covers the new part automatically.
- **Standard library in the SBOM.**
  - `rust.cdx.json` lists those sysroot crates as components, with their versions and licenses.
  - `verify_wheel.py` checks that they are present.
- **Notices cover the shipped graph.**
  - `verify_wheel.py` fails a wheel when a non-workspace crate in its `rust.cdx.json` has no entry in its `THIRD_PARTY_NOTICES`.
- **Release reruns the license checks.**
  - A new `license` job in `wheels.yml` runs the same checks as `ci.yml`'s `deny` job on the commit being released: cargo-deny, the copyleft negative proof, the C/C++ check and its self-test, and the notices drift check.
  - `files`, and so `release`, depend on it.
- **Weekly advisory check.**
  - A new small workflow runs `cargo deny check advisories` weekly, and on dispatch, on one Ubuntu runner.
  - It is the repository's first `schedule` trigger. The project owner approved it because it is cheap (a few runner-minutes a week). The cap on scheduled heavy workflows still applies to everything else.
- **C/C++ runtime statement.**
  - The notices and §14.3 state that the C++ runtime (libstdc++ or libc++) and libc come from the host system and are not shipped.
  - On manylinux, the gcc-toolset pieces that are linked statically fall under the GCC Runtime Library Exception, which needs no notice.
- **Docs.**
  - Fix the reference in §14.3.
  - Mark the Phase 3 `cargo-deny` exit criterion met in §16, and record that Phase 3 is complete.
  - Add an ADR for the standard library notices and the scheduled advisory check.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `licensing`:
  - The notices requirement gains the standard library and the C/C++ runtime statement.
  - The advisory requirement gains the weekly scheduled run.
  - New requirements: the shipped graph is covered by the notices, and the license checks rerun on release.
- `packaging`: the SBOM requirement covers the statically linked Rust standard library crates.

## Impact

- **Code:**
  - `tools/license/gen-notices.sh`, `about.hbs` (if needed), `tools/license/sbom.py`, `tools/build/verify_wheel.py` and `crates/testkit/tests/license.rs`.
  - Their tests: `tests/python/test_sbom*.py` and `test_verify_wheel*.py`.
- **Config:** `rust-toolchain.toml` (`rust-src`), `THIRD_PARTY_NOTICES` (regenerated, larger), `.github/workflows/wheels.yml` (new `license` job), and a new `.github/workflows/advisories.yml`.
- **Docs:** `docs/design-spec.md` §14.3 and §16, `ARCHITECTURE.md` (licensing note), `THEORY.md`, a new ADR, and an ADR 0054 addendum.
- **No library behaviour change** and no API change. The wheel layout is unchanged, apart from larger notice and SBOM files.
- **Out of scope:**
  - Python runtime dependencies (there are none).
  - The license of the Tesseract language data (`tesseract-ocr-eng` is a system package, not shipped).
  - Relicensing.
