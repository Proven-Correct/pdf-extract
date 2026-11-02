# Spec Delta

## Purpose

Coverage-guided fuzzing of the parsing, extraction and rendering entry points under tightened limits. It defines the fuzz targets, what counts as a finding, how reproducers are kept and replayed, where fuzzing runs, and how clean fuzzing time is counted towards the Phase 3 exit criterion.

## ADDED Requirements

### Requirement: Five fuzz targets cover the hostile-input entry points
The system SHALL provide coverage-guided fuzz targets that take raw bytes for:
1. opening a document and reading its page count;
2. extracting the text pages of the first six pages and the last page;
3. rendering page 0 at 36 dpi;
4. parsing a CMap;
5. the font-to-Unicode fallbacks (glyph names, codes, PDF text strings).

#### Scenario: Every target builds and runs
- **WHEN** the fuzz crate is built and each of the five targets is run
- **THEN** each target executes its seeds and generated inputs, and reports its executions and coverage [§13.6]

#### Scenario: Long documents are fuzzed
- **WHEN** a document-level target is given a document of several hundred pages
- **THEN** the document opens (the fuzzing page cap is 1,000), the open target walks its whole page tree, and the text target extracts its first six pages and its last page, so the per-input work stays bounded whatever the page count [§13.6]

#### Scenario: Fuzzing reaches the shipped code
- **WHEN** a target opens or parses an input
- **THEN** it runs the same forked parser, interpreter and codecs, with every vendor patch applied, that the library ships [§13.6; ADR-0003]

### Requirement: Fuzz targets run with every limit tightened
Each fuzz target SHALL run the library with every limit of §10 set, including the page time and operator budgets, image and render pixel caps, decoded-stream cap and file-size cap, at values tight enough that the fuzzer's own timeout and memory ceiling sit above the library's budgets.

#### Scenario: Limits bind before the fuzzer's ceilings
- **WHEN** an input drives a page to its operator budget or time budget, or an image to its pixel cap
- **THEN** the library returns its limit error or warning before the fuzzer's per-input timeout or memory ceiling is reached [§13.6; R2; R4]

### Requirement: Any panic, overrun or crash is a finding
For a fuzz target, a panic of any kind, including a panic in an image codec, SHALL count as a finding, even though production catches some panics. So SHALL a timeout beyond the per-input ceiling, memory beyond the ceiling, and a sanitizer or stack-overflow abort.

#### Scenario: A codec panic is not swallowed while fuzzing
- **WHEN** a fuzz input makes an image codec panic
- **THEN** the fuzz run reports a crash for that input and does not convert it into an `image: decode failed` warning [§13.6; internal #76]

#### Scenario: A finding stops the session with its input kept
- **WHEN** any target records a finding, locally or in the CI smoke run
- **THEN** the run fails and keeps the crashing input and its minimised form (as a run artifact in CI) [§13.6]

### Requirement: Seeds come from the committed corpora
Each document-level fuzz target SHALL be seeded with every corpus B and corpus C file. The CMap and fallback targets SHALL be seeded with representative inputs of their own kind.

#### Scenario: Document seeds
- **WHEN** a fuzz run starts with no accumulated corpus
- **THEN** the open, text and render targets start from all corpus B and corpus C files [§13.6]

### Requirement: Fixed findings leave a committed reproducer
Every fixed finding SHALL leave its minimised input in a committed regression set for its target. An ordinary test run SHALL replay the whole regression set through the same entry point and limits as the fuzz target, without a nightly toolchain.

#### Scenario: Regression replay in the ordinary suite
- **WHEN** the standard test suite runs
- **THEN** every committed regression input is run through its target's entry point, and each ends without panicking [§13.6]

### Requirement: The fuzz corpus is replayed on 512 KiB thread stacks
The committed regression set, and at the end of each local fuzzing session the accumulated corpus, SHALL be replayed through the document-level entry points on threads with a 512 KiB stack.

#### Scenario: Small-stack replay
- **WHEN** the regression set or an accumulated local corpus is replayed on 512 KiB threads
- **THEN** every input ends in a result or a typed error, with no stack overflow and no panic [§8.6; R1]

### Requirement: Long fuzzing runs locally with a persistent corpus
Long fuzzing sessions SHALL run on a developer machine through a committed script. The script SHALL be resumable, SHALL take a per-target duration, and SHALL keep each target's accumulated corpus between sessions. Running it SHALL consume no CI runner time.

#### Scenario: Corpus carries over
- **WHEN** two local sessions run in succession
- **THEN** the second starts each target from the corpus the first one left, merged with the seeds [§13.6]

#### Scenario: Interrupted session
- **WHEN** a session is interrupted before its duration elapses
- **THEN** the corpus grown so far is kept, and only the time actually fuzzed without a finding is credited [§13.6]

### Requirement: CI fuzzing is a manual, time-capped smoke run
CI SHALL run fuzzing only on manual dispatch, never on a schedule or on push, with a short per-target duration whose default bounds the whole run to about one runner-hour. The smoke run SHALL start from the seeds and the regression set.

#### Scenario: No scheduled or push-triggered fuzzing
- **WHEN** commits are pushed to `main` or time passes
- **THEN** no fuzzing job starts. The per-push CI runs only the stable regression replay, and its triggers stay as ADR 0085 sets them [ADR-0085]

#### Scenario: Dispatched smoke run
- **WHEN** the hardening workflow is dispatched with default inputs
- **THEN** each of the five targets fuzzes for its short default duration, and any finding fails the run [§13.6]

### Requirement: Clean fuzzing hours are counted per target in a committed ledger
Each target SHALL have a cumulative clean fuzzing time in a committed ledger, updated by the local fuzzing script. A session with no finding SHALL add the time it fuzzed. A finding SHALL reset that target's total to zero. Phase 3's fuzzing exit criterion SHALL count as met only when every target's total is at least 72 hours.

#### Scenario: Clean session adds hours
- **WHEN** a local session fuzzes a target with no finding
- **THEN** the ledger's clean total for that target grows by the time fuzzed, and the script prints every target's total [§16 Phase 3]

#### Scenario: A finding resets the count
- **WHEN** a target records a finding in a local session
- **THEN** its clean total in the ledger is reset to zero [§16 Phase 3]

#### Scenario: Exit criterion
- **WHEN** every target's clean total in the ledger is at least 72 hours
- **THEN** the script reports the fuzzing exit criterion as met [§16 Phase 3]
