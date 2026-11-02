# ocr-process-isolation Specification

## Purpose

OCR in helper child processes, so that a Tesseract crash (an `ASSERT_HOST`, `abort()` or `exit()`) fails one page instead of ending the caller's process. This spec covers how the helper is found, started, bounded in time and replaced, and how its failures are classified. Process isolation is the default from Python (ADR 0092); from Rust it is opt-in, and in-process OCR stays the Rust default.

## Requirements

### Requirement: In-process is the default; process isolation is opt-in
The system SHALL default to in-process OCR in Rust and to process isolation in Python. In Rust, process isolation SHALL be used only when the caller sets `OcrConfig.isolation = Isolation::Process`, and Rust's `default_ocr_engine()` SHALL run in process. In Python, `OcrEngine()`, `default_ocr_engine()` and `Page.ocr()` without an engine SHALL use process isolation unless the caller passes `isolation="in_process"`.

#### Scenario: Rust default configuration
- **WHEN** a caller builds `OcrConfig::default()`
- **THEN** its `isolation` is `Isolation::InProcess` [§7.6; 0002 O8, D-7; ADR-0061]

#### Scenario: Python default engine options
- **WHEN** a caller constructs `OcrEngine()` without an `isolation` argument
- **THEN** `engine.isolation` and `engine.info().isolation` are both `"process"` [internal #53]

#### Scenario: Python opt-out
- **WHEN** a caller constructs `OcrEngine(isolation="in_process")`
- **THEN** `engine.isolation` and `engine.info().isolation` are both `"in_process"`, and no helper is spawned [0002 O8, §3.6; internal #53]

#### Scenario: An abort in the default mode is not contained
- **WHEN** Tesseract calls `abort()` while an in-process engine (the Rust default) is recognising a page
- **THEN** the host process ends, the same exposure PyMuPDF has; only process isolation contains it [§7.6; 0002 §2.3 item 5; ADR-0061]

#### Scenario: An abort under Python's default is contained
- **WHEN** a Python caller uses `Page.ocr()` with no engine and the helper aborts on that page
- **THEN** `OcrPageError` is raised and the interpreter continues [internal #53; 0002 O27]

### Requirement: Isolation option values
The Python `isolation` argument SHALL accept exactly `"in_process"` and `"process"`. The Rust `Isolation` enum SHALL have exactly the variants `InProcess` and `Process`. The engine SHALL report the mode it uses through its `isolation` getter and through `info().isolation`.

#### Scenario: Unknown value from Python
- **WHEN** a caller constructs `OcrEngine(isolation="thread")`
- **THEN** `ValueError` is raised, naming `'in_process'` and `'process'` as the accepted values [0002 §3.6, O8]

#### Scenario: A process engine reports its mode
- **WHEN** an engine is built with process isolation
- **THEN** `engine.isolation == "process"` and `engine.info().isolation == "process"` in Python, and `info().isolation == Isolation::Process` in Rust [0002 O13, O26, §4.9]

### Requirement: Each pool slot is a helper process
Under process isolation, every slot of the engine's instance pool SHALL be a separate `pdf-extract-ocr-helper` child process that holds one Tesseract instance and serves one page at a time over its stdin and stdout pipes. The number of live helpers SHALL be bounded by `max_instances`, as in-process instances are.

#### Scenario: Concurrent pages share a bounded set of helpers
- **WHEN** more threads than `max_instances` call `ocr()` on one process-isolated engine at once
- **THEN** every page completes and no more than `max_instances` helpers are alive at any time [0002 O26, §3.5]

### Requirement: Results identical to in-process
A process-isolated engine SHALL expose the same public API, error classes and results as an in-process engine with the same configuration. Its `TextPage`s SHALL be indistinguishable from in-process ones; the only observable difference SHALL be `info().isolation`.

#### Scenario: Same page, both modes
- **WHEN** the same page is recognised by an in-process engine and by a process-isolated engine with the same configuration and model
- **THEN** the two texts are identical, and the two `info()` values differ only in `isolation` [0002 O26, §4.9]

#### Scenario: Per-call resolution override
- **WHEN** a Python caller passes `dpi=72` to `page.ocr(engine)` on a process-isolated engine
- **THEN** the result equals the in-process result for the same page and options [0002 O26, §4.9]

### Requirement: Construction validates inside one helper
Building a process-isolated engine SHALL locate the helper, resolve the model directory in the parent, spawn exactly one helper and run the full up-front engine validation inside that helper, so that configuration and model problems surface at construction (fail fast). The resolved model directory SHALL be passed to the helper, which SHALL NOT search for one itself.

#### Scenario: Exactly one helper at construction
- **WHEN** a caller builds a process-isolated engine
- **THEN** exactly one helper is spawned, and the first page reuses it rather than spawning another [0002 O29, §4.9]

#### Scenario: A model the helper cannot load
- **WHEN** a process-isolated engine is built over a model directory whose `traineddata` Tesseract cannot load (for example a 0-byte file, or a requested language that does not load)
- **THEN** construction fails with `OcrUnavailable` carrying `InitFailed`, the same reason the in-process engine gives, and the helper is reaped [0002 O29, O7; 0003 F20]

#### Scenario: A missing model directory
- **WHEN** no model directory holds the requested language
- **THEN** construction fails with `OcrUnavailable` carrying the same reason and `searched` list that the in-process engine reports, without spawning a helper [0002 O29, O9]

### Requirement: The parent loads no Tesseract
Under process isolation, the parent process SHALL NOT load or initialise Tesseract or Leptonica. All recognition, including the noise pre-check that runs before recognition, SHALL leave the parent free of native OCR code.

#### Scenario: Construction and a page in the parent
- **WHEN** a process that has built no in-process engine builds a process-isolated engine and recognises a page with it
- **THEN** the process-wide Tesseract initialisation has run zero times in the parent [0002 O29, §4.9]

#### Scenario: The noise pre-check runs in the parent
- **WHEN** a process-isolated engine renders a page at 144 dpi or more with the noise pre-check enabled
- **THEN** the pre-check runs in the parent before the raster is sent, and the parent still loads no Tesseract [0003 F12; ADR-0081]

### Requirement: Helper discovery order
The system SHALL locate the helper at engine construction in this order, and the first source that is configured SHALL decide: `$PDF_EXTRACT_OCR_HELPER`; then `pdf-extract-ocr-helper` in `OcrConfig.helper_dir`; then, when `helper_dir` is unset, `pdf-extract-ocr-helper` beside the current executable. The system SHALL NOT search `PATH`. The file SHALL be an executable regular file.

#### Scenario: PATH is never consulted
- **WHEN** a process-isolated engine is built from Python with `PATH` set to the empty string
- **THEN** the helper is found beside the extension module and pages are recognised normally [0002 O28, §4.9; ADR-0071]

#### Scenario: A configured location that lacks the helper
- **WHEN** `PDF_EXTRACT_OCR_HELPER` names a file that does not exist
- **THEN** construction fails with `OcrUnavailable` carrying `EngineNotBuilt` and a detail naming that path; no later source is tried [0002 O28; ADR-0071]

### Requirement: Relative helper locations are refused
The system SHALL refuse a relative `PDF_EXTRACT_OCR_HELPER` value or a relative `helper_dir`, with `OcrUnavailable` carrying `EngineNotBuilt`, rather than resolve it against the working directory.

#### Scenario: Relative environment value
- **WHEN** `PDF_EXTRACT_OCR_HELPER` is `relative/helper` and a process-isolated engine is built
- **THEN** `OcrUnavailableError` is raised with `reason == "engine_not_built"` and a message saying the path must be absolute [0002 O28, §2.2; ADR-0071]

### Requirement: A missing helper fails at construction
A missing or non-executable helper SHALL be reported as `OcrUnavailable` carrying `EngineNotBuilt` when the engine is built, never when a page is recognised.

#### Scenario: No helper installed
- **WHEN** a process-isolated engine is built and no executable helper exists at the location discovery selects
- **THEN** construction raises `OcrUnavailable` (`OcrUnavailableError` in Python) with reason `EngineNotBuilt` [0002 O28, O7; ADR-0071]

### Requirement: The Python package ships and finds its helper
The Python binding SHALL set `helper_dir` to the absolute directory of its extension module for every process-isolated engine it builds, its default engine included. Every built wheel SHALL ship the helper binary, executable, in that directory, so that process-isolated OCR works with no environment configuration.

#### Scenario: Installed package, no configuration
- **WHEN** a Python caller with the package installed (or staged for development) constructs `OcrEngine(isolation="process")` without setting `PDF_EXTRACT_OCR_HELPER`
- **THEN** the helper beside `pdf_extract/_native` is used [0002 O28, §3.7; ADR-0076]

#### Scenario: Default engine in an installed wheel
- **WHEN** a wheel is installed into a fresh environment and `Page.ocr()` is called with no engine and no `PDF_EXTRACT_OCR_HELPER`
- **THEN** the page is recognised by a helper from the installed package directory [internal #53; ADR-0076]

### Requirement: Helper and library are version-locked
The system SHALL refuse a helper built from a different library version, or speaking a different protocol version, with `OcrUnavailable` carrying `EngineNotBuilt` at engine construction, naming both versions.

#### Scenario: Helper from another build
- **WHEN** the helper reports a library version different from the parent's
- **THEN** construction fails with `EngineNotBuilt`, and the detail names the helper's version and the library's [0002 §3.5]

### Requirement: Helper crash fails only the page
When a helper exits, is killed or crashes while a page is in flight, the system SHALL fail that page with `OcrPageError::Recognition` whose message begins `helper exited: ` followed by the exit status. The calling process SHALL continue, and the engine SHALL remain usable.

#### Scenario: Injected abort
- **WHEN** the helper calls `abort()` on receiving a page, as an `ASSERT_HOST` would
- **THEN** that page fails with `OcrPageError::Recognition` whose message names the signal (`helper exited: signal 6 (SIGABRT)`), and the crashed helper is reaped [§7.6; 0002 O27, §3.5, §2.3 item 5]

#### Scenario: The Python interpreter survives
- **WHEN** a Python caller's process-isolated engine meets the injected abort twice in a row
- **THEN** each call raises `OcrPageError` (exactly that class), and the interpreter continues and exits normally [0002 O27, O38, §4.9]

#### Scenario: Helper exits with a status
- **WHEN** the helper exits with status 3 while recognising
- **THEN** the page fails with `OcrPageError::Recognition("helper exited: exit status 3")` [0002 O27, O31]

### Requirement: Malformed helper replies fail the page
When the helper's reply violates the frame protocol (bad magic or version, a declared length over the 64 MiB cap, an unknown message, a truncated payload, invalid UTF-8, or a count over 2^20), the system SHALL kill and reap the helper and fail the page with `OcrPageError::Recognition` whose message begins `helper: ` and names the violation.

#### Scenario: Over-long frame header
- **WHEN** the helper answers a page with a frame whose declared length exceeds the cap
- **THEN** the page fails with `OcrPageError::Recognition` starting `helper: frame too large`, and the helper is reaped [0002 O27, §2.2, §3.5; ADR-0060]

### Requirement: Failed helpers are replaced at the next checkout
After any page failure, the system SHALL discard that helper's slot and spawn a fresh helper at the next checkout that needs one. The failed page SHALL NOT be retried inside the failing `ocr()` call.

#### Scenario: Recovery after a crash
- **WHEN** a page fails because its helper aborted, and the caller then recognises another page with the same engine
- **THEN** a new helper with a different process id serves the second page and it succeeds [0002 O27, §4.9]

### Requirement: Respawn failure is reported as unavailability
When a replacement helper cannot be spawned (for example the binary has been removed since construction), the system SHALL fail that `ocr()` call with `OcrUnavailable` carrying `EngineNotBuilt`.

#### Scenario: Binary removed after construction
- **WHEN** a helper fails, its binary is then deleted, and the caller recognises another page
- **THEN** that call returns `OcrError::Unavailable` with reason `EngineNotBuilt` and a detail saying the helper cannot be started [0002 §3.5, O27]

### Requirement: Respawn start-up allowance
A replacement helper's start-up (model load and validation) SHALL be allowed `max(page_timeout, 60 s) + 2 s`, measured separately from the page's `page_timeout` budget.

#### Scenario: Short page timeout
- **WHEN** an engine with a `page_timeout` shorter than the model's load time must respawn a helper
- **THEN** the respawn is given its own start-up allowance and does not fail merely because the page budget is shorter than a model load [0002 O27, §3.5]

### Requirement: Parent-enforced page deadline
Under process isolation, the parent SHALL enforce each page's deadline itself: `page_timeout` measured from the start of the `ocr()` call (rendering included) plus a 2 s transport grace. On expiry it SHALL kill the helper with `SIGKILL`, reap it and fail the page with `OcrPageError::Timeout`. The deadline SHALL also be sent to the helper.

#### Scenario: Hanging helper
- **WHEN** the helper never answers a page and `page_timeout` is 500 ms
- **THEN** the call returns `OcrPageError::Timeout` carrying the configured `page_timeout` within `page_timeout` + the 2 s transport grace + 2 s of slack for the kill and reap, and the helper is reaped after being killed by a signal [0002 O27, §3.5, §2.4]

#### Scenario: Rendering time counts
- **WHEN** rendering a page uses most of `page_timeout` before the raster is sent
- **THEN** the remaining time, not a fresh `page_timeout`, bounds the wait for the helper's reply [0002 O16, §3.5, §4.9]

### Requirement: Raster size limit on the wire
Under process isolation, the system SHALL refuse a `max_render_pixels` above 66,977,776 when the engine is configured, so that every rendered raster fits one frame (64 MiB, about 67.1 megapixels, less the request header and the rounding slack of the renderer). As a backstop the client SHALL still refuse a raster larger than one frame before sending it, failing the page with `OcrPageError::Recognition`.

#### Scenario: Oversized cap
- **WHEN** an engine with process isolation is constructed with `max_render_pixels = 80_000_000`
- **THEN** Rust returns `OcrUnavailable` with reason `InitFailed` and Python raises `ValueError` [0002 O8, O26; internal #56 B10]

#### Scenario: Oversized raster
- **WHEN** the client is handed a raster that does not fit in one frame
- **THEN** the page fails with `OcrPageError::Recognition` saying the raster does not fit in one frame, and nothing is written to the helper [0002 O26, §4.9]

### Requirement: Minimal helper environment
The system SHALL start each helper with an empty environment plus only `TESSDATA_PREFIX` (set to the resolved model directory), `LANG` and `LC_ALL` when the parent has them, and the two `PDF_EXTRACT_OCR_HELPER_FAULT*` test variables when the parent has them.

#### Scenario: Environment report
- **WHEN** a helper reports the names of its environment variables
- **THEN** every name is one of `TESSDATA_PREFIX`, `LANG`, `LC_ALL`, `PDF_EXTRACT_OCR_HELPER_FAULT`, `PDF_EXTRACT_OCR_HELPER_FAULT_ARMED`, and `TESSDATA_PREFIX` is present [0002 O29, §3.5, §4.9]

### Requirement: Helpers inherit only the standard descriptors
The system SHALL start each helper with exactly file descriptors 0, 1 and 2 open, even when the host process holds inheritable descriptors, so that a helper never holds a document's file or another helper's pipe, except that on a host without `close_range` a descriptor at or above the soft `RLIMIT_NOFILE` (lowered below a descriptor the host still holds) or at or above 1,048,576 (the sweep bound is exclusive; a plausible `sysconf(_SC_OPEN_MAX)` sets it, else the soft limit) is not marked and stays inheritable.

#### Scenario: Inheritable descriptor in the host
- **WHEN** the host process has an open descriptor without close-on-exec and spawns a helper
- **THEN** the helper's open descriptors are exactly 0, 1 and 2 [0002 O30, §4.9; 0003 F21]

### Requirement: Helpers are silent
Each helper's stderr SHALL be the null device and Tesseract's debug output inside it SHALL go to the null device, so that process isolation adds nothing to the host's stdout or stderr (the silence rule of `error-model` holds under process isolation, including when a helper crashes).

#### Scenario: Crashing helper, captured descriptors
- **WHEN** a Python process recognises pages with a process-isolated engine, including pages whose helper aborts
- **THEN** the process's file descriptors 1 and 2 receive no bytes [F11; 0002 O26, O40, §4.9]

### Requirement: No threads for isolation
The system SHALL NOT create a thread for process isolation. Pipe reads and writes SHALL run synchronously on the calling thread, with deadlines enforced by polling.

#### Scenario: One page under isolation
- **WHEN** a caller recognises a page with a process-isolated engine outside `extract(workers > 1)`
- **THEN** the library has created no thread to do it [C5; 0002 O32, §8.5]

### Requirement: Helpers survive fork in the parent
After `fork()`, an engine used in the child SHALL abandon the pool it inherited: it SHALL close the child's copies of the helpers' pipe descriptors without signalling or waiting for the parent's helpers, and SHALL spawn fresh helpers. The parent's engine SHALL continue to work. The fork-deadlock hazard is stated by the `concurrency` capability.

#### Scenario: Fork after a page
- **WHEN** a Python process recognises a page with a process-isolated engine, calls `os.fork()`, and both parent and child then recognise a page with that engine
- **THEN** both succeed, and the child exits with status 0 [0002 O30, O11, §4.9]

### Requirement: Engine drop shuts helpers down
Dropping a process-isolated engine in the process that created it SHALL ask each idle helper to shut down, wait up to 2 s for it to exit, then kill it, and SHALL reap every helper it spawned so that no zombie remains. A helper SHALL exit when its input pipe reaches end of file, so that helpers do not outlive a parent that has gone.

#### Scenario: Orderly drop
- **WHEN** an engine with idle helpers is dropped
- **THEN** each helper exits with status 0 and is reaped [0002 §3.5, §4.9]

#### Scenario: Parent disappears
- **WHEN** the parent closes or loses its end of an idle helper's pipe
- **THEN** the helper exits at the end of input [0002 §2.4]

### Requirement: Fault hooks need two variables
The helper binary SHALL honour `PDF_EXTRACT_OCR_HELPER_FAULT` (`abort-on-recognize`, `exit-on-recognize`, `hang-on-recognize`, `garbage-on-recognize`) only when `PDF_EXTRACT_OCR_HELPER_FAULT_ARMED=1` is also set in its environment. The library itself SHALL NOT change behaviour because of either variable.

#### Scenario: Armed hook
- **WHEN** both variables are set in the parent's environment with `abort-on-recognize` and a process-isolated engine recognises a page
- **THEN** the helper aborts on its first page and the page fails with `OcrPageError` [0002 O31, §4.9]

#### Scenario: Stray value without arming
- **WHEN** only `PDF_EXTRACT_OCR_HELPER_FAULT` is set
- **THEN** helpers recognise pages normally [0002 O31]

### Requirement: Containment, not sandboxing
Process isolation SHALL contain process termination by Tesseract. It SHALL NOT be relied on as a security boundary: the helper runs with the caller's privileges and no sandbox, and its well-formed replies are trusted.

#### Scenario: Well-formed but wrong reply
- **WHEN** a helper returns a well-formed result frame with incorrect text
- **THEN** that text becomes the page's result; the protocol authenticates nothing beyond the helper discovery rules [0002 §2.2; ADR-0060]

### Requirement: The Python raster-cap error names the opt-out
When a Python caller's `max_render_pixels` exceeds the process-isolation frame cap and the engine is process-isolated, including by default, the `ValueError` SHALL name both the cap and `isolation="in_process"` as the way to use a larger value.

#### Scenario: Raised cap under the default isolation
- **WHEN** a Python caller constructs `OcrEngine(max_render_pixels=80_000_000)` without an `isolation` argument
- **THEN** `ValueError` is raised, and its message contains `66977776` and `isolation='in_process'` [internal #53; internal #56 B10]

#### Scenario: Raised cap in process
- **WHEN** a Python caller constructs `OcrEngine(max_render_pixels=80_000_000, isolation="in_process")`
- **THEN** the cap is accepted [0002 O8]
