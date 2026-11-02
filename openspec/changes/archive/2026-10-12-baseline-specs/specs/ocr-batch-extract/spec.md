# ocr-batch-extract Specification

## Purpose

The `extract()` batch helper reads every page's text layer and sends each sparse page to OCR, choosing one text source per page. It returns one result row per page in page order, plus a document-level OCR status and a list of warnings.

## ADDED Requirements

### Requirement: Rust batch entry point
The system SHALL provide `Document::extract(&ExtractOptions) -> Result<Extraction, PageError>`. `ExtractOptions` SHALL default to `min_chars = 50`, `ocr = None`, `max_ocr_pages = 500`, `workers = 1`, `block_separator = Newline`, and default text options.

#### Scenario: Default options
- **WHEN** a Rust caller builds `ExtractOptions::default()`
- **THEN** `min_chars` is 50, `ocr` is `None`, `max_ocr_pages` is 500, `workers` is 1 and `block_separator` is `Newline` [§12.4; 0002 O22]

#### Scenario: Extraction on an open document
- **WHEN** `doc.extract(&ExtractOptions::default())` is called on an open document
- **THEN** it returns `Ok(Extraction)` with one row for each of the document's pages [§12.4; 0002 O22]

### Requirement: Pluggable recogniser
The Rust helper SHALL get its OCR through the public, object-safe `PageRecognizer` trait (`Send + Sync`), passed as `ExtractOptions.ocr`. The OCR engine of `pdf-extract-ocr` SHALL implement that trait, so building the core crate never needs the OCR engine.

#### Scenario: Engine passed as recogniser
- **WHEN** a Rust caller sets `ExtractOptions.ocr = Some(&engine)`, where `engine` is an `OcrEngine`
- **THEN** `Document::extract` recognises sparse pages with that engine [0002 O22; ADR-0056]

#### Scenario: Stand-in recogniser
- **WHEN** a caller passes any other type that implements `PageRecognizer`
- **THEN** `Document::extract` applies the same routing policy to that type's results [0002 O22; ADR-0056]

### Requirement: Python batch entry point
The system SHALL provide `pdf_extract.extract(path, *, max_pages=None, min_chars=50, ocr=None, max_ocr_pages=500, workers=1, block_separator="newline", limits=None) -> Extraction`. All arguments after `path` SHALL be keyword-only. It SHALL open `path` with `max_pages` and `limits` and then apply the same policy as `Document::extract`.

#### Scenario: Open-time options are applied
- **WHEN** `extract(path, max_pages=10)` is called on a 40-page document
- **THEN** it raises `TooManyPagesError` [0002 O22]

#### Scenario: Open failures surface as open exceptions
- **WHEN** `extract()` is given a path that does not exist
- **THEN** it raises `DocumentNotFoundError` [0002 O22]

#### Scenario: Limits reach the text pass
- **WHEN** `extract(path, limits=Limits(max_ops_per_page=50))` is called and a page has more operators than that
- **THEN** that page's text pass hits the limit and is handled as a text-pass limit [0002 O22, O23]

### Requirement: Python argument validation
The Python `extract()` SHALL raise `ValueError` when `workers` is below 1, when `min_chars`, `max_ocr_pages` or `max_pages` is negative, or when `block_separator` is not a recognised separator name.

#### Scenario: Zero workers
- **WHEN** `extract(path, workers=0)` is called
- **THEN** it raises `ValueError` with a message saying workers must be at least 1 [0002 O23]

#### Scenario: Negative counts
- **WHEN** `extract()` is called with `min_chars=-1`, `max_ocr_pages=-1` or `max_pages=-1`
- **THEN** it raises `ValueError` naming the argument [0002 O22]

#### Scenario: Unknown block separator
- **WHEN** `extract(path, block_separator="none")` is called
- **THEN** it raises `ValueError` [0002 O22]

### Requirement: Rust zero workers
The Rust `Document::extract` SHALL treat `workers = 0` as `workers = 1`.

#### Scenario: workers set to zero in Rust
- **WHEN** a Rust caller passes `ExtractOptions { workers: 0, .. }`
- **THEN** the call behaves as with `workers = 1` and does not fail [0002 O23]

### Requirement: One row per page in page order
`Extraction` SHALL have exactly one `PageResult { index, text, source, ocr_error }` per page, in page order, and `index` SHALL be the page's 0-based index. `Extraction.page_count` SHALL equal the number of rows.

#### Scenario: Forty-page document
- **WHEN** a 40-page document is extracted
- **THEN** `page_count == 40`, `pages` has 40 entries, and `pages[i].index == i` for every `i` [§12.4; 0002 O22]

#### Scenario: Document with no pages
- **WHEN** a document with zero pages is extracted
- **THEN** the `Extraction` has `page_count == 0`, no rows and `ocr_status == NotNeeded` [0002 O22, O23]

### Requirement: Python result values
In Python, `PageResult` and `Extraction` SHALL be immutable, getter-only classes in module `pdf_extract`. `Extraction.pages` SHALL be a tuple and `Extraction.warnings` a tuple of `str`. `PageResult.source` SHALL be one of `"text"`, `"ocr"` or `"text_fallback"`. `Extraction.ocr_status` SHALL be one of `"not_needed"`, `"ok"`, `"unavailable"`, `"partial"` or `"budget_exhausted"`.

#### Scenario: Assigning to a result fails
- **WHEN** Python code assigns to `extraction.pages[0].text`
- **THEN** `AttributeError` is raised [0002 §3.6]

#### Scenario: String vocabularies
- **WHEN** a text-only extraction completes
- **THEN** every `PageResult.source` is `"text"` and `ocr_status` is `"not_needed"` [§12.2; 0002 §3.6]
- **AND** `warnings` is a tuple of strings, each the display text of one warning [0002 §3.6]

### Requirement: Text pass equals per-page text
The text pass SHALL extract every page's text layer. Each page's text SHALL equal what that page's own text extraction returns with the same `block_separator`.

#### Scenario: Text matches Page.text
- **WHEN** a document is extracted without OCR
- **THEN** each `PageResult.text` equals `doc[i].text()` [§12.4; 0002 O22]

#### Scenario: Block separator is honoured
- **WHEN** `extract(path, block_separator="blank_line")` is called on a page with two blocks
- **THEN** the page's text equals `page.text(block_separator="blank_line")`, with the blocks separated by a blank line [0002 O22]

### Requirement: Parallel text pass
The text pass SHALL split the page indices into `workers` contiguous ranges and process one range per worker. The resulting `Extraction` SHALL be identical whatever the worker count.

#### Scenario: Worker count does not change text results
- **WHEN** a 40-page document is extracted once with `workers = 1` and once with `workers = 4`
- **THEN** the two `Extraction` values are equal [§12.4, C6; 0002 O22]

### Requirement: Sparse-page rule
A page SHALL be sparse when its text, trimmed of Unicode whitespace at both ends, has fewer than `min_chars` code points. This SHALL match Python's `len(text.strip()) < min_chars`.

#### Scenario: Just below and at the threshold
- **WHEN** one page has 49 non-whitespace characters plus 20 spaces and another has 50 non-whitespace characters, with `min_chars = 50`
- **THEN** the first page is sparse and the second is not [§12.4; 0002 O22; ADR-0070]

#### Scenario: Unicode whitespace is trimmed
- **WHEN** a page's text consists only of U+00A0 and U+2003 characters
- **THEN** the page is sparse [0002 O22; ADR-0070]

#### Scenario: Same verdict as Python strip
- **WHEN** the shared table of test strings is evaluated by the Rust rule and by `len(s.strip()) < 50` in Python
- **THEN** both give the same verdict for every string [0002 O22]

#### Scenario: Zero threshold
- **WHEN** `min_chars = 0`
- **THEN** no page is sparse and the recogniser is never called [0002 O22]

### Requirement: OCR off means text only
When no recogniser is given, or no page is sparse, every row SHALL have `source = Text` and `ocr_error = None`, and `ocr_status` SHALL be `NotNeeded`.

#### Scenario: No engine
- **WHEN** `extract()` is called with `ocr = None` on a document with sparse pages
- **THEN** every row is `Text` with no `ocr_error`, and `ocr_status == NotNeeded` [§12.4; 0002 O23]

#### Scenario: Engine given but nothing sparse
- **WHEN** an engine is given and every page has at least `min_chars` trimmed code points
- **THEN** the engine is never called and `ocr_status == NotNeeded` [0002 O23]

### Requirement: OCR budget selects in page order
When a recogniser is given, the system SHALL recognise only the first `max_ocr_pages` sparse pages in page order. A sparse page beyond the budget SHALL never be attempted, and its row SHALL stay `source = Text` with `ocr_error = None`.

#### Scenario: Budget smaller than sparse count
- **WHEN** pages 3, 7 and 12 of 40 are sparse and `max_ocr_pages = 2`
- **THEN** pages 3 and 7 are recognised and page 12 is not attempted (`source == Text`, `ocr_error == None`) [§12.4; 0002 O23; ADR-0070]
- **AND** with `workers = 1` the recogniser is called for pages 3 then 7; with `workers = 4` it is called for the same set of pages [0002 O24]

#### Scenario: Zero budget
- **WHEN** `max_ocr_pages = 0` and some pages are sparse
- **THEN** no page is recognised and `ocr_status == BudgetExhausted` [0002 O23]

### Requirement: Non-empty OCR replaces the text layer
When recognition of a selected page returns text that is non-empty after Unicode whitespace trimming, that page's row SHALL hold the OCR text (formatted with `block_separator`), with `source = Ocr` and `ocr_error = None`.

#### Scenario: Successful recognition
- **WHEN** the recogniser returns non-empty text for a sparse page
- **THEN** that row's `text` is the OCR text, `source == Ocr` and `ocr_error == None` [§12.4; 0002 O23]

### Requirement: Empty OCR keeps the text layer
When recognition of a selected page succeeds but returns text that is empty or whitespace-only after Unicode trimming, the row SHALL keep the text-layer text, with `source = TextFallback` and `ocr_error = None`. This SHALL count as a successful recognition for `ocr_status`.

#### Scenario: Empty result
- **WHEN** the recogniser returns an empty result for page 7, and every other recognition succeeds
- **THEN** page 7 is `TextFallback` with `ocr_error == None`, and `ocr_status == Ok` [§12.4; 0002 O23]

#### Scenario: Page skipped by the noise pre-check
- **WHEN** the engine's noise pre-check skips a sparse page
- **THEN** the row is `TextFallback` with `ocr_error == None`, and `ocr_status` is `Ok` if nothing else failed [0003 F14; ADR-0070]

### Requirement: Page-level OCR failure keeps the text layer
When recognition of a selected page fails with a page-level OCR error, including a timeout, the row SHALL keep the text-layer text, with `source = TextFallback` and `ocr_error` set to the error's message. The other selected pages SHALL still be attempted.

#### Scenario: Recognition error
- **WHEN** the recogniser fails page 7 with a recognition error
- **THEN** page 7 is `TextFallback` and `ocr_error` holds the error message [§12.4; 0002 O23]

#### Scenario: Timeout
- **WHEN** recognition of a page exceeds the engine's page timeout of 0.5 s
- **THEN** that row's `ocr_error` is `"OCR exceeded the page timeout of 500ms"` [0002 O23]

### Requirement: Unavailable OCR stops the OCR pass
The first OCR-unavailable error SHALL stop all further recognition for the document. The page that reported it, and every selected page not yet started, SHALL be `TextFallback`, with `ocr_error` set to the unavailability message.

#### Scenario: Unavailable on the first candidate
- **WHEN** pages 3, 7 and 12 are sparse and the recogniser reports OCR unavailable on page 3
- **THEN** pages 3, 7 and 12 are all `TextFallback` with the unavailability message, and `ocr_status == Unavailable` [§12.4; 0002 O23]
- **AND** the recogniser receives no call after the one that failed [0002 O23]

#### Scenario: No new start in parallel
- **WHEN** OCR becomes unavailable while `workers > 1`
- **THEN** no worker starts a new recognition after that point [0002 O24]

### Requirement: OCR status and precedence
When at least one page is sparse and a recogniser is given, `ocr_status` SHALL be `Unavailable` if OCR became unavailable, otherwise `BudgetExhausted` if there were more sparse pages than `max_ocr_pages`, otherwise `Partial` if any page-level OCR error occurred, otherwise `Ok`.

#### Scenario: Budget outranks partial
- **WHEN** one recognition fails and there are more sparse pages than `max_ocr_pages`
- **THEN** `ocr_status == BudgetExhausted` [0002 O23; ADR-0070]

#### Scenario: Unavailable outranks budget
- **WHEN** OCR becomes unavailable and there are more sparse pages than `max_ocr_pages`, with `workers` of 1 or more
- **THEN** `ocr_status == Unavailable` [0002 O23; ADR-0070]

#### Scenario: All within budget succeed
- **WHEN** every selected sparse page is recognised without error and all sparse pages fit within the budget
- **THEN** `ocr_status == Ok` [0002 O23]

#### Scenario: A failure within budget
- **WHEN** one selected page fails with a page-level OCR error, all sparse pages fit within the budget, and OCR stays available
- **THEN** `ocr_status == Partial` [§12.4; 0002 O23]

### Requirement: Text-pass limit yields an empty page
When a page's text pass exceeds a per-page budget, `extract()` SHALL NOT fail. That row SHALL have `text = ""`, `source = Text` and `ocr_error = None`. The page SHALL NOT be sent to OCR, and a `PageLimit` warning SHALL be added that displays as `page N: <limit error message>`, where `N` is the 0-based index.

#### Scenario: Operator limit on page 1
- **WHEN** page 1's text pass exceeds `max_ops_per_page`
- **THEN** page 1 has `text == ""`, `source == Text` and `ocr_error == None` [0002 O23; ADR-0070]
- **AND** `Extraction.warnings` contains an entry starting `page 1: limit ops_per_page exceeded` [0002 O23]

#### Scenario: Limit-hit page is not recognised
- **WHEN** a recogniser is given and a page hits a text-pass limit
- **THEN** the recogniser is not called for that page [0002 O23; ADR-0070]

### Requirement: Warning order
`Extraction.warnings` SHALL list the document's own warnings first, then the text-pass page-limit warnings, then, in page order, one OCR note for each recognised page whose result carried a noise pre-check warning. Each OCR note's displayed text SHALL be `page N: ocr: <text>`, where `N` is the 0-based index.

#### Scenario: Document warnings come first
- **WHEN** the document has open-time warnings (for example from xref repair)
- **THEN** they appear at the start of `Extraction.warnings` [0002 O23]

#### Scenario: Noise notes after limit warnings
- **WHEN** page 3 is skipped by the noise pre-check and another page is downsampled, with `workers = 4`
- **THEN** `Extraction.warnings` ends with one `OcrNoiseSkipped` and one `OcrNoiseDownsampled` entry, in page order, after any page-limit warnings [0003 F14; ADR-0070]
- **AND** the skipped page's entry starts with `page 3: ocr: noise-dominated raster` [0003 F14]

### Requirement: Deterministic page selection
For given input bytes, options and library version, the set of pages attempted by OCR SHALL depend only on page order and `min_chars`, never on worker count or scheduling. The one documented exception: with `workers > 1`, which pages already in flight when OCR becomes unavailable still end as `Ocr` depends on timing.

#### Scenario: Same pages recognised for any worker count
- **WHEN** the same document is extracted with the same recogniser using `workers = 1` and `workers = 4`
- **THEN** the same pages are recognised and the rows are equal [C6, §12.4; 0002 O24]

#### Scenario: Unavailable in flight
- **WHEN** OCR becomes unavailable while other workers are mid-recognition
- **THEN** those in-flight pages may end as `Ocr`; this is the only timing-dependent part of the result [0002 O24; ADR-0070]

### Requirement: Bounded scoped worker threads
With `workers = 1`, `extract()` SHALL create no thread. With `workers > 1`, it SHALL use scoped threads, with at most `workers` running per pass (the calling thread included), all joined before the call returns. OCR concurrency SHALL be at most `workers`, and the engine's pool SHALL bound Tesseract concurrency independently.

#### Scenario: Single worker
- **WHEN** `extract()` runs with `workers = 1`, with or without OCR
- **THEN** no thread is spawned and at most one recognition runs at a time [0002 O24]

#### Scenario: Four workers
- **WHEN** `extract()` runs with `workers = 4`
- **THEN** no more than 4 threads (including the caller) run the text pass, and no more than 4 recognitions run at once [0002 O24]
- **AND** no extra thread remains once the call returns [0002 O24]

### Requirement: Worker panics propagate
A panic in any worker of `extract()`, whether in the text pass or the recogniser, SHALL be resumed on the calling thread with its original payload, after all workers have joined. At the Python boundary it SHALL become `InternalError`.

#### Scenario: Recogniser panics in Rust
- **WHEN** the recogniser panics while handling a page
- **THEN** `Document::extract` panics on the calling thread with the same payload, for `workers` 1 and 4 [0002 O24]

#### Scenario: Recogniser panics in Python
- **WHEN** a scripted engine panics with `"scripted worker panic"` inside `pdf_extract.extract`
- **THEN** `InternalError` is raised and its message contains `"scripted worker panic"`, for `workers` 1 and 4 [0002 O24, O39]

### Requirement: Close during extraction
If the document is closed before or during `Document::extract`, the call SHALL return `Err(PageError::Closed)` and discard any partial result. Closing concurrently from another thread SHALL never cause a panic.

#### Scenario: Already closed
- **WHEN** `extract` is called on a closed document
- **THEN** it returns `Err(PageError::Closed)` [0002 O25]

#### Scenario: Closed mid-call
- **WHEN** another thread closes the document while `extract` is running, repeated over 100 rounds
- **THEN** each round returns either `Err(PageError::Closed)` or a complete `Extraction`, and never panics [0002 O25]

#### Scenario: Recogniser reports the document closed
- **WHEN** the recogniser returns the closed-document OCR error for a page
- **THEN** `extract` returns `Err(PageError::Closed)` [0002 O25]
