"""A sample consumer of the public API, checked by ``mypy --strict`` only
(run spec §4.11: "mypy --strict passes on the stubs and on a sample
consumer"). It is not a test module and pytest does not collect it.

The ``# type: ignore[...]`` lines are negative checks: under ``--strict``
(``warn_unused_ignores``) mypy fails if one of them stops being an error,
so they pin that the stubs reject what the runtime rejects.
"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Literal, assert_type

import pdf_extract
from pdf_extract import (
    BlockSeparator,
    Document,
    Extraction,
    LimitExceededError,
    Limits,
    OcrEngine,
    OcrInfo,
    OcrPageError,
    OcrStatus,
    OcrTimeoutError,
    OcrUnavailableError,
    OpenError,
    Page,
    PageResult,
    PageSource,
    PathLike,
    TextPage,
    TooManyPagesError,
    Unmapped,
)


def page_texts(path: PathLike, separator: BlockSeparator = "newline") -> list[str]:
    limits = Limits(max_ops_per_page=1_000_000, page_time_budget=5.0)
    with pdf_extract.open(path, max_pages=500, limits=limits) as doc:
        assert_type(doc, Document)
        texts: list[str] = []
        for page in doc:
            assert_type(page, Page)
            try:
                texts.append(page.text(block_separator=separator))
            except LimitExceededError as e:
                assert_type(e.limit_name, str)
                assert_type(e.value, int)
                texts.append("")
        return texts


def inspect(data: bytes | bytearray | memoryview, unmapped: Unmapped) -> tuple[str, ...]:
    try:
        doc = pdf_extract.open_bytes(data, limits=None)
    except TooManyPagesError as e:
        assert_type(e.limit, int)
        assert_type(e.at_least, int)
        return ()
    except OpenError:
        return ()
    assert_type(doc.page_count, int)
    assert_type(len(doc), int)
    assert_type(doc.closed, bool)
    assert_type(doc.is_encrypted, bool)
    assert_type(doc.warnings, tuple[str, ...])
    last = doc[-1]
    assert_type(last.index, int)
    tp = last.text_page(unmapped=unmapped)
    assert_type(tp, TextPage)
    assert_type(tp.source, Literal["content", "ocr"])
    assert_type(tp.dpi, int | None)
    doc.close()
    return tp.warnings


def ocr_vocabulary() -> None:
    limits = Limits(max_image_pixels=64_000_000)
    assert_type(limits.max_image_pixels, int)
    err = OcrPageError("m", "image_pixels", 5)
    assert_type(err.limit_name, str | None)
    assert_type(err.value, int | None)
    timeout = OcrTimeoutError("m", 60.0)
    assert_type(timeout.timeout, float)


def recognise(path: PathLike, page: Page) -> list[str]:
    try:
        engine = OcrEngine(
            languages=["eng"], dpi=150, max_instances=2, isolation="process", noise_precheck=False
        )
    except OcrUnavailableError as e:
        assert_type(e.reason, str)
        assert_type(e.searched, tuple[str, ...])
        engine = pdf_extract.default_ocr_engine()
    assert_type(engine.tessdata, str)
    assert_type(engine.languages, tuple[str, ...])
    assert_type(engine.page_timeout, float)
    assert_type(engine.noise_precheck, bool)
    info = engine.info()
    assert_type(info, OcrInfo)
    assert_type(info.model_sha256, dict[str, str])
    assert_type(info.linkage, Literal["vendored", "system"])
    tp = page.ocr(engine, dpi=72)
    assert_type(tp, TextPage)
    page.ocr()
    result = pdf_extract.extract(path, ocr=engine, max_ocr_pages=10, workers=4, limits=None)
    assert_type(result, Extraction)
    status: OcrStatus = result.ocr_status
    assert_type(result.pages, tuple[PageResult, ...])
    out: list[str] = [status]
    for row in result.pages:
        source: PageSource = row.source
        assert_type(row.ocr_error, str | None)
        out.append(f"{row.index}:{source}:{row.text}")
    return out


def negative_checks(doc: Document, page: Page) -> None:
    page.text(block_separator="blank")  # type: ignore[arg-type]
    page.text(unmapped="raw")  # type: ignore[arg-type]
    page.text("newline")  # type: ignore[call-arg]
    doc["0"]  # type: ignore[index]
    pdf_extract.open(3)  # type: ignore[arg-type]
    pdf_extract.open_bytes("%PDF-")  # type: ignore[arg-type]
    Limits(10)  # type: ignore[call-arg]
    OcrTimeoutError("m")  # type: ignore[call-arg]
    pdf_extract.open_bytes(b"", max_pages="1")  # type: ignore[arg-type]
    OcrEngine(isolation="thread")  # type: ignore[arg-type]
    OcrEngine(noise_precheck="no")  # type: ignore[arg-type]
    page.ocr(dpi="300")  # type: ignore[arg-type]


def main() -> None:
    version: str = pdf_extract.__version__
    print(version, page_texts(Path(os.devnull)), inspect(b"", "cid"))
