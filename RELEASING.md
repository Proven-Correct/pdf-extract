# Releasing pdf-extract

Releases are GitHub Release assets of this repository: six wheels, the sdist
and `SHA256SUMS`. Nothing is published to a package index, and the Rust
crates stay `publish = false` (design spec §17.2 Q1, Q6). The `Wheels`
workflow (`.github/workflows/wheels.yml`) builds, verifies and publishes a
release from a `v<version>` tag. This checklist is what a person does around
it.

## 1. Prepare (on a branch, merged with CI green)

1. **Version.** Bump `[workspace.package] version` in `Cargo.toml`. The
   Python package takes its version from it.
   Run `cargo metadata --format-version 1 >/dev/null` to refresh `Cargo.lock`.
2. **Release notes.** Write `docs/releases/v<version>.md`: what changed since
   the previous release, in user terms. The workflow refuses to run without
   it (`tools/build/release-version.sh`).
   Update the version in the README's "Installing a release" URLs too; a
   test fails while they name another version.
3. **Render goldens.** If `Cargo.lock`'s entries for any `hayro*`, `vello*` or
   `pic-scale` crate changed since the last release, the render golden hashes
   (`crates/render/tests/render-golden/`) must be re-blessed and the change
   reviewed: run the golden test with `RENDER_GOLDEN_BLESS=1` on macOS, take
   the Linux hashes from the `render-golden-ubuntu-24.04` CI artefact, and
   check in the `render-cross-os` job that the platforms still agree. The
   golden test fails on any unblessed change, but a release must not be the
   moment the hashes are first refreshed without review.
4. **Notices.** After any change to `Cargo.lock` or `rust-toolchain.toml`,
   run `tools/license/gen-notices.sh` and commit `THIRD_PARTY_NOTICES` and
   `tools/license/rust-std-COPYRIGHT.html`. CI's `deny` job fails if they are
   stale, and so does the release's `license` job.
5. **Rehearse** (optional; about 3–5 runner-hours):
   `gh workflow run wheels.yml --ref main` runs everything except the
   GitHub Release and uploads the files as workflow artifacts.

## 2. Tag

```sh
git tag -a v<version> -m "pdf-extract <version>" <commit on main>
git push origin v<version>
```

The workflow checks that the tag matches the version and that the notes
exist, reruns the licence checks on this commit, builds and verifies the six
wheels and the sdist, installs the Linux wheels on Ubuntu 24.04 and 26.04,
writes `SHA256SUMS`, and creates the GitHub Release with the notes.

## 3. Check

1. The release lists six wheels, `pdf_extract-<version>.tar.gz` and
   `SHA256SUMS`.
2. Download the files and run `sha256sum --check SHA256SUMS`.
3. Install one wheel by URL with no credentials, as the README's "Installing
   a release" shows, and import it.
