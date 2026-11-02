## Why

Phase 3's exit criterion requires 72 clean fuzzing hours per target, counted
in `fuzz/ledger.json` from local sessions (ADR 0087). Five targets make that
360 machine-hours. That is not practical on a developer machine, and the cost
cap rules out CI. Every target's ledger total is 0 h. The owner decided that a
GA release does not wait for it.

Skipping it leaves availability risk. A malformed or hostile PDF that nobody
has found yet could crash the process that opens it (stack overflow, abort,
out of memory) or hang it. Memory-safety risk is small: the engine is Rust
apart from Tesseract and Leptonica, sanitizer and loom runs cover the unsafe
and concurrent code, and Tesseract aborts are contained by process isolation,
which is the Python default.

## What Changes

- Fuzzing depth stops being a Phase 3 exit and release gate. The ledger keeps
  counting clean hours against a 72-hour depth goal, and its status line says
  the goal is not a release gate.
- Callers on untrusted input contain failures themselves. They run
  extraction in a process they can afford to lose, under a wall-clock limit
  enforced by killing that process, and quarantine a document after a
  bounded number of failures instead of retrying it forever. This becomes a
  precondition of the Phase 4 shadow step, and the README and design spec say
  so.
- A one-off cloud fuzzing campaign of a few hours per target is recorded as
  an optional follow-on (a GitHub issue). Nothing waits on it.
- ADR 0093 records the decision. ADR 0087 gains an addendum pointing to it.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `fuzzing`: the ledger requirement no longer defines a Phase 3 exit. It
  reports progress against a 72-hour depth goal that is not a release gate.

## Impact

- `tools/fuzz/ledger.py` (the status verdict line) and
  `tests/python/test_fuzz_ledger.py`.
- `docs/design-spec.md` §8.5, §15.3, §16 and §17.1. README, ARCHITECTURE.md
  and THEORY.md.
- `docs/adr/0093-*` (new), and an addendum to `docs/adr/0087-*`.
- No library behaviour changes. Requirement R1 (no input may crash the
  process) is unchanged. What changes is how much evidence a release needs
  that it holds.
