"""OCR across ``os.fork()`` (run 0002 §4.11 item 6; O11, O30; design spec
C7).

The sequence is the spec's: the parent recognises a page, forks, the child
recognises a page with the same engine (whose pool it must rebuild: the
parent's instances or helper pipes are not its own) and exits 0 on the right
text, then the parent recognises again. The fork happens while no other
thread is using the engine; a fork *during* another thread's use is the
documented, undefended residual (build engines before forking, or use the
"spawn" start method).
"""

from __future__ import annotations

import os
import sys
import warnings
from pathlib import Path

import pytest

import pdf_extract
from pdf_extract import _native

from ocrkit import SAMPLE_LINE, needs_helper, needs_tessdata, sample_pdf

pytestmark = pytest.mark.skipif(not hasattr(os, "fork"), reason="needs os.fork")


def fork_and_recognise(
    engine: pdf_extract.OcrEngine, page: pdf_extract.Page, expected: str
) -> None:
    assert page.ocr(engine).text() == expected
    with warnings.catch_warnings():
        # CPython warns when forking a process that has other threads (the
        # test runner may); this test is about our own state, which holds
        # no lock at this point.
        warnings.simplefilter("ignore", DeprecationWarning)
        pid = os.fork()
    if pid == 0:  # pragma: no cover - the child reports only through its status
        code = 3
        try:
            code = 0 if page.ocr(engine).text() == expected else 1
        except BaseException:
            code = 2
        finally:
            os._exit(code)
    _, status = os.waitpid(pid, 0)
    assert os.waitstatus_to_exitcode(status) == 0
    assert page.ocr(engine).text() == expected


def test_fork_with_a_scripted_engine(tmp_path: Path) -> None:
    engine = _native._scripted_engine(default=("text", "scripted text"))
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    fork_and_recognise(engine, page, "scripted text\n")
    stats = _native._scripted_stats(engine)
    # The parent's counters see only its own two pages; the child's pool
    # was rebuilt in the child.
    assert stats["calls"] == 2


@needs_tessdata
def test_fork_in_process(tmp_path: Path) -> None:
    engine = pdf_extract.OcrEngine(dpi=72, max_instances=1, isolation="in_process")
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    fork_and_recognise(engine, page, SAMPLE_LINE + "\n")


@needs_helper
def test_fork_with_process_isolation(tmp_path: Path) -> None:
    engine = pdf_extract.OcrEngine(dpi=72, max_instances=1, isolation="process")
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    fork_and_recognise(engine, page, SAMPLE_LINE + "\n")


@needs_helper
def test_fork_with_the_default_engine(tmp_path: Path) -> None:
    # The default engine is built once per process and reused by the child
    # (O12), which rebuilds its pool; it is process-isolated (internal #53), so the
    # child closes the inherited helper pipes and spawns its own helper.
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    fork_and_recognise(pdf_extract.default_ocr_engine(), page, SAMPLE_LINE + "\n")
    assert sys.modules["pdf_extract"] is pdf_extract
