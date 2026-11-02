# Tasks

## 1. Standard library crate set (D1)

- [x] 1.1 Add `rust-src` to `rust-toolchain.toml`'s components, with a comment explaining why: `sbom.py` reads std's crate graph from it. On both macOS and the manylinux image, compare `share/doc/rust/COPYRIGHT-library.html` and the std crate set for every shipped triple. Record the result in design.md's risk entry. Verify that `rustup run 1.98.0 rustc --print sysroot` contains `lib/rustlib/src/rust/library/Cargo.lock` on both hosts.
- [x] 1.2 Add `std_crates(target)` to `tools/license/sbom.py`.
  - It resolves the sysroot of the pinned toolchain through `rustup run`.
  - It runs `cargo tree --offline --locked -p std --features backtrace,panic-unwind -e normal --target <triple>` in `library/`, with `RUSTC_BOOTSTRAP=1` in that subprocess only.
  - It parses name, version, license and in-tree/vendored origin.
  - It cross-checks against the target's rlibs when `rust-std` for that target is installed.

  Add pytest cases that plant a fake sysroot (a fake `cargo tree` output and an rlib directory) and check: the parsed set; a missing rlib fails; an extra rlib outside the known-unlinked list fails; a license outside the allow-list fails. Verify that the tests pass, and that ruff, mypy and coverage for the new code are at 100%.

## 2. Notices Part 5 and the copyright document (D2)

- [x] 2.1 Add `sbom.py --std-notices` to emit Part 5:
  - a heading with the Rust version;
  - the system runtime statement (libc and libstdc++/libc++ from the host, not shipped; GCC Runtime Library Exception for the gcc-toolset static parts);
  - the crate table across the shipped triples;
  - each vendored crates.io std crate's license file verbatim (the first file in allow-list order).

  Call it from `tools/license/gen-notices.sh`. Have the script also copy `share/doc/rust/COPYRIGHT-library.html` to `tools/license/rust-std-COPYRIGHT.html`. Regenerate `THIRD_PARTY_NOTICES`. Verify that `gen-notices.sh` run twice gives no diff, and that shellcheck passes.
- [x] 2.2 Add `tools/license/rust-std-COPYRIGHT.html` to `pyproject.toml` `license-files`, and add it to `ci.yml`'s `deny` drift check (`git diff --exit-code`). Extend `crates/testkit/tests/license.rs` to check:
  - Part 5 names the toolchain version from `rust-toolchain.toml`;
  - every std crate listed in Part 5 carries its license text;
  - the runtime statement is present;
  - `license-files` lists the new file.

  Verify that `cargo test -p pdf-extract-testkit --test license` passes, and that the test fails when the toolchain version in Part 5 is edited.
- [x] 2.3 Update `ARCHITECTURE.md` (licensing note: Part 5, the new license file, `rust-src`, and that `RUSTC_BOOTSTRAP=1` is scoped to one metadata read) and add an addendum to ADR 0054. Verify that `adr_index.py docs/adr` exits 0.

## 3. Standard library in the SBOM (D3)

- [x] 3.1 Have `rust_document` append the std components:
  - crates.io: `pkg:cargo` purl, a `bom-ref` with `?origin=rust-std`, and the `pdf-extract:origin` property;
  - in-tree: `pkg:github/rust-lang/rust@<ver>#library/<dir>`, with the toolchain version as their version.

  Have `rust_problems` accept the std set as a second expected list, matched on origin, name and version. Pass it from `build-wheel.sh` and `verify_wheel.py`. Extend the SBOM and verifier pytest cases: std present; a missing std crate fails; a lockfile crate with the same name and version as a std crate stays distinct; no absolute path from the sysroot appears. Verify that the tests pass, and that `build-wheel.sh abi3` locally produces a wheel that `verify_wheel.py` accepts.
- [x] 3.2 Update §14.3 of `docs/design-spec.md` (the SBOM bullet names the std components) and the `sbom.py` paragraph in ARCHITECTURE.md. Verify by reading both against the generated `rust.cdx.json`.

## 4. Notices cover every shipped crate (D4)

- [x] 4.1 Add a check to `tools/build/verify_wheel.py`: every non-`pdf-extract-*` component of the wheel's `rust.cdx.json` must appear as `name version` in the wheel's `THIRD_PARTY_NOTICES`. Add pytest cases that plant a crate missing from the notices (it must fail) and a workspace crate (it is exempt). Verify that the tests pass, and that the locally built wheel from 3.1 passes.

## 5. License checks on release (D5)

- [x] 5.1 Move the `deny` job's steps into a reusable `.github/workflows/license.yml` (`on: workflow_call`), with a header comment. Make `ci.yml`'s `deny` job call it. Add a `license` job to `wheels.yml` that calls it, and add `license` to `files`' `needs`. Update the `wheels.yml` header comment that lists the jobs. Verify with `actionlint` (or the repository's workflow lint) locally, then check that the CI run on the PR shows the `deny` job passing through the reusable workflow.
- [x] 5.2 Document the release gate in ARCHITECTURE.md (release workflow job list) and in §14.3. Verify by reading `wheels.yml` against both.

## 6. Weekly advisory check (D6)

- [x] 6.1 Add `.github/workflows/advisories.yml`: `schedule` (Mondays, `17 6 * * 1`) and `workflow_dispatch`; one `ubuntu-24.04` job, `timeout-minutes: 10`; `EmbarkStudios/cargo-deny-action@v2` with `check advisories`. Add a header comment noting that it is the only schedule trigger, that the owner approved it, and that GitHub disables it after 60 idle days. Verify with workflow lint and one `gh workflow run advisories.yml` dispatch after merge, which must pass.
- [x] 6.2 Note the workflow in ARCHITECTURE.md (CI section, including the 60-day auto-disable landmine) and in §14.3. Verify by reading.

## 7. Recording and closure

- [x] 7.1 Write new ADRs (ids from `adr_index.py docs/adr --next`), one decision each as the ADR template requires: 0094 for the std notices approach (D1, D2) and 0095 for the first schedule trigger (D6), with the alternatives from design.md. Regenerate the index. Verify that `adr_index.py docs/adr` exits 0.
- [x] 7.2 Fix the stale `vendor/LICENSES.md` reference in §14.3. In §16, mark "`cargo-deny` clean" as met and record that Phase 3 is complete. Update THEORY.md's licensing narrative with why the std notices were missing and how they are now derived. Verify with a grep: no remaining `LICENSES.md` reference, and every §14.3 claim matches a file or a workflow.
- [ ] 7.3 Integration: run the full local check set (pytest, mypy, ruff, `cargo test`, `gen-notices.sh` with no diff, shellcheck). Then run `wheels.yml` by `workflow_dispatch` on the merged main and confirm that every job, including `license`, passes and that the six wheels verify with Part 5 and the std SBOM components.
  - Deferred at archive (2026-10-22, owner's decision): the local check set passed, CI passed on main at internal commit 1ae9afd, and CI's `wheel` job verified a Linux x86_64 wheel with Part 5 and the std SBOM group. The full `wheels.yml` run, including the `license` gate and all six wheels, happens with the next release tag, which runs this workflow anyway.
