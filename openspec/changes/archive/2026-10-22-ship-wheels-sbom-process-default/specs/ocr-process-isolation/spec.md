# Spec Delta

## MODIFIED Requirements

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

### Requirement: The Python package ships and finds its helper
The Python binding SHALL set `helper_dir` to the absolute directory of its extension module for every process-isolated engine it builds, its default engine included. Every built wheel SHALL ship the helper binary, executable, in that directory, so that process-isolated OCR works with no environment configuration.

#### Scenario: Installed package, no configuration
- **WHEN** a Python caller with the package installed (or staged for development) constructs `OcrEngine(isolation="process")` without setting `PDF_EXTRACT_OCR_HELPER`
- **THEN** the helper beside `pdf_extract/_native` is used [0002 O28, §3.7; ADR-0076]

#### Scenario: Default engine in an installed wheel
- **WHEN** a wheel is installed into a fresh environment and `Page.ocr()` is called with no engine and no `PDF_EXTRACT_OCR_HELPER`
- **THEN** the page is recognised by a helper from the installed package directory [internal #53; ADR-0076]

## ADDED Requirements

### Requirement: The Python raster-cap error names the opt-out
When a Python caller's `max_render_pixels` exceeds the process-isolation frame cap and the engine is process-isolated, including by default, the `ValueError` SHALL name both the cap and `isolation="in_process"` as the way to use a larger value.

#### Scenario: Raised cap under the default isolation
- **WHEN** a Python caller constructs `OcrEngine(max_render_pixels=80_000_000)` without an `isolation` argument
- **THEN** `ValueError` is raised, and its message contains `66977776` and `isolation='in_process'` [internal #53; internal #56 B10]

#### Scenario: Raised cap in process
- **WHEN** a Python caller constructs `OcrEngine(max_render_pixels=80_000_000, isolation="in_process")`
- **THEN** the cap is accepted [0002 O8]
