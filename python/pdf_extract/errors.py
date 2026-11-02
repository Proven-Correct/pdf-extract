"""Exception hierarchy of pdf_extract (design spec §9.2; run spec T44, T47b).

The classes are plain Python so that they can use multiple inheritance:
``DocumentNotFoundError`` is also a ``FileNotFoundError``, and
``OcrUnavailableError`` is also a ``RuntimeError``. The native module
imports them from here the first time it raises one.

The whole hierarchy is defined now, including the OCR classes that nothing
raises yet, so that ``__all__`` and the rule "only ``OcrUnavailableError`` is a
``RuntimeError``" are stable before OCR exists (D-17). Every class derives from
``Exception`` through ``PdfExtractError``, so ``except Exception`` catches all
of them, ``InternalError`` (a Rust panic caught at the boundary) included.

Classes that carry attributes take them as constructor arguments after the
message and define ``__reduce__``, so they survive pickling (for example when
raised in a ``multiprocessing`` worker) with their attributes intact.
"""

from __future__ import annotations

from collections.abc import Sequence

__all__ = [
    "CorruptDocumentError",
    "DocumentClosedError",
    "DocumentNotFoundError",
    "EmptyFileError",
    "FileTooLargeError",
    "InternalError",
    "LimitExceededError",
    "NotAPdfError",
    "OcrError",
    "OcrPageError",
    "OcrTimeoutError",
    "OcrUnavailableError",
    "OpenError",
    "PageError",
    "PasswordRequiredError",
    "PdfExtractError",
    "TooManyPagesError",
    "UnsupportedEncryptionError",
]


class PdfExtractError(Exception):
    """Base class of every exception raised by pdf_extract."""


class OpenError(PdfExtractError):
    """The document could not be opened.

    Raised directly for I/O failures other than a missing file (including a
    path that is not a regular file); the original ``OSError`` is the
    ``__cause__``.
    """


class DocumentNotFoundError(OpenError, FileNotFoundError):
    """The path does not exist. Also a ``FileNotFoundError``."""


class EmptyFileError(OpenError):
    """The input has zero bytes."""


class NotAPdfError(OpenError):
    """No ``%PDF-`` header lies within the first 1024 bytes."""


class CorruptDocumentError(OpenError):
    """The input has a PDF header but cannot be parsed, or exceeds
    ``Limits.max_objects`` (the message names the limit)."""


class PasswordRequiredError(OpenError):
    """The document needs a user password, which is not supported."""


class UnsupportedEncryptionError(OpenError):
    """The document uses an encryption scheme the parser does not support."""


class FileTooLargeError(OpenError):
    """The input is larger than ``Limits.max_file_size``; it was not parsed."""


class TooManyPagesError(OpenError):
    """The document has more pages than ``max_pages``.

    ``limit`` is the ``max_pages`` that was passed; ``at_least`` is how many
    pages were found before the page-tree walk stopped (``limit + 1``).
    """

    def __init__(self, message: str, limit: int, at_least: int) -> None:
        super().__init__(message)
        self.limit = limit
        self.at_least = at_least

    def __reduce__(self) -> tuple[type[TooManyPagesError], tuple[str, int, int]]:
        return (type(self), (str(self), self.limit, self.at_least))


class DocumentClosedError(PdfExtractError):
    """A page operation was started after ``Document.close()``."""


class PageError(PdfExtractError):
    """A page could not be processed."""


class LimitExceededError(PageError):
    """A per-page budget ran out.

    ``limit_name`` is ``"ops_per_page"`` or ``"page_time_budget"``; ``value``
    is the configured limit (operators, or milliseconds for time).
    """

    def __init__(self, message: str, limit_name: str, value: int) -> None:
        super().__init__(message)
        self.limit_name = limit_name
        self.value = value

    def __reduce__(self) -> tuple[type[LimitExceededError], tuple[str, str, int]]:
        return (type(self), (str(self), self.limit_name, self.value))


class OcrError(PdfExtractError):
    """Base class of OCR failures."""


class OcrUnavailableError(OcrError, RuntimeError):
    """OCR cannot run at all (no Tesseract data, missing language, ...).

    The only class in the hierarchy that is also a ``RuntimeError``, so code
    that already treats "OCR not installed" as a ``RuntimeError`` keeps
    working. ``reason`` names the cause, ``tessdata`` the directory used (if
    any), ``languages`` the languages requested and ``searched`` the
    directories that were tried.
    """

    def __init__(
        self,
        message: str,
        reason: str = "unknown",
        tessdata: str | None = None,
        languages: Sequence[str] = (),
        searched: Sequence[str] = (),
    ) -> None:
        super().__init__(message)
        self.reason = reason
        self.tessdata = tessdata
        self.languages: tuple[str, ...] = tuple(languages)
        self.searched: tuple[str, ...] = tuple(searched)

    def __reduce__(
        self,
    ) -> tuple[
        type[OcrUnavailableError],
        tuple[str, str, str | None, tuple[str, ...], tuple[str, ...]],
    ]:
        return (
            type(self),
            (str(self), self.reason, self.tessdata, self.languages, self.searched),
        )


class OcrPageError(OcrError):
    """OCR of one page failed (rendering or recognition). Not a
    ``RuntimeError``.

    When a document limit ran out while rendering the page, ``limit_name``
    names it (``"ops_per_page"``, ``"page_time_budget"``, ...) and ``value``
    is the configured limit; for every other failure both are ``None``.
    """

    def __init__(
        self, message: str, limit_name: str | None = None, value: int | None = None
    ) -> None:
        super().__init__(message)
        self.limit_name = limit_name
        self.value = value

    def __reduce__(
        self,
    ) -> tuple[type[OcrPageError], tuple[str, str | None, int | None]]:
        return (type(self), (str(self), self.limit_name, self.value))


class OcrTimeoutError(OcrPageError):
    """OCR of one page took longer than the engine's ``page_timeout``.

    ``timeout`` is that timeout in seconds; ``limit_name`` and ``value`` are
    ``None`` (a timeout is the engine's budget, not a document limit).
    """

    def __init__(self, message: str, timeout: float) -> None:
        super().__init__(message)
        self.timeout = timeout

    # The parent's ``__reduce__`` would rebuild this class with
    # ``(message, limit_name, value)``, the wrong signature, so it is
    # overridden too.
    def __reduce__(  # type: ignore[override]
        self,
    ) -> tuple[type[OcrTimeoutError], tuple[str, float]]:
        return (type(self), (str(self), self.timeout))


class InternalError(PdfExtractError):
    """A bug in pdf_extract: a Rust panic caught at the Python boundary.

    Raised instead of ``pyo3.PanicException`` (a ``BaseException``), so that
    ``except Exception`` handlers and worker threads survive it. The message
    includes the panic text and its source location.
    """
