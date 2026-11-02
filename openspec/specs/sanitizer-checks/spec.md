# sanitizer-checks Specification

## Purpose

On-demand runs of the Rust test suite under AddressSanitizer and ThreadSanitizer, including the vendored C and C++ reached through the Tesseract FFI. They catch memory-safety and data-race defects that ordinary tests cannot observe, without recurring CI cost.

## Requirements

### Requirement: The Rust test suite can run under AddressSanitizer
The Rust workspace test suite SHALL be runnable with AddressSanitizer, including leak detection, through a committed local script and a manually dispatched CI job. The vendored Tesseract and Leptonica C and C++ code and the FFI shim SHALL be instrumented as well.

#### Scenario: Clean ASan run
- **WHEN** the AddressSanitizer run executes the workspace tests
- **THEN** it passes only if every included test passes and the sanitizer reports no error [§13.3]

#### Scenario: The FFI path is instrumented
- **WHEN** the in-process OCR tests run under AddressSanitizer
- **THEN** the vendored Tesseract code they call is compiled with the sanitizer, so an out-of-bounds access inside it is reported [§13.3]

### Requirement: The Rust test suite can run under ThreadSanitizer
The Rust workspace test suite SHALL be runnable with ThreadSanitizer, through the same script and dispatched job. The standard library and the vendored Tesseract and Leptonica code SHALL be built with the sanitizer, so races across the FFI boundary and in the concurrency tests are reported.

#### Scenario: Clean TSan run
- **WHEN** the ThreadSanitizer run executes the workspace tests, including the concurrency and OCR pool tests
- **THEN** it passes only if every included test passes and the sanitizer reports no data race [§13.3]

### Requirement: Sanitizer runs are manual only
Sanitizer jobs SHALL run in CI only on manual dispatch, never on a schedule or on push, and the per-push CI SHALL NOT depend on them.

#### Scenario: No recurring sanitizer cost
- **WHEN** commits are pushed to `main` or time passes
- **THEN** no sanitizer job starts [ADR-0085]

#### Scenario: Report kept
- **WHEN** a dispatched sanitizer job reports an error
- **THEN** the job fails and the sanitizer's report is uploaded as a run artifact [§13.3]

### Requirement: Sanitizer suppressions are explicit and justified
Any suppression of a sanitizer report SHALL live in a committed suppressions file. Each entry SHALL name the code it covers and why the report is a false positive or out of our control. A suppression SHALL NOT cover code in this workspace's own crates.

#### Scenario: Suppression without a reason
- **WHEN** a suppressions file contains an entry without a justification comment, or an entry matching a function in the workspace crates
- **THEN** the sanitizer run fails before running the tests, and the per-push lint check of the suppressions files fails too [§13.3]
