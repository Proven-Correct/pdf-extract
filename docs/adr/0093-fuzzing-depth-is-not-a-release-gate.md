---
id: 0093
title: Fuzzing depth is not a release gate; callers contain crashes and hangs
date: 2026-10-22
status: Accepted
summary: The Phase 3 exit of 72 clean fuzzing hours per target is waived for GA; the ledger keeps counting against a 72-hour depth goal; callers on untrusted input run extraction in a killable process under a wall-clock limit with bounded retries (a precondition of the Phase 4 shadow step); a one-off cloud campaign is an optional follow-on (internal #167).
---

# 0093. Fuzzing depth is not a release gate; callers contain crashes and hangs

## Context

Phase 3 of the design spec (§16) set its fuzzing exit at 72 clean hours per
target. ADR 0087 counts those hours in the committed `fuzz/ledger.json`, from
sessions on a developer machine, because the cost cap rules out CI. Five
targets make that 360 machine-hours. That is not practical locally, and when
the rest of Phase 3 was done (wheels and the release workflow, ADR 0091) every
target's ledger total was 0 h. The owner decided that a GA release does not
wait for it.

Fuzzing at this depth is evidence for requirement R1 (§2.3): no input may
crash the process through a segfault, abort, stack overflow or unbounded
memory. Without it, inputs that crash the process or hang it may still exist.
From Python, a Rust panic is caught and raised as `InternalError` (the Rust
API propagates panics, ADR 0035), and codec panics are caught at the decoder
boundary (H18). The cases that matter are stack
overflow, abort, running out of memory, and a hang or a slowdown that looks
like one.

The memory-safety exposure is smaller. The engine is Rust apart from
Tesseract and Leptonica. The dispatch-only ASan and TSan runs check the unsafe
and FFI code on the test suite (not on hostile input), and the loom models
check the concurrent code (ADR 0087). From Python, Tesseract aborts are
contained by process isolation, which is the default there (ADR 0092).
Short fuzzing runs had already found and fixed internal #119, #120 and #147 (H19, H20,
H24). Their reproducers in `fuzz/regressions/` are replayed in the ordinary
suite on a 512 KiB stack. That also shows the targets were still turning up bugs.

## Decision

1. **The 72-hour figure stops being a Phase 3 exit and a release gate.**
   `tools/fuzz/ledger.py` keeps recording clean hours as ADR 0087 describes.
   Its status line now reports a "fuzzing depth goal" of 72 hours per target
   and says it is not a release gate. The `fuzzing` spec's ledger
   requirement is amended to match (OpenSpec change
   `waive-fuzzing-exit-gate`). R1 is unchanged. What changes is how much
   evidence a release needs that R1 holds.
2. **Callers on untrusted input contain failures.** The design spec's
   deployment guidance (§8.5) and the README say this. A caller that
   extracts uploads:
   - runs extraction in a process whose death costs nothing more than the
     one job, such as a job-queue worker or a subprocess, and never in the
     process that serves requests;
   - enforces a wall-clock limit per document by killing that process. A
     Python-level timeout cannot interrupt a call in progress, and the
     library's own time limits are per page (`page_time_budget`, OCR
     `page_timeout`), not per document;
   - caps retries per document and quarantines a document that keeps
     killing or timing out its worker, so one file cannot take capacity down
     in a loop;
   - logs the document's identity when a worker dies, so the input can be
     reproduced and filed.
   The first consumer's Phase 4 shadow step (§15.3) requires this containment to be
   in place before shadow traffic starts. This is a change on the consumer's
   side: §8.5 records that it runs extraction threads inside its
   web workers, where a crash takes the worker down. Shadow mode on production traffic
   is then the real-world fuzzing that remains.
3. **A one-off cloud fuzzing campaign is an optional follow-on** (internal #167): a
   few hours per target on a rented VM, recorded in the ledger. No release
   waits on it, and it adds no scheduled or push-triggered workflow
   (ADR 0087).

## Alternatives considered

- **Keep the 72-hour gate and run it locally**: 360 machine-hours on a
  developer machine. Rejected as impractical. Releases would wait
  indefinitely.
- **Run the 72 hours in CI**: 360 runner-hours, which the cost cap forbids
  (ADR 0087).
- **Gate GA on a shorter budget, such as a few hours per target in the
  cloud**: this is most of the value at a fraction of the cost, because
  discovery falls off steeply with time. The owner chose to keep it as an
  optional follow-on rather than a gate.
- **Contain failures inside the library**, for example an
  `extract_isolated()` helper that runs every call in a subprocess with a
  deadline. It would protect callers who do not follow the guidance. It was
  not taken up in this decision, which puts containment on the caller's
  side, where the caller's own job system can handle timeouts and retries.
  Adding the helper later remains possible.

## Consequences

- GA can ship with every target at 0 ledger hours. The design spec's §16 no
  longer claims a bar that was not met.
- Any crash or hang still to be found is first met in production. Until it
  is fixed, its cost is one worker per attempt, capped by the retry limit,
  and not an outage. This holds only if callers follow the containment
  guidance. A caller that extracts inside its request-serving process takes
  the full risk.
- Fixing a reported crash is ordinary bug-fix work. The minimised input
  joins the regression set, which CI replays on every push.
- The ledger and `run.sh` remain useful. Hours still accumulate when
  someone fuzzes, including in the internal #167 campaign.
- §17.1 gains a risk row for undiscovered crashing and hanging inputs, with
  these mitigations.

## Addendum (2026-10-22)

The rollout no longer has a production shadow step. Running PyMuPDF beside
pdf-extract in the consumer's releases would keep the AGPL dependency shipping
until the switch, so parity is now measured offline on a snapshot of the
consumer's stored documents, and one cut-over release replaces PyMuPDF (design
spec §15.3; change `publish-publicly-for-adoption`). The containment this
record requires is unchanged. It ships in that cut-over release, instead of
before a shadow step, and the cut-over's production traffic is the real-world
exposure.
