# Spec Delta

## ADDED Requirements

### Requirement: The close protocol is model-checked
The document close protocol SHALL be covered by exhaustive model-checking tests, using the same synchronisation code the library ships. The tests SHALL explore every interleaving, within the model checker's bounds, of operations with a concurrent close, a repeated close and the release of the last operation.

#### Scenario: Close races an operation
- **WHEN** the model checker interleaves one operation acquiring the document with a concurrent `close()`
- **THEN** in every interleaving the operation either completes on the loaded document or fails as closed, and never observes a partially closed state [§8.2; §13.3; ADR-0040]

#### Scenario: Memory released by the last operation
- **WHEN** the model checker interleaves `close()` with two in-flight operations finishing
- **THEN** in every interleaving the document is released exactly once, after both operations and the close have finished with it [§8.2; §13.3]

#### Scenario: Double close
- **WHEN** two threads call `close()` concurrently with an operation
- **THEN** in every interleaving the document is released exactly once and neither close blocks the operation [§8.2]

### Requirement: The OCR pool's checkout and return are model-checked
The OCR instance pool's checkout and return SHALL be covered by exhaustive model-checking tests, using the same synchronisation code the library ships, with a scripted factory that needs no Tesseract.

#### Scenario: Never more than max_instances
- **WHEN** the model checker interleaves three callers over a pool with `max_instances = 2`
- **THEN** in no interleaving are more than two instances live or reserved, and every caller eventually gets an instance (no lost wake-up) [§13.3; ocr-engine "A bounded pool of instances"]

#### Scenario: Failed creation frees its slot
- **WHEN** an instance creation fails while another caller waits on a full pool
- **THEN** in every interleaving the waiting caller is woken and the live count returns to its prior value [§13.3]

#### Scenario: Return across a fork reset
- **WHEN** the model checker interleaves an instance being returned with a simulated process-id change seen by another caller
- **THEN** in every interleaving the old instance is abandoned rather than reused, the live count is never negative or above the limit, and no waiting caller is left asleep [§13.3; concurrency "Lazily created OCR state survives fork() when no OCR is in progress"]

### Requirement: Model-checking tests run in per-push CI
The model-checking tests SHALL run as part of the per-push CI on the pinned stable toolchain, and a failure SHALL fail CI.

#### Scenario: Model checker failure
- **WHEN** a change to the close protocol or the pool introduces an interleaving that breaks an asserted property
- **THEN** the CI model-checking job fails and reports the failing interleaving [§13.3]
