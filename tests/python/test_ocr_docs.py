"""The fork hazard is stated in the docstrings users see (run 0003 F2).

``OcrEngine.__doc__`` comes from the Rust pyclass comment, so this also
pins that it stays in step with the stub's text.
"""

from __future__ import annotations

import pytest

import pdf_extract

DOCUMENTED = [
    pytest.param(pdf_extract.OcrEngine, id="OcrEngine"),
    pytest.param(pdf_extract.default_ocr_engine, id="default_ocr_engine"),
    pytest.param(pdf_extract.extract, id="extract"),
]


@pytest.mark.parametrize("obj", DOCUMENTED)
def test_docstring_names_the_fork_hazard_and_the_safe_start_method(obj: object) -> None:
    doc = obj.__doc__ or ""
    assert "fork" in doc
    assert "spawn" in doc
    assert "forkserver" in doc


def test_engine_docstring_quotes_the_python_314_change() -> None:
    doc = pdf_extract.OcrEngine.__doc__ or ""
    assert "3.14" in doc
    assert "from *fork* to *forkserver*" in doc
