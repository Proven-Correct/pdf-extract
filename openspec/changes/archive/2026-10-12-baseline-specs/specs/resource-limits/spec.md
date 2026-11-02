# resource-limits Specification

## Purpose

Bound the work, time, memory and recursion that any one document, page or OCR call can cost, through limits with documented, overridable defaults, so hostile input cannot exhaust the process.

## ADDED Requirements

### Requirement: Every growable quantity is capped by an overridable limit
The system SHALL cap every quantity that hostile input can grow by a limit with a documented default that the caller can override. Document limits SHALL live in one `Limits` value (fields `max_file_size`, `max_page_tree_depth`, `max_objects`, `max_stream_decoded_bytes`, `max_ops_per_page`, `page_time_budget`, `max_chars_per_page`, `max_image_pixels`); the OCR limits `page_timeout` and `max_render_pixels` SHALL live on the OCR engine configuration.

#### Scenario: Defaults of the limits this capability owns
- **WHEN** a caller constructs `Limits` with no arguments (Rust `Limits::default()`, Python `Limits()`)
- **THEN** `max_stream_decoded_bytes` is 268,435,456 (256 MiB), `max_ops_per_page` is 10,000,000, `page_time_budget` is 30 seconds and `max_image_pixels` is 256,000,000 [R2; §10; 0001 §1.4; 0002 O4]
- **AND** the defaults of `max_file_size` and `max_objects` (document-open), `max_page_tree_depth` (page-index) and `max_chars_per_page` (text-extraction) are those their owning capabilities state

#### Scenario: OCR limits are configured on the engine
- **WHEN** a caller builds an OCR engine with default configuration
- **THEN** `page_timeout` is 60 seconds and `max_render_pixels` is 40,000,000, and both are overridable on the engine, not on `Limits` [§10; 0002 O8]

#### Scenario: No cumulative per-document decode budget
- **WHEN** a document's streams together decode to more than 2 GiB while each stream stays under `max_stream_decoded_bytes`
- **THEN** the document opens and extracts without a limit error, because no per-document decoded-bytes budget exists [§10; 0001 §1.4; ADR-0021]

### Requirement: Limits are supplied per document at open
The system SHALL apply the `Limits` given when a document is opened to every operation on that document, and SHALL use the default `Limits` when none is given. Rust callers SHALL pass them as `OpenOptions.limits`; Python callers SHALL pass them as the `limits` keyword of `open`, `open_bytes` and `extract`.

#### Scenario: A lowered limit takes effect on the opened document
- **WHEN** a document is opened from Python with `limits=Limits(max_ops_per_page=1000)` and a page holding 400,000 operators is extracted
- **THEN** `Page.text()` raises `LimitExceededError` [R2; 0001 §1.4; 0001 §3.5]

#### Scenario: Omitted limits mean the defaults
- **WHEN** the same page is extracted from a document opened without `limits`
- **THEN** the page extracts normally [R2; 0001 §1.4]

### Requirement: The Python Limits object is immutable and keyword-only
The system SHALL expose `pdf_extract.Limits` as an immutable class whose constructor takes only keyword arguments, defaulting to the core defaults, and whose fields are read-only properties. `page_time_budget` SHALL be given and reported in seconds as a float.

#### Scenario: Round trip and read-only fields
- **WHEN** `Limits(max_objects=3, page_time_budget=0.25)` is constructed and `max_objects` is then assigned
- **THEN** the properties return 3 and 0.25, and the assignment raises `AttributeError` [0001 §3.5]

#### Scenario: Positional arguments are refused
- **WHEN** `Limits(1)` is called
- **THEN** `TypeError` is raised [0001 §3.5]

### Requirement: The Python Limits constructor validates its arguments
The system SHALL reject an out-of-range `Limits` argument with `ValueError` naming the argument, and an argument of the wrong type with `TypeError`.

#### Scenario: Negative or oversized integers
- **WHEN** `Limits(max_file_size=-1)` or `Limits(max_image_pixels=2**64)` is called
- **THEN** `ValueError` is raised whose message names the argument and says it must not be negative or is too large [0001 §3.5]

#### Scenario: Non-finite or negative time budget
- **WHEN** `page_time_budget` is negative, NaN or infinite
- **THEN** `ValueError` naming `page_time_budget` is raised [0001 §3.5]

#### Scenario: Wrong types
- **WHEN** `max_objects` is given as `"many"` or `1.5`
- **THEN** `TypeError` is raised [0001 §3.5]

### Requirement: Limit errors name the limit and its value
The system SHALL identify an exceeded limit in errors by a stable snake-case name (`file_size`, `pages`, `page_tree_depth`, `objects`, `stream_decoded_bytes`, `ops_per_page`, `page_time_budget`, `chars_per_page`, `image_pixels`, `render_pixels`, `ocr_page_timeout`), not by the `Limits` keyword spelling, together with the configured value of that limit.

#### Scenario: Operator budget error attributes
- **WHEN** a page exceeds `max_ops_per_page=200_000`
- **THEN** `LimitExceededError.limit_name` is `"ops_per_page"` and `.value` is 200000 [0001 §3.5; 0001 §1.4]

#### Scenario: Time budget value is in milliseconds
- **WHEN** a page exceeds `page_time_budget=0.5`
- **THEN** `limit_name` is `"page_time_budget"` and `value` is 500 [0001 §3.5]

### Requirement: Per-page operator budget
The system SHALL count every content-stream operator interpreted for one page, including operators of nested Form XObjects and Type 3 glyph procedures, against `max_ops_per_page`, and SHALL end the page with a page-level `LimitExceededError` (`limit_name` `ops_per_page`) when the count exceeds it. The budget SHALL start afresh for every page operation.

#### Scenario: An operator flood exceeds the budget
- **WHEN** a page with 20,000 operators is extracted under `max_ops_per_page = 1000`
- **THEN** extraction fails with `PageError::Limit { kind: OpsPerPage, value: 1000 }` in Rust and `LimitExceededError` in Python [R4; §10; 0001 §1.4]

#### Scenario: The budget is inclusive
- **WHEN** `max_ops_per_page` equals the page's operator count
- **THEN** every operator runs and no error is raised [0001 §1.4; ADR-0027]

#### Scenario: Nested content shares the page budget
- **WHEN** a Form XObject fan-out or Type 3 glyph procedures together exceed the budget although the page's own content is short
- **THEN** the page fails with the operator-budget error [0001 §1.4; 0001 §2.3; ADR-0027]

#### Scenario: Exhaustion is page-level, not document-level
- **WHEN** one page exhausts its budget
- **THEN** the document stays open and its other pages extract normally [R4; 0001 §2.2]

### Requirement: Per-page wall-clock budget
The system SHALL bound the interpretation of one page by `page_time_budget`, checked every 4,096 operators, and SHALL end the page with a page-level `LimitExceededError` (`limit_name` `page_time_budget`) when the deadline has passed. The time budget, not the operator cap, SHALL be the wall-clock guarantee.

#### Scenario: A deadline stops a flood
- **WHEN** a page of 50,000,000 operators is extracted under `page_time_budget = 50 ms`
- **THEN** extraction fails with `PageError::Limit { kind: PageTimeBudget, value: 50 }` [R4; §10; 0001 §1.4]

#### Scenario: Bounded lateness
- **WHEN** the deadline passes during interpretation
- **THEN** extraction stops within `page_time_budget` plus the time of 4,096 operators plus one non-interruptible operator (one stream decode, font parse or CMap parse) [0001 §2.3; 0001 §2.4; ADR-0027]

#### Scenario: Form fan-out is cut by the deadline
- **WHEN** a page whose Form XObject fan-out would run for minutes is extracted from Python under `page_time_budget=0.5`
- **THEN** `LimitExceededError` is raised within 2.5 seconds [0001 §2.3; 0001 §4.10]

#### Scenario: An unrepresentable budget means no deadline
- **WHEN** a Rust caller sets `page_time_budget` to `Duration::MAX`
- **THEN** no deadline applies and the page extracts without panicking [0001 §1.4]

#### Scenario: The operator cap bounds work, not time
- **WHEN** a page is interpreted under the default limits
- **THEN** the operator cap is not relied upon to bound wall-clock time; only `page_time_budget` is [0001 §1.4; 0001 §2.3]

### Requirement: Interpretation budgets apply to OCR rendering
The system SHALL apply the document's `max_ops_per_page`, `page_time_budget` and `max_stream_decoded_bytes` unchanged when a page is rendered for OCR. Exhausting either interpretation budget while rendering SHALL fail the OCR call with `OcrPageError` carrying `limit_name` and `value`, and the page SHALL NOT be passed to recognition.

#### Scenario: Render over the operator budget
- **WHEN** `Page.ocr` is called on a page of a document opened with `Limits(max_ops_per_page=20)` whose content has more operators
- **THEN** `OcrPageError` (not `OcrTimeoutError`) is raised with `limit_name == "ops_per_page"` and `value == 20` [R4; 0002 O3; 0002 O15]
- **AND** the OCR backend is never called

#### Scenario: Render over the time budget
- **WHEN** rendering exceeds `page_time_budget`
- **THEN** the OCR call fails with a page-level limit error naming `page_time_budget` [0002 O3]

### Requirement: Per-stream decoded size cap
The system SHALL fail the decoding of any stream whose decoded output would exceed `max_stream_decoded_bytes`, SHALL treat that stream as absent, and SHALL report a `stream: decoded size limit` warning instead of raising an error.

#### Scenario: A flate bomb in page content
- **WHEN** a page's content stream would decode past the cap
- **THEN** the page's text is empty and the `TextPage` carries a `stream: decoded size limit` warning; no exception is raised [§10; 0001 §1.4; 0001 §2.2; ADR-0020]

#### Scenario: Boundary
- **WHEN** a stream decodes to exactly `max_stream_decoded_bytes`
- **THEN** it decodes unchanged, and one byte more fails [0001 §1.4]

#### Scenario: Filter chains and structural streams
- **WHEN** a stream has several filters, or is a cross-reference stream or an object stream
- **THEN** the cap applies to each filter's output and to those streams as well [0001 §1.4; 0001 §3.2]

### Requirement: Image pixel cap
The system SHALL NOT decode an image whose pixel count exceeds `max_image_pixels`, judged by its declared width times height before any filter runs and, for CCITT, JBIG2 and JPEG 2000, also by the dimensions the codec reports from its header before allocating. A refused image SHALL be skipped with an `image: pixel limit` warning on the page.

#### Scenario: An oversized image is skipped during rendering
- **WHEN** a page drawing an image that declares 100,000 × 100,000 pixels is OCR'd under `max_image_pixels = 1,000`
- **THEN** the image is not decoded, the page is still rendered and recognised, and its `TextPage` carries an `image: pixel limit` warning [§10; 0002 O4; ADR-0062]

#### Scenario: Codec-reported dimensions are checked
- **WHEN** a JBIG2, JPEG 2000 or CCITT stream's header reports more pixels than the cap although the dictionary declares fewer
- **THEN** the image is refused with the same warning before the codec allocates its output [0002 O4; ADR-0062]

#### Scenario: The cap is inclusive and judged by area
- **WHEN** an image has exactly `max_image_pixels` pixels, or is long and thin with the same area
- **THEN** it decodes; one pixel more is refused [0002 O4]

#### Scenario: The text path never decodes images
- **WHEN** text is extracted from a page whose images exceed the pixel cap or the codec memory budgets
- **THEN** no image is decoded and no image warning is raised [0001 §2.4; 0002 O4]

### Requirement: Codec memory budgets derive from the image pixel cap
The system SHALL decode each JBIG2 image under a memory budget of 2 bytes per pixel of `max_image_pixels`, and each JPEG 2000 image under 64 bytes per pixel of it. No separate `Limits` field SHALL exist for these budgets.

#### Scenario: Budgets at the default cap
- **WHEN** the default `Limits` are used
- **THEN** the JBIG2 budget is 512,000,000 bytes and the JPEG 2000 budget is 16,384,000,000 bytes [0003 F5; ADR-0079]

#### Scenario: Raising the cap raises the budgets
- **WHEN** a caller raises `max_image_pixels`
- **THEN** both codec budgets scale with it, at 2 and 64 bytes per pixel [0003 F5; 0003 F9]

### Requirement: Codec budget refusals skip the image with a memory-limit warning
The system SHALL refuse a JBIG2 or JPEG 2000 decode before an allocation that would take its counted memory over the budget. The refused image SHALL draw nothing, the rest of the page SHALL be rendered and recognised, and the image SHALL be reported once as an `image: memory limit` warning, never as a pixel-limit or decode-failure warning.

#### Scenario: A JBIG2 symbol-export bomb
- **WHEN** a rendered page draws a JBIG2 image whose symbol dictionary exports one symbol 65,535 times
- **THEN** the decode is refused before the copies are allocated and the page carries exactly one `image: memory limit` warning [0003 F6; 0003 F8]

#### Scenario: A JPEG 2000 tile bomb
- **WHEN** a rendered page draws a JPEG 2000 image tiled 1 × 1 over a large grid
- **THEN** the decode is refused from the header, before any tile record is built, with one `image: memory limit` warning [0003 F7; 0003 F8]

#### Scenario: The pixel cap is checked first
- **WHEN** an image exceeds both `max_image_pixels` and its codec budget
- **THEN** it is reported as `image: pixel limit` only [0003 F8]

#### Scenario: Malformed streams stay decode failures
- **WHEN** a JBIG2 or JPEG 2000 stream is malformed but within budget, or its JPEG 2000 tile count overflows 32 bits
- **THEN** it is reported as `image: decode failed`, not as a memory limit [0003 F8; 0003 §2.2]

### Requirement: Codec budgets are cumulative per decode
The system SHALL charge each counted allocation of one JBIG2 or JPEG 2000 decode against a total that starts at zero for every decode and is never refunded when a buffer is freed.

#### Scenario: Each decode starts afresh
- **WHEN** the same image is decoded twice under one budget
- **THEN** each decode is judged on its own total and both succeed if each fits [0003 F9]

#### Scenario: A legitimate file over the budget
- **WHEN** a JBIG2 symbol dictionary builds thousands of symbols by refinement or aggregation, so its transient allocations exceed the default budget
- **THEN** the image is refused with `image: memory limit`, and raising `max_image_pixels` admits it [0003 F9; 0003 §2.2]

### Requirement: Images within the codec budgets decode unchanged
The system SHALL decode every JBIG2 and JPEG 2000 image that fits its budget byte-identically to an unbudgeted decode, and SHALL leave CCITT and DCT decoding unchanged.

#### Scenario: Within budget
- **WHEN** a JBIG2 or JPEG 2000 image within its budget is rendered
- **THEN** it draws, with output identical to the unbudgeted decode [0003 F10]

#### Scenario: Legitimate JPEG 2000 geometries fit
- **WHEN** a legitimate JPEG 2000 image (gray, RGB, tiled, four components, eight resolutions) is decoded at the cap
- **THEN** its counted memory stays under 64 bytes per pixel [0003 F5]

### Requirement: Fixed object nesting cap
The system SHALL cap the nesting depth of PDF objects, content-stream operands, Type 4 PostScript functions and CMap PostScript at 256 levels, a fixed bound that `Limits` does not expose. An over-deep object SHALL be treated as missing and reported with an `object: nesting limit` warning; parsing SHALL NOT fail.

#### Scenario: Over-deep operand in page content
- **WHEN** a content stream contains an operand nested deeper than 256 levels
- **THEN** the text before it is kept, the rest of the stream is not interpreted, and the page carries an `object: nesting limit` warning [R1; §8.6; 0001 §1.4; ADR-0018]

#### Scenario: Over-deep resources
- **WHEN** a page's resource dictionary is nested past the cap
- **THEN** the resources are treated as missing and the text is still extracted with the fallback font [0001 §1.4; ADR-0018]

#### Scenario: Boundary
- **WHEN** an object is nested exactly 256 levels
- **THEN** it reads normally, on a 512 KiB thread stack [0001 §1.4; 0001 §2.3; ADR-0019]

### Requirement: Nested interpretation cap
The system SHALL interpret nested content (Form XObjects, Type 3 glyph procedures, tiling patterns and soft masks, in any mix) to at most 50 levels, and SHALL skip deeper content with an `interpreter: nesting limit` warning rather than failing the page.

#### Scenario: Nested forms
- **WHEN** a page nests Form XObjects 60 levels deep, each showing text
- **THEN** the text of the first 50 levels is extracted and the page carries an `interpreter: nesting limit` warning [§8.6; 0001 §1.4; KP21]

#### Scenario: Self-recursive kinds during rendering
- **WHEN** a page with a self-referential Type 3 glyph, a self-filling tiling pattern or a self-referencing soft mask is rendered for OCR on a 512 KiB stack
- **THEN** recursion stops at 50 levels without exhausting the stack, and the render completes with an `interpreter: nesting limit` warning [R1; 0002 O3; 0002 §2.3; 0002 §4.7; ADR-0029]

### Requirement: Form XObject cycles are drawn once
The system SHALL skip a Form XObject that is drawn again while it is already being drawn, and SHALL report it with a `form:` warning. Drawing the same form repeatedly in sequence SHALL NOT count as a cycle.

#### Scenario: A self-drawing form
- **WHEN** a Form XObject draws itself, directly or through another form
- **THEN** each form is drawn once and the page carries a form-cycle warning [0001 §1.4]

#### Scenario: Repetition is not a cycle
- **WHEN** a page draws one form several times, or again after it finished
- **THEN** every draw is interpreted and no cycle warning is raised [0001 §1.4]

### Requirement: Other recursion is bounded by construction
The system SHALL bound every other input-driven recursion: CMap `usecmap` chains SHALL be followed at most 16 levels, and the page-tree walk SHALL be iterative, its depth bounded by `max_page_tree_depth` as page-index states.

#### Scenario: A deep usecmap chain
- **WHEN** a font's CMap chains more than 16 `usecmap` references
- **THEN** the chain stops at 16 levels without crashing or recursing further [§8.6; 0001 §2.3]

### Requirement: OCR page deadline covers the whole call
The system SHALL bound each `Page.ocr` call by the engine's `page_timeout`, measured from the start of the call, so rendering, the noise pre-check and recognition share one deadline. Exceeding it SHALL raise `OcrTimeoutError` (Rust `OcrPageError::Timeout`) whose `timeout` is the configured `page_timeout` in seconds and whose `limit_name` and `value` are `None`.

#### Scenario: Recognition runs past the deadline
- **WHEN** recognition is still running when `page_timeout` passes
- **THEN** the call raises `OcrTimeoutError` with `timeout == page_timeout` [R4; §10; 0002 O16; 0002 O38]

#### Scenario: The deadline counts from before rendering
- **WHEN** the backend receives a recognition request
- **THEN** its deadline is `page_timeout` after the start of the `ocr()` call, not after the render or the pipe write [0002 O16; 0002 O27]

#### Scenario: Process isolation adds a transport grace
- **WHEN** a process-isolated page is still running when `page_timeout` passes
- **THEN** the parent allows a further 2 s transport grace before killing the helper, so the call can take up to `page_timeout` + 2 s, as ocr-process-isolation specifies [0002 O27; 0002 §3.5]

#### Scenario: Pre-check time is charged
- **WHEN** the noise pre-check runs
- **THEN** its time counts against `page_timeout` like rendering does [0003 F12]

#### Scenario: Invalid or huge timeouts
- **WHEN** `page_timeout` is zero, negative or non-finite in Python, or zero in Rust
- **THEN** engine construction is refused (`ValueError` in Python); a timeout too large to represent saturates instead of panicking [0002 O8]

### Requirement: A render that consumes the deadline is not recognised
The system SHALL NOT interrupt rendering on `page_timeout` (rendering is bounded by `page_time_budget`), and SHALL raise `OcrTimeoutError` without calling recognition when the deadline has passed by the time rendering or the pre-check ends.

#### Scenario: Render eats the budget
- **WHEN** `page_timeout` is 1 ns and a page is OCR'd
- **THEN** the call fails with a timeout and the backend is called zero times [0002 O16; 0002 §2.3]

### Requirement: Timeouts are classified by the clock
The system SHALL classify an OCR failure as a timeout only when the engine's own clock is past the deadline; a recognition failure before the deadline SHALL be `OcrPageError`, not `OcrTimeoutError`. Any backend-reported timeout SHALL be reported with the configured `page_timeout`.

#### Scenario: Early failure is not a timeout
- **WHEN** Tesseract's recognition reports failure before the deadline
- **THEN** the call raises `OcrPageError` for a recognition failure [0002 O15; ADR-0066]

#### Scenario: Backend timeout carries the configured timeout
- **WHEN** a backend sleeps past `page_timeout = 0.2 s` and reports a timeout
- **THEN** `OcrTimeoutError.timeout` is 0.2 [0002 O16; 0002 O38]

### Requirement: Timeout precision depends on isolation
The system SHALL enforce `page_timeout` exactly under `Isolation::Process`, through the parent's own deadline; in process, the system SHALL raise the timeout no later than Tesseract's next monitor poll, which may be one recognition unit or one layout-analysis pass after the deadline.

#### Scenario: Process isolation
- **WHEN** a helper process hangs during recognition
- **THEN** the call raises `OcrTimeoutError` at the deadline [0002 O27; 0002 §2.5]

#### Scenario: In-process lateness
- **WHEN** an in-process recognition is in layout analysis when the deadline passes
- **THEN** the timeout is raised when Tesseract next polls its monitor, possibly a whole layout pass late [0002 §2.4; 0002 §2.5]
