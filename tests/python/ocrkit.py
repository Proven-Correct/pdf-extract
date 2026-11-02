"""Helpers for the OCR tests (run 0002 §4.11): test PDFs, the model and
helper prerequisites, and child interpreters.

Tests that need real Tesseract models (``needs_tessdata``) or the staged
helper binary (``needs_helper``) skip when they are missing on a developer
machine, but **fail** when the ``CI`` variable is set: CI installs the model
and stages the helper, so a skip there would hide a broken setup.
"""

from __future__ import annotations

import os
import subprocess
import sys
import textwrap
from collections.abc import Mapping, Sequence
from pathlib import Path

import pytest

import pdf_extract
from pdf_extract.testing import PdfBuilder

from pdfs import REPO

#: Where the staged helper must be: beside the extension module (O28).
HELPER = Path(pdf_extract._native.__file__).resolve().parent / "pdf-extract-ocr-helper"

#: The known install locations of O9, in order (crates/ocr/src/tessdata.rs).
KNOWN_DIRS = (
    "/usr/share/tesseract-ocr/5/tessdata",
    "/usr/share/tesseract-ocr/4.00/tessdata",
    "/usr/share/tesseract/tessdata",
    "/usr/share/tessdata",
    "/usr/local/share/tessdata",
    "/opt/homebrew/share/tessdata",
    "/opt/local/share/tessdata",
)

#: One line of text, large enough to recognise reliably at 72 dpi.
SAMPLE_LINE = "Hello OCR world from pdf extract"


def find_tessdata() -> Path | None:
    """The directory O9 would choose for ``eng``, or ``None``."""
    candidates: list[Path] = []
    prefix = os.environ.get("TESSDATA_PREFIX")
    if prefix:
        candidates += [Path(prefix), Path(prefix) / "tessdata"]
    candidates += [Path(d) for d in KNOWN_DIRS]
    for d in candidates:
        if (d / "eng.traineddata").is_file():
            return d
    return None


def _required(present: bool, what: str) -> pytest.MarkDecorator:
    # Under CI the test is never skipped, so a missing prerequisite fails it.
    return pytest.mark.skipif(not present and not os.environ.get("CI"), reason=f"needs {what}")


needs_tessdata = _required(find_tessdata() is not None, "eng.traineddata (TESSDATA_PREFIX)")
#: The helper alone, for tests that need no model (e.g. that its absence of
#: models is reported by the process-isolated default engine).
needs_staged_helper = _required(
    HELPER.is_file(), f"the staged helper ({HELPER}; run tools/build/stage-helper.sh)"
)
needs_helper = _required(
    HELPER.is_file() and find_tessdata() is not None,
    f"the staged helper ({HELPER}; run tools/build/stage-helper.sh) and eng.traineddata",
)


def text_pdf(pages: Sequence[Sequence[str]], *, fontsize: float = 11.0) -> bytes:
    """A PDF with one page per entry; each string is one line of Helvetica,
    18 pt apart from the top-left margin."""
    b = PdfBuilder()
    for lines in pages:
        page = b.new_page()
        for i, line in enumerate(lines):
            page.insert_text((72, 72 + i * fontsize * 1.6), line, fontsize=fontsize)
    return b.to_bytes()


def sample_pdf(tmp_path: Path) -> Path:
    """A one-page file holding ``SAMPLE_LINE`` at 24 pt (sparse: 32 chars)."""
    path = tmp_path / "sample.pdf"
    path.write_bytes(text_pdf([[SAMPLE_LINE]], fontsize=24))
    return path


def small_page_pdf(path: Path, line: str) -> Path:
    """One small page (360 x 60 pt) holding ``line`` at 20 pt: quick to
    render and recognise, for stress tests."""
    b = PdfBuilder()
    b.new_page(360, 60).insert_text((10, 40), line, fontsize=20)
    b.save(path)
    return path


def child_env(**overrides: str | None) -> dict[str, str]:
    """This process's environment with ``overrides`` applied (``None``
    removes a variable) and coverage/GIL settings of the parent dropped."""
    env = dict(os.environ)
    env.pop("COV_CORE_SOURCE", None)
    for k, v in overrides.items():
        if v is None:
            env.pop(k, None)
        else:
            env[k] = v
    return env


def run_child(
    code: str,
    *args: str,
    env: Mapping[str, str] | None = None,
    timeout: float = 300,
) -> subprocess.CompletedProcess[str]:
    """Runs ``code`` (dedented) in a fresh interpreter with ``args`` as
    ``sys.argv[1:]``; fails the test with the child's stderr if it exits
    non-zero."""
    proc = subprocess.run(
        [sys.executable, "-c", textwrap.dedent(code), *args],
        env=dict(env) if env is not None else child_env(),
        cwd=REPO,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        check=False,
        timeout=timeout,
    )
    assert proc.returncode == 0, proc.stderr
    return proc
