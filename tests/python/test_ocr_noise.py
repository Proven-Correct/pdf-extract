"""The OCR noise pre-check from Python (run 0003 §4.2 "Python"; F12-F14):
the ``noise_precheck`` keyword and getter, the downsample and skip cases
through ``Page.ocr`` and the ``page N: ocr:`` rows of ``extract()``.

The engine is ``_native._scripted_engine``: pages are rendered and
pre-checked for real, only recognition is scripted. The ``ocr_noise``
fixture's page 0 is a small white scan, page 1 the pure noise field
(skipped at 300 dpi), page 2 corpus D's sparse page under noise
(recognised at 150 dpi).
"""

from __future__ import annotations

from pathlib import Path

import pytest

import pdf_extract
from pdf_extract import _native

from ocrkit import needs_tessdata
from pdfs import Fixtures

DOWNSAMPLED = "ocr: noise-dominated raster recognised at 150 dpi (rendered at 300)"
SKIPPED = "ocr: noise-dominated raster skipped (no convincing foreground at 300 or 150 dpi)"


@pytest.fixture
def noisy(fixtures: Fixtures) -> Path:
    return fixtures.path("ocr_noise")


def calls(engine: pdf_extract.OcrEngine) -> int:
    return _native._scripted_stats(engine)["calls"]


def test_the_keyword_defaults_to_on() -> None:
    assert _native._scripted_engine().noise_precheck is True
    assert _native._scripted_engine(noise_precheck=False).noise_precheck is False


@needs_tessdata
def test_ocr_engine_takes_the_keyword() -> None:
    assert pdf_extract.OcrEngine().noise_precheck is True
    assert pdf_extract.OcrEngine(noise_precheck=False).noise_precheck is False


def test_a_speckled_page_is_recognised_at_half_resolution(noisy: Path) -> None:
    engine = _native._scripted_engine(default=("text", "Exhibit 12"))
    tp = pdf_extract.open(noisy)[2].ocr(engine)
    assert calls(engine) == 1
    assert tp.source == "ocr"
    assert tp.dpi == 150
    assert tp.text() == "Exhibit 12\n"
    assert [w for w in tp.warnings if w.startswith("ocr: noise-dominated")] == [DOWNSAMPLED]


def test_a_pure_noise_page_is_skipped(noisy: Path) -> None:
    engine = _native._scripted_engine(default=("panic", "must not be called"))
    tp = pdf_extract.open(noisy)[1].ocr(engine)
    assert calls(engine) == 0
    assert (tp.source, tp.dpi, tp.text()) == ("ocr", 300, "")
    assert SKIPPED in tp.warnings


def test_the_opt_out_recognises_the_raster_as_rendered(noisy: Path) -> None:
    engine = _native._scripted_engine(default=("text", "noise"), noise_precheck=False)
    doc = pdf_extract.open(noisy)
    for index in (1, 2):
        tp = doc[index].ocr(engine)
        assert tp.dpi == 300
        assert not [w for w in tp.warnings if w.startswith("ocr:")]
    assert calls(engine) == 2


def test_below_144_dpi_nothing_is_screened(noisy: Path) -> None:
    engine = _native._scripted_engine(default=("empty",))
    tp = pdf_extract.open(noisy)[1].ocr(engine, dpi=100)
    assert calls(engine) == 1
    assert tp.dpi == 100
    assert not [w for w in tp.warnings if w.startswith("ocr:")]


@pytest.mark.parametrize("workers", [1, 3])
def test_extract_reports_the_noise_notes_in_page_order(noisy: Path, workers: int) -> None:
    engine = _native._scripted_engine(default=("text", "found"), max_instances=3)
    result = pdf_extract.extract(noisy, ocr=engine, workers=workers)
    assert result.ocr_status == "ok"
    sources = [(p.source, p.text, p.ocr_error) for p in result.pages]
    assert sources == [
        ("ocr", "found\n", None),
        ("text_fallback", "", None),
        ("ocr", "found\n", None),
    ]
    assert result.warnings[-2:] == (f"page 1: {SKIPPED}", f"page 2: {DOWNSAMPLED}")
    assert calls(engine) == 2
