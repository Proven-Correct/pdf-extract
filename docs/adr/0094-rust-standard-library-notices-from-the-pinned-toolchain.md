---
id: 0094
title: The Rust standard library is acknowledged from the pinned toolchain's own sources
date: 2026-10-22
status: Accepted
summary: THIRD_PARTY_NOTICES Part 5 and the wheel's Rust SBOM list the statically linked standard library, derived by cargo tree over the pinned toolchain's rust-src (std with backtrace,panic-unwind, cross-checked against the shipped rlibs), and the toolchain's COPYRIGHT-library.html ships verbatim as a fourth licence file.
---

# 0094. The Rust standard library is acknowledged from the pinned toolchain's own sources

## Context

An audit of the v0.1.0 wheels against spec §14.3 (2026-10-22) found that the
extension module and the OCR helper link the Rust standard library
statically: the Rust project's own crates (`std`, `core`, `alloc`,
`compiler_builtins`, `panic_unwind`, `unwind`, `std_detect`) and ten
crates.io crates that std depends on (`hashbrown`, `libc`, `cfg-if`,
`rustc-demangle`, `addr2line`, `gimli`, `object`, `memchr`, `miniz_oxide`,
`adler2`). Their MIT notices shipped nowhere: `THIRD_PARTY_NOTICES` (ADR
0054) comes from cargo-about and the SBOM (ADR 0091) from `cargo tree`, and
both see only our own Cargo graph, never the sysroot. v0.1.0 shipped
without them.

The `rust-src` component holds std's sources as a Cargo workspace,
`library/`, with its own `Cargo.lock` and a `vendor/` directory of the
crates.io dependencies, each with its licence files. The `rustc` component
ships `share/doc/rust/COPYRIGHT-library.html`, the Rust project's notice for
the standard library. On 2026-10-22 both were byte-identical between the
macOS and Linux 1.98.0 toolchains, and the std crate set was identical for
all three shipped targets.

## Decision

We derive the standard library from the toolchain pinned in
`rust-toolchain.toml`, never by hand:

- `tools/license/rust_std.py` runs `cargo tree -p std --features
  backtrace,panic-unwind -e normal --target <triple>` in that toolchain's
  `library/`, through `rustup run <channel>`. These are the features the Rust
  distribution builds std with. When the target's `rust-std` is installed,
  the result is cross-checked against the shipped rlibs, minus a named list of
  rlibs that no shipped binary links (`test`, `getopts`, `proc_macro`, …).
  The manifest needs `RUSTC_BOOTSTRAP=1`, which is set in that one
  subprocess only; nothing is compiled.
- `THIRD_PARTY_NOTICES` gains Part 5. It names the Rust version and lists
  every std crate for the shipped targets, with each crate's origin and licence
  expression. It also contains the system-runtime statement (libc and the C++
  runtime come from the host; the GCC Runtime Library Exception covers the
  gcc-toolset parts linked statically), and each crates.io crate's licence
  file verbatim, chosen in the same accepted order as Part 1.
- The toolchain's `COPYRIGHT-library.html` is copied verbatim to
  `tools/license/rust-std-COPYRIGHT.html` and becomes the fourth
  `license-files` entry. It covers the in-tree crates.
- `rust.cdx.json` lists the std crates as their own group: property
  `pdf-extract:origin`; crates.io ones keep their `pkg:cargo` purl but get a
  `?origin=rust-std` bom-ref, because std's `hashbrown` 0.17.1 is also in our
  `Cargo.lock`; in-tree ones use `pkg:github/rust-lang/rust@<ver>#library/<dir>`.
- `verify_wheel.py` fails a wheel whose notices do not name, as
  a Part 1 `Used by:` entry or a Part 5 row, every non-`pdf-extract-*` crate
  of its Rust SBOM.

`rust-src` joins `rust-toolchain.toml`'s components.

## Alternatives considered

- **A committed, hand-written std notice.** This rots silently at each
  toolchain bump, and it puts typed content where everything else in the
  notices is derived.
- **Parse `library/Cargo.lock` with a fixed list of roots.** This needs its
  own reachability and feature logic, and a hand-kept root list.
- **List the rlib file names in the sysroot.** This gives no versions or
  licences, and it includes crates that no shipped binary links.
- **cargo-about over `library/`.** It would need a second cargo-about
  configuration, and it handles the bootstrap-only manifest poorly. One
  resolver now feeds both the notices and the SBOM.
- **Convert `COPYRIGHT-library.html` to text inside the notices.** The
  converter's output would become part of the file (the trap ADR 0054 avoided
  by pinning cargo-about). Shipping the Rust project's document unchanged is
  the most faithful acknowledgement.
- **Re-release v0.1.0.** We fix forward in the next version instead, and its
  release notes call out the gap.

## Consequences

- A toolchain bump now changes `THIRD_PARTY_NOTICES` and
  `rust-std-COPYRIGHT.html`. The `deny` drift check fails until
  `gen-notices.sh` is rerun.
- `gen-notices.sh` needs rustup with the pin, and Python ≥ 3.11 (`$PYTHON`).
- The verbatim HTML is 1.5 MB, about 118 KB compressed in a wheel.
- `crates/testkit/tests/wordlist.rs` allow-lists two upstream licence
  sentences in it (Apache-2.0 §4(d) and the LLVM exception).
- If a future toolchain renames std's features or reshapes `library/`,
  generation fails loudly at the bump. That is acceptable, because bumps are
  deliberate and made in their own commit.
