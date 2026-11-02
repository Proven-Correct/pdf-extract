---
id: 0083
title: Heavy Linux CI jobs free runner disk through a composite action
date: 2026-10-05
status: Accepted
summary: The rust, coverage and python-coverage jobs run .github/actions/free-disk after checkout to delete unused preinstalled SDKs before building.
---

# 0083. Heavy Linux CI jobs free runner disk through a composite action

## Context

The `rust` job's second test build (`pdf-extract-core` with `render-support`) filled the ubuntu-24.04 runner's 72 GB root filesystem: 2.9 GB free before it, 54 MB after, and the linker died with SIGBUS (`ld terminated with signal 7 [Bus error]`) writing a test binary, which does not look like a disk problem (internal PR #68).

## Decision

The `rust`, `coverage` and `python-coverage` jobs run the composite action `.github/actions/free-disk` right after checkout. It deletes preinstalled SDKs the project never uses (.NET, Android, GHC, CodeQL; well over 20 GB) and logs `df -h /` so remaining headroom is visible. It is a no-op on non-Linux runners.

## Alternatives considered

- **Larger runners** - cost and a different runner class for a problem a delete solves.
- **Dropping the second test build or splitting jobs** - loses coverage or adds CI time.
- **Trimming the Rust cache** - the cache is what keeps builds fast.

## Consequences

Jobs gain a few seconds. A job that grows past the freed headroom will fail again; the logged free space is the early signal. Any new Linux job that builds the workspace more than once or with instrumentation should run the action.
