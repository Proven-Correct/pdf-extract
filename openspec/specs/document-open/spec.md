# document-open Specification

## Purpose

Opening a PDF from a file path or from an in-memory buffer: size limits, content-only file-type detection, classification of parser and encryption outcomes, the object-count check, xref-repair warnings, and the lifecycle of an open document up to and after `close()`.

## Requirements

### Requirement: Open from a path
The system SHALL open a PDF from a file path through `Document::open` (Rust) and `pdf_extract.open` (Python), reading the whole file into memory and closing the file descriptor before the call returns.

#### Scenario: Path opens and the descriptor is released
- **WHEN** `pdf_extract.open(path)` is called on a readable regular file that holds a valid PDF
- **THEN** a `Document` is returned, and no file descriptor for `path` remains open in the process [F1, §5.1; 0001 T1]

#### Scenario: Path-like arguments
- **WHEN** `pdf_extract.open` receives a `str`, a `pathlib.Path` or any object implementing `__fspath__`
- **THEN** the document opens [F1, §12.2; 0001 §3.5]
- **AND** a non-path argument such as an integer raises `TypeError`

### Requirement: Open from memory
The system SHALL open a PDF from an in-memory buffer through `Document::from_bytes` and `Document::from_shared` (Rust) and `pdf_extract.open_bytes` (Python), and the resulting document SHALL NOT depend on the caller's buffer after the call returns.

#### Scenario: Python buffer types accepted
- **WHEN** `open_bytes` receives `bytes`, `bytearray`, or a `memoryview` (including a non-contiguous byte view, or a C-contiguous view of a wider item format)
- **THEN** the document opens from those bytes [F1, §5.1; 0001 T2]

#### Scenario: Buffer copied once, no reference kept
- **WHEN** a `bytearray` passed to `open_bytes` is cleared and refilled with other bytes after the call returns
- **THEN** the resize succeeds and the document still extracts the original text [§5.1; 0001 T2]

#### Scenario: Shared buffer is not copied
- **WHEN** a Rust caller opens with `Document::from_shared`
- **THEN** the document uses the caller's shared buffer without copying it [0001 §3.3]

#### Scenario: Non-buffer arguments rejected
- **WHEN** `open_bytes` receives a `str`, an integer, `None`, a list, or a non-contiguous memoryview of a wider item format
- **THEN** `TypeError` is raised [0001 §3.5]

### Requirement: File size limit
The system SHALL refuse a file larger than `max_file_size` (default 512 MiB, overridable through `Limits`) with `FileTooLargeError`, judging the size from file metadata before reading any content.

#### Scenario: Oversized file refused before reading
- **WHEN** `open` is called on a file whose size exceeds `Limits(max_file_size=N)`
- **THEN** `FileTooLargeError` is raised (Rust `OpenError::TooLarge { size, limit }`) and the content is not read [§5.1, §10; 0001 T1, 0001 §1.4]

#### Scenario: Exactly at the limit
- **WHEN** a file or buffer is exactly `max_file_size` bytes long
- **THEN** it is not refused on size [§10; 0001 T1]

#### Scenario: File that grows after the size check
- **WHEN** a file grows past `max_file_size` between the metadata check and the read
- **THEN** at most `max_file_size + 1` bytes are read and the open fails with `FileTooLargeError` [0001 T1]

### Requirement: Size limit applies to buffers
The system SHALL apply `max_file_size` to in-memory buffers as well, so that bytes refused on disk are never parsed when passed as a buffer; the Python binding SHALL check the size before copying the buffer.

#### Scenario: Oversized buffer
- **WHEN** `open_bytes` or `Document::from_bytes` receives a buffer longer than `max_file_size`
- **THEN** `FileTooLargeError` (Rust `OpenError::TooLarge`) is raised and the buffer is neither copied nor parsed [0001 T2; ADR-0026]

### Requirement: Regular files only
The system SHALL refuse a path that is not a regular file (directory, FIFO, socket or device) with `OpenError` before opening or reading it, and SHALL follow symbolic links.

#### Scenario: FIFO does not block
- **WHEN** `open` is called on a FIFO with no writer
- **THEN** the call returns promptly with `OpenError` whose message says "not a regular file" and whose `__cause__` is an `OSError` (Rust `OpenError::Io` of kind `InvalidInput`) [ADR-0026; 0001 §4.8]

#### Scenario: Directory and device
- **WHEN** `open` is called on a directory or on `/dev/null`
- **THEN** `OpenError` (exactly that class, not a subclass) is raised [ADR-0026; 0001 §4.8]

#### Scenario: Symlink to a regular file
- **WHEN** `open` is called on a symbolic link to a regular PDF file
- **THEN** the document opens [ADR-0026; 0001 §4.8]

### Requirement: Missing file
The system SHALL raise `DocumentNotFoundError`, which is also a `FileNotFoundError`, when the path does not exist.

#### Scenario: Missing path
- **WHEN** `open` is called on a path that does not exist
- **THEN** `DocumentNotFoundError` is raised (Rust `OpenError::NotFound(path)`), carrying the path as its filename [F3, §9.2; 0001 T47b]

### Requirement: Other file-system failures
The system SHALL convert any other `OSError` raised while opening a path into the base `OpenError`, with the original `OSError` as `__cause__`.

#### Scenario: Unreadable file
- **WHEN** `open` is called on a file the process has no permission to read
- **THEN** `OpenError` is raised whose `__cause__` is a `PermissionError` [§9.2; 0001 T47]

### Requirement: Content-only file-type detection
The system SHALL decide whether input is a PDF from its content alone and SHALL NOT consult the file name or extension, nor guess any other format.

#### Scenario: Valid PDF with any name
- **WHEN** a valid PDF is saved as `doc.txt` or with no extension and opened
- **THEN** it opens normally [F2, §5.2; 0001 T3]

#### Scenario: Plain text named .pdf
- **WHEN** a plain-text file named `notes.pdf` is opened
- **THEN** `NotAPdfError` is raised (Rust `OpenError::NotPdf`) [F2, §5.2; 0001 T3]

### Requirement: Empty input
The system SHALL raise `EmptyFileError` for a zero-length file or buffer.

#### Scenario: Zero-byte file
- **WHEN** a zero-byte file or an empty buffer is opened
- **THEN** `EmptyFileError` is raised (Rust `OpenError::Empty`) [F3, §5.2; 0001 T3]

### Requirement: PDF header window
The system SHALL treat input as a PDF only if the five bytes `%PDF-` lie wholly within its first 1024 bytes, and SHALL raise `NotAPdfError` otherwise.

#### Scenario: Header after a junk prefix
- **WHEN** a valid PDF is preceded by 500 bytes of junk
- **THEN** it opens [F2, §5.2; 0001 §4.8]

#### Scenario: Header straddling byte 1024
- **WHEN** the `%PDF-` tag starts at byte 1020, so that it spills past the window
- **THEN** `NotAPdfError` is raised [0001 T3; ADR-0026]

#### Scenario: Header beyond the window
- **WHEN** the header starts at byte 1500
- **THEN** `NotAPdfError` is raised, a documented divergence from PyMuPDF, which opens such a file [F2; 0001 §4.8; ADR-0026]

### Requirement: Unparseable structure
The system SHALL raise `CorruptDocumentError` when input passes header detection but neither normal parsing nor xref repair yields a usable trailer and catalog.

#### Scenario: Header followed by garbage
- **WHEN** `%PDF-1.4` is followed by bytes that are not PDF syntax
- **THEN** `CorruptDocumentError` is raised (Rust `OpenError::Corrupt`) [F3, §5.2; 0001 T4, 0001 T5]

#### Scenario: Catalog without a /Pages key
- **WHEN** the catalog has no `/Pages` entry at all
- **THEN** `CorruptDocumentError` is raised, a documented divergence from PyMuPDF, which opens it with zero pages [0001 T5; ADR-0025]

### Requirement: Every open failure raises
The system SHALL report every failure to open by raising an exception from the library's `OpenError` hierarchy (Rust: returning an `OpenError`), never by a sentinel return value or a silently empty document.

#### Scenario: All open failures share a base class
- **WHEN** opening fails for a missing file, an empty file, an oversized file, non-PDF content, an unparseable structure, a user password, an unsupported security handler or `max_pages`
- **THEN** the exception raised is an instance of `pdf_extract.OpenError` and of `pdf_extract.PdfExtractError` [F3, §9.2; 0001 T47b]

### Requirement: Owner-password-only documents open
The system SHALL open and extract documents encrypted with the standard security handler and an empty user password (RC4 40-bit and 128-bit, AES-128, AES-256 R6) transparently, with no password argument.

#### Scenario: Owner-only fixtures
- **WHEN** each owner-password-only fixture (RC4 40, RC4 128, AES-128, AES-256) is opened
- **THEN** it opens, `page_count` is correct and its text extracts [F4, §5.5; 0001 T6]

### Requirement: Encryption flag
The system SHALL report `is_encrypted` as true exactly when the parsed trailer installs a decryptor, regardless of how the `/Encrypt` key is spelled in the file.

#### Scenario: Encrypted and unencrypted documents
- **WHEN** an owner-password-only document and an unencrypted document are opened
- **THEN** `is_encrypted` is true for the first and false for the second [F4, §5.5; 0001 T6; ADR-0031]

#### Scenario: Escaped or incidental /Encrypt
- **WHEN** a trailer spells the key as `/Encr#79pt`, or an unencrypted file mentions `/Encrypt` only in a content-stream comment
- **THEN** the first reports `is_encrypted` true and the second false [ADR-0031]

### Requirement: User password raises at open
The system SHALL raise `PasswordRequiredError` from the open call itself for a document that needs a non-empty user password, and SHALL NOT accept a password.

#### Scenario: User-password fixtures
- **WHEN** each user-password fixture (RC4 40, RC4 128, AES-128, AES-256) is opened
- **THEN** `PasswordRequiredError` is raised by `open`/`open_bytes` (Rust `OpenError::PasswordRequired`), not by a later page operation [F4, §5.5; 0001 T6]

### Requirement: Unsupported encryption
The system SHALL raise `UnsupportedEncryptionError` at open for an encryption dictionary it cannot use: an invalid dictionary, an unsupported algorithm, or a non-standard (public-key or unknown) security handler.

#### Scenario: Unknown revision
- **WHEN** the trailer's `/Encrypt` names the standard filter with `/V 9 /R 9`
- **THEN** `UnsupportedEncryptionError` is raised, its message says the algorithm is not supported, and it is an `OpenError` [§5.5; 0001 T4]

#### Scenario: Missing /ID tolerated
- **WHEN** an encrypted document lacks a trailer `/ID` but decrypts with an empty `/ID`
- **THEN** it opens; `UnsupportedEncryptionError` is raised for a missing `/ID` only when the parser cannot proceed without it [0001 T4]

### Requirement: Permission bits ignored
The system SHALL ignore the `/P` permission bits of an encrypted document, so text extraction is always allowed once the document opens.

#### Scenario: Restrictive permissions
- **WHEN** an owner-password-only document whose `/P` forbids content extraction is opened
- **THEN** its text extracts as if no restriction were set [§5.5]

### Requirement: Object-count limit
The system SHALL refuse a document whose cross-reference table holds more than `max_objects` entries (default 10,000,000, overridable through `Limits`) with `CorruptDocumentError` whose message names the limit (Rust `OpenError::Limit { kind: Objects, value }`).

#### Scenario: Limit exceeded
- **WHEN** any fixture is opened with `Limits(max_objects=3)`
- **THEN** `CorruptDocumentError` is raised whose message mentions "objects" [§10; 0001 T7, 0001 T47b]

#### Scenario: Limit exceeded only after repair during the page-tree walk
- **WHEN** the table read at open is within `max_objects` but a repair triggered during the page-tree walk rebuilds it with more entries than the limit
- **THEN** the open still fails with `CorruptDocumentError` naming the objects limit [0001 T7]

### Requirement: Xref repair reported at open
The system SHALL open a document whose cross-reference data is damaged but repairable, and SHALL attach the document warning `xref: repaired` (kind `XrefRepaired`) when repair happened during open.

#### Scenario: Broken startxref or truncated tail
- **WHEN** a document with a wrong `startxref` offset, or with its tail cut through the trailer or xref table, is opened
- **THEN** it opens with the correct page count and `Document.warnings` contains `xref: repaired` [§5.3; 0001 T8]

#### Scenario: Repair triggered by the page-tree walk
- **WHEN** the only damage is a wrong xref offset for a page-tree node, so that repair first happens while the page tree is walked
- **THEN** the document opens and `xref: repaired` is still among its warnings [0001 T8]

#### Scenario: Damage not reached at open
- **WHEN** the only wrong xref offset belongs to a content stream, which open never reads
- **THEN** the document opens with no warnings [0001 T8]

### Requirement: Later xref repair belongs to the document
The system SHALL add exactly one `xref: repaired` to `Document.warnings` when a page operation after open is the first to trigger repair, SHALL NOT attach it to that operation's `TextPage`, and SHALL never report it twice.

#### Scenario: Lazy repair during extraction
- **WHEN** a document opens with no warnings and a later `text_page()` triggers xref repair
- **THEN** all pages extract correctly, `Document.warnings == ("xref: repaired",)`, and no `TextPage.warnings` entry mentions the xref [0001 T8]

#### Scenario: Repaired at open and again observed later
- **WHEN** a document repaired at open is used by page operations that also observe the repair
- **THEN** `Document.warnings` contains `xref: repaired` once [0001 §3.3]

### Requirement: Closing a document
The system SHALL provide `close()`, which is idempotent, after which every page operation on the document or its pages raises `DocumentClosedError` (Rust `PageError::Closed`) and the parsed document is freed once operations already running finish.

#### Scenario: Use after close
- **WHEN** `close()` is called twice and then `doc[0]`, `page.text()` or `page.text_page()` is called
- **THEN** the second `close()` does nothing and each operation raises `DocumentClosedError` [§5.6, §8.2; 0001 T40]

#### Scenario: Memory released
- **WHEN** a document with no operation in flight is closed
- **THEN** its parsed document and buffer are freed immediately [§5.6, §8.2; 0001 T40]

### Requirement: Document facts survive close
The system SHALL keep `page_count`, `len()`, `is_encrypted`, `warnings` and `closed` readable after `close()`.

#### Scenario: Facts after close
- **WHEN** a repaired single-page document is closed
- **THEN** `closed` is true, `page_count == 1`, `len(doc) == 1`, `is_encrypted` is false and `xref: repaired` is still in `warnings` [§5.6, §11; 0001 §3.3]

### Requirement: Handles and results outlive close
The system SHALL keep `Page` and `TextPage` objects valid after their document closes: a `Page` still reports its `index` and its operations raise `DocumentClosedError`, and a `TextPage` stays fully readable because it owns its data.

#### Scenario: Results obtained before close
- **WHEN** a `TextPage` is built, the document is closed, and the `TextPage` and its `Page` are used
- **THEN** `TextPage.text()` returns the same text as before, `Page.index` answers, and `Page.text()` raises `DocumentClosedError` [§5.6; 0001 §4.8]

#### Scenario: Iterating a closed document
- **WHEN** a closed document is iterated
- **THEN** it yields one handle per page, and each handle's operations raise `DocumentClosedError` [0001 §3.3]

### Requirement: Context manager
The system SHALL make `Document` a context manager whose `__enter__` returns the document and whose `__exit__` always calls `close()` and never suppresses an exception.

#### Scenario: Exception inside the block
- **WHEN** an exception is raised inside `with pdf_extract.open(path) as doc:`
- **THEN** the exception propagates unchanged and `doc.closed` is true afterwards [§5.6, §12.2; 0001 §3.5]

#### Scenario: Normal exit
- **WHEN** the `with` block exits normally
- **THEN** the bound name is the opened document and it is closed afterwards [§5.6; 0001 §3.5]

### Requirement: Dropping the last reference frees the document
The system SHALL free a document's memory when the last reference to it is dropped, through garbage collection or `del`, without an explicit `close()`.

#### Scenario: Last clone dropped
- **WHEN** every Rust clone of a `Document`, or every Python reference to it, is dropped
- **THEN** the parsed document is freed [§5.6; 0001 §4.8]
