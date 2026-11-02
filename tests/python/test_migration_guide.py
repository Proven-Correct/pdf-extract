"""Every Python example in ``docs/migrating-from-pymupdf.md`` runs
(``migration-parity`` spec, "Migration guide uses the parity API").

The guide promises its examples run with two names defined: ``pdf_path``, a
PDF file, and ``old_pages``, the page texts the reader's current extractor
returned for it. Here the PDF is built with the test kit and ``old_pages``
is pdf_extract's own text, standing in for PyMuPDF's (which this project
never installs). The blocks run in order in one namespace, as a reader
pasting them in sequence would.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

import pdf_extract
from pdf_extract.testing import PdfBuilder

from pdfs import REPO

GUIDE = REPO / "docs" / "migrating-from-pymupdf.md"
BLOCK = re.compile(r"^```python\n(.*?)^```$", re.MULTILINE | re.DOTALL)


def blocks() -> list[str]:
    return BLOCK.findall(GUIDE.read_text(encoding="utf-8"))


def test_the_guide_has_examples() -> None:
    assert len(blocks()) >= 2


def test_every_example_runs(tmp_path: Path) -> None:
    doc = PdfBuilder()
    for i in range(3):
        doc.new_page().insert_text((72, 100), f"Page {i + 1} of a migration test, long enough.")
    pdf_path = tmp_path / "sample.pdf"
    doc.save(str(pdf_path))
    with pdf_extract.open(pdf_path) as d:
        old_pages = [page.text() for page in d]
    namespace: dict[str, object] = {"pdf_path": pdf_path, "old_pages": old_pages}
    for i, code in enumerate(blocks()):
        try:
            exec(compile(code, f"{GUIDE.name} example {i + 1}", "exec"), namespace)
        except Exception as e:  # pragma: no cover - the failure message is the point
            pytest.fail(f"example {i + 1} of {GUIDE.name} failed: {e!r}\n{code}")


def test_rollout_is_offline_then_one_cutover() -> None:
    """The guide measures parity offline and cuts over in one release; it
    never recommends running PyMuPDF beside pdf-extract in production."""
    text = GUIDE.read_text(encoding="utf-8")
    rollout = text[text.index("## 4.") :]
    assert "Don't run PyMuPDF beside pdf-extract in production" in rollout
    assert "tools/oracle/run_private_corpus.sh" in rollout
    assert "### Cut over" in rollout
    assert "drops `pymupdf`" in rollout
