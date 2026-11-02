# ocr-rendering Specification

## Purpose

Rendering one page into an 8-bit gray raster for OCR: the resolution actually used, the bounds that lower it, how page content, images and annotations are drawn, the warnings and errors a render produces, and the determinism and silence guarantees of the render path.

## Requirements

### Requirement: Gray page raster
The system SHALL provide `render_gray(page, options)` in the `pdf-extract-render` crate, returning a `Rendered` value that holds an 8-bit gray image (`width`, `height`, row-major `data` of `width × height` bytes, 0 black and 255 white), the resolution actually used (`dpi`) and the page's render warnings. From Python, rendering is reached only through `Page.ocr`.

#### Scenario: Letter and A4 pages at 72 dpi
- **WHEN** a Letter page and an A4 page are rendered at 72 dpi
- **THEN** the rasters are 612 × 792 and 595 × 842 pixels, each with `width × height` data bytes [§7.1; 0002 O1, §3.3, §4.7]

### Requirement: Raster dimensions from the visible box
The system SHALL size the raster as `ceil(w_pt × dpi' / 72) × ceil(h_pt × dpi' / 72)`, where `w_pt × h_pt` is the page's visible box (CropBox intersected with MediaBox) after `/Rotate` and `dpi'` is the resolution actually used.

#### Scenario: Each side is rounded up
- **WHEN** an A4 page is rendered at 300 dpi
- **THEN** the raster is 2480 × 3509 pixels (842 × 300 / 72 = 3508.33 rounds up) [0002 O1, §4.7]

#### Scenario: CropBox page
- **WHEN** a page whose CropBox lies inside its MediaBox is rendered
- **THEN** the raster has the crop's dimensions
- **AND** a CropBox that extends beyond the MediaBox is intersected with it first [0002 O1; 0001 T22]

### Requirement: Page rotation applied
The system SHALL apply the page's `/Rotate` to the raster, rotating both the raster dimensions and the drawn content.

#### Scenario: A page rotated 90 degrees renders landscape
- **WHEN** an A4 portrait page with `/Rotate 90` is rendered at 72 dpi
- **THEN** the raster is 842 × 595 pixels and the content is turned with it [0002 O1, §4.7]

### Requirement: Opaque white background
The system SHALL draw the page on an opaque white background, so that unpainted areas are 255.

#### Scenario: Blank page
- **WHEN** a page with no content is rendered
- **THEN** every byte of the raster is 255 [§7.1; 0002 O1, §4.7]

#### Scenario: Filled rectangle
- **WHEN** a page with a black filled rectangle is rendered
- **THEN** pixels inside the rectangle are 0 and pixels outside are 255, within ±2 at the edges [0002 O1, §4.7]

### Requirement: Luminance conversion
The system SHALL convert each rendered colour pixel to gray as `round(0.2126 R + 0.7152 G + 0.0722 B)` after compositing it over white, using arithmetic that gives the same value on every host.

#### Scenario: Colour content becomes luminance
- **WHEN** a page with coloured content is rendered
- **THEN** each gray byte equals the Rec. 709 weighted sum of the pixel's colour composited over white, rounded half up [§7.1; 0002 O1, O5]

### Requirement: Annotation appearances drawn under the visibility filter
The system SHALL draw annotation appearance streams under the same filter as the text path (annotations that are hidden, `NoView`, `Popup` or `Link` are not drawn), and SHALL let a Rust caller turn annotation drawing off with `RenderOptions.render_annotations`, which defaults to on. OCR always renders with annotations on.

#### Scenario: Visible and hidden annotations
- **WHEN** a page carries a visible stamp annotation and a hidden one, and is rendered with default options
- **THEN** the visible stamp's appearance is drawn and the hidden one's area stays white [§7.1; 0002 O1; ADR-0043]

#### Scenario: Annotations switched off
- **WHEN** the same page is rendered with `render_annotations = false`
- **THEN** no annotation appearance is drawn [0002 O1, §3.3]

### Requirement: Default render options
The system SHALL default `RenderOptions` to 300 dpi, `max_render_pixels` of 40,000,000 and annotations drawn, matching the OCR engine's defaults.

#### Scenario: Default options
- **WHEN** `RenderOptions::default()` is used
- **THEN** `dpi` is 300, `max_render_pixels` is 40,000,000 and `render_annotations` is true [F8, §10; 0002 O8, §3.3]

### Requirement: Configurable OCR resolution
The system SHALL render pages for OCR at a configurable resolution: the engine's `dpi` (default 300) unless a call to `Page.ocr` passes its own `dpi`. The value 72, the resolution PyMuPDF used, SHALL be accepted so parity runs can use it.

#### Scenario: Engine default
- **WHEN** `page.ocr(engine)` is called on an engine built with default settings
- **THEN** the page is rendered at 300 dpi and the `TextPage.dpi` is 300 [F8; 0002 O8, O16]

#### Scenario: Per-call override
- **WHEN** `page.ocr(engine, dpi=72)` is called
- **THEN** the page is rendered at 72 dpi and the `TextPage.dpi` is 72 [F8, §7.1; 0002 O16, O21]

### Requirement: OCR resolution range
The system SHALL accept an OCR resolution only in the range 1 to 2400 dpi. An engine `dpi` outside that range SHALL be rejected at construction with `OcrUnavailable { reason: InitFailed }` in Rust and `ValueError` in Python, and a per-call Python `dpi` outside it SHALL raise `ValueError`.

#### Scenario: Engine dpi out of range
- **WHEN** an engine is constructed with `dpi = 0` or `dpi = 2401`
- **THEN** Rust returns `OcrUnavailable` with reason `InitFailed` naming `dpi`, and Python raises `ValueError` ("dpi must be between 1 and 2400") [0002 O8]

#### Scenario: Per-call dpi out of range from Python
- **WHEN** `page.ocr(engine, dpi=0)` or `page.ocr(engine, dpi=2401)` is called
- **THEN** `ValueError` is raised and nothing is rendered [0002 O8, O21]

### Requirement: Resolution floor of one
The system SHALL treat a requested render resolution below 1 dpi as 1 dpi, so the resolution actually used is never below 1.

#### Scenario: Zero dpi in Rust
- **WHEN** `render_gray` is called with `dpi = 0`
- **THEN** the page renders at 1 dpi and `Rendered.dpi` is 1 [0002 O2, §3.3]

### Requirement: Pixel cap lowers the resolution
The system SHALL lower the resolution actually used to `dpi' = min(dpi, floor(sqrt(max_render_pixels × 72² / (w_pt × h_pt))))`, never below 1, so that the raster holds about `max_render_pixels` pixels at most. Lowering SHALL NOT be an error and SHALL be visible only through the reported resolution (`Rendered.dpi`, and `TextPage.dpi` for OCR).

#### Scenario: A4 at 300 dpi under a one-megapixel cap
- **WHEN** an A4 page is rendered at 300 dpi with `max_render_pixels = 1_000_000`
- **THEN** `dpi'` is 101, the raster is 835 × 1182 pixels and `Rendered.dpi` is 101 [§7.1, §10; 0002 O2, §4.7]

#### Scenario: The OCR result reports the lowered resolution
- **WHEN** `page.ocr` runs on an A4 page at 72 dpi with an engine whose `max_render_pixels` is 100,000
- **THEN** the call succeeds and `TextPage.dpi` is 32
- **AND** a page the noise pre-check downsamples reports half of that value, as specified by `ocr-engine` [§10; 0002 O2, O18; 0003 F13]

### Requirement: Engine pixel-cap setting
The system SHALL expose `max_render_pixels` as an OCR engine setting (Rust `OcrConfig.max_render_pixels`, Python `OcrEngine(max_render_pixels=...)` and its getter), defaulting to 40,000,000 and rejecting values below 1 at construction (`InitFailed` in Rust, `ValueError` in Python), and under process isolation values above 66,977,776.

#### Scenario: Default and getter
- **WHEN** an `OcrEngine` is constructed with default settings
- **THEN** its `max_render_pixels` is 40,000,000 [§10; 0002 O8, §3.6]

#### Scenario: Zero is rejected
- **WHEN** an engine is constructed with `max_render_pixels = 0`
- **THEN** Rust returns `OcrUnavailable` with reason `InitFailed` and Python raises `ValueError` [0002 O8]

### Requirement: Raster side bound
The system SHALL keep each raster side at or below 65,535 pixels, lowering the resolution further when a page would otherwise exceed it, whatever `max_render_pixels` allows.

#### Scenario: The largest legal page
- **WHEN** a 14,400 × 14,400 pt page is rendered at 300 dpi with default options
- **THEN** the resolution is lowered by `max_render_pixels` and both sides stay at or below 65,535 pixels [0002 O2, §2.2, §4.7]

### Requirement: Degenerate pages render as one white pixel
The system SHALL render a page whose visible box is degenerate (zero or negative area, a non-finite side, or a side over 14,400 pt) as a 1 × 1 white image rather than failing or allocating by the declared size.

#### Scenario: Zero-area visible box
- **WHEN** a page whose visible box is 0 × 0 is rendered
- **THEN** the result is a 1 × 1 image whose one byte is 255, with no error [0002 O2, §4.7]

#### Scenario: Oversized box
- **WHEN** a page whose visible box has a side over 14,400 pt is rendered
- **THEN** the result is a 1 × 1 white image [0002 O2]

### Requirement: Unresolvable page renders empty with a warning
The system SHALL render a page whose page dictionary no longer resolves (after a lazy xref repair) as a 1 × 1 white image carrying a `PageTreeNodeUnreadable` warning, matching the empty page the text path gives.

#### Scenario: Page leaf unresolvable after repair
- **WHEN** a page handle's dictionary cannot be resolved at render time
- **THEN** the render succeeds with a 1 × 1 white image and one `PageTreeNodeUnreadable` warning [0002 §3.3]

### Requirement: Document limits apply to rendering
The system SHALL render under the document's `Limits`: the per-page operator budget (`max_ops_per_page`), the per-page time budget (`page_time_budget`), the per-stream decode cap and the nesting caps apply to rendering as they do to text extraction.

#### Scenario: Within budget
- **WHEN** a page well inside its operator and time budgets is rendered
- **THEN** the render completes and returns the full raster [0002 O3]

### Requirement: Budget exhaustion is an error, not a partial raster
The system SHALL discard the raster and return `RenderError::Limit { kind, value }` when the operator or time budget runs out during rendering, with `kind` `OpsPerPage` (value: the operator limit) or `PageTimeBudget` (value: the limit in milliseconds).

#### Scenario: Operator flood
- **WHEN** an operator-flood page is rendered with `max_ops_per_page = 10_000`
- **THEN** the result is `RenderError::Limit` with kind `OpsPerPage` and value 10,000 [0002 O3, §4.7]

#### Scenario: Time budget
- **WHEN** a fan-out page is rendered with `page_time_budget = 200 ms`
- **THEN** the result is `RenderError::Limit` with kind `PageTimeBudget` within one second [0002 O3, §4.7]

### Requirement: A render limit fails the OCR call before recognition
The system SHALL fail `Page.ocr` with a page-level limit error when rendering exhausts a budget, and SHALL NOT pass any raster to the recognition backend for that page (Rust `OcrPageError::Limit { kind, value }`; Python `OcrPageError` carrying `limit_name` and `value`).

#### Scenario: Operator budget during OCR
- **WHEN** `page.ocr` is called on an operator-flood page opened with a small `max_ops_per_page`
- **THEN** Rust returns `OcrPageError::Limit` with kind `OpsPerPage`, Python raises `OcrPageError` with `limit_name == "ops_per_page"`
- **AND** the backend is called zero times [0002 O3, O15]

### Requirement: Closed document
The system SHALL return `RenderError::Closed` when the page's document was closed before the render started.

#### Scenario: Render after close
- **WHEN** a document is closed and one of its page handles is then rendered
- **THEN** the result is `RenderError::Closed` and `Page.ocr` reports the closed-document error [0002 O15, §3.3]

### Requirement: Images are decoded and drawn
The system SHALL decode and draw page images in the DCT (JPEG), JPEG 2000, JBIG2, CCITT and Flate encodings, without starting threads.

#### Scenario: One image per codec
- **WHEN** a page carrying a DCT, a JPX, a JBIG2 and a CCITT image is rendered
- **THEN** the render succeeds without panic and has non-white pixels where each image lies [0002 O4, §4.7]

### Requirement: Oversized images are skipped with a warning
The system SHALL skip any image whose pixel count exceeds `max_image_pixels`, checked against the declared `/Width × /Height` before any filter runs, against the dimensions each codec takes from its header or parameters before allocating for them, and against the dimensions the codec reports after decoding (for CCITT, which pairs `/Columns` with the declared height, these can exceed both). Each skip SHALL produce the warning kind `ImagePixelLimit` (`image: pixel limit`), not an error.

#### Scenario: Declared size over the cap
- **WHEN** a page draws an image whose declared size exceeds `max_image_pixels`
- **THEN** the render succeeds with one `ImagePixelLimit` warning and no `ImageDecodeFailure` warning [§10; 0002 O4, §4.4; ADR-0062]

#### Scenario: Codec header over the cap
- **WHEN** an image's dictionary declares a small size but its codec header reports one over the cap
- **THEN** the image is skipped with one `ImagePixelLimit` warning [0002 O4; ADR-0062]

#### Scenario: Inline image over the cap
- **WHEN** a page draws an inline image over the cap
- **THEN** it is skipped with one `ImagePixelLimit` warning [0002 O4, §4.4]

### Requirement: Codec memory-budget refusals are skipped with a warning
The system SHALL skip a JBIG2 or JPEG 2000 image whose decoder would exceed its memory budget (derived from `max_image_pixels`, as specified by `resource-limits`) and SHALL report it as the warning kind `ImageMemoryLimit` (`image: memory limit`), distinct from `ImagePixelLimit`; images within the budget SHALL draw unchanged.

#### Scenario: JBIG2 over budget
- **WHEN** a page draws a JBIG2 image whose decoder working set exceeds its budget
- **THEN** the render succeeds with one `ImageMemoryLimit` warning and the OCR `TextPage` carries the same warning [0003 F8, F10; ADR-0079]

#### Scenario: JPEG 2000 within budget
- **WHEN** a page draws a JPEG 2000 image within its budget
- **THEN** the image is drawn and no `image:` warning is raised [0003 F10; ADR-0079]

### Requirement: Undecodable images are skipped with a warning
The system SHALL skip an image that fails to decode and SHALL report it as the warning kind `ImageDecodeFailure` (`image: decode failed`), not as an error.

#### Scenario: Broken image stream
- **WHEN** a page draws an image whose data cannot be decoded
- **THEN** the render succeeds with one `ImageDecodeFailure` warning [0002 O4, §4.4]

### Requirement: Render warnings
The system SHALL report what rendering worked around on a page as warnings carrying the page index, at most one warning per kind with an occurrence count, drawn from the kinds the text path uses (`ObjectDepth`, `StreamDecodeLimit`, `ContentDamaged`, `FontLoadFailed`, `InterpreterNesting`, `FormXObjectCycle`, `UnsupportedFont`) plus the three image kinds. The warnings of an OCR `TextPage` SHALL include the render's warnings.

#### Scenario: Hostile content
- **WHEN** a page with damaged content and over-deep nesting is rendered
- **THEN** `Rendered.warnings` holds one warning of each kind that fired, each with `page` set and `count` at least 1 [0002 O3, O4, O19]

#### Scenario: Warnings reach the OCR result
- **WHEN** such a page is OCRed
- **THEN** the `TextPage` carries the same render warnings [0002 O19]

### Requirement: Hostile inputs render or hit a limit
The system SHALL either render each page of a hostile document or return `RenderError::Limit` for it, never crashing the process. A panic inside an image codec SHALL skip that image with an `image: decode failed` warning and let the page render (as specified by `error-model`). Any other panic in the render path SHALL propagate to Rust callers as a panic and SHALL reach Python callers as `InternalError` (as specified by `error-model`).

#### Scenario: Hostile fixture set
- **WHEN** every page of every hostile fixture is rendered at 36 dpi on a 512 KiB thread stack
- **THEN** each outcome is a raster or `RenderError::Limit`, with no panic and no stack overflow [R1; 0002 §2.2, §2.3, §4.7; ADR-0035]

#### Scenario: Codec panic while rendering
- **WHEN** a page draws an image whose codec panics, with the test-only codec panic hook armed
- **THEN** the page renders without that image, and the result carries one `image: decode failed` warning [R1; internal #76]

### Requirement: Deterministic rendering per host
The system SHALL produce a raster that depends only on the document bytes, the render options and the library version: rendering the same page twice on one host SHALL give identical bytes. Across hosts, rasters SHALL agree on at least 99.9 % of pixels with no pixel differing by more than 8 gray levels.

#### Scenario: Repeated render
- **WHEN** the same fixture page is rendered twice with the same options
- **THEN** the two rasters are byte-identical and match the committed per-OS golden hash [C6; 0002 O5, §4.7; ADR-0067]

#### Scenario: Cross-host comparison
- **WHEN** the golden fixtures rendered on Linux and macOS are compared
- **THEN** at least 99.9 % of pixels are identical and the largest difference is at most 8 gray levels [0002 §2.4, §4.7; ADR-0067]

### Requirement: Single-threaded and stateless rendering
The system SHALL perform each render entirely on the calling thread, starting no threads, and SHALL keep no rendering state between calls.

#### Scenario: Render starts no threads
- **WHEN** a page is rendered
- **THEN** no thread other than the caller's does rendering work and nothing from the render is cached for the next call [C5, C6; 0002 O5]

### Requirement: Silent render path
The system SHALL write nothing to file descriptors 1 or 2 while rendering, including on hostile inputs and image decode failures.

#### Scenario: Hostile and image fixtures
- **WHEN** the hostile fixture set and the image-codec fixtures are rendered in a child process whose fd 1 and fd 2 are captured
- **THEN** both captures are empty [F11; 0002 O6, §4.7]
