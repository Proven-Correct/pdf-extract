---
id: 0017
title: Tracked files carry no assistant or tool-vendor references; a test enforces it
date: 2026-10-02
status: Accepted
summary: A wordlist test fails the build if a tracked non-vendor text file names an assistant product or a tool vendor.
---

# 0017. Tracked files carry no assistant or tool-vendor references; a test enforces it

## Context

Unit 15 of the run spec (section 4.15) requires that no assistant or tool-vendor references appear anywhere in the
repository, with a wordlist check in the tree as a test. The spec records no further motive.

## Decision

`crates/testkit/tests/wordlist.rs` scans every tracked non-vendor text file case-insensitively for the listed
vendor and assistant terms. Exclusions are narrow and justified in the test; a legitimate hit is allow-listed as an
exact occurrence with a comment.

## Alternatives considered

Run spec section 4.15 placed the check "in `tools/`"; it is implemented instead as a Rust test next to `license.rs`, so
it runs in the ordinary `cargo test --workspace` locally and in CI. No other alternative is recorded.

## Consequences

Commit messages are outside the scan (they are not tracked files); that is covered by review. New documents must
avoid the listed terms or add an allow-list entry.
