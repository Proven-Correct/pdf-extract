## MODIFIED Requirements

### Requirement: Clean fuzzing hours are counted per target in a committed ledger
Each target SHALL have a cumulative clean fuzzing time in a committed ledger, updated by the local fuzzing script. A session with no finding SHALL add the time it fuzzed. A finding SHALL reset that target's total to zero. The ledger SHALL report progress against a depth goal of 72 hours per target. The goal SHALL NOT be a Phase 3 exit criterion or a release gate, and the status report SHALL say so.

#### Scenario: Clean session adds hours
- **WHEN** a local session fuzzes a target with no finding
- **THEN** the ledger's clean total for that target grows by the time fuzzed, and the script prints every target's total [§16 Phase 3]

#### Scenario: A finding resets the count
- **WHEN** a target records a finding in a local session
- **THEN** its clean total in the ledger is reset to zero [§16 Phase 3]

#### Scenario: Exit criterion
- **WHEN** every target's clean total in the ledger is at least 72 hours
- **THEN** the script reports the 72-hour depth goal as met. No Phase 3 exit or release depends on that report [ADR-0093]

#### Scenario: Goal not met does not gate a release
- **WHEN** any target's clean total is below 72 hours
- **THEN** the script reports the depth goal as not met and states that it is not a release gate [ADR-0093]
