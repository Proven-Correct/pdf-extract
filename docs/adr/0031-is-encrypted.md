---
id: 0031
title: Expose Pdf::is_encrypted() instead of re-parsing with a bogus password
date: 2026-10-01
status: Accepted
summary: A one-line fork accessor reports whether a decryptor is installed, replacing a second parse in core.
---

# 0031. Expose Pdf::is_encrypted() instead of re-parsing with a bogus password

## Context

Decision D-35 (revision 5). Core detected encryption by parsing a second time with a bogus password to observe an error.

## Decision

The fork adds `Pdf::is_encrypted()` (`!matches!(*xref.decryptor, Decryptor::None)`), shipped in the H9 patch.

## Alternatives considered

The bogus-password second parse was dropped. Recorded rationale: one line in the fork saves 1-17 ms per open and a code
path that only exists to observe an error.

## Consequences

`Document::is_encrypted` reads the parsed trailer, so `#xx`-escaped `/Encrypt` names count (a byte search would miss them).
