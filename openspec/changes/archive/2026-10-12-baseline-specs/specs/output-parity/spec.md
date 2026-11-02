# output-parity Specification

## Purpose

Defines how pdf-extract's output is held to black-box parity with the PyMuPDF 1.28.2 oracle: the committed oracle expectations, the corpus manifest and its gate kinds, the corpus-level open, routing and similarity gates, the OCR quality gate on corpus D, and the rule that every difference from the oracle is either configurable back to parity or documented.

## ADDED Requirements

### Requirement: Black-box oracle pinned to PyMuPDF 1.28.2
Parity SHALL be measured only against recorded outputs of PyMuPDF 1.28.2 used as a black box. For each input file the oracle record SHALL hold whether it opened (and the exception class name if not), whether it needs a password, the page count, and every page's `get_text("text")` result, keyed by the SHA-256 of the file's bytes and generated from the file as read from disk.

#### Scenario: Expectation from another oracle release
- **WHEN** an expectation file records an oracle version other than `1.28.2`
- **THEN** the expectation consistency check fails, naming the file [D9; 0001 §3.6, §4.13; ADR-0005]

#### Scenario: Optional content measured from disk
- **WHEN** the oracle records a fixture
- **THEN** it opens the file from disk rather than from an in-memory document, because the oracle's optional-content handling differs for unsaved documents [0001 §4.13; ADR-0044]

### Requirement: Gates run from committed expectations without the oracle
The parity gates SHALL compare pdf-extract's output with checked-in expectation files only. The library, its tests and its CI SHALL NOT install or import PyMuPDF; the expectations SHALL be regenerated only by the separate oracle tooling, run by hand, and never by the gates themselves.

#### Scenario: CI parity run
- **WHEN** the CI `parity` job runs
- **THEN** it scores pdf-extract against `tests/fixtures/expectations/*.json` with no PyMuPDF installed anywhere in the job [§13.1; 0001 §3.7, D-21; ADR-0006]

#### Scenario: Re-blessing snapshots never touches expectations
- **WHEN** the parity test is run in snapshot-bless mode
- **THEN** it rewrites only the pinned snapshots of `divergence` and `hostile` fixtures and never an oracle expectation [0001 §4.13; ADR-0006]

### Requirement: Expectations and manifest are mutually consistent
Every expectation file SHALL belong to exactly one manifest fixture (same SHA-256, source path and byte size) or one pinned built file, and every manifest fixture whose gate is not `skip` SHALL have an expectation. An expectation's page count SHALL equal its number of page records, in index order, and each stored page hash SHALL match its stored text.

#### Scenario: Orphan expectation
- **WHEN** an expectation file's SHA-256 matches no manifest fixture and no built-file pin
- **THEN** the consistency check fails, naming the file [0001 §4.13 check_expectations]

#### Scenario: Gated fixture without an expectation
- **WHEN** a manifest fixture with gate `exact`, `divergence`, `hostile`, `similarity` or `ocr` has no expectation
- **THEN** the parity gate fails for that fixture and tells the developer to run the oracle generator [0001 §4.13]

### Requirement: Fixture manifest lists every committed fixture
`tests/fixtures/manifest.toml` SHALL list every committed fixture file exactly once with its SHA-256, generator, compat-point list and one gate kind from `exact`, `similarity`, `divergence`, `hostile`, `ocr` or `skip`. A fixture not produced by the test kit SHALL also have a `PROVENANCE.md` row carrying the same hash.

#### Scenario: Unlisted or mismatched file
- **WHEN** a file under `tests/fixtures/` is missing from the manifest, or its bytes do not hash to the listed SHA-256
- **THEN** the manifest test fails [0001 §3.6, §4.3]

#### Scenario: Unknown gate kind
- **WHEN** a manifest entry names a gate outside the six kinds
- **THEN** the manifest test fails [0001 §3.6; 0002 §3.8]

#### Scenario: Per-gate required keys
- **WHEN** an entry is gated `divergence` without a written `divergence` reason, or `divergence`/`hostile` without a `snapshots/<sha256>.txt` snapshot that exists, or `ocr` without a subset and a ground-truth file
- **THEN** the manifest test fails [0001 §3.6; 0002 §3.8]

### Requirement: Corpus B is redistributable real-world PDFs
Corpus B SHALL consist only of files with a redistribution license, imported from hayro's test PDFs and Apache PDFBox test resources, each manifest entry carrying its origin URL, license and license note. pdf.js test files SHALL NOT be included unless a per-file license is established.

#### Scenario: Corpus B entry without licensing data
- **WHEN** a `corpus-b/` manifest entry lacks `origin`, `license` or `license_note`
- **THEN** the manifest test fails [§13.1; 0001 §4.14, D-25; ADR-0007]

### Requirement: Exact gate
For every fixture gated `exact`, opened from disk with default options, pdf-extract's open outcome class (opened, needs a password, failed) SHALL equal the oracle's, and when the document opens, the page count and every page's compat-profile `text()` SHALL equal the oracle's exactly.

#### Scenario: One character differs
- **WHEN** an `exact` fixture's page text differs from the oracle's by any character
- **THEN** gate C fails, showing both texts [K1; 0001 §4.13 Gate C; ADR-0006]

#### Scenario: Open outcome differs
- **WHEN** the oracle opens an `exact` fixture and pdf-extract raises an open error, or the reverse
- **THEN** gate C fails for that fixture [0001 §4.13 Gate C]

### Requirement: Divergence gate pins documented differences
For every fixture gated `divergence`, pdf-extract's output SHALL equal its pinned snapshot and SHALL differ from the oracle's. Each such entry SHALL carry a written reason naming the compat point or residual it exhibits.

#### Scenario: Divergence disappears
- **WHEN** a `divergence` fixture's output becomes equal to the oracle's
- **THEN** gate C fails and asks for the entry to be made `exact`, so no stale exception is carried [K2; 0001 §4.13 Gate C]

#### Scenario: Output drifts from the snapshot
- **WHEN** a `divergence` fixture's output differs from its snapshot
- **THEN** gate C fails, showing our output and the snapshot [0001 §3.6, §4.13]

### Requirement: Hostile gate never compares oracle text
For every fixture gated `hostile`, pdf-extract's open outcome class SHALL equal the oracle's, its output SHALL equal its pinned snapshot, and the set of warning kinds it reports SHALL equal the entry's `warnings` list exactly. The oracle's page text SHALL NOT be compared.

#### Scenario: Hostile entry without a warnings list
- **WHEN** a `hostile` entry has no `warnings` list
- **THEN** gate C fails for that entry [0001 §4.13, D-27; ADR-0016]

#### Scenario: Oracle text differs on over-deep input
- **WHEN** the oracle's text for a deeply nested hostile fixture differs from our pinned output
- **THEN** the hostile gate still passes provided the outcome class, snapshot and warning kinds match [0001 §2.5 Hostile inputs, D-27; ADR-0016]

### Requirement: Expected warning kinds are exact
For any gated fixture whose manifest entry lists `warnings`, the warning kinds reported by the document and all its pages together, including any reported by the document after extraction, SHALL equal that list exactly, with neither extra nor missing kinds.

#### Scenario: Extra warning kind
- **WHEN** a fixture listing `["ObjectDepth"]` also reports `XrefRepaired`
- **THEN** gate C fails, naming both sets [0001 §3.6, §4.13]

### Requirement: Gate C extraction is panic-attributed and small-stack
Gate C SHALL extract each fixture on a 512 KiB thread stack. A panic during one fixture's extraction SHALL fail the gate for that fixture without preventing the remaining fixtures from being judged or the report from being written.

#### Scenario: Regression panics on one fixture
- **WHEN** extraction of one corpus-C fixture panics
- **THEN** gate C reports that fixture as failed with the panic message and still judges every other fixture [0001 §2.3, §4.13]

### Requirement: Corpus-wide open gate
Over corpus B, every file the oracle opens SHALL also open in pdf-extract, except at most two files on a documented exception list, each with a reason. Files the oracle does not open SHALL contribute nothing to the corpus metrics.

#### Scenario: Stale open exception
- **WHEN** a file on the exception list now opens, or the oracle does not open it
- **THEN** gate B fails and asks for the entry to be removed [0001 §4.14 Gate B (1)]

#### Scenario: Unexcused open failure
- **WHEN** the oracle opens a corpus-B file and pdf-extract does not, and the file is not on the exception list
- **THEN** gate B fails, naming the file and our error [§13.2; 0001 §4.14 Gate B (1)]

### Requirement: Routing-decision gate
The share of pages on which pdf-extract and the oracle agree on `len(text.strip()) >= 50` (Python `str.strip()` semantics) SHALL be at least 99% over corpus B and at least 99.9% over corpus A. Pages are paired by index; a page one side lacks, including one lost to our open failure or page error, SHALL count as empty text on that side.

#### Scenario: Page-count mismatch costs routing
- **WHEN** the oracle reports a page with 80 characters that pdf-extract does not produce
- **THEN** that page counts as empty on our side and as a routing disagreement [K1; §13.2; 0001 §4.14 Gate B (2)]

#### Scenario: Oracle page failure excluded
- **WHEN** the oracle itself failed to extract a page
- **THEN** that page has no reference and is left out of the routing and similarity samples [0001 §4.14]

### Requirement: Character-similarity gate
Per page, similarity SHALL be `1 − Levenshtein(ours, oracle) / max(length)` in characters over stripped texts (1.0 for two empty texts). Over corpus B the median SHALL be at least 0.98 and the 10th percentile at least 0.90; over corpus A the median SHALL be at least 0.999 and the 1st percentile at least 0.98. Percentiles SHALL use the nearest-rank rule.

#### Scenario: Low tail below the floor
- **WHEN** the 10th-percentile similarity over corpus B is below 0.90
- **THEN** gate B fails, stating the measured value and the floor [§13.2; 0001 §4.14 Gate B (3)]

#### Scenario: Even sample count
- **WHEN** the number of pages is even
- **THEN** the median is the lower middle value, never more lenient than interpolation [0001 §4.14]

### Requirement: Gate thresholds change only by spec change
The thresholds of the open, routing and similarity gates for corpora A and B, and of the OCR quality gate, SHALL change only through a spec change. Compat layout defaults SHALL be tuned only within their compat-point brackets, and any such tuning SHALL be followed by re-running gate C and gate B.

#### Scenario: Layout default retuned
- **WHEN** a compat layout threshold default changes
- **THEN** the whole corpus is re-scored by gates C and B before the change lands [§13.2; 0001 §4.14]

### Requirement: Parity reports are produced on every run
Each parity run SHALL write a per-fixture, per-page report (lengths, Levenshtein distance, similarity, outcome classes, warning kinds) for corpus C and a corpus-level report for corpus B, and CI SHALL keep both as job artefacts even when the gates fail. Exact-match rate after strip and per-page line-count delta SHALL be reported and SHALL NOT gate.

#### Scenario: Failing parity job
- **WHEN** the CI `parity` job fails
- **THEN** `parity-report.json` and `parity-report-b.json` are still uploaded as artefacts [§13.2; 0001 §3.7, §4.13]

#### Scenario: Line-count delta over two
- **WHEN** a corpus-B page's line count differs from the oracle's by more than 2
- **THEN** it is reported for investigation and the gate result is unaffected [§13.2; 0001 §4.14 Gate B (4)]

### Requirement: Private corpus A gate
The corpus-A gate SHALL run only by hand against a private directory outside the repository, never on CI, applying 100% opens with no exceptions, routing agreement of at least 99.9%, median similarity of at least 0.999 and 1st-percentile similarity of at least 0.98. Its expectations and report SHALL stay outside the repository, and the report SHALL contain metrics and lengths only, never page text.

#### Scenario: Corpus inside the repository
- **WHEN** the private-corpus script is given a path inside the repository
- **THEN** it refuses with exit status 2 [§13.1; 0001 §4.14]

#### Scenario: Private OCR scans
- **WHEN** the private-scans script runs over a directory of scans
- **THEN** it refuses in-repository paths and writes a report without text, with the same 0/1/2 exit statuses [0002 §3.8, §4.12]

### Requirement: Parity and OCR quality gates block CI
The CI pipeline SHALL run gates C and B on every pull request and every push to the main branch, and SHALL run the corpus-D OCR quality gate with the pinned model; a failure of either SHALL fail the pipeline. Performance numbers SHALL NOT gate.

#### Scenario: Gate failure
- **WHEN** gate C, gate B or an O34 CER gate fails on CI
- **THEN** the corresponding job (`parity` or `corpus-d`) fails and the pipeline is red [0001 §3.7; 0002 §3.7, §4.12; ADR-0013]

### Requirement: Corpus D text path agrees with the oracle
Every corpus-D fixture (gate `ocr`) SHALL also pass the exact gate on the text path: the oracle and pdf-extract both open it with the same page count and both extract empty text from its image-only pages.

#### Scenario: Image-only scan
- **WHEN** a corpus-D PDF is extracted through `text()`
- **THEN** each page's text equals the oracle's empty text [0002 §3.8, §4.12]

### Requirement: OCR quality gate on corpus D
Over corpus D, CER SHALL be the Levenshtein distance between recognised text and ground truth over NFC-normalised, whitespace-collapsed, case-preserving text, divided by the ground-truth length in code points, aggregated as the per-subset mean. Ours at 72 dpi SHALL be at most the oracle's 72-dpi CER plus 0.01 on the clean, noise, skew and blur subsets, and ours at 300 dpi SHALL be strictly below both on the clean subset.

#### Scenario: 72 dpi parity tolerance
- **WHEN** ours@72 mean CER on the skew subset exceeds the oracle@72 mean plus 0.01
- **THEN** the OCR quality gate fails, naming gate O34(a) and the subset [§13.4; 0002 O34; ADR-0065]

#### Scenario: 300 dpi must improve on clean pages
- **WHEN** ours@300 mean CER on the clean subset is not below min(ours@72, oracle@72)
- **THEN** the OCR quality gate fails, naming gate O34(b) [§13.4; 0002 O34; ADR-0065]

#### Scenario: Noise variant at 300 dpi
- **WHEN** ours@300 on the noise subset is worse than ours@72
- **THEN** the gate is unaffected, because noise, skew and blur are gated by O34(a) only [0002 O34 (c), §4.12]

### Requirement: OCR quality reporting is report-only beyond CER
WER, per-page latency, process-isolation overhead and peak memory per instance SHALL be recorded in the OCR quality report and SHALL NOT gate. The low-resolution and JPEG subsets SHALL be scored and reported but not gated.

#### Scenario: Slow page on a noisy runner
- **WHEN** per-page latency on CI doubles on unchanged code
- **THEN** the OCR quality gate result is unaffected and the latency appears only in `ocr-report.json` [§13.4; 0002 O34; ADR-0013, ADR-0065]

### Requirement: OCR comparisons are bound to the pinned model
The oracle's corpus-D OCR SHALL be recorded at 72 dpi, language `eng`, with the SHA-256 of the model used. The quality gate SHALL fail for any page whose expectation was made with a model other than the one in use, and SHALL skip with a message, not fail, when no model can be found.

#### Scenario: Model mismatch
- **WHEN** the resolved `eng.traineddata` hashes differently from the expectation's recorded model
- **THEN** the gate fails for that page, naming both hashes, because the comparison would be meaningless [0002 O34, §4.12, D-17]

#### Scenario: No model available
- **WHEN** no `eng.traineddata` resolves on a developer machine
- **THEN** the quality test reports that it skipped and does not fail [0002 §4.12]

### Requirement: Built corpus-D files are verified before scoring
Corpus-D files that are built at test time rather than committed SHALL be rebuilt deterministically from committed sources and checked against their SHA-256 pins in the manifest before any recognition is scored; built files SHALL NOT be committed.

#### Scenario: Built file drifts
- **WHEN** a built noise variant hashes differently from its `[[built]]` pin
- **THEN** the quality gate fails before scoring, naming the file, both hashes and the re-pin steps [0002 §3.8, §4.12; ADR-0064]

### Requirement: Every difference from the oracle is configurable or documented
Every behaviour in which pdf-extract differs from the oracle SHALL either be selectable back to the oracle's behaviour through a public option or be documented as a deliberate difference or residual, together with a divergence-gated fixture wherever the corpus exhibits it.

#### Scenario: Configurable differences
- **WHEN** a caller needs oracle-equivalent behaviour for OCR resolution or block separation
- **THEN** `OcrEngine(dpi=72)` gives the oracle's OCR resolution and the default `block_separator="newline"` gives the oracle's block shape, the opt-in `"blank_line"` being the only departure [K2; §15.2]

#### Scenario: Always-on documented differences
- **WHEN** a behaviour differs from the oracle with no parity setting
- **THEN** it is one of the documented changes: OCR unavailable without `eng.traineddata`, `PasswordRequiredError` at open, honest `page_count`, U+FFFD for unsafe unmapped codes, silence on stdout and stderr, a different rasteriser and Tesseract build, or a documented compat-point divergence such as Tr 4–6 emitted once [K2; §15.2; 0001 §1.8, §2.5]

#### Scenario: Corpus exhibits a new difference
- **WHEN** a corpus-C fixture's output differs from the oracle's and the difference is accepted
- **THEN** the fixture is gated `divergence` with a written reason and a pinned snapshot, never silently dropped or skipped [K2; 0001 §3.6, §4.13]
