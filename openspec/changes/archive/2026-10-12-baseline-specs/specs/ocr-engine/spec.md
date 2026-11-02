# ocr-engine Specification

## Purpose

The Tesseract OCR engine: validated construction, finding the language data, a bounded instance pool, the noise pre-check, recognition of a rendered page into a compat-shaped `TextPage`, and classification of OCR failures as host-level or page-level.

## ADDED Requirements

### Requirement: Construction validates the host once, before any page
The system SHALL validate an OCR setup when `OcrEngine::tesseract(OcrConfig)` (Python `OcrEngine(...)`) is constructed, in this order: check the configuration, resolve the tessdata directory, check that every language's model file is a readable regular file, start one Tesseract instance and run one warm-up recognition of a 1 x 1 white image. The validated instance SHALL be kept as the first member of the pool.

#### Scenario: A good configuration builds with one pooled instance
- **WHEN** an engine is built with a tessdata directory holding `eng.traineddata`
- **THEN** construction succeeds and the pool holds exactly one instance before any page is recognised [D4; §7.2; 0002 O7]

#### Scenario: A host that cannot OCR is known at construction
- **WHEN** any validation step fails
- **THEN** construction fails with `OcrUnavailable` (Python `OcrUnavailableError`) carrying `reason`, `tessdata`, `languages`, `searched` and a detail message [D5; §7.2; 0002 O7, O38]
- **AND** no page has been rendered or recognised [D5; §7.2; 0002 O7, O38]

### Requirement: Engine configuration defaults
The system SHALL give `OcrConfig` these defaults: no explicit `tessdata`, `languages = ["eng"]`, `dpi = 300`, `max_instances = min(available_parallelism, 4)`, `page_timeout = 60 s`, `psm = Auto`, `oem = LstmOnly`, `max_render_pixels = 40,000,000`, `isolation = InProcess` and `noise_precheck = true`. The Python `OcrEngine` keyword arguments SHALL have the same defaults.

#### Scenario: Default configuration
- **WHEN** a caller builds `OcrConfig::default()` or calls `OcrEngine()` with no arguments
- **THEN** the engine uses 300 dpi, English, a 60-second page timeout, automatic page segmentation, the LSTM engine, in-process isolation and the noise pre-check on [F8; §7.2; 0002 O8; 0003 §3.3, §3.5]

#### Scenario: Several languages are joined for Tesseract
- **WHEN** `languages` is `["eng", "deu"]` (Python also accepts the string `"eng+deu"`)
- **THEN** Tesseract is started with the language list `eng+deu` [§7.2; 0002 O8]

### Requirement: Out-of-range configuration is rejected at construction
The system SHALL reject an out-of-range configuration before touching the filesystem: `dpi` outside 1..=2400, `max_instances` of 0, a zero `page_timeout`, a zero `max_render_pixels`, an empty language list, or a language name that does not match `[A-Za-z0-9_]+`. From Rust this SHALL be `OcrUnavailable { reason: InitFailed }` naming the field; from Python it SHALL be `ValueError`.

#### Scenario: A path-like language name never reaches the filesystem
- **WHEN** a language name contains a path separator or `..`
- **THEN** construction fails naming the language and no model file is looked up [0002 O8, §2.2]

#### Scenario: Python reports an argument error
- **WHEN** Python calls `OcrEngine(dpi=0)` or `OcrEngine(max_instances=0)`
- **THEN** `ValueError` is raised, not `OcrUnavailableError` [0002 O8]

#### Scenario: Rust names the field
- **WHEN** Rust builds an engine with `dpi = 2401`
- **THEN** the result is `OcrUnavailable { reason: InitFailed }` whose detail names `dpi` [0002 O8]

### Requirement: One language-list grammar at every layer
The system SHALL accept a joined language list only when it matches `[A-Za-z0-9_]+(\+[A-Za-z0-9_]+)*`, in the OCR crate, the Tesseract binding, the process-isolation helper and the Python binding alike, before any file is touched.

#### Scenario: Empty segments are refused
- **WHEN** the language list is `"+"`, `"eng+"`, `"+eng"` or `"eng++fra"`
- **THEN** it is refused as an invalid argument and no model is loaded [0003 F19]

### Requirement: Legacy engine modes fail fast on the vendored build
The system SHALL refuse `EngineMode::TesseractOnly` and `EngineMode::TesseractLstmCombined` at construction on the vendored build, which does not contain the legacy engine, with `OcrUnavailable { reason: InitFailed }` (Python `OcrUnavailableError`) whose detail says the legacy engine is not built.

#### Scenario: Legacy mode on the vendored build
- **WHEN** an engine is built with `oem = TesseractOnly` against the vendored Tesseract
- **THEN** construction fails with `InitFailed` and the detail "legacy engine not built" before `Init` runs [0002 D-4, Q3; ADR-0058]

### Requirement: Tessdata resolution order without subprocesses
The system SHALL resolve the tessdata directory once per engine, by filesystem checks only and with no subprocess, shell or `PATH` search, trying in order: `OcrConfig.tessdata`; `$TESSDATA_PREFIX` and then `$TESSDATA_PREFIX/tessdata`; then `/usr/share/tesseract-ocr/5/tessdata`, `/usr/share/tesseract-ocr/4.00/tessdata`, `/usr/share/tesseract/tessdata`, `/usr/share/tessdata`, `/usr/local/share/tessdata`, `/opt/homebrew/share/tessdata`, `/opt/local/share/tessdata`.

#### Scenario: The prefix may name the parent of tessdata
- **WHEN** `TESSDATA_PREFIX=P` and the model is at `P/tessdata/eng.traineddata`
- **THEN** `P/tessdata` is chosen [§7.3; 0002 O9]

#### Scenario: The last known directory is reached
- **WHEN** no explicit directory or prefix is set and only `/opt/local/share/tessdata` holds the model
- **THEN** that directory is chosen after the other candidates are tried [§7.3; 0002 O9; ADR-0057]

#### Scenario: No directory anywhere
- **WHEN** no candidate holds the first language's model
- **THEN** construction fails with `reason = NoTessdata` (Python `"no_tessdata"`) and `searched` lists every candidate tried, in order [§7.3; 0002 O9]

### Requirement: The first language chooses the directory
The system SHALL accept a candidate directory when it contains `<first language>.traineddata`, and SHALL then require every other language's model in that same directory. An explicit `OcrConfig.tessdata` that lacks the first language's model SHALL fall through to the next candidate rather than fail by itself.

#### Scenario: A second language is missing from the chosen directory
- **WHEN** the chosen directory holds `eng.traineddata` but not `deu.traineddata` and `languages` is `["eng", "deu"]`
- **THEN** construction fails with `reason = LanguageDataMissing` (Python `"language_data_missing"`) naming the chosen directory, without trying later candidates [§7.4; 0002 O9]

#### Scenario: An explicit directory without the model falls through
- **WHEN** `OcrConfig.tessdata` lacks `eng.traineddata` and `TESSDATA_PREFIX` holds it
- **THEN** the prefix directory is chosen and the explicit one is listed in `searched` [0002 O9]

### Requirement: The engine reports the directory it chose and those it rejected
The system SHALL return the chosen model directory from `OcrEngine::tessdata()` (Python `OcrEngine.tessdata`), and SHALL list in an `OcrUnavailable`'s `searched` only the candidates that were tried and rejected, in order, never the chosen one.

#### Scenario: Searched excludes the chosen directory
- **WHEN** resolution rejects two candidates and accepts the third, and a later step fails
- **THEN** `searched` holds the two rejected candidates and `tessdata` holds the third [0002 O9]

### Requirement: The no-tessdata message names what was tried
The system SHALL begin the `NoTessdata` detail with `no directory holds <lang>.traineddata` and continue with `; the tessdata setting <P> lacks it` when an explicit directory was given, `; TESSDATA_PREFIX=<P> and <P>/tessdata lack it` when the variable was set, and `; set the tessdata option or TESSDATA_PREFIX` only when neither was.

#### Scenario: The hint does not tell the caller to set what they set
- **WHEN** `TESSDATA_PREFIX` is set and no candidate holds the model
- **THEN** the detail names the prefix and its `tessdata` subdirectory and does not advise setting `TESSDATA_PREFIX` [0003 F22]

### Requirement: Every requested language must load
The system SHALL fail Tesseract initialisation when any requested language is absent from the languages Tesseract reports as loaded, with the message `language "<x>" did not load`, reported as `OcrUnavailable { reason: InitFailed }`. Extra languages loaded by Tesseract itself SHALL NOT be an error.

#### Scenario: A model that loads nothing
- **WHEN** a requested language's model file exists but Tesseract does not load it (for example a 0-byte file)
- **THEN** construction fails with `reason = InitFailed` (Python `"init_failed"`) [0003 F20; 0002 O7]

### Requirement: A bounded pool of instances
The system SHALL create Tesseract instances lazily, up to `max_instances`, and SHALL make a caller wait, without a timeout of its own, while every instance is busy. Engines SHALL be cheap clones that share one backend and pool. A successful instance SHALL be cleared and reused for the next page.

#### Scenario: Concurrency is capped by the pool size
- **WHEN** more threads than `max_instances` recognise pages through one engine at once
- **THEN** at most `max_instances` recognitions run concurrently, the rest wait, and every page completes [C5; §7.2; 0002 O10]

#### Scenario: Instances are reused
- **WHEN** one thread recognises several pages in sequence
- **THEN** the pool still holds one instance [§7.2; 0002 O10]

### Requirement: A failed instance is dropped, not reused
The system SHALL drop, never return to the pool, an instance whose recognition returned an error, timed out, reported a C++ exception or panicked; the next checkout SHALL create a replacement. A failure to create the replacement SHALL be `OcrUnavailable { reason: InitFailed }` raised from that page's OCR call.

#### Scenario: An error drops the instance
- **WHEN** a recognition fails and another page is then recognised
- **THEN** the second page runs on a newly created instance [§7.2; 0002 O10]

#### Scenario: The model disappears after construction
- **WHEN** a replacement instance cannot be created because the model file was removed
- **THEN** that `ocr()` call fails with `OcrUnavailable` (Python `OcrUnavailableError`), a host-level failure [§7.2; 0002 O10]

### Requirement: OCR starts no threads the caller did not ask for
The system SHALL run each recognition on the calling thread and SHALL NOT start threads for OCR: the vendored Tesseract is built without OpenMP and with no hidden thread pool.

#### Scenario: Vendored build flags
- **WHEN** the vendored Tesseract is built
- **THEN** OpenMP is disabled and recognition runs only on the thread that called `ocr()` [C5; §7.6; 0002 D-4; ADR-0058]

### Requirement: Process-wide default engine
The system SHALL provide `default_ocr_engine()`, which builds one engine per process on first use from the default configuration and `TESSDATA_PREFIX`. A construction failure SHALL be stored and returned on every later call without resolving again. A panic during construction SHALL propagate to the caller and leave nothing stored, so the next call constructs again. Python's `default_ocr_engine()` SHALL return the engine or raise the stored `OcrUnavailableError`.

#### Scenario: Both calls return the same engine
- **WHEN** `default_ocr_engine()` is called twice in one process
- **THEN** both calls return the same engine [§7.2; 0002 O12]

#### Scenario: A stored failure is not retried
- **WHEN** the first construction fails and `default_ocr_engine()` is called again
- **THEN** the same error is returned and tessdata resolution does not run a second time [§7.2; 0002 O12]

#### Scenario: A panic is not stored
- **WHEN** the first construction panics
- **THEN** the panic reaches the caller and the next call attempts construction again [0002 O12; ADR-0035]

### Requirement: Engine information
The system SHALL report through `OcrEngine::info()` (Python `OcrEngine.info()` returning `OcrInfo`) the backend name `"tesseract"`, the Tesseract version, the configured languages, the tessdata directory, the SHA-256 of each language's model file computed once at construction, the linkage (`vendored` or `system`) and the isolation mode.

#### Scenario: Model checksums are available without reading files again
- **WHEN** a caller reads `info().model_sha256` after construction
- **THEN** it holds one digest per configured language, and Python exposes them as a dict of hex strings [§7.5; 0002 O13]

### Requirement: Process-global Tesseract parameters are refused
The Tesseract binding SHALL refuse, with an invalid-argument error naming the variable, any per-instance variable whose name is one of Tesseract's own process-global parameters, and the file-writing parameters `tessedit_write_params_to_file` and `tessedit_write_images`. `OcrConfig` and the Python API SHALL expose no way to set Tesseract variables.

#### Scenario: debug_file cannot undo the silence
- **WHEN** an instance is created with the variable `("debug_file", "")`
- **THEN** it is refused as an invalid argument [0003 F18; ADR-0082]

#### Scenario: Instance-level variables still work
- **WHEN** an instance is created with `("tessedit_do_invert", "0")`
- **THEN** the instance is created [0003 F18]

### Requirement: Tesseract diagnostics never reach stdout or stderr
The system SHALL route Tesseract's debug output to the platform null device and silence Leptonica's error handler, so that engine construction (a failing `Init` included), recognition and the noise pre-check write nothing to file descriptors 1 or 2. This applies to the vendored build; the general silence rule belongs to the error-model capability.

#### Scenario: A failing Init is silent
- **WHEN** construction fails because a model file cannot be loaded
- **THEN** nothing is written to fd 1 or fd 2, although Tesseract would otherwise print "Error opening data file" [F11; 0002 O14, O40]

### Requirement: Page recognition renders and recognises the whole page
The system SHALL implement `page.ocr(&engine, &OcrOptions { dpi })` (Python `Page.ocr(engine=None, *, dpi=None)`) by rendering the whole page at `dpi`, or at the engine's `dpi` when none is given, and passing the raster to one pooled instance with the rendered resolution and the engine's page segmentation mode. From Python, `engine=None` SHALL use `default_ocr_engine()`.

#### Scenario: Default resolution
- **WHEN** `page.ocr(engine)` is called with no `dpi` on an engine built with the defaults
- **THEN** the page is rendered and recognised at 300 dpi [F8; §7.1; 0002 O16]

#### Scenario: Parity resolution
- **WHEN** a caller passes `dpi=72`
- **THEN** the page is rendered and recognised at 72 dpi, the resolution PyMuPDF used [F8; §7.1]

#### Scenario: Python with no engine
- **WHEN** Python calls `page.ocr()` with no engine
- **THEN** the page is recognised by `default_ocr_engine()`, and that engine's stored failure is raised if it could not be built [0002 O21, O12]

### Requirement: The page timeout covers the whole call
The system SHALL measure `page_timeout` from the start of rendering, so that rendering, the noise pre-check and recognition share one budget, and SHALL report an exceeded budget as `OcrPageError::Timeout(page_timeout)` (Python `OcrTimeoutError` with `timeout` in seconds).

#### Scenario: A render that uses the budget is not recognised
- **WHEN** rendering finishes after the deadline
- **THEN** the call fails with a timeout and the backend is not called [0002 O16]

#### Scenario: Tesseract's monitor stops a long recognition
- **WHEN** the deadline passes during recognition
- **THEN** the call fails with `OcrTimeoutError` carrying the configured `page_timeout`, whatever the backend measured [§7.2; 0002 O16, O38]

### Requirement: Recognition result in the compat shape
The system SHALL return an OCR result as an immutable `TextPage` with `source` Ocr (Python `"ocr"`) and `dpi` equal to the resolution of the raster recognised. Tesseract blocks SHALL become blocks and text lines SHALL become lines, in Tesseract's reading order, with the words of a line joined by single spaces and empty words and lines dropped. `text()` SHALL have the same shape and sanitisation as the text path.

#### Scenario: Words become lines in reading order
- **WHEN** the backend returns two blocks of lines of words
- **THEN** `text()` holds one line per text line, words separated by one space, every line ending in `\n` and blocks separated as for the text path [F8; F9; §7.1; 0002 O17, O18]

#### Scenario: The render clamp is visible in dpi
- **WHEN** the rendering resolution is lowered to fit `max_render_pixels`
- **THEN** the `TextPage`'s `dpi` reports the lowered resolution [§7.1; 0002 O18]

#### Scenario: Word confidence is not exposed
- **WHEN** a caller inspects an OCR `TextPage` through the public API
- **THEN** only whole-page text, `source`, `dpi` and warnings are available; word confidences and the structured block model are not part of the contract [0002 O18, §2.1]

### Requirement: Recognition that finds nothing is an empty result
The system SHALL return a `TextPage` whose `text()` is `""` when recognition runs and finds no text, not an error.

#### Scenario: A blank page
- **WHEN** a blank page is recognised
- **THEN** `ocr()` succeeds and `text()` is `""` [§7.4; 0002 O15]

### Requirement: OCR page warnings
The system SHALL give an OCR `TextPage` the renderer's warnings, plus `OcrNoiseDownsampled` or `OcrNoiseSkipped` (category `ocr`) when the noise pre-check acted. No other warning kind SHALL be defined for recognition.

#### Scenario: Renderer warnings carry through
- **WHEN** rendering skipped an image whose codec memory budget was exceeded
- **THEN** the OCR `TextPage` carries exactly one image-memory warning for that page [0002 O19; 0003 F14]

#### Scenario: Noise warning text
- **WHEN** a page rendered at 300 dpi is downsampled by the pre-check
- **THEN** its `TextPage` warning reads `ocr: noise-dominated raster recognised at 150 dpi (rendered at 300)` [0003 F14]
- **AND** a skipped page's warning reads `ocr: noise-dominated raster skipped (no convincing foreground at 300 or 150 dpi)` [0003 F14]

### Requirement: Host-level and page-level OCR failures are distinct
The system SHALL report host-level failures (no tessdata, missing language data, Tesseract `Init` failure, engine not built) as `OcrUnavailable` (Python `OcrUnavailableError`, the only OCR class that is a `RuntimeError`), and page-level failures (render failure, render budget exceeded, recognition failure or C++ exception, timeout) as `OcrPageError`. Recognising a page of a closed document SHALL be `DocumentClosedError`.

#### Scenario: A render limit is a page error
- **WHEN** rendering exhausts the document's operator or time budget
- **THEN** the call fails with `OcrPageError` carrying `limit_name` and `value`, and the page is not recognised [§7.4; 0002 O15, O38; ADR-0073]

#### Scenario: A recognition failure is a page error
- **WHEN** the backend reports a recognition failure or Tesseract throws a C++ exception
- **THEN** the call fails with `OcrPageError`, which is not a `RuntimeError` [D5; §7.4; 0002 O15, §2.2]

#### Scenario: Missing language data is host-level
- **WHEN** the tessdata directory is found but a language's model is missing
- **THEN** construction fails with `OcrUnavailableError`, a `RuntimeError` with `reason == "language_data_missing"`, reported once and not per page [D5; §7.4; 0002 O15]

#### Scenario: A closed document
- **WHEN** `ocr()` is called on a page whose document is closed
- **THEN** the call fails with `DocumentClosedError` [§7.4; 0002 O15]

### Requirement: Noise pre-check statistic
The system SHALL classify a raster as `Speckle` or `Clear` by Tesseract's own global Otsu split: compute the threshold and class as Tesseract does, take the side Tesseract binarises as ink, and let `p` be that side's share of the raster. The verdict SHALL be `Speckle` iff the class is not `Empty`, `p >= 0.01` and the share of ink-side pixels with no ink-side 4-neighbour is at least `0.5 x (1 - p)^4`; otherwise `Clear`. The check SHALL be deterministic.

#### Scenario: An empty class is clear
- **WHEN** the raster is a single gray level
- **THEN** the verdict is `Clear` [0003 F11]

#### Scenario: The class never decides by itself
- **WHEN** a page has a connected dark region covering 25-75 % of the raster (an inverted banner, a figure, a shaded half)
- **THEN** the verdict is `Clear` [0003 F11, §2.3; ADR-0081]
- **AND** an inverted page whose light side is noise is `Speckle` [0003 F11, §2.3; ADR-0081]

#### Scenario: A little dust is ignored
- **WHEN** the ink side is under 1 % of the raster
- **THEN** the verdict is `Clear` and the raster goes to the backend unchanged [0003 F11, §2.3]

### Requirement: Where the noise pre-check runs
The system SHALL run the pre-check after rendering and before the backend call, for every backend (in the parent process under process isolation), only when `noise_precheck` is true and the rendered resolution is at least 144 dpi. Its time SHALL count against `page_timeout`.

#### Scenario: Below the resolution floor
- **WHEN** a noise-dominated page is recognised at 72 dpi
- **THEN** the raster goes to the backend unchanged and no `ocr:` warning is added [0003 F12, F16]

#### Scenario: The pre-check is inside the page budget
- **WHEN** the deadline passes during the pre-check
- **THEN** the call fails with a timeout and the backend is not called [0003 F12]

### Requirement: Noise remedy: downsample, re-check, else skip
The system SHALL, on a `Speckle` verdict, box-filter the raster 2 x 2 (dropping a trailing odd row or column) and classify it again. If the result is `Clear`, the system SHALL recognise the half-resolution raster with `dpi` halved and boxes kept in page points. If it is still `Speckle`, the system SHALL skip recognition and return an empty OCR `TextPage` at the rendered `dpi`. At most one downsample step SHALL be taken.

#### Scenario: The sparse noisy page is read at half resolution
- **WHEN** a near-blank page with one text line under gray noise is recognised at 300 dpi
- **THEN** the backend receives a raster of half the width and height with `dpi` 150, and the `TextPage` has `dpi() == 150` and one `OcrNoiseDownsampled` warning [0003 F13, F14]

#### Scenario: A pure-noise page is skipped
- **WHEN** a page holding only noise is recognised at 300 dpi
- **THEN** the backend is not called, `text()` is `""`, `dpi()` is 300 and the page carries one `OcrNoiseSkipped` warning [0003 F13, F14]

#### Scenario: A raster too small to halve
- **WHEN** a `Speckle` raster has a side under 2 pixels
- **THEN** it goes to the backend as rendered [0003 F13]

### Requirement: The noise pre-check can be turned off
The system SHALL send every raster to the backend as rendered when `OcrConfig.noise_precheck` is false (Python `OcrEngine(noise_precheck=False)`, reported by the `noise_precheck` property). On pages where the pre-check would not act, results with it on and off SHALL be identical within one run on one host.

#### Scenario: Opt-out restores run-0002 behaviour
- **WHEN** the sparse noisy page is recognised at 300 dpi with the pre-check off
- **THEN** it is recognised at 300 dpi with no `ocr:` warning [0003 F17]

#### Scenario: In-process and process isolation agree
- **WHEN** the noise pages are recognised with the pre-check on under `InProcess` and `Process` isolation
- **THEN** the results are identical [0003 F17]

### Requirement: The noise pre-check does not regress corpus D
The system SHALL leave every corpus-D page other than `sparse_below_min_chars_noise` without an `ocr:` warning at 300 dpi, and every page at 72 dpi. At 300 dpi `sparse_below_min_chars_noise` SHALL be recognised at 150 dpi with CER at most 0.05 in under 5 seconds.

#### Scenario: The stall is fixed
- **WHEN** the corpus-D quality test runs with the pre-check on
- **THEN** `sparse_below_min_chars_noise` at 300 dpi reports `dpi() == 150`, one `OcrNoiseDownsampled` warning, CER <= 0.05 and completes in under 5 s [0003 F16]

#### Scenario: Realistic adversarial pages are recognised in full
- **WHEN** corpus-D pages with a dark banner, inverted half, shaded box, flat gray figure, photo-like band, dark slide or full inversion are checked at 300 dpi
- **THEN** each is `Clear` and recognised as rendered [0003 F16, §2.3]

### Requirement: Custom backends behind the same engine
The system SHALL let a caller build an engine over any `OcrBackend` (`Send + Sync`) with `OcrEngine::with_backend`, which skips construction-time validation and the pool and calls the backend directly. Rendering, the noise pre-check, the page timeout, result building and failure classification SHALL apply unchanged.

#### Scenario: Every error path is testable without Tesseract
- **WHEN** an engine is built over a scripted backend that returns text, nothing, a recognition failure, a timeout or a panic
- **THEN** each outcome surfaces as the text, empty, `OcrPageError`, `OcrTimeoutError` or panic (Python `InternalError`) that the same outcome from Tesseract would give [§7.5; 0002 O33]
