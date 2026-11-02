---
id: 0039
title: Defer the mmap source
date: 2026-10-01
status: Accepted
summary: Source::Mmap and open(mmap=True) are not in this run.
---

# 0039. Defer the mmap source

## Context

Decision D-20.

## Decision

`mmap` is deferred; `Document::open` reads the whole file into memory and closes the descriptor.

## Alternatives considered

Recorded rationale: it is an opt-in feature with a SIGBUS caveat and no consumer in this run.

## Consequences

Memory use at open equals the file size (bounded by `max_file_size`, default 512 MiB).
