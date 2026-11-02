# error-model Specification

## Purpose

How pdf-extract reports failure and trouble: the Rust error enums, the Python
exception hierarchy and the mapping between them, panics converted at the
Python boundary, the bounded-stack guarantee, silence on stdout and stderr,
and the structured warnings that report non-fatal problems.

## ADDED Requirements

### Requirement: Rust open failures are typed
The Rust API SHALL report a failure to open a document as an `OpenError` with exactly the variants `NotFound(path)`, `Io(io::Error)`, `Empty`, `TooLarge { size, limit }`, `NotPdf`, `Corrupt(detail)`, `PasswordRequired`, `UnsupportedEncryption(detail)`, `TooManyPages { limit, at_least }` and `Limit { kind, value }`, each with a message naming what went wrong.

#### Scenario: Each open failure has its own variant
- **WHEN** a caller opens a missing file, an empty input, an over-size input, a non-PDF or a damaged PDF through the Rust API
- **THEN** the result is `Err` with the matching `OpenError` variant, and its message names the path, size, header or detail involved [§9.1; 0001 T47b]

#### Scenario: An object-count overrun is a limit variant
- **WHEN** a document has more objects than `max_objects`
- **THEN** the Rust error is `OpenError::Limit` carrying the limit kind `objects` and the value found [§10; 0001 T47b]

### Requirement: Rust page failures are typed
The Rust API SHALL report a failed page operation as a `PageError` with exactly the variants `Closed`, `IndexOutOfRange { index, count }` and `Limit { kind, value }`, where `kind` is a limit kind with a stable snake_case name.

#### Scenario: A closed document refuses page work
- **WHEN** a page operation starts after the document was closed
- **THEN** it returns `PageError::Closed` [§9.1; 0001 T40]

#### Scenario: A per-page budget runs out
- **WHEN** a page exceeds `max_ops_per_page` or `page_time_budget`
- **THEN** it returns `PageError::Limit` with kind `ops_per_page` or `page_time_budget` and the configured limit as `value` [§9.1; §10; 0001 T47b]

### Requirement: Rust OCR failures are typed
The Rust API SHALL report OCR failures as an `OcrError` with the variants `Unavailable` (host-level, carrying `reason`, `tessdata`, `languages`, `searched` and `detail`), `Page` (one page failed: `Render`, `Recognition`, `Timeout(duration)` or `Limit { kind, value }`) and `Closed`.

#### Scenario: Unavailable reasons are a closed set
- **WHEN** OCR cannot run on the host
- **THEN** `OcrUnavailable.reason` is one of `no_tessdata`, `language_data_missing`, `init_failed` or `engine_not_built` [D5; §9.1; 0002 O38]

#### Scenario: A page timeout carries its duration
- **WHEN** recognition of one page exceeds the engine's `page_timeout`
- **THEN** the error is `OcrError::Page(Timeout(d))` with `d` the configured timeout [§9.1; 0002 O38]

### Requirement: The Rust API lets panics propagate
The Rust API SHALL NOT catch panics: a panic inside the library propagates to the Rust caller as a panic, and only the Python boundary converts panics into exceptions.

#### Scenario: Rust caller sees a panic
- **WHEN** a library bug panics during a call made directly from Rust
- **THEN** the panic unwinds into the caller rather than being returned as an error value [ADR-0035; 0001 T45]

### Requirement: The Python hierarchy is defined in full in pure Python
The library SHALL define its exception classes in the pure-Python module `pdf_extract.errors`, list every class in its `__all__`, re-export every class from `pdf_extract`, and define the whole tree of §9.2, including OCR classes, with `PdfExtractError(Exception)` at the root.

#### Scenario: The tree matches the design
- **WHEN** a caller inspects the classes
- **THEN** `OpenError`, `DocumentClosedError`, `PageError`, `OcrError` and `InternalError` derive from `PdfExtractError`; `DocumentNotFoundError`, `EmptyFileError`, `NotAPdfError`, `CorruptDocumentError`, `PasswordRequiredError`, `UnsupportedEncryptionError`, `FileTooLargeError` and `TooManyPagesError` from `OpenError`; `LimitExceededError` from `PageError` [§9.2; 0001 T44; ADR-0036]
- **AND** `OcrUnavailableError` and `OcrPageError` derive from `OcrError`, and `OcrTimeoutError` from `OcrPageError` [§9.2; 0001 T44]

#### Scenario: Every class is catchable as Exception
- **WHEN** any exception of the hierarchy, `InternalError` included, is raised
- **THEN** `except Exception` and `except PdfExtractError` both catch it [§9.2; R1; 0001 T44]

### Requirement: Only OcrUnavailableError is a RuntimeError
`OcrUnavailableError` SHALL be the only class in the hierarchy that also derives from `RuntimeError`; `OcrPageError` and `OcrTimeoutError` SHALL NOT derive from `RuntimeError`.

#### Scenario: Host-level and page-level OCR failures are distinguishable by base class
- **WHEN** a caller catches `RuntimeError` around OCR
- **THEN** it catches a missing-Tesseract or missing-language failure but not a failure of one page [D5; §9.2; 0001 T44]

### Requirement: A missing file is also a FileNotFoundError
`DocumentNotFoundError` SHALL derive from both `OpenError` and `FileNotFoundError`, and SHALL carry `errno` ENOENT and `filename` equal to the path the caller passed.

#### Scenario: Opening a missing path
- **WHEN** `pdf_extract.open` is given a path that does not exist
- **THEN** it raises `DocumentNotFoundError`, which `except FileNotFoundError` catches, with `errno == 2` and `filename` equal to the path given [§9.2; 0001 T44; 0001 T47b]

### Requirement: Open failures map to their own Python classes
The Python binding SHALL map `OpenError` variants to `NotFound`→`DocumentNotFoundError`, `Io`→`OpenError`, `Empty`→`EmptyFileError`, `TooLarge`→`FileTooLargeError`, `NotPdf`→`NotAPdfError`, `Corrupt`→`CorruptDocumentError`, `PasswordRequired`→`PasswordRequiredError`, `UnsupportedEncryption`→`UnsupportedEncryptionError`, `TooManyPages`→`TooManyPagesError`.

#### Scenario: Each Rust variant raises its class
- **WHEN** an open fails with one of these Rust variants
- **THEN** Python raises the mapped class with the Rust message [0001 T47b]

#### Scenario: An object-count overrun is a corrupt document
- **WHEN** an open fails with the `objects` limit
- **THEN** Python raises `CorruptDocumentError` whose message names the limit [§10; 0001 T47b]

#### Scenario: Too many pages carries its counts
- **WHEN** an open with `max_pages = n` finds more than `n` pages
- **THEN** `TooManyPagesError` is raised with `.limit == n` and `.at_least == n + 1` [§9.2; 0001 T14; 0001 T47b]

### Requirement: Other I/O failures become OpenError with the OSError as cause
Any I/O failure while opening other than a missing file SHALL raise the base `OpenError`, whose message names the path and whose `__cause__` is the original `OSError` subclass carrying `errno` and `filename`.

#### Scenario: Permission denied
- **WHEN** a caller opens a file it lacks permission to read
- **THEN** `OpenError` is raised with `__cause__` a `PermissionError` [§9.2; 0001 T47]

#### Scenario: A path that is not a regular file
- **WHEN** a caller opens a directory or FIFO
- **THEN** `OpenError` is raised (not `DocumentNotFoundError`) with an `OSError` as `__cause__` [0001 T47; ADR-0026]

### Requirement: Page failures map to Python classes
The binding SHALL map `PageError::Closed` to `DocumentClosedError`, `IndexOutOfRange` to the builtin `IndexError`, and `Limit` to `LimitExceededError(message, limit_name, value)`, where `limit_name` is the limit kind's snake_case name and `value` is the configured limit, in milliseconds for `page_time_budget`.

#### Scenario: Operation budget exceeded
- **WHEN** `Page.text()` exceeds `max_ops_per_page`
- **THEN** `LimitExceededError` is raised with `.limit_name == "ops_per_page"` and `.value` the configured operator cap [§9.2; 0001 T47b; 0001 §3.5]

#### Scenario: Time budget exceeded
- **WHEN** `Page.text()` exceeds `page_time_budget`
- **THEN** `LimitExceededError` is raised with `.limit_name == "page_time_budget"` and `.value` the budget in milliseconds [0001 T47b; 0001 §3.5]

#### Scenario: Use after close
- **WHEN** a page operation starts after `Document.close()`
- **THEN** `DocumentClosedError` is raised [§9.2; 0001 T47b]

### Requirement: OCR failures map to Python classes with their data
The binding SHALL map `OcrError::Unavailable` to `OcrUnavailableError(message, reason, tessdata, languages, searched)`, `Page(Render|Recognition)` to `OcrPageError(message)`, `Page(Timeout(d))` to `OcrTimeoutError(message, timeout)` with `timeout` in seconds, `Page(Limit)` to `OcrPageError` with `limit_name` and `value`, and `Closed` to `DocumentClosedError`.

#### Scenario: Non-limit page failure has no limit data
- **WHEN** recognition of a page fails for a reason other than a document limit, or times out
- **THEN** the raised exception has `.limit_name is None` and `.value is None` [0002 O38; ADR-0073]

#### Scenario: Timeout carries seconds
- **WHEN** `Page.ocr` exceeds a `page_timeout` of 60 s
- **THEN** `OcrTimeoutError` is raised with `.timeout == 60.0` [0002 O38; ADR-0073]

#### Scenario: Unavailable carries diagnosis
- **WHEN** no language data can be found
- **THEN** `OcrUnavailableError` carries `.reason`, `.tessdata` (the directory used, or `None`), `.languages` (a tuple) and `.searched` (a tuple of directories tried) [§9.2; 0002 O38]

### Requirement: Attribute-carrying exceptions survive pickling
`TooManyPagesError`, `LimitExceededError`, `OcrUnavailableError`, `OcrPageError` and `OcrTimeoutError` SHALL accept their attributes as constructor arguments after the message and SHALL round-trip through `pickle` with message and attributes intact.

#### Scenario: Raised in a worker process
- **WHEN** one of these exceptions is pickled and unpickled, as when raised in a `multiprocessing` worker
- **THEN** the copy has the same class, message and attribute values [0002 O38; 0001 T44]

### Requirement: A panic becomes InternalError at the Python boundary
Every exported native Python function and method SHALL run its body inside a panic guard, so that a Rust panic raises `InternalError` whose message contains the panic text and its source location.

#### Scenario: Injected panic
- **WHEN** the test hook `_native._panic_for_test()` panics, attached or with the GIL released
- **THEN** the caller receives `InternalError` containing the panic message [R1; §9.2; 0001 T45]

#### Scenario: Every entry point is guarded
- **WHEN** the source scan of the binding runs
- **THEN** every `#[pyfunction]` and `#[pymethods]` body is a single guarded tail call, and no generated getter or setter bypasses the guard [0001 T45; 0002 O39]

#### Scenario: Release builds unwind
- **WHEN** the library is built in any profile
- **THEN** `panic = "unwind"` is set, so a panic reaches the guard instead of aborting the process [§9.2; 0001 T45]

### Requirement: PanicException never reaches callers
`pyo3.PanicException` SHALL never be raised to a caller; a panic SHALL be catchable by `except Exception` and SHALL NOT end the calling thread.

#### Scenario: Worker thread survives a panic
- **WHEN** a Python worker thread wrapping calls in `except Exception` triggers a panic
- **THEN** the handler catches `InternalError` and the thread keeps running [R1; §9.2; 0001 T45]

### Requirement: Panics on internal threads also become InternalError
A panic in a worker thread of `extract()` or inside the OCR instance pool SHALL reach the Python caller as `InternalError`.

#### Scenario: Panic in an extract worker
- **WHEN** `extract()` with several workers hits a panic while recognising a page
- **THEN** the call raises `InternalError` containing the panic message [0002 O39; 0001 T45]

#### Scenario: Panic during recognition
- **WHEN** `Page.ocr` hits a panic inside the engine
- **THEN** `InternalError` is raised [0002 O39]

### Requirement: The library runs on 512 KiB thread stacks
Every operation SHALL complete without stack overflow on a thread with a 512 KiB stack, for every input, in Rust and from Python; a hostile input SHALL end in a result or a `PdfExtractError` other than `InternalError`.

#### Scenario: Hostile fixtures from Python on a small stack
- **WHEN** every generated fixture and every corpus-C file is opened and extracted on a Python thread started after `threading.stack_size(512 * 1024)`
- **THEN** the process survives and no call raises `InternalError` [R1; 0001 §2.3]

#### Scenario: Hostile fixtures in Rust, text and rendering
- **WHEN** the hostile fixture set is extracted, and rendered at 36 dpi, on 512 KiB Rust threads
- **THEN** each completes with a result or a typed error, without stack overflow or panic [R1; 0001 §2.3; 0002 §2.3]

### Requirement: The library writes nothing to stdout or stderr
No library call SHALL write to file descriptor 1 or 2, including on error paths, on caught panics, and from vendored parser code.

#### Scenario: Text path over every fixture
- **WHEN** a child process with fd 1 and fd 2 redirected opens and extracts every generated and encrypted fixture, triggers every error row and the panic hook
- **THEN** both captured files are empty [F11; 0001 T48]

#### Scenario: Panic message is not printed
- **WHEN** a panic is caught at the boundary
- **THEN** its message appears only in the `InternalError`, never on stderr [F11; 0001 T45; 0001 T48]

### Requirement: OCR writes nothing to stdout or stderr
OCR SHALL write nothing to fd 1 or 2: not `OcrEngine(...)`, including a failing Tesseract initialisation, not `Page.ocr`, not `extract()`, not a helper process, and not the restart of a crashed helper.

#### Scenario: Failing initialisation is silent
- **WHEN** `OcrEngine` is built with a language whose data is missing
- **THEN** `OcrUnavailableError` is raised and nothing appears on fd 1 or 2 [F11; 0002 O40]

#### Scenario: Helper abort is silent
- **WHEN** a process-isolated helper aborts and is restarted
- **THEN** the page fails with `OcrPageError` and nothing appears on the parent's fd 1 or 2 [F11; 0002 O40]

### Requirement: Malformed content is a warning, not an error
Malformed or hostile content that the library can work around SHALL NOT raise; extraction SHALL continue and the problem SHALL be reported as a warning on the `Document` (open-time problems) or the page's `TextPage` (per-page problems).

#### Scenario: Damaged page content
- **WHEN** a page's content stream has syntax errors
- **THEN** `Page.text_page()` returns the partial text and its `warnings` include a `content:` entry [R3; §9.1; F11; 0001 T35]

#### Scenario: Repaired xref
- **WHEN** a document opens only after its cross-reference table is rebuilt
- **THEN** `Document.warnings` includes an `xref:` entry [F11; 0001 §3.3]

### Requirement: A Rust warning carries page, kind, detail and count
Each Rust `Warning` SHALL carry `page` (the 0-based page index, or `None` for document-level), `kind` (a `WarningKind`, the stable, testable part), `detail` (human-readable, not a stable format) and `count` (at least 1).

#### Scenario: Document-level warning
- **WHEN** an open-time problem is recorded
- **THEN** its warning has `page == None` [§9.1; 0001 §3.3]

#### Scenario: Category of a kind
- **WHEN** a caller asks a `WarningKind` for its `category()`
- **THEN** it gets a non-empty static word such as `xref`, `page tree`, `object`, `stream`, `content`, `font`, `text`, `interpreter`, `form`, `image`, `page` or `ocr` [0001 §3.3; 0003 F8; 0003 F14]

### Requirement: The warning-kind vocabulary
`WarningKind` SHALL comprise `XrefRepaired`, `PageTreeRevisited`, `PageTreeDepth`, `PageTreeBruteForce`, `PageTreeNodeUnreadable`, `ObjectDepth`, `StreamDecodeLimit`, `StreamDecodeFailed`, `ContentDamaged`, `FontLoadFailed`, `CharLimit`, `InterpreterNesting`, `FormXObjectCycle`, `UnsupportedFont`, `ImageDecodeFailure`, `ImagePixelLimit`, `ImageMemoryLimit`, `PageLimit`, `OcrNoiseDownsampled` and `OcrNoiseSkipped`.

#### Scenario: Codec memory refusal has its own kind
- **WHEN** a JBIG2 or JPEG 2000 image is refused by its codec memory budget during rendering
- **THEN** the warning kind is `ImageMemoryLimit` (category `image`), never `ImagePixelLimit` or `ImageDecodeFailure` [0003 F8; ADR-0079]

#### Scenario: Noise pre-check kinds
- **WHEN** OCR downsamples or skips a noise-dominated raster
- **THEN** the `TextPage` carries `OcrNoiseDownsampled` or `OcrNoiseSkipped` respectively (category `ocr`) [0003 F14]

### Requirement: Repeated warnings are coalesced per kind
Warnings SHALL be collected one per kind per scope, in order of first occurrence; a repeat SHALL increment the first occurrence's `count` (saturating at `u32::MAX`) and SHALL NOT add an entry or replace the first `detail`.

#### Scenario: A defect repeated a million times
- **WHEN** a hostile page tree repeats the same defect once per `/Kids` entry
- **THEN** the document carries one warning of that kind whose count is the number of occurrences [0001 §3.3; 0001 D-36; ADR-0026]

### Requirement: Warnings are exposed as strings in a fixed shape
A warning's text, and each entry of Python's `Document.warnings`, `TextPage.warnings` and `Extraction.warnings` (each a `tuple[str, ...]`), SHALL be `"<category>: <detail>"`, followed by `" (×N)"` when the warning occurred `N > 1` times.

#### Scenario: Coalesced warning text
- **WHEN** an `object` warning with detail `nesting limit` occurred twice
- **THEN** its text is `object: nesting limit (×2)` [F11; 0001 §3.3; 0001 §3.5]

#### Scenario: A page-limit note names its page
- **WHEN** `extract()` records that page 3's text pass ran out of a per-page budget
- **THEN** the entry in `Extraction.warnings` reads `page 3: <the limit message>` instead of the category form [0002 O23; 0003 F14]

#### Scenario: Per-page OCR notes in extract
- **WHEN** `extract()` skips page 3 as noise-dominated
- **THEN** `Extraction.warnings` contains an entry starting `page 3: ocr: ` while the page's own `TextPage` warning reads `ocr: <detail>` [0003 F14]
