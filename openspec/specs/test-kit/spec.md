# test-kit Specification

## Purpose

The test kit builds PDF fixtures for tests: the Rust crate `pdf-extract-testkit` and the Python module `pdf_extract.testing` create documents with text placed at exact positions, plus the hostile, defective, scanned and codec-stress files the other capabilities' tests rely on.

## Requirements

### Requirement: Builder available in Rust and Python
The system SHALL provide a document builder (`PdfBuilder`) and a page builder (`PageBuilder`) together with `text_width`, in the Rust crate `pdf-extract-testkit` and in the Python module `pdf_extract.testing`, whose public names are exactly `PageBuilder`, `PdfBuilder` and `text_width`.

#### Scenario: Python module surface
- **WHEN** a test imports `pdf_extract.testing`
- **THEN** its `__all__` is `["PageBuilder", "PdfBuilder", "text_width"]` and both classes report the module `pdf_extract.testing` [F10; §12.3; 0001 T49]

#### Scenario: Rust surface
- **WHEN** a Rust test depends on `pdf-extract-testkit`
- **THEN** it can construct a `PdfBuilder`, add pages, insert text, call `text_width`, and serialise with `to_bytes` or `save` [F10; 0001 T49]

### Requirement: Pages of a given size
The system SHALL let a test add a page of a given width and height in points, defaulting to A4 (595 x 842 pt), and SHALL return a page builder bound to that page.

#### Scenario: Default page size
- **WHEN** Python calls `PdfBuilder().new_page()` with no arguments
- **THEN** the page is 595 x 842 pt [F10; §12.3; 0001 T49]

#### Scenario: Explicit page size
- **WHEN** a page is added with width 612 and height 792
- **THEN** the written page has that size and the next page added is independent of it [F10; 0001 T49]

#### Scenario: Each page builder writes to its own page
- **WHEN** two pages are added and text is inserted into the second before the first
- **THEN** extraction yields each run on the page whose builder inserted it [0001 T49]

### Requirement: Top-left coordinate convention
The system SHALL interpret `insert_text`'s point as the baseline origin of the first glyph, measured from the page's top-left corner with y growing downwards, the same convention as PyMuPDF's `insert_text`.

#### Scenario: Baseline on an A4 page
- **WHEN** `insert_text((100, 100), "A")` is called on an 842 pt high page
- **THEN** the content stream positions the text at PDF user-space y = 742 [§12.3; 0001 T49]

#### Scenario: Baseline follows page height
- **WHEN** `insert_text((72, 100), "x")` is called on a page 200 pt high
- **THEN** the text is positioned at PDF user-space y = 100 [§12.3; 0001 T49]

### Requirement: Standard-14 Helvetica with WinAnsiEncoding
The system SHALL set inserted text in the standard-14 Helvetica font with WinAnsiEncoding, not embedded, so that every WinAnsi character round-trips through text extraction.

#### Scenario: WinAnsi text round-trips
- **WHEN** a page carries `"Café — “quoted” €5 ½ ß ÿ Ÿ œ ‰"` and is extracted
- **THEN** the extracted text is that string followed by a newline [§12.3; 0001 T49]

### Requirement: Font size and render mode
The system SHALL accept a font size (default 11) and a text rendering mode (default 0) for each inserted run, SHALL accept every render mode 0 through 7, and SHALL not let a run's render mode affect later content.

#### Scenario: Every render mode accepted
- **WHEN** text is inserted with any render mode from 0 to 7
- **THEN** the run is written with that mode [F10; §12.3; 0001 T49]

#### Scenario: Default style
- **WHEN** text is inserted without a size or mode
- **THEN** it is written at 11 pt in render mode 0 [§12.3; 0001 §3.4]

### Requirement: Inputs the builder refuses
The system SHALL refuse, with `BuildError` in Rust and `ValueError` in Python, any inserted text containing a character outside WinAnsiEncoding (control characters included), a render mode outside 0-7, or a non-finite coordinate or font size, and SHALL append nothing to the page when it refuses.

#### Scenario: Character outside WinAnsi
- **WHEN** `insert_text` or `text_width` is given `"日本"`, a tab, a newline, NUL or `"Ā"`
- **THEN** Rust returns `BuildError::NotWinAnsi` and Python raises `ValueError` mentioning WinAnsiEncoding
- **AND** the document's bytes are unchanged [§12.3; 0001 T49]

#### Scenario: Render mode out of range
- **WHEN** Python passes `render_mode` of -1, 8, 255 or 2**70
- **THEN** `ValueError` is raised stating the mode is outside 0-7 [0001 T49]

#### Scenario: Non-finite numbers
- **WHEN** a coordinate or font size is NaN, infinite, or (from Python) too large to be finite as a 32-bit float
- **THEN** Rust returns `BuildError::NonFinite` and Python raises `ValueError` [0001 T49]

#### Scenario: Non-finite page size from Python
- **WHEN** Python calls `new_page` with a NaN, infinite or out-of-range width or height
- **THEN** `ValueError` is raised and no page is added [0001 T49]

### Requirement: Exact text widths
The system SHALL provide `text_width(text, fontsize)`, defaulting to 11 pt, returning the run's advance in points from Adobe's Helvetica AFM metrics, so tests can place runs at exact gaps.

#### Scenario: AFM width
- **WHEN** `text_width("W", 1000)` is called
- **THEN** it returns 944.0, and `text_width("")` returns 0.0 [§12.3; 0001 T49]

#### Scenario: Agreement with the oracle
- **WHEN** widths are compared with the vector measured once from PyMuPDF
- **THEN** they match `get_text_length(fontname='helv')` within 0.01 pt for every Latin-1 WinAnsi character and the oracle's rendered advance within 0.05 pt for every row, the 0x80-0x9F characters included [0001 T49; 0001 §4.3]

#### Scenario: Placing runs exactly
- **WHEN** a second run starts 0.1 em after the end of `"Alpha"` as measured by `text_width`, and in another document 0.3 em after
- **THEN** extraction yields `"AlphaBeta"` and `"Alpha Beta"` respectively [§12.3; 0001 T49]

### Requirement: Serialisation to bytes or a path
The system SHALL serialise a built document with `to_bytes()` and write the same bytes to a file with `save(path)`, where Python accepts `str` or path-like objects.

#### Scenario: Save writes to_bytes
- **WHEN** a document is saved to a path
- **THEN** the file's contents equal `to_bytes()` and the file opens with `pdf_extract.open` [F10; §12.3; 0001 T49]

#### Scenario: Save failure in Python
- **WHEN** Python saves into a directory that does not exist, or onto a directory
- **THEN** `FileNotFoundError` (with `filename` set) or `IsADirectoryError` is raised, as `builtins.open` would [0001 T49]

#### Scenario: Save failure in Rust
- **WHEN** Rust `save` cannot write the file
- **THEN** it returns the `std::io::Error` [0001 §3.4]

### Requirement: Deterministic output
The system SHALL produce identical bytes from identical builder calls, and the Python and Rust kits SHALL produce identical bytes and identical `text_width` values for the same call sequence.

#### Scenario: Same calls, same bytes
- **WHEN** the same document is built twice
- **THEN** both `to_bytes()` results are equal [0001 T49; 0001 §4.3]

#### Scenario: Python and Rust parity
- **WHEN** the shared golden call script is replayed through `pdf_extract.testing` and through the Rust kit
- **THEN** every document is byte-identical and every `text_width` result is bit-identical as a 32-bit float [0001 T49]

### Requirement: Builder output is readable
The system SHALL produce well-formed documents that the library opens and that the PyMuPDF oracle opens without raising; a builder with no pages SHALL produce a document with zero pages.

#### Scenario: Empty builder
- **WHEN** `PdfBuilder().to_bytes()` is opened
- **THEN** `page_count` is 0 [0001 T49]

#### Scenario: Builder samples open
- **WHEN** the test kit's builder samples are parsed
- **THEN** each opens and reports the expected page count [0001 §4.3]

### Requirement: Python builders shared between threads
The system SHALL allow one Python `PdfBuilder` and its page builders to be used from several threads concurrently without losing or mixing runs, and a page builder SHALL keep its document alive.

#### Scenario: Concurrent page building
- **WHEN** 8 threads each add 25 pages and insert one run per page into a shared builder
- **THEN** the document holds all 200 pages, each with exactly its own run [0001 T43; 0001 T49]

#### Scenario: Page builder outlives its document reference
- **WHEN** a page builder is used after the last reference to its `PdfBuilder` is dropped
- **THEN** `insert_text` succeeds [0001 T49]

### Requirement: Test kit is opt-in
The system SHALL not import `pdf_extract.testing` when `pdf_extract` is imported, and the Rust library crates SHALL not depend on the test kit.

#### Scenario: Import inertness
- **WHEN** a fresh interpreter runs `import pdf_extract`
- **THEN** `pdf_extract.testing` is not in `sys.modules` [0001 T49; 0001 §3.1]

### Requirement: Rust-only page features
The Rust kit SHALL additionally offer raw content streams, CropBox and Rotate, FreeText and generic annotations with appearance streams, optional-content groups (content `BDC` and form-level `/OC`), Form XObjects, and non-embedded Type 3 and Type 0 font dictionaries.

#### Scenario: Page features reach the file
- **WHEN** a page is built with a CropBox, a rotation, an extra font, a Form XObject, an annotation and an optional-content group
- **THEN** each appears in the written page dictionary and the groups are registered in the catalog's default configuration with their on/off state [0001 T50; 0001 §3.4]

#### Scenario: Earlier pages remain editable
- **WHEN** a Rust test reaches an earlier page through `page_mut(index)`
- **THEN** changes made through it land on that page [0001 §3.4]

### Requirement: Builder never encrypts
The test kit SHALL never produce encrypted files; fixtures it cannot build SHALL be produced with PyMuPDF used as a black-box tool, committed as binaries, and listed in `tests/fixtures/PROVENANCE.md` with the exact command, with the manifest hash being that of the committed file.

#### Scenario: Encrypted fixtures recorded
- **WHEN** the fixture manifest test runs
- **THEN** eight encrypted fixtures exist, each listed in the manifest with a SHA-256 matching the committed file [§12.3; 0001 T51]

#### Scenario: Regeneration changes bytes
- **WHEN** the encrypted fixtures are regenerated
- **THEN** their bytes differ (random `/ID` and file key) and the manifest entries are rewritten to the new hashes [0001 T51]

### Requirement: Hostile fixture generators
The Rust kit SHALL generate the hostile fixtures of 0001 Appendix C, each well formed apart from the one hazard it targets: deep nesting (page dictionary, content, resources, Type 4 function), page-tree shapes, a lying `/Count`, flate bombs, operator floods, Form XObject fan-out, self-recursion and linear chains, deep page trees, and 100k/1M-page trees written to a path.

#### Scenario: Page-tree shapes
- **WHEN** a test generates a flat, balanced, cyclic, duplicate-leaf, lying-count, typeless-leaf or direct-dictionary-kid page tree
- **THEN** each leaf shows the text `Page <n>` in document order so a test can tell which pages survived [0001 T50]

#### Scenario: Large variants generated at test time
- **WHEN** a test needs a 1M-page tree, 200,000-deep nesting or a large flate bomb
- **THEN** the kit generates it at test time and it is never committed [0001 T50; 0001 §3.1]

#### Scenario: Generators accepted or rejected as catalogued
- **WHEN** each Appendix C generator's output is parsed
- **THEN** it is accepted or rejected as Appendix C states [0001 §4.3]

### Requirement: Open-contract defect generators
The Rust kit SHALL generate the everyday defects the open contract is tested against: garbage after the header, a junk prefix before a valid file, a broken `startxref`, a truncated tail, plain text, a catalog without `/Pages`, an inline `/Pages` root, and a file with one wrong xref offset.

#### Scenario: Defect files available
- **WHEN** a test calls the contract generators
- **THEN** each returns a file whose only defect is the named one [0001 §3.4; 0002 §4.2]

### Requirement: Corpus D noise variants built at test time
The Rust kit SHALL build each corpus-D noise variant from its committed clean scan by adding seeded integer-only Gaussian noise (σ 12, seed 20261003) to the decoded gray samples, producing host-independent bytes that are hashed against `[[built]]` pins in `manifest.toml`, and SHALL refuse a source that is not a one-image 8-bit gray scan.

#### Scenario: Pins match and files are reused
- **WHEN** the twelve noise variants are built into the build directory
- **THEN** every file's SHA-256 equals its pin, and a second build reuses the existing files unchanged [0002 §3.8; ADR-0064]

#### Scenario: Deterministic deviates
- **WHEN** the noise generator runs with seed 20261003
- **THEN** its first eight deviates are 5, -21, 10, -16, -18, -7, -17, 6 [0002 §3.8; 0002 §4.12]

#### Scenario: Unsuitable source
- **WHEN** the source page has a text layer, two images or a 1-bit image
- **THEN** the kit returns `BuildError` [0002 §3.8; 0002 §4.12]

#### Scenario: Stale file rebuilt
- **WHEN** a file in the build directory does not match its pin
- **THEN** it is rebuilt from the clean source and the result reports both hashes without failing [0002 §3.8]

### Requirement: Image-only scan pages
The Rust kit SHALL assemble image-only PDFs from gray rasters, one unfiltered 8-bit DeviceGray image over each page's MediaBox, with bytes that depend only on the pages, and SHALL provide a pure-noise A4 page.

#### Scenario: Pure-noise page
- **WHEN** a test requests the pure-noise page
- **THEN** it gets an A4 page whose only content is a 2480 x 3509 noise raster that renders 1:1 at 300 dpi [0003 F16; 0003 §4.2]

### Requirement: Codec memory-stress generators
The Rust kit SHALL hand-assemble JBIG2 streams and JPEG 2000 codestreams in the shapes that exhaust a decoder's memory (symbol-export and referring-dictionary bombs, halftone grids, intermediate regions, tile bombs, precinct and palette shapes), with controls that decode, and a PDF wrapper drawing each.

#### Scenario: Bomb and control
- **WHEN** a test generates the JBIG2 symbol-export bomb or the JPEG 2000 tile bomb and the matching small control
- **THEN** the bomb is a small stream whose full decode would need memory far beyond its own size, and the control decodes to a valid image [0003 F6; 0003 F7; 0003 §4.4; 0003 §4.5]

### Requirement: Scripted OCR backend
The Rust kit SHALL provide, behind its `ocr` feature, a `ScriptedBackend` usable as an OCR backend or a pool factory that returns scripted outputs, errors, timeouts, panics or slow results in order, and counts calls, instances and concurrency, so every OCR error path is testable without Tesseract.

#### Scenario: Scripted failures
- **WHEN** an engine built on a `ScriptedBackend` is scripted to return a timeout, a recognition error, a panic and an empty result
- **THEN** each surfaces through the engine as the OCR error model specifies [0002 O33; 0002 §4.8]

#### Scenario: Observing requests
- **WHEN** the engine recognises a page through the backend
- **THEN** the backend records the request's dpi and the raster's dimensions, and reports call, in-flight, create, drop and clear counts [0002 O33; 0003 §4.2]
