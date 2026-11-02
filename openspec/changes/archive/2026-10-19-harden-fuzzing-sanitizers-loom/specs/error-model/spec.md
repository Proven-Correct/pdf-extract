# Spec Delta

## ADDED Requirements

### Requirement: An image codec panic skips the image
A panic raised while decoding an image (JBIG2, JPEG 2000, DCT or CCITT) SHALL be caught at the codec boundary. The image SHALL draw nothing and be reported as an `image: decode failed` warning, and the rest of the page SHALL be processed as for any other undecodable image. Fuzzing builds SHALL NOT catch it.

#### Scenario: Injected codec panic, Rust
- **WHEN** a Rust caller renders a page whose image codec panics, with the test-only codec panic hook armed
- **THEN** the call returns a raster without that image and with one `image: decode failed` warning, and does not panic [R1; internal #76]

#### Scenario: Injected codec panic, Python
- **WHEN** a Python caller OCRs such a page
- **THEN** the call returns a `TextPage` whose warnings include `image: decode failed`, and raises no `InternalError` [R1; internal #76]

#### Scenario: Only the codec call is guarded
- **WHEN** a panic happens outside the codec decode, for example in resolving the image's parameters or in the interpreter
- **THEN** it is not caught by the codec boundary and follows "The Rust API lets panics propagate" [internal #76]

#### Scenario: Fuzzing sees codec panics
- **WHEN** the library is built for fuzzing
- **THEN** a codec panic propagates as a panic [§13.6; internal #76]

## MODIFIED Requirements

### Requirement: The Rust API lets panics propagate
The Rust API SHALL NOT catch panics, with one exception: a panic inside an image codec decode is caught and becomes an image warning (see "An image codec panic skips the image"). Any other panic inside the library propagates to the Rust caller as a panic, and only the Python boundary converts panics into exceptions.

#### Scenario: Rust caller sees a panic
- **WHEN** a library bug outside an image codec panics during a call made directly from Rust
- **THEN** the panic unwinds into the caller rather than being returned as an error value [ADR-0035; 0001 T45]

#### Scenario: Codec panic is not propagated
- **WHEN** an image codec panics during a call made directly from Rust
- **THEN** the call returns normally with an `image: decode failed` warning [internal #76]

### Requirement: The library writes nothing to stdout or stderr
No library call made through the Python package SHALL write to file descriptor 1 or 2, including on error paths, on caught panics (at the Python boundary or at the image codec boundary), and from vendored parser code. Rust callers own the process-global panic hook: the library SHALL NOT install or replace it for them, and SHALL write nothing to fd 1 or 2 itself.

#### Scenario: Text path over every fixture
- **WHEN** a child process with fd 1 and fd 2 redirected opens and extracts every generated and encrypted fixture, triggers every error row and the panic hook
- **THEN** both captured files are empty [F11; 0001 T48]

#### Scenario: Panic message is not printed
- **WHEN** a panic is caught at the boundary
- **THEN** its message appears only in the `InternalError`, never on stderr [F11; 0001 T45; 0001 T48]

#### Scenario: Caught codec panic is silent from Python
- **WHEN** a child process with fd 1 and fd 2 redirected renders or OCRs a page whose codec panics, through the Python package
- **THEN** both captured files are empty, and a later unrelated panic's `InternalError` carries that later panic's message, not the codec panic's [F11; internal #76]

#### Scenario: Rust callers keep their own panic hook
- **WHEN** a Rust program that installed its own panic hook calls the library, and an image codec panic is caught
- **THEN** the program's hook is the one that runs, and the library has not replaced it [internal #76]

### Requirement: The library runs on 512 KiB thread stacks
Every operation SHALL complete without stack overflow on a thread with a 512 KiB stack, for every input, in Rust and from Python; a hostile input SHALL end in a result or a `PdfExtractError` other than `InternalError`.

#### Scenario: Hostile fixtures from Python on a small stack
- **WHEN** every generated fixture and every corpus-C file is opened and extracted on a Python thread started after `threading.stack_size(512 * 1024)`
- **THEN** the process survives and no call raises `InternalError` [R1; 0001 §2.3]

#### Scenario: Hostile fixtures in Rust, text and rendering
- **WHEN** the hostile fixture set is extracted, and rendered at 36 dpi, on 512 KiB Rust threads
- **THEN** each completes with a result or a typed error, without stack overflow or panic [R1; 0001 §2.3; 0002 §2.3]

#### Scenario: Fuzz corpus on a small stack
- **WHEN** the committed fuzz regression set, and at the end of each local fuzzing session the accumulated corpus, is opened, extracted and rendered at 36 dpi on 512 KiB Rust threads
- **THEN** each input completes with a result or a typed error, without stack overflow or panic [R1; §8.6; §13.6]
