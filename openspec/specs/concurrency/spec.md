# concurrency Specification

## Purpose

How pdf-extract behaves when used from many threads and processes: thread-safe public types, parallel use of separate and shared documents, GIL release and free-threaded CPython, safe concurrent close, deterministic output, import-time inertness, and behaviour across `fork()`.

## Requirements

### Requirement: Public types are Send and Sync
The system SHALL make every public Rust type a caller can share between threads (`Document`, `Page`, the page iterator, `TextPage`, `Extraction`, `ExtractOptions`, `OcrEngine`, `OcrConfig`, `GrayImage`) both `Send` and `Sync`, enforced at compile time, and SHALL expose no public type that borrows from a `Document` through a lifetime.

#### Scenario: Compile-time enforcement
- **WHEN** the crates are built
- **THEN** a static assertion fails the build if any of these types stops being `Send + Sync` [D6; §8.1; 0001 T38; 0002 O20]

#### Scenario: Handles move between threads
- **WHEN** a `Page` obtained on one thread is moved to another thread and extracted there
- **THEN** the extraction succeeds and returns the same text as on the original thread [D6; §8.1; 0001 T38]

#### Scenario: Python classes are shareable
- **WHEN** a Python `Document`, `Page`, `TextPage` or `OcrEngine` object is used from several Python threads
- **THEN** each call behaves as it would from a single thread, because the Python classes wrap the thread-safe Rust types [§8.1; 0001 T38]

### Requirement: Separate documents run fully in parallel
The system SHALL let different `Document`s be opened and extracted on different threads in parallel, with no process-wide lock on the open or text-extraction path.

#### Scenario: Parallel opens
- **WHEN** 8 threads each open a separate document at the same time
- **THEN** every open succeeds and each document's page count and text equal those of a sequential open [C1; 0001 T38]

#### Scenario: Stress over separate documents
- **WHEN** 16 threads each perform 200 text extractions over 16 separate documents
- **THEN** every result is byte-identical to the single-threaded output [C1; §13.3; 0001 T38]

### Requirement: One document is shareable between threads
The system SHALL allow one `Document` to be used from several threads at once, and SHALL give each thread results identical to sequential use, whether the threads extract the same page or different pages.

#### Scenario: Shared-document stress
- **WHEN** 16 threads each call `text()` 200 times on pages of one 40-page document
- **THEN** every result is byte-identical to the sequential output for that page [C2; §13.3; 0001 T39]

#### Scenario: Same page from many threads
- **WHEN** N threads extract the same page of one document concurrently
- **THEN** all N results are identical to each other and to a sequential extraction [C2; §13.3; 0001 T39]

#### Scenario: Shared document under lazy xref repair
- **WHEN** 16 threads extract all pages of one document whose cross-reference entry for one page's content stream is wrong, so the table is repaired during extraction, over 50 rounds
- **THEN** no call panics, every thread's output equals the sequential output
- **AND** the document carries exactly one `xref: repaired` warning and no page's `TextPage` warnings mention it [C2; 0001 T38; ADR-0030]

### Requirement: Per-page warnings are attributed to the operation that caused them
The system SHALL attribute each `TextPage` warning only to the extraction that produced it, so that concurrent operations on one document never add warnings to each other's results.

#### Scenario: Two threads, one page over a decode limit
- **WHEN** two threads extract different pages of one document concurrently and only one page exceeds the stream decode limit
- **THEN** only that page's `TextPage` carries the `StreamDecodeLimit` warning [C2; 0001 §2.4; ADR-0019]

### Requirement: Shared page iterators hand out each page once
The system SHALL keep the position of a page iterator (`iter(document)` in Python, the pages iterator in Rust) safe to share between threads, so that threads consuming one iterator together receive every page index exactly once overall.

#### Scenario: Two threads share one iterator
- **WHEN** two or four threads consume a single `iter(doc)` until it is exhausted
- **THEN** the union of the pages they received covers every index exactly once, with no duplicates and no gaps [§8.4; 0001 §4.12]

#### Scenario: Iteration while extracting
- **WHEN** threads share one iterator and extract each page they receive
- **THEN** every page is extracted exactly once and each text equals the sequential text for that page [§8.4; 0001 §4.12]

### Requirement: close() is safe while other threads use the document
The system SHALL make `close()` safe to call while other threads are using the document: operations already running SHALL finish normally, operations started after `close()` returns SHALL fail with `PageError::Closed` (Python `DocumentClosedError`), and `close()` SHALL neither wait for nor block running operations.

#### Scenario: Close during use (Python)
- **WHEN** one thread closes a document at a random moment while 8 worker threads extract from it, repeated for 500 rounds
- **THEN** every call either returns the correct text or raises `DocumentClosedError`, the process never crashes
- **AND** no call that started after `close()` returned produces text [C4; §13.3; 0001 T40]

#### Scenario: Close during use (Rust)
- **WHEN** a document is closed while other threads extract from it, over 1,000 iterations
- **THEN** every call returns `Ok` or `Err(PageError::Closed)` and none panics [C4; 0001 T40]

#### Scenario: Held page handles after close
- **WHEN** a `Page` handle obtained before `close()` is extracted after it
- **THEN** the call fails with `DocumentClosedError`, and indexing `doc[i]` on the closed document also raises `DocumentClosedError` [C4; §8.2; 0001 T40]

### Requirement: Memory is freed when the last running operation finishes
The system SHALL release the parsed document when `close()` has been called and the last operation that was running at the time has finished, without requiring any further call.

#### Scenario: Close with an extraction in flight
- **WHEN** `close()` is called while a long extraction is running
- **THEN** the extraction completes with its normal result and the document's memory is released when that extraction ends [C4; §8.2; 0001 T40; ADR-0040]

### Requirement: Python releases the GIL for non-constant work
The system SHALL release the GIL around every native Python operation whose work is not constant: `open`, `open_bytes`, `Page.text`, `Page.text_page`, `TextPage.text`, `Page.ocr`, `OcrEngine(...)` construction, `default_ocr_engine()`, `extract`, and the test-kit builder's page creation, text insertion, `to_bytes` and `save`.

#### Scenario: Long extraction does not stall other threads
- **WHEN** one thread runs `page.text()` or `page.text_page()` on an operator-heavy page bounded by `page_time_budget` while a second thread runs a pure-Python counter
- **THEN** the counter thread is never blocked for more than 50 ms [C3; §8.4; 0001 T43]

#### Scenario: Open, OCR and engine construction
- **WHEN** another Python thread runs during `open`/`open_bytes`, `Page.ocr`, or `OcrEngine(...)` construction
- **THEN** that thread keeps running for the duration of the native call [C3; §8.4; 0002 O21]

### Requirement: Free-threaded CPython is supported without re-enabling the GIL
The system SHALL declare free-threading support so that importing `pdf_extract` on free-threaded CPython 3.14t or later leaves the GIL disabled, SHALL pass its full test suite, including the concurrency stress tests, on 3.14t with `PYTHON_GIL=0`, and SHALL support the regular GIL builds of CPython 3.11 and later.

#### Scenario: Import keeps the GIL off
- **WHEN** `import pdf_extract` runs on CPython 3.14t without `PYTHON_GIL` set
- **THEN** `sys._is_gil_enabled()` returns `False` afterwards and no GIL warning is emitted [C3; D6; 0001 T43; ADR-0011]

#### Scenario: CI on free-threaded CPython
- **WHEN** CI runs the Python suite on 3.14t with `PYTHON_GIL=0`
- **THEN** the suite, including the stress, shared-document and close-during-use tests, passes [C3; §8.4; §13.3; 0001 T43]

#### Scenario: 3.13t is not a supported target
- **WHEN** a user targets free-threaded CPython 3.13t
- **THEN** it is not a supported platform: free-threaded support begins at 3.14t, a deliberate narrowing of "3.13t and later" [C3; 0001 T43; ADR-0011]

### Requirement: Output is deterministic
The system SHALL produce output that depends only on the input bytes, the options, the library version and (for OCR) the language-data file, and never on thread count, worker count or scheduling. The per-host rendering rule is in ocr-rendering, and `extract()` page selection (with its one documented exception) is in ocr-batch-extract.

#### Scenario: Repeated and threaded extraction
- **WHEN** the same page is extracted 50 times on each of 8 threads
- **THEN** every result is byte-identical [C6; 0001 T41]

### Requirement: Importing the module does no work
The system SHALL do no work and start no threads when `pdf_extract` is imported: the OCR engine, Tesseract initialisation, helper discovery and model checksums SHALL all be deferred to first use.

#### Scenario: Import cost and threads
- **WHEN** `python -X importtime -c "import pdf_extract"` runs
- **THEN** the cumulative import time of `pdf_extract` is under 50 ms and `threading.active_count()` is 1 afterwards [C7; 0001 T42; 0002 O41]

### Requirement: The library creates threads only inside a multi-worker extract
The system SHALL create threads only inside `extract()` with `workers > 1`, as scoped threads that are all joined before the call returns; it SHALL keep no global thread pool, and process-isolated OCR SHALL perform its pipe I/O on the calling thread.

#### Scenario: Single worker
- **WHEN** `extract()` is called with `workers = 1` (or 0, treated as 1)
- **THEN** no thread is created [§8.5; 0002 O24]

#### Scenario: Multiple workers
- **WHEN** `extract()` is called with `workers = n > 1`
- **THEN** at most n worker threads run during the call and the process's thread count has returned to its prior value when the call returns [§8.5; 0002 O24]

#### Scenario: Worker panic
- **WHEN** a worker thread panics inside `extract()`
- **THEN** the panic propagates after all workers have joined (Rust) and surfaces as `InternalError` in Python [§8.5; 0002 O24]

#### Scenario: Process isolation creates no threads
- **WHEN** OCR runs with `isolation="process"`
- **THEN** no library thread is created; the deadline is enforced by polling the pipe on the calling thread [§8.5; 0002 O32]

### Requirement: Lazily created OCR state survives fork() when no OCR is in progress
The system SHALL let a process that forks while no thread is inside the OCR code keep using, in the child, any `OcrEngine` (including the default engine) built in the parent: the child's first OCR call SHALL rebuild the engine's pool and return correct results, without freeing the parent's instances.

#### Scenario: In-process engine across fork
- **WHEN** the parent recognises a page, forks, and the child recognises a page with the same engine
- **THEN** the child gets the correct text, paying one engine initialisation, and the parent's next recognition is also correct [C7; §13.3; 0002 O11]

#### Scenario: Default engine across fork
- **WHEN** `default_ocr_engine()` was built in the parent and the child calls `Page.ocr()` with no engine
- **THEN** the child reuses the same default engine with a rebuilt pool rather than constructing a new one, and gets correct text [C7; 0002 O12; 0002 O11]

#### Scenario: Process-isolated engine across fork
- **WHEN** a process-isolated engine is used in the parent, the process forks, and the child uses the engine
- **THEN** the child closes its inherited helper pipes without waiting on the parent's helpers, spawns its own helper, and gets correct text [C7; 0002 O30]

### Requirement: The fork-deadlock hazard is documented, not defended
The system SHALL install no `atfork` handlers, and SHALL state the fork hazard (a fork while another thread is constructing an engine, initialising Tesseract, or holding an OCR lock can deadlock the child's next OCR call) with the safe patterns in the Rust crate docs, the Python docstrings of `OcrEngine`, `default_ocr_engine` and `extract`, and the README.

#### Scenario: Python docstrings
- **WHEN** a user reads `OcrEngine.__doc__`, `default_ocr_engine.__doc__` or `extract.__doc__`
- **THEN** each names the fork hazard and recommends the `spawn` or `forkserver` start method or building engines before forking
- **AND** the `OcrEngine` docstring quotes the CPython 3.14 change of the default POSIX start method to `forkserver` [C7; 0003 F2; ADR-0078]

#### Scenario: Rust crate docs
- **WHEN** a user reads the OCR crate documentation
- **THEN** a `Forking` section gives the hazard and three rules: build every engine before forking, do not fork while another thread may be inside OCR or an OCR-enabled `extract`, and prefer `posix_spawn`/`exec` over a bare `fork` in a threaded process [C7; 0003 F1; ADR-0078]

#### Scenario: README
- **WHEN** a user reads the README's OCR section
- **THEN** a "Forking and multiprocessing" paragraph gives the same three rules and the `spawn`/`forkserver` recommendation [C7; 0003 F3; ADR-0078]

#### Scenario: Fork during OCR is not defended
- **WHEN** a process forks while another thread is inside the first `default_ocr_engine()` construction or holds an engine's pool lock
- **THEN** the child's next OCR call may deadlock; this is a documented residual, not a supported use [0002 §2.4; 0003 §1.1; ADR-0078]

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
