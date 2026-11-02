# Type stub of the native extension pdf_extract._native (crates/python).
#
# This is the normative Python stub of run spec §3.5 for everything the
# extension defines; pdf_extract/__init__.py re-exports it together with the
# type aliases and the exception classes (errors.py). Keep the two in step
# with crates/python/src: `mypy.stubtest pdf_extract._native` compares this
# file with the built module.
#
# Methods that do real work (open, open_bytes, Page.text, Page.text_page,
# Page.ocr, TextPage.text, OcrEngine(...), default_ocr_engine, extract,
# PdfBuilder.to_bytes/save, text_width) release the GIL; everything else is
# constant time. The OCR part is run 0002 §3.6; names starting with `_` are
# test hooks, not API. PdfBuilder, PageBuilder and text_width
# are the test kit, public as pdf_extract.testing (testing.py re-exports
# them); their part of the stub is spec §3.5's `pdf_extract.testing`.

from collections.abc import Iterator, Sequence
from typing import Literal, NoReturn, final

from . import (
    BlockSeparator,
    EngineMode,
    Isolation,
    OcrStatus,
    PageSegMode,
    PageSource,
    PathLike,
    Unmapped,
)

# PyO3 lists every name the module adds.
__all__ = [
    "Document",
    "Extraction",
    "Limits",
    "OcrEngine",
    "OcrInfo",
    "Page",
    "PageBuilder",
    "PageResult",
    "PdfBuilder",
    "TextPage",
    "__version__",
    "_arm_codec_panic_for_test",
    "_panic_for_test",
    "_scripted_engine",
    "_scripted_stats",
    "_set_tessdata_candidates",
    "default_ocr_engine",
    "extract",
    "open",
    "open_bytes",
    "text_width",
]

__version__: str

@final
class Limits:
    """Resource limits for one document; immutable."""

    def __init__(
        self,
        *,
        max_file_size: int = 536_870_912,
        max_page_tree_depth: int = 256,
        max_objects: int = 10_000_000,
        max_stream_decoded_bytes: int = 268_435_456,
        max_ops_per_page: int = 10_000_000,
        page_time_budget: float = 30.0,
        max_chars_per_page: int = 2_000_000,
        max_image_pixels: int = 256_000_000,
    ) -> None: ...
    @property
    def max_file_size(self) -> int: ...
    @property
    def max_page_tree_depth(self) -> int: ...
    @property
    def max_objects(self) -> int: ...
    @property
    def max_stream_decoded_bytes(self) -> int: ...
    @property
    def max_ops_per_page(self) -> int: ...
    @property
    def page_time_budget(self) -> float: ...
    @property
    def max_chars_per_page(self) -> int: ...
    @property
    def max_image_pixels(self) -> int:
        """Largest image (width x height), in pixels, that rendering will decode.

        Checked as the image declares it, as its codec reads it from the header, and as
        the codec reports it after decoding. Also sets the JBIG2 and JPEG 2000 decoders'
        memory budgets, 2 and 64 bytes per pixel of this cap, and JBIG2's allowance for
        placing symbol instances, 16 pixels per pixel of it; an image over the cap or its
        decoder's budget is skipped with an ``image:`` warning. JPEG 2000 files using
        1x1 code-blocks (non-default encoder settings) are refused above about a quarter
        of this value; raise it to admit them.
        """

def open(
    path: PathLike, *, max_pages: int | None = None, limits: Limits | None = None
) -> Document: ...
def open_bytes(
    data: bytes | bytearray | memoryview,
    *,
    max_pages: int | None = None,
    limits: Limits | None = None,
) -> Document: ...

@final
class Document:
    """An open PDF document."""

    @property
    def page_count(self) -> int: ...
    @property
    def closed(self) -> bool: ...
    @property
    def is_encrypted(self) -> bool: ...
    @property
    def warnings(self) -> tuple[str, ...]: ...
    def __len__(self) -> int: ...
    def __getitem__(self, index: int, /) -> Page: ...
    def __iter__(self) -> Iterator[Page]: ...
    def close(self) -> None: ...
    def __enter__(self) -> Document: ...
    def __exit__(self, *exc_info: object) -> None: ...

@final
class Page:
    """One page of a Document."""

    @property
    def index(self) -> int: ...
    def text(
        self, *, block_separator: BlockSeparator = "newline", unmapped: Unmapped = "cid"
    ) -> str: ...
    def text_page(self, *, unmapped: Unmapped = "cid") -> TextPage: ...
    def ocr(self, engine: OcrEngine | None = None, *, dpi: int | None = None) -> TextPage: ...

@final
class TextPage:
    """The extracted text of one page; immutable."""

    @property
    def source(self) -> Literal["content", "ocr"]: ...
    @property
    def dpi(self) -> int | None: ...
    @property
    def warnings(self) -> tuple[str, ...]: ...
    def text(self, *, block_separator: BlockSeparator = "newline") -> str: ...

@final
class OcrEngine:
    """Tesseract models plus a pool of instances; share between threads.

    With ``isolation="process"`` (the default), each pool slot is a helper
    process, so a crash inside Tesseract fails one page instead of ending
    the interpreter. ``isolation="in_process"`` runs Tesseract in this
    process (no helper; an abort inside Tesseract ends the interpreter) and
    is the only mode that allows ``max_render_pixels`` above 66977776.

    Forking. An engine built before ``os.fork()`` is reused by the child,
    which rebuilds its pool on first use. A fork while another thread is
    inside OCR, or is building an engine (the first ``default_ocr_engine()``
    call included), leaves a lock held forever in the child and deadlocks
    it. The same applies to ``multiprocessing`` with the ``fork`` start
    method. Safe patterns: use the ``spawn`` or ``forkserver`` start method
    (CPython 3.14 made ``forkserver`` the default on POSIX, per its
    release notes "Changed in version 3.14: On POSIX platforms the default
    start method was changed from *fork* to *forkserver* to retain the
    performance but avoid common multithreaded process incompatibilities";
    earlier versions default to ``fork`` on Linux), or fork before the
    first OCR call from a process with no other thread using OCR, or build
    every engine, the default one included, before forking.
    """

    def __init__(
        self,
        *,
        languages: str | Sequence[str] = "eng",
        tessdata: PathLike | None = None,
        dpi: int = 300,
        max_instances: int | None = None,
        page_timeout: float = 60.0,
        max_render_pixels: int = 40_000_000,
        psm: PageSegMode = "auto",
        oem: EngineMode = "lstm_only",
        isolation: Isolation = "process",
        noise_precheck: bool = True,
    ) -> None: ...
    @property
    def tessdata(self) -> str: ...
    @property
    def languages(self) -> tuple[str, ...]: ...
    @property
    def dpi(self) -> int: ...
    @property
    def max_instances(self) -> int: ...
    @property
    def page_timeout(self) -> float: ...
    @property
    def max_render_pixels(self) -> int: ...
    @property
    def psm(self) -> PageSegMode: ...
    @property
    def oem(self) -> EngineMode: ...
    @property
    def isolation(self) -> Isolation: ...
    @property
    def noise_precheck(self) -> bool:
        """Whether a raster dominated by speckle is recognised at half the
        resolution, or skipped when it still is; the page then carries an
        ``"ocr: noise-dominated raster ..."`` warning."""
    def info(self) -> OcrInfo: ...

@final
class OcrInfo:
    """What an OcrEngine runs."""

    @property
    def name(self) -> str: ...
    @property
    def version(self) -> str: ...
    @property
    def languages(self) -> tuple[str, ...]: ...
    @property
    def tessdata(self) -> str | None: ...
    @property
    def model_sha256(self) -> dict[str, str]: ...
    @property
    def linkage(self) -> Literal["vendored", "system"]: ...
    @property
    def isolation(self) -> Isolation: ...

def default_ocr_engine() -> OcrEngine:
    """The process-wide engine, built on first use from the defaults
    (process isolation, with the helper beside this package) and
    ``TESSDATA_PREFIX``; a failure is stored and raised again on every call
    (``OcrUnavailableError``).

    Forking: after it was built, ``os.fork()`` is safe (the child reuses the
    engine and rebuilds the pool); a fork while another thread is building
    it, or is inside OCR, deadlocks the child. That includes
    ``multiprocessing`` with the ``fork`` start method: call this once
    before starting workers, or use the ``spawn`` or ``forkserver`` start
    method (see ``OcrEngine``).
    """

def extract(
    path: PathLike,
    *,
    max_pages: int | None = None,
    min_chars: int = 50,
    ocr: OcrEngine | None = None,
    max_ocr_pages: int = 500,
    workers: int = 1,
    block_separator: BlockSeparator = "newline",
    limits: Limits | None = None,
) -> Extraction:
    """Read every page's text, recognising the sparse ones with ``ocr``.

    Forking: do not fork (``os.fork()``, or ``multiprocessing`` with the
    ``fork`` start method) while another thread is inside ``extract()``,
    ``Page.ocr()`` or the first ``default_ocr_engine()`` call; the child
    can deadlock on a lock that thread held. Use the ``spawn`` or
    ``forkserver`` start method, or build the engine before forking and
    pass it as ``ocr=``; see ``OcrEngine``.
    """

@final
class PageResult:
    """The outcome of extract() for one page."""

    @property
    def index(self) -> int: ...
    @property
    def text(self) -> str: ...
    @property
    def source(self) -> PageSource: ...
    @property
    def ocr_error(self) -> str | None: ...

@final
class Extraction:
    """The result of extract(): one PageResult per page, in page order."""

    @property
    def page_count(self) -> int: ...
    @property
    def pages(self) -> tuple[PageResult, ...]: ...
    @property
    def ocr_status(self) -> OcrStatus: ...
    @property
    def warnings(self) -> tuple[str, ...]:
        """Document-level warnings, then text-pass failures of single pages
        (``"page N: ..."``), then the noise pre-check notes of recognised
        pages (``"page N: ocr: noise-dominated raster ..."``), in page order."""

# Test hooks (run 0002 §4.11). A scripted step is ("text", str), ("empty",),
# ("recognition", str), ("timeout",), ("panic", str) or
# ("sleep", seconds, step).
def _scripted_engine(
    steps: Sequence[tuple[object, ...]] = ...,
    *,
    default: tuple[object, ...] | None = None,
    dpi: int = 300,
    page_timeout: float = 60.0,
    max_instances: int = 1,
    fail_creates: int = 0,
    noise_precheck: bool = True,
) -> OcrEngine: ...
def _scripted_stats(engine: OcrEngine) -> dict[str, int]: ...
def _set_tessdata_candidates(known: Sequence[PathLike] | None) -> None: ...

@final
class PdfBuilder:
    """Builds a PDF document page by page (test kit)."""

    def __init__(self) -> None: ...
    def new_page(self, width: float = 595.0, height: float = 842.0) -> PageBuilder: ...
    def to_bytes(self) -> bytes: ...
    def save(self, path: PathLike) -> None: ...

@final
class PageBuilder:
    """One page of a PdfBuilder (test kit)."""

    def insert_text(
        self, point: tuple[float, float], text: str, *, fontsize: float = 11.0, render_mode: int = 0
    ) -> None: ...

def text_width(text: str, fontsize: float = 11.0) -> float: ...
def _panic_for_test(*, detached: bool = False, resumed: bool = False) -> NoReturn:
    """Test hook: panics in Rust, which arrives as InternalError."""

def _arm_codec_panic_for_test() -> None:
    """Test hook: the next image-codec decode on this thread panics inside the
    codec boundary and is skipped with an ``image: decode failed`` warning."""
