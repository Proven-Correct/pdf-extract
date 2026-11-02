## 1. Spec and tooling

- [x] 1.1 Ledger status line reports the 72 h depth goal and says it is not a release gate; update `tests/python/test_fuzz_ledger.py`.

## 2. Decision record and docs

- [x] 2.1 ADR 0093 (fuzzing depth waived as a release gate; caller containment; optional cloud campaign), ADR 0087 addendum, regenerate the ADR index.
- [x] 2.2 Design spec: §16 Phase 3 exit, §8.5 caller containment guidance, §15.3 shadow precondition, §17.1 risk row.
- [x] 2.3 README "Running on untrusted uploads"; ARCHITECTURE ledger note; THEORY hardening paragraph.

## 3. Archive

- [x] 3.1 At archive, edit the main `fuzzing` spec's Purpose: clean time counts towards the 72-hour depth goal, not the Phase 3 exit criterion.

## 4. Follow-on

- [x] 4.1 File a GitHub issue for the optional one-off cloud fuzzing campaign (not a release gate).
