# Design

## Context

The motivation is in proposal.md (Why), and the required behavior is in `specs/licensing` and `specs/packaging`.

Facts that shape the approach, all checked on 2026-10-22 against the v0.1.0 wheels and the pinned toolchain 1.98.0:

- **Shipped binaries.**
  - `_native` and `pdf-extract-ocr-helper` link the Rust standard library statically.
  - On Linux their dynamic dependencies are only `libstdc++.so.6`, `libgcc_s.so.1`, `libm`, `libpthread`, `libdl` and `libc`. On macOS they are only libc++ and libSystem.
  - auditwheel grafts nothing (the packaging spec's "No grafted libraries" scenario).
- **The standard library's crate set.**
  - It can be resolved without network access from the `rust-src` component: `library/` is a Cargo workspace with its own `Cargo.lock` and a `vendor/` directory.
  - `cargo tree -p std --features backtrace,panic-unwind -e normal --target <triple>` gives 20 crates for both the Linux and the macOS targets:
    - in-tree: `std`, `core`, `alloc`, `compiler_builtins`, `panic_unwind`, `panic_abort`, `unwind`, `std_detect` and the `rustc-std-workspace-*` shims;
    - vendored from crates.io: `hashbrown`, `libc`, `cfg-if`, `rustc-demangle`, `addr2line`, `gimli`, `object`, `memchr`, `miniz_oxide` and `adler2`.
  - That set equals the rlibs shipped in `lib/rustlib/<triple>/lib`, minus `test`, `getopts`, `proc_macro`, `profiler_builtins`, `sysroot`, `rustc_literal_escaper` and `rustc-std-workspace-std`. None of those are linked into a cdylib or a non-test binary.
  - The manifest parses only with `RUSTC_BOOTSTRAP=1`, because it uses `profile-rustflags`. That is a metadata read, and nothing is compiled.
- **License texts.**
  - Every vendored std crate carries its own license files under `library/vendor/<crate>-<ver>/`.
  - The `rustc` component ships `share/doc/rust/COPYRIGHT-library.html` ("Copyright notices for The Rust Standard Library") and `share/doc/rust/licenses/*.txt`. Both are present under the `minimal` profile that CI uses.
- **Every std license is on the allow-list.**
  - `compiler_builtins` is `MIT AND Apache-2.0 WITH LLVM-exception AND (MIT OR Apache-2.0)`.
  - `memchr` is `Unlicense OR MIT`.
  - `miniz_oxide` is `MIT OR Zlib OR Apache-2.0`.
- **Current SBOM behavior.**
  - `sbom.rust_problems` requires the Rust SBOM to equal the `cargo tree` crate set exactly, keyed by (name, version).
  - Some std crates may share a name and version with crates in our own lockfile; `libc` and `hashbrown` are candidates.

## Goals / Non-Goals

**Goals:**

- The notices and the SBOM name everything in the standard library that is linked, derived from the pinned toolchain itself and never typed by hand. A toolchain bump shows up as drift.
- A single function produces the std crate set, and the notices generator, the SBOM writer and the wheel verifier all use it.
- On release, the license checks run in the same way as in `ci.yml`, without copying a list of steps that could drift.

**Non-Goals:**

- Acknowledging the system C/C++ runtimes beyond a statement. They are not shipped.
- Acknowledging the LLVM and compiler-rt pieces outside `compiler_builtins`. They are not linked into user binaries by default; `compiler_builtins` is the Rust port.
- Windows targets, which are not shipped.
- Any change to the cargo-deny policy itself.

## Decisions

### D1. Derive the std crate set with `cargo tree` over the `rust-src` workspace

- **The set comes from one new function**, `sbom.std_crates(target)`. It:
  - resolves the sysroot of the toolchain pinned in `rust-toolchain.toml`, through `rustup run`, never the `rustc` on PATH (this machine has a MacPorts `rustc` that differs);
  - runs `cargo tree --offline --locked -p std --features backtrace,panic-unwind -e normal --target <triple>` in `library/`, with `RUSTC_BOOTSTRAP=1`;
  - cross-checks every resulting crate against an rlib present in `lib/rustlib/<triple>/lib`, when that target's `rust-std` component is installed, and fails on a mismatch.
- **Why `backtrace,panic-unwind`:** these are the features the Rust project's distribution builds std with. The cross-check against the shipped rlibs is what proves the feature choice right: a crate missing from the rlibs, or an rlib crate unaccounted for, fails generation.
- **Alternatives:**
  - *Parse `library/Cargo.lock` with a fixed root list:* this needs a hand-maintained root list and its own reachability logic.
  - *Read the rlib file names only:* this gives no versions or licenses, and includes `test`/`getopts`.
  - *A committed, hand-written std notice:* this rots silently on every toolchain bump and breaks the rule against invented content.
- **`rust-src` joins `rust-toolchain.toml`'s components.** It is about 3 MB, and every CI job already installs that toolchain.

### D2. Notices Part 5, plus the copyright document as a license file

- **Part 5 of `THIRD_PARTY_NOTICES`**, written by `gen-notices.sh` through `sbom.py --std-notices`, contains:
  - a heading naming the Rust version;
  - the system runtime statement;
  - a table of the std crates (name, version, license expression, and the targets that link it; the set is the same for all three shipped triples today, but the generator unions the sets and labels any difference);
  - for each vendored crates.io crate, its own `LICENSE-MIT` (or the first allow-listed license file) verbatim, sorted by name.
- **In-tree crates** (`core`, `std` and the rest) are covered by the Rust project's own document. `share/doc/rust/COPYRIGHT-library.html` is copied verbatim to `tools/license/rust-std-COPYRIGHT.html` and added to `license-files`. The drift check compares the committed copy with the toolchain's.
- **Why HTML verbatim, not text:** converting HTML to text would need a converter whose output becomes part of the file (the same trap ADR 0054 avoided by pinning cargo-about). Shipping the Rust project's own document unchanged is the most faithful acknowledgement.
- **Alternative:** run cargo-about over `library/`. It does not accept `RUSTC_BOOTSTRAP` manifests reliably, and it would re-pick dual licenses by our order, which is fine but adds a second cargo-about configuration to keep consistent. Rejected for one source of truth (D1).

### D3. Std crates in `rust.cdx.json` as a distinct component group

- **Crates.io std crates** are listed with:
  - `purl` `pkg:cargo/<name>@<ver>`;
  - `bom-ref` `pkg:cargo/<name>@<ver>?origin=rust-std`, so that a name and version shared with our lockfile cannot collide;
  - property `pdf-extract:origin=rust-toolchain-<ver>`.
- **In-tree crates** keep the version cargo reports for them (`0.0.0` for `std`, `core` and `alloc`; `0.1.160` for `compiler_builtins`). The toolchain version goes in their `purl`, `pkg:github/rust-lang/rust@<ver>#library/<dir>`, and in the `pdf-extract:origin` property, so no version is invented.
- **`std_problems`**, a new check, matches the std group on name and version, and fails on a duplicate, a wrong origin, purl, bom-ref or licence. `rust_problems` now checks only the components without an origin property.

### D4. Notices cover what ships: a check in `verify_wheel.py`

- **The check:** every non-`pdf-extract-*` component of the wheel's `rust.cdx.json` must appear as `<name> <version>` in the wheel's `THIRD_PARTY_NOTICES`, in Part 1 or Part 5.
- **Why compare in the shipped wheel and not in the repository:** that compares the two artefacts a recipient actually gets.
- **A unit test plants a missing crate** to prove the check bites.

### D5. A reusable license workflow

- **The `deny` job's steps move** into a reusable workflow, `.github/workflows/license.yml` (`on: workflow_call`).
  - `ci.yml`'s `deny` job calls it.
  - `wheels.yml` adds a `license` job that calls it, and `files` adds `license` to its `needs`.
- **Why:** one list of steps means CI and release cannot disagree.
- **Alternative:** copy the steps into `wheels.yml`. Rejected because the copy would drift.

### D6. Weekly advisories workflow

- **`.github/workflows/advisories.yml`:**
  - triggers: `schedule: cron '17 6 * * 1'` (Mondays) and `workflow_dispatch`;
  - one `ubuntu-24.04` job, `timeout-minutes: 10`;
  - it runs `EmbarkStudios/cargo-deny-action@v2` with `command: check advisories`.
- **No build and no toolchain install.** The project owner approved this as the only schedule trigger (2026-10-22).
- **A failing scheduled run emails the repository owner,** which is GitHub's default notification.

### D7. Recording

- **Two new ADRs**, one decision each: 0094 records D1–D2 (std notices and the copyright document), and 0095 records D6 (the first schedule trigger and why it is exempt from the cost cap).
- **ADR 0054** gets an addendum: Part 5 and the new license file.
- **Spec §14.3** is fixed: `vendor/LICENSES.md` becomes `vendor/PROVENANCE.md` plus `check-c-licenses.sh`, and std notices, the release rerun and the weekly advisories are added.
- **§16:** "`cargo-deny` clean" is marked met, and Phase 3 is recorded as complete.

## Risks / Trade-offs

- **`COPYRIGHT-library.html` or the std set differs between the Linux and macOS toolchain packages.** The drift check runs on Linux; local generation runs on macOS.
  - Mitigation: task 1.1 compares both hosts before anything is committed. If they differ, the generator records the Linux file, and the local regenerate path documents running it on Linux (as `gen-notices.sh` already must match CI).
  - Checked in task 1.1 (2026-10-22, Rust 1.98.0): `COPYRIGHT-library.html` has the same SHA-256 (`68129500…afa6`) in the aarch64-apple-darwin and aarch64-unknown-linux-gnu toolchains, and the std crate set (with licenses) is identical for x86_64/aarch64 Linux and aarch64 macOS, resolved on either host. No host-specific handling is needed.
- **`RUSTC_BOOTSTRAP=1` could leak into a real build.**
  - Mitigation: set it only in the environment of the one `cargo tree` subprocess, inside `library/`, never exported.
- **A future toolchain changes std's feature names or layout, and generation fails at a toolchain bump.**
  - Acceptable: bumps are deliberate, in their own commit (`rust-toolchain.toml`'s comment). The failure is loud, not silent.
- **The schedule trigger runs on the default branch even when the repository is idle.** GitHub disables scheduled workflows after 60 days with no repository activity.
  - Mitigation: noted in ARCHITECTURE. Re-enabling takes one click; the release workflow still checks advisories on every tag.
- **Larger notices.** Part 5 adds about 14 KB to `THIRD_PARTY_NOTICES`. The verbatim `COPYRIGHT-library.html` is 1.5 MB, because it repeats license texts per crate, but it compresses to about 118 KB in the wheel (measured 2026-10-22; the macOS abi3 wheel is 5.3 MB). Acceptable for a verbatim copy; the alternative, extracting text from it, was rejected in D2.

## Migration Plan

There is no runtime migration.

- The first release after this change (0.1.1 or later) carries the new files.
- v0.1.0 is not re-released. Its missing std notices are fixed forward and noted in the release notes of the next version.
- Rollback: revert the PRs. The reusable workflow change is self-contained.
