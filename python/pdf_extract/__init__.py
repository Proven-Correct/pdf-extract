"""pdf_extract: thread-safe PDF text extraction.

Open a document with :func:`open` (a path) or :func:`open_bytes` (a buffer),
then read each page's text::

    import pdf_extract

    with pdf_extract.open("report.pdf") as doc:
        for page in doc:
            print(page.index, page.text())

Pages without a usable text layer can be recognised with Tesseract:
``page.ocr()`` (with the process-wide :func:`default_ocr_engine`, or an
:class:`OcrEngine` you build), or :func:`extract`, which reads every page and
recognises only the sparse ones.

The text has PyMuPDF's ``get_text("text")`` shape: every line ends in
``"\\n"`` and an empty page is ``""``. Malformed content is worked around and
reported in ``warnings``; failures raise subclasses of
:class:`PdfExtractError` (see :mod:`pdf_extract.errors`).

This module only re-exports: the classes and functions live in the native
extension ``pdf_extract._native`` (typed by ``_native.pyi``) and the
exceptions in ``pdf_extract.errors``. Importing it does no work and starts
no threads (run spec T42). Every object may be shared between threads, and
extraction releases the GIL; on free-threaded CPython the module keeps the
GIL disabled.
"""

from __future__ import annotations

import os
from typing import Literal, TypeAlias

# errors first: the native module caches these classes when it initialises.
# isort: split
from .errors import (
    CorruptDocumentError,
    DocumentClosedError,
    DocumentNotFoundError,
    EmptyFileError,
    FileTooLargeError,
    InternalError,
    LimitExceededError,
    NotAPdfError,
    OcrError,
    OcrPageError,
    OcrTimeoutError,
    OcrUnavailableError,
    OpenError,
    PageError,
    PasswordRequiredError,
    PdfExtractError,
    TooManyPagesError,
    UnsupportedEncryptionError,
)

# isort: split
from ._native import (
    Document,
    Extraction,
    Limits,
    OcrEngine,
    OcrInfo,
    Page,
    PageResult,
    TextPage,
    __version__,
    default_ocr_engine,
    extract,
    open,
    open_bytes,
)

#: Accepted values of the ``block_separator`` option: ``"newline"`` (compat:
#: nothing marks a block boundary) or ``"blank_line"`` (an empty line between
#: blocks).
BlockSeparator: TypeAlias = Literal["newline", "blank_line"]
#: Accepted values of the ``unmapped`` option: what a glyph without Unicode
#: becomes (the raw code, U+FFFD, or nothing).
Unmapped: TypeAlias = Literal["cid", "replacement", "skip"]
#: What ``open`` accepts as a path.
PathLike: TypeAlias = str | os.PathLike[str]
#: Accepted values of ``OcrEngine(isolation=...)``: Tesseract in this process,
#: or in helper processes so that a crash fails one page only.
Isolation: TypeAlias = Literal["in_process", "process"]
#: Tesseract's page segmentation modes (``--psm``), in Tesseract's order.
PageSegMode: TypeAlias = Literal[
    "osd_only",
    "auto_osd",
    "auto_only",
    "auto",
    "single_column",
    "single_block_vert_text",
    "single_block",
    "single_line",
    "single_word",
    "circle_word",
    "single_char",
    "sparse_text",
    "sparse_text_osd",
    "raw_line",
]
#: Tesseract's engine modes (``--oem``); the legacy modes need a build with
#: the legacy engine.
EngineMode: TypeAlias = Literal["tesseract_only", "lstm_only", "tesseract_lstm_combined", "default"]
#: Where a :class:`PageResult`'s text came from.
PageSource: TypeAlias = Literal["text", "ocr", "text_fallback"]
#: How OCR went for a whole :func:`extract` call.
OcrStatus: TypeAlias = Literal["not_needed", "ok", "unavailable", "partial", "budget_exhausted"]

__all__ = [
    "BlockSeparator",
    "CorruptDocumentError",
    "Document",
    "DocumentClosedError",
    "DocumentNotFoundError",
    "EmptyFileError",
    "EngineMode",
    "Extraction",
    "FileTooLargeError",
    "InternalError",
    "Isolation",
    "LimitExceededError",
    "Limits",
    "NotAPdfError",
    "OcrEngine",
    "OcrError",
    "OcrInfo",
    "OcrPageError",
    "OcrStatus",
    "OcrTimeoutError",
    "OcrUnavailableError",
    "OpenError",
    "Page",
    "PageError",
    "PageResult",
    "PageSegMode",
    "PageSource",
    "PasswordRequiredError",
    "PathLike",
    "PdfExtractError",
    "TextPage",
    "TooManyPagesError",
    "Unmapped",
    "UnsupportedEncryptionError",
    "__version__",
    "default_ocr_engine",
    "extract",
    "open",
    "open_bytes",
]
