"""Python's OCR isolation default (internal #53; ocr-process-isolation spec "In-process
is the default; process isolation is opt-in" as modified for Python, "The
Python package ships and finds its helper", "The Python raster-cap error
names the opt-out"; ocr-engine spec "Process-wide default engine").

``OcrEngine()``, ``default_ocr_engine()`` and ``Page.ocr()`` without an
engine run Tesseract in helper processes; ``isolation="in_process"`` opts
out. The default engine is the binding's own, with the helper looked up
beside the extension module, and a missing helper is a stored
``engine_not_built`` with no fallback to in-process OCR.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

import pdf_extract

from ocrkit import HELPER, SAMPLE_LINE, child_env, needs_helper, run_child, sample_pdf

PACKAGE = Path(pdf_extract._native.__file__).resolve().parent


@needs_helper
def test_ocr_engine_defaults_to_process_isolation() -> None:
    engine = pdf_extract.OcrEngine()
    assert engine.isolation == "process"
    assert engine.info().isolation == "process"


@needs_helper
def test_in_process_is_the_opt_out(tmp_path: Path) -> None:
    engine = pdf_extract.OcrEngine(isolation="in_process", dpi=72)
    assert engine.isolation == engine.info().isolation == "in_process"
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    assert page.ocr(engine).text() == SAMPLE_LINE + "\n"


@needs_helper
def test_the_default_engine_uses_the_helper_beside_the_package(tmp_path: Path) -> None:
    # No PATH and no PDF_EXTRACT_OCR_HELPER: only the package directory can
    # supply the helper (O28).
    out = run_child(
        """
        import sys
        import pdf_extract
        engine = pdf_extract.default_ocr_engine()
        assert engine.isolation == engine.info().isolation == "process"
        page = pdf_extract.open(sys.argv[1])[0]
        assert page.ocr().text() == sys.argv[2] + "\\n", page.ocr().text()
        print("ok")
        """,
        str(sample_pdf(tmp_path)),
        SAMPLE_LINE,
        env=child_env(PATH="", PDF_EXTRACT_OCR_HELPER=None),
    )
    assert out.stdout.strip() == "ok"


DEFAULT_ABORT = """
    import sys
    import pdf_extract

    page = pdf_extract.open(sys.argv[1])[0]
    for _ in range(2):  # the default engine respawns its helper, which aborts again
        try:
            page.ocr()
        except pdf_extract.OcrPageError as e:
            assert type(e) is pdf_extract.OcrPageError
            assert "helper exited: signal 6" in str(e), str(e)
        else:
            raise AssertionError("the injected abort did not fail the page")
    open(sys.argv[2], "w").write("ok")
"""


@needs_helper
def test_an_abort_under_the_default_engine_is_contained(tmp_path: Path) -> None:
    verdict = tmp_path / "verdict"
    proc = subprocess.run(
        [
            sys.executable,
            "-c",
            textwrap.dedent(DEFAULT_ABORT),
            str(sample_pdf(tmp_path)),
            str(verdict),
        ],
        env=child_env(
            PDF_EXTRACT_OCR_HELPER_FAULT="abort-on-recognize",
            PDF_EXTRACT_OCR_HELPER_FAULT_ARMED="1",
        ),
        capture_output=True,
        stdin=subprocess.DEVNULL,
        check=False,
        timeout=300,
    )
    assert proc.returncode == 0, proc.stderr
    assert verdict.read_text() == "ok"


NO_HELPER = """
    import shutil, sys
    sys.path.insert(0, sys.argv[1])
    import pdf_extract
    from pathlib import Path

    package = Path(pdf_extract._native.__file__).resolve().parent
    assert package == Path(sys.argv[1]).resolve() / "pdf_extract", package
    seen = []
    for attempt in range(2):
        try:
            pdf_extract.default_ocr_engine()
        except pdf_extract.OcrUnavailableError as e:
            assert e.reason == "engine_not_built", e.reason
            assert "stage-helper.sh" in str(e), str(e)
            assert "isolation='in_process'" in str(e), str(e)
            seen.append(e.args)
        else:
            raise AssertionError("built a process engine without a helper")
        if attempt == 0:
            # The helper appears after the first failure: the stored error
            # must still be returned (O12), never a new search.
            shutil.copy2(sys.argv[2], package / "pdf-extract-ocr-helper")
    assert seen[0] == seen[1], seen
    # Page.ocr() without an engine raises the same stored error; no
    # fallback to in-process OCR.
    from pdf_extract.testing import PdfBuilder
    b = PdfBuilder(); b.new_page().insert_text((72, 72), "x")
    try:
        pdf_extract.open_bytes(b.to_bytes())[0].ocr()
    except pdf_extract.OcrUnavailableError as e:
        assert e.args == seen[0]
    else:
        raise AssertionError("page.ocr() fell back to in-process OCR")
    print("ok")
"""


@needs_helper
def test_a_missing_helper_is_a_stored_engine_not_built(tmp_path: Path) -> None:
    # A copy of the installed package without its helper, imported first.
    copy = tmp_path / "site"
    shutil.copytree(
        PACKAGE, copy / "pdf_extract", ignore=shutil.ignore_patterns(HELPER.name, "__pycache__")
    )
    out = run_child(
        NO_HELPER,
        str(copy),
        str(HELPER),
        env=child_env(PDF_EXTRACT_OCR_HELPER=None),
    )
    assert out.stdout.strip() == "ok"


HINTS = """
    import os, sys
    sys.path.insert(0, sys.argv[1])
    import pdf_extract

    def failure(**env):
        for k, v in env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        try:
            pdf_extract.OcrEngine(isolation="process")
        except pdf_extract.OcrUnavailableError as e:
            assert e.reason == "engine_not_built", e.reason
            return str(e)
        raise AssertionError("built a process engine without a helper")

    # The constructor gets the same hint as the default engine.
    assert "stage-helper.sh" in failure(PDF_EXTRACT_OCR_HELPER=None)
    # An empty override is unset, for discovery and for the hint alike.
    assert "stage-helper.sh" in failure(PDF_EXTRACT_OCR_HELPER="")
    # An explicit override chose another file: staging is not the cause.
    missing = os.path.join(sys.argv[1], "elsewhere", "pdf-extract-ocr-helper")
    message = failure(PDF_EXTRACT_OCR_HELPER=missing)
    assert "stage-helper.sh" not in message, message
    assert missing in message, message
    print("ok")
"""


@needs_helper
def test_the_stage_hint_appears_only_when_staging_is_the_cause(tmp_path: Path) -> None:
    copy = tmp_path / "site"
    shutil.copytree(
        PACKAGE, copy / "pdf_extract", ignore=shutil.ignore_patterns(HELPER.name, "__pycache__")
    )
    out = run_child(HINTS, str(copy), env=child_env(PDF_EXTRACT_OCR_HELPER=None))
    assert out.stdout.strip() == "ok"


def test_the_raster_cap_error_names_the_opt_out() -> None:
    with pytest.raises(ValueError, match=r"66977776") as info:
        pdf_extract.OcrEngine(max_render_pixels=80_000_000)
    assert "isolation='in_process'" in str(info.value)


@needs_helper
def test_a_larger_raster_cap_is_accepted_in_process() -> None:
    engine = pdf_extract.OcrEngine(max_render_pixels=80_000_000, isolation="in_process")
    assert engine.max_render_pixels == 80_000_000
