# Spec Delta

## MODIFIED Requirements

### Requirement: Engine configuration defaults
The system SHALL give `OcrConfig` these defaults: no explicit `tessdata`, `languages = ["eng"]`, `dpi = 300`, `max_instances = min(available_parallelism, 4)`, `page_timeout = 60 s`, `psm = Auto`, `oem = LstmOnly`, `max_render_pixels = 40,000,000`, `isolation = InProcess` and `noise_precheck = true`. The Python `OcrEngine` keyword arguments SHALL have the same defaults, except `isolation`, whose Python default SHALL be `"process"`.

#### Scenario: Default configuration
- **WHEN** a caller builds `OcrConfig::default()` or calls `OcrEngine()` with no arguments
- **THEN** the engine uses 300 dpi, English, a 60-second page timeout, automatic page segmentation, the LSTM engine and the noise pre-check on [F8; §7.2; 0002 O8; 0003 §3.3, §3.5]
- **AND** the Rust configuration uses in-process isolation, and the Python engine uses process isolation [internal #53]

#### Scenario: Several languages are joined for Tesseract
- **WHEN** `languages` is `["eng", "deu"]` (Python also accepts the string `"eng+deu"`)
- **THEN** Tesseract is started with the language list `eng+deu` [§7.2; 0002 O8]

### Requirement: Process-wide default engine
The system SHALL provide `default_ocr_engine()`, which builds one engine per process on first use from the default configuration and `TESSDATA_PREFIX`. A construction failure SHALL be stored and returned on every later call without resolving again. A panic during construction SHALL propagate to the caller and leave nothing stored, so the next call constructs again. Python's `default_ocr_engine()` SHALL return its own process-wide engine, built from the Python defaults (process isolation, with the helper looked up in the package directory), or raise the stored `OcrUnavailableError`.

#### Scenario: Both calls return the same engine
- **WHEN** `default_ocr_engine()` is called twice in one process
- **THEN** both calls return the same engine [§7.2; 0002 O12]

#### Scenario: A stored failure is not retried
- **WHEN** the first construction fails and `default_ocr_engine()` is called again
- **THEN** the same error is returned and tessdata resolution does not run a second time [§7.2; 0002 O12]

#### Scenario: A panic is not stored
- **WHEN** the first construction panics
- **THEN** the panic reaches the caller and the next call attempts construction again [0002 O12; ADR-0035]

#### Scenario: Python's default engine is process-isolated
- **WHEN** a Python caller calls `default_ocr_engine()`, or `Page.ocr()` with no engine, in an installed package
- **THEN** the engine's `isolation` is `"process"`, and the helper used is the one beside `pdf_extract._native` [internal #53; ADR-0076]

#### Scenario: Rust's default engine is unchanged
- **WHEN** a Rust caller calls `pdf_extract_ocr::default_ocr_engine()`
- **THEN** the engine runs in process [internal #53]

#### Scenario: Python's default engine without a helper
- **WHEN** the package directory has no helper and `default_ocr_engine()` is called from Python
- **THEN** `OcrUnavailableError` with `reason == "engine_not_built"` is raised and stored; the engine does not fall back to in-process OCR [internal #53]
