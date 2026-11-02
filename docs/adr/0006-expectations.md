---
id: 0006
title: Oracle expectations are checked-in JSON; corpus C gates on exact match, corpus B on thresholds
date: 2026-10-01
status: Accepted
summary: Expected outputs are JSON keyed by SHA-256; corpus C must match exactly (or a pinned snapshot), corpus B must meet open, routing and similarity thresholds.
---

# 0006. Oracle expectations are checked-in JSON; corpus C gates on exact match, corpus B on thresholds

## Context

Decision D-21 of the run spec. Library CI must never need PyMuPDF (L2), yet parity has to be a regression gate.

## Decision

Oracle expectations are checked-in JSON keyed by SHA-256 (written by `tools/oracle/gen.py`, which always opens files
from disk). **Gate C** (synthetic corpus C): for `gate = "exact"` our `text()` equals the oracle text and the open
outcome class matches; for `gate = "divergence"` our output equals the pinned `snapshots/<sha>.txt` and still
differs from the oracle (a divergence that disappears fails the test so the manifest is cleaned); hostile rows
follow ADR 0016. **Gate B** (real-world corpus B), over all pages: every file the oracle opens we open
(at most 2 documented exceptions); agreement on `len(text.strip()) >= 50` for at least 99% of pages; similarity
(1 - normalised Levenshtein after `.strip()`) median at least 0.98 and 10th percentile at least 0.90; exact-match
rate and line-count delta are reported only. The gate is `crates/core/tests/parity.rs`.

## Alternatives considered

Not recorded beyond the rationale: exactness on synthetic fixtures catches regressions, and corpus-B thresholds
reflect its font mix.

## Consequences

Corpus C has 284 manifest entries (256 exact, 19 divergence, 9 hostile); corpus B 60 files / 91 pages scoring opens
60/60, routing 1.000, similarity median 1.000, p10 0.968, so the B gates have headroom and may be tightened (a change
to any gate is a spec change). The three position-sweep fixtures were removed from corpus C because they test the
oracle's floating point, not a rule; they remain our own tests. Encrypted fixtures are not byte-reproducible
(random `/ID`), so provenance records the committed hash. The corpus-A run against private uploads is a script the
user runs (`tools/oracle/run_private_corpus.sh`), never on CI.
