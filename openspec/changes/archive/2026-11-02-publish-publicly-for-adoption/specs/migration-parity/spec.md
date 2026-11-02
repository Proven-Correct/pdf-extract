## Purpose

Lets an application that is migrating from PyMuPDF measure offline, on its own documents and without storing any extracted text, whether pdf-extract's output agrees with its current extractor, using the same per-page metrics and gates the project applies to its own corpora (spec §13.2), so that the cut-over release is the first and only one in which PyMuPDF is replaced.

## ADDED Requirements

### Requirement: Page comparison returns the parity metrics only
The core library SHALL provide a public Rust function that compares a reference page text with a candidate page text and returns only the §13.2 per-page metrics:
- OCR-routing agreement (both texts, after stripping whitespace, are at least 50 characters long, or both are shorter);
- character similarity, 1 − normalised Levenshtein distance over Unicode scalar values;
- exact match after stripping whitespace;
- the signed line-count delta.

The result SHALL NOT contain or retain either input text.

#### Scenario: Identical pages
- **WHEN** the reference and candidate texts are equal
- **THEN** similarity is 1.0, exact match is true, routing agrees and the line delta is 0

#### Scenario: Similarity matches the project's parity definition
- **WHEN** two different non-empty texts are compared
- **THEN** similarity equals 1 − levenshtein(a, b) / max(len(a), len(b)), counted in Unicode scalar values, exactly as the project's own parity test computes it [§13.2]

#### Scenario: Routing disagreement
- **WHEN** the reference has 60 non-whitespace-stripped characters and the candidate has 10
- **THEN** routing agreement is false

#### Scenario: Both pages empty
- **WHEN** both texts are empty or whitespace only
- **THEN** similarity is 1.0, exact match is true and routing agrees

#### Scenario: No text retained
- **WHEN** a page comparison or a report is inspected
- **THEN** it holds only numbers and booleans, and no field holds either input text

### Requirement: Gate scoring over many pages
The core library SHALL provide a public way to accumulate page comparisons and report the §13.2 aggregates and gate verdicts: the routing-agreement rate (gate at least 99.9%), the median similarity (gate at least 0.999), the 1st-percentile similarity (gate at least 0.98), the exact-match rate (reported, no gate) and the line-delta distribution (reported). The accumulator SHALL be mergeable, so per-process or per-day partial results can be combined.

#### Scenario: Gates pass
- **WHEN** 1,000 pages are accumulated with routing agreement on all of them and every similarity at 1.0
- **THEN** each gated aggregate reports a pass

#### Scenario: Routing gate fails
- **WHEN** 2 of 1,000 accumulated pages disagree on routing
- **THEN** the routing-agreement rate is 99.8% and its gate reports a fail

#### Scenario: Partial results merge
- **WHEN** two accumulators with disjoint pages are merged
- **THEN** the merged report equals the report of one accumulator fed all those pages

#### Scenario: Empty accumulator
- **WHEN** no pages have been accumulated
- **THEN** the report states that no pages were scored and reports no gate as passed

### Requirement: Offline corpus scoring uses the parity module
The private-corpus parity run (`tools/oracle/run_private_corpus.sh`, gate A) SHALL score with the core parity module and the §13.2 gates, SHALL require every file the reference extractor opens to open, and SHALL write a report that holds lengths and metrics but no page text, outside the repository.

#### Scenario: Gate A uses the library's gates
- **WHEN** the gate-A run scores a corpus
- **THEN** its thresholds are the core library's §13.2 gates, and its per-page similarities equal `compare_page`'s

#### Scenario: Report holds no text
- **WHEN** the gate-A report is read
- **THEN** no field holds page text

### Requirement: Migration guide describes an offline rollout
The project SHALL publish a PyMuPDF migration guide that names no specific consumer. It SHALL cover the call mapping, the behaviour changes and their parity settings, and failure containment for untrusted input (§8.5). Its rollout SHALL score parity offline on a snapshot of the consumer's documents, then make one cut-over release that replaces PyMuPDF and removes the dependency. It SHALL NOT recommend running PyMuPDF beside pdf-extract in production. Its Python code examples SHALL be executed by the test suite.

#### Scenario: No production shadow
- **WHEN** the guide's rollout section is read
- **THEN** it measures parity offline and cuts over in a single release, and does not recommend running both extractors in production

#### Scenario: Examples run
- **WHEN** the test suite runs
- **THEN** every Python code example in the migration guide executes without error against the built package
