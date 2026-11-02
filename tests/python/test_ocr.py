"""OCR from Python: OcrEngine, OcrInfo, Page.ocr and default_ocr_engine
(run 0002 §4.11; O8, O12, O13, O15, O21, O26-O28, O38; design spec §13.5's
OCR rows).

The error paths run without Tesseract through ``_native._scripted_engine``,
whose recognitions follow a script (the page is still rendered for real).
Model-directory resolution is process-global and the default engine is
built once per process, so those rows run in child interpreters with a
patched list of known directories (``_native._set_tessdata_candidates``).
Real recognition needs ``eng.traineddata``; process isolation also needs the
staged helper (``tools/build/stage-helper.sh``).
"""

from __future__ import annotations

import hashlib
import os
import pickle
import subprocess
import sys
import textwrap
from pathlib import Path
from unittest import mock

import pytest

import pdf_extract
from pdf_extract import _native

from ocrkit import (
    HELPER,
    SAMPLE_LINE,
    child_env,
    find_tessdata,
    needs_helper,
    needs_staged_helper,
    needs_tessdata,
    run_child,
    sample_pdf,
    text_pdf,
)
from pdfs import CORPUS_C, Fixtures

# --- arguments -------------------------------------------------------------


@pytest.mark.parametrize(
    ("kwargs", "match"),
    [
        ({"dpi": 0}, "dpi must be between 1 and 2400, got 0"),
        ({"dpi": 2401}, "dpi must be between 1 and 2400"),
        ({"dpi": -5}, "dpi"),
        ({"max_instances": 0}, "max_instances must be at least 1"),
        ({"page_timeout": 0.0}, "page_timeout"),
        ({"page_timeout": -1.0}, "page_timeout"),
        ({"page_timeout": float("nan")}, "page_timeout"),
        ({"page_timeout": float("inf")}, "page_timeout"),
        ({"max_render_pixels": 0}, "max_render_pixels must be at least 1"),
        ({"max_render_pixels": 2**70}, "max_render_pixels is too large"),
        # A raster must fit one helper frame (64 MiB) under process isolation.
        (
            {"max_render_pixels": 66_977_777, "isolation": "process"},
            "max_render_pixels must be at most 66977776 with isolation='process'",
        ),
        ({"psm": "automatic"}, "psm must be one of 'osd_only', "),
        ({"oem": "lstm"}, "oem must be one of"),
        ({"isolation": "thread"}, "isolation must be 'in_process' or 'process'"),
        ({"languages": "../eng"}, 'invalid language name "../eng"'),
        # Empty members: "+", "eng+", "+eng", "eng++fra" (run 0003 F19).
        ({"languages": "+"}, 'invalid language name ""'),
        ({"languages": "eng+"}, 'invalid language name ""'),
        ({"languages": "+eng"}, 'invalid language name ""'),
        ({"languages": "eng++fra"}, 'invalid language name ""'),
        ({"languages": ["eng", "de u"]}, 'invalid language name "de u"'),
        ({"languages": []}, "at least one language"),
    ],
)
def test_out_of_range_arguments_are_value_errors(kwargs: dict[str, object], match: str) -> None:
    # Argument errors are checked before any model is looked for (O8), so
    # these hold with or without Tesseract data.
    with pytest.raises(ValueError, match=match):
        pdf_extract.OcrEngine(**kwargs)  # type: ignore[arg-type]


def test_languages_of_the_wrong_type_are_type_errors() -> None:
    with pytest.raises(TypeError):
        pdf_extract.OcrEngine(languages=3)  # type: ignore[arg-type]


def test_legacy_engine_mode_is_unavailable_on_the_vendored_build() -> None:
    # D-4: the vendored build has no legacy engine; that is a property of the
    # build (OcrUnavailableError), not a bad argument.
    for oem in ("tesseract_only", "tesseract_lstm_combined"):
        with pytest.raises(pdf_extract.OcrUnavailableError) as info:
            pdf_extract.OcrEngine(oem=oem)
        assert info.value.reason == "init_failed"
        assert "legacy engine" in str(info.value)


def test_page_ocr_dpi_is_checked(tmp_path: Path) -> None:
    engine = _native._scripted_engine()
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    for bad in (0, 2401):
        with pytest.raises(ValueError, match="dpi must be between 1 and 2400"):
            page.ocr(engine, dpi=bad)


# --- §13.5 contract rows: model directory ------------------------------------

NO_TESSDATA = """
    import pickle, sys
    import pdf_extract
    from pdf_extract import _native

    known = sys.argv[1:]
    _native._set_tessdata_candidates(known)
    try:
        pdf_extract.OcrEngine(isolation="in_process")
    except pdf_extract.OcrUnavailableError as e:
        assert isinstance(e, RuntimeError)
        assert e.reason == "no_tessdata", e.reason
        assert e.searched == tuple(known), e.searched
        assert e.tessdata is None
        assert e.languages == ("eng",)
        assert "no directory holds eng.traineddata" in str(e)
        clone = pickle.loads(pickle.dumps(e))
        assert (type(clone), clone.args, clone.reason, clone.searched) == (
            type(e), e.args, e.reason, e.searched)
    else:
        raise AssertionError("OcrEngine() built without a model")

    # The default engine fails the same way, and the failure is stored: the
    # same error, every time, without searching again (O12).
    errors = []
    for _ in range(2):
        try:
            _native.default_ocr_engine()
        except pdf_extract.OcrUnavailableError as e:
            errors.append((e.args, e.reason, e.searched))
    assert len(errors) == 2 and errors[0] == errors[1], errors
    _native._set_tessdata_candidates(None)
    try:
        _native.default_ocr_engine()
    except pdf_extract.OcrUnavailableError as e:
        assert (e.args, e.reason, e.searched) == errors[0]

    # Page.ocr() without an engine uses the default one.
    from pdf_extract.testing import PdfBuilder
    b = PdfBuilder(); b.new_page().insert_text((72, 72), "x")
    page = pdf_extract.open_bytes(b.to_bytes())[0]
    try:
        page.ocr()
    except pdf_extract.OcrUnavailableError as e:
        assert e.reason == "no_tessdata"
    else:
        raise AssertionError("page.ocr() ran without a model")
    print("ok")
"""


@needs_staged_helper  # the child also builds the (process-isolated) default engine
def test_no_tessdata_anywhere_is_ocr_unavailable_and_a_runtime_error(tmp_path: Path) -> None:
    # TESSDATA_PREFIX unset and no known directory holds a model. The known
    # list is replaced by two empty directories so `searched` has something
    # to list.
    known = [tmp_path / "a", tmp_path / "b"]
    known[0].mkdir()
    out = run_child(NO_TESSDATA, *map(str, known), env=child_env(TESSDATA_PREFIX=None))
    assert out.stdout.strip() == "ok"


def test_no_tessdata_with_an_empty_known_list_searched_nothing() -> None:
    code = """
        import pdf_extract
        from pdf_extract import _native
        _native._set_tessdata_candidates([])
        try:
            pdf_extract.OcrEngine(isolation="in_process")
        except pdf_extract.OcrUnavailableError as e:
            assert e.reason == "no_tessdata" and e.searched == (), (e.reason, e.searched)
            print("ok")
    """
    assert run_child(code, env=child_env(TESSDATA_PREFIX=None)).stdout.strip() == "ok"


NO_TESSDATA_HINT = """
    import sys
    import pdf_extract
    from pdf_extract import _native

    explicit = sys.argv[1] or None
    _native._set_tessdata_candidates([])
    try:
        pdf_extract.OcrEngine(tessdata=explicit, isolation="in_process")
    except pdf_extract.OcrUnavailableError as e:
        assert e.reason == "no_tessdata", e.reason
        print(str(e))
    else:
        raise AssertionError("OcrEngine() built without a model")
"""


def test_the_no_tessdata_hint_names_what_was_tried(tmp_path: Path) -> None:
    # Run 0003 F22: the message keeps its prefix and names the setting or
    # the variable that was tried, instead of advising to set it.
    prefix = tmp_path / "prefix"
    prefix.mkdir()
    explicit = tmp_path / "explicit"
    explicit.mkdir()
    cases = [
        ("", None, ["; set the tessdata option or TESSDATA_PREFIX"]),
        ("", str(prefix), [f"; TESSDATA_PREFIX={prefix} and {prefix}/tessdata lack it"]),
        (str(explicit), None, [f"; the tessdata setting {explicit} lacks it"]),
    ]
    for arg, variable, wanted in cases:
        env = child_env(TESSDATA_PREFIX=variable)
        message = run_child(NO_TESSDATA_HINT, arg, env=env).stdout
        assert "no directory holds eng.traineddata" in message, message
        for text in wanted:
            assert text in message, (arg, variable, message)
        if arg or variable:
            assert "set the tessdata option" not in message, message


LANGUAGE_MISSING = """
    import os, sys
    import pdf_extract
    from pdf_extract import _native

    prefix = os.environ["TESSDATA_PREFIX"]
    _native._set_tessdata_candidates([])
    # The prefix holds a model for the first language asked for, so it is
    # the chosen directory, and eng is missing from it (O9).
    try:
        pdf_extract.OcrEngine(languages="deu+eng", isolation="in_process")
    except pdf_extract.OcrUnavailableError as e:
        assert isinstance(e, RuntimeError)
        assert e.reason == "language_data_missing", e.reason
        assert e.tessdata == prefix, e.tessdata
        assert e.languages == ("deu", "eng")
        assert "eng.traineddata is missing" in str(e), str(e)
    else:
        raise AssertionError("built without eng")
    # With eng alone, a prefix that lacks it is not chosen at all, and with
    # nothing else to search there is no model directory.
    try:
        pdf_extract.OcrEngine(isolation="in_process")
    except pdf_extract.OcrUnavailableError as e:
        assert e.reason == "no_tessdata", e.reason
        assert e.searched == (prefix, os.path.join(prefix, "tessdata")), e.searched
    print("ok")
"""


def test_tessdata_prefix_without_eng_is_ocr_unavailable(tmp_path: Path) -> None:
    # Internal survey §6.2: TESSDATA_PREFIX points at a model directory that
    # has no eng.traineddata.
    (tmp_path / "deu.traineddata").write_bytes(b"")
    out = run_child(LANGUAGE_MISSING, env=child_env(TESSDATA_PREFIX=str(tmp_path)))
    assert out.stdout.strip() == "ok"


def test_a_broken_model_fails_init(tmp_path: Path) -> None:
    (tmp_path / "eng.traineddata").write_bytes(b"")
    with pytest.raises(pdf_extract.OcrUnavailableError) as info:
        pdf_extract.OcrEngine(tessdata=tmp_path, isolation="in_process")
    assert info.value.reason == "init_failed"
    assert info.value.tessdata == str(tmp_path)


def test_a_missing_or_relative_helper_is_engine_not_built(tmp_path: Path) -> None:
    code = """
        import pdf_extract
        try:
            pdf_extract.OcrEngine(isolation="process")
        except pdf_extract.OcrUnavailableError as e:
            assert e.reason == "engine_not_built", e.reason
            print(str(e))
    """
    out = run_child(code, env=child_env(PDF_EXTRACT_OCR_HELPER="relative/helper"))
    assert "must be an absolute path" in out.stdout
    missing = str(tmp_path / "no-such-helper")
    out = run_child(code, env=child_env(PDF_EXTRACT_OCR_HELPER=missing))
    assert missing in out.stdout


# --- §13.5 contract rows: page failures (scripted) ---------------------------


def test_injected_recognition_failure_is_ocr_page_error(tmp_path: Path) -> None:
    engine = _native._scripted_engine([("recognition", "scripted failure")])
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    with pytest.raises(pdf_extract.OcrPageError) as info:
        page.ocr(engine)
    e = info.value
    assert type(e) is pdf_extract.OcrPageError
    assert not isinstance(e, RuntimeError)
    assert isinstance(e, pdf_extract.OcrError)
    assert str(e) == "recognition failed: scripted failure"
    assert (e.limit_name, e.value) == (None, None)


def test_injected_timeout_is_ocr_timeout_error(tmp_path: Path) -> None:
    engine = _native._scripted_engine([("timeout",)], page_timeout=0.5)
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    with pytest.raises(pdf_extract.OcrTimeoutError) as info:
        page.ocr(engine)
    e = info.value
    assert not isinstance(e, RuntimeError)
    assert isinstance(e, pdf_extract.OcrPageError)
    assert e.timeout == 0.5
    assert (e.limit_name, e.value) == (None, None)
    clone = pickle.loads(pickle.dumps(e))
    assert (type(clone), clone.args, clone.timeout) == (type(e), e.args, e.timeout)


def test_a_slow_backend_past_the_budget_is_a_timeout(tmp_path: Path) -> None:
    # The scripted backend sleeps past the deadline and then reports a
    # timeout, as Tesseract's monitor would.
    engine = _native._scripted_engine([("sleep", 0.3, ("timeout",))], page_timeout=0.2)
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    with pytest.raises(pdf_extract.OcrTimeoutError) as info:
        page.ocr(engine)
    assert info.value.timeout == pytest.approx(0.2)


def test_a_render_limit_is_ocr_page_error_with_the_limit(tmp_path: Path) -> None:
    path = tmp_path / "busy.pdf"
    path.write_bytes(text_pdf([[f"line {i}" for i in range(40)]]))
    doc = pdf_extract.open(path, limits=pdf_extract.Limits(max_ops_per_page=20))
    engine = _native._scripted_engine()
    with pytest.raises(pdf_extract.OcrPageError) as info:
        doc[0].ocr(engine)
    e = info.value
    assert type(e) is pdf_extract.OcrPageError
    assert (e.limit_name, e.value) == ("ops_per_page", 20)
    assert "ops_per_page" in str(e)
    # The page was not recognised (O3).
    assert _native._scripted_stats(engine)["calls"] == 0
    clone = pickle.loads(pickle.dumps(e))
    assert (clone.limit_name, clone.value) == ("ops_per_page", 20)


def test_injected_panic_is_internal_error(tmp_path: Path) -> None:
    engine = _native._scripted_engine([("panic", "scripted panic")])
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    with pytest.raises(pdf_extract.InternalError, match="scripted panic"):
        page.ocr(engine)
    # The engine survives: the next page is recognised.
    assert page.ocr(engine).text() == ""


def test_an_instance_that_cannot_start_is_ocr_unavailable(tmp_path: Path) -> None:
    engine = _native._scripted_engine(fail_creates=1)
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    with pytest.raises(pdf_extract.OcrUnavailableError) as info:
        page.ocr(engine)
    assert info.value.reason == "init_failed"
    assert page.ocr(engine).text() == ""


def test_a_closed_document_is_document_closed_error(tmp_path: Path) -> None:
    doc = pdf_extract.open(sample_pdf(tmp_path))
    page = doc[0]
    doc.close()
    with pytest.raises(pdf_extract.DocumentClosedError):
        page.ocr(_native._scripted_engine())


def test_scripted_text_has_the_text_path_shape(tmp_path: Path) -> None:
    engine = _native._scripted_engine(
        [("text", "first line\nsecond  line here"), ("empty",)], dpi=150
    )
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    tp = page.ocr(engine)
    assert tp.source == "ocr"
    assert tp.dpi == 150
    assert tp.text() == "first line\nsecond line here\n"
    assert tp.text(block_separator="blank_line") == "first line\nsecond line here\n"
    assert tp.warnings == ()
    # Nothing found: an empty string, not an error.
    assert page.ocr(engine).text() == ""
    assert page.ocr(engine, dpi=72).dpi == 72
    stats = _native._scripted_stats(engine)
    assert stats == {"calls": 3, "max_in_flight": 1, "creates": 1, "clears": 3, "drops": 0}


def test_scripted_engine_arguments() -> None:
    with pytest.raises(TypeError, match="a scripted step is"):
        _native._scripted_engine([("bogus",)])
    with pytest.raises(TypeError, match="a scripted step is"):
        _native._scripted_engine(["text"])  # type: ignore[list-item]
    with pytest.raises(TypeError, match="a scripted step is"):
        _native._scripted_engine([("text",)])
    with pytest.raises(ValueError, match="bad sleep duration"):
        _native._scripted_engine([("sleep", -1.0, ("empty",))])


def test_scripted_engine_getters_and_info() -> None:
    engine = _native._scripted_engine(dpi=200, page_timeout=1.5, max_instances=3)
    assert engine.tessdata == ""
    assert engine.languages == ("eng",)
    assert (engine.dpi, engine.max_instances, engine.page_timeout) == (200, 3, 1.5)
    assert (engine.psm, engine.oem, engine.isolation) == ("auto", "lstm_only", "in_process")
    assert engine.max_render_pixels == 40_000_000
    info = engine.info()
    assert (info.name, info.version, info.tessdata, info.model_sha256) == (
        "scripted",
        "0",
        None,
        {},
    )
    assert (info.linkage, info.isolation, info.languages) == ("vendored", "in_process", ("eng",))
    assert repr(engine) == (
        "<pdf_extract.OcrEngine languages=\"eng\" dpi=200 max_instances=3 isolation='in_process'>"
    )
    assert repr(info).startswith("<pdf_extract.OcrInfo name='scripted' version='0'")


def test_classes_are_final_frozen_and_patchable() -> None:
    for cls in (pdf_extract.OcrEngine, pdf_extract.OcrInfo):
        assert cls.__module__ == "pdf_extract"
        with pytest.raises(TypeError):

            class Sub(cls):  # type: ignore[misc, valid-type]
                pass

    engine = _native._scripted_engine()
    with pytest.raises(AttributeError):
        engine.dpi = 5  # type: ignore[misc]
    # Not immutable types: mock.patch.object works on them (0001 §3.5).
    with mock.patch.object(pdf_extract.Page, "ocr", return_value="patched") as patched:
        assert pdf_extract.Page.ocr(None) == "patched"  # type: ignore[arg-type, comparison-overlap]
    assert patched.call_count == 1


def test_hostile_files_through_page_ocr(fixtures: Fixtures) -> None:
    # Spec 0002 §2.3 item 1: the hostile fixtures rendered at 36 dpi from
    # Python, recognised by the scripted backend. Every outcome is a result
    # or a PdfExtractError; a panic would be InternalError, a crash would end
    # the run.
    engine = _native._scripted_engine(dpi=36)
    limits = pdf_extract.Limits(page_time_budget=2.0)
    paths = [p for p, g in fixtures.index() if g in {"hostile", "sample-hostile"}]
    paths += sorted(CORPUS_C.glob("*.pdf"))
    assert len(paths) > 20
    outcomes: dict[str, str] = {}
    for path in paths:
        try:
            doc = pdf_extract.open(path, limits=limits)
        except pdf_extract.OpenError as e:
            outcomes[path.name] = type(e).__name__
            continue
        for page in list(doc)[:3]:
            try:
                assert page.ocr(engine).source == "ocr"
                outcomes[path.name] = "ok"
            except pdf_extract.InternalError:
                raise
            except pdf_extract.PdfExtractError as e:
                outcomes[path.name] = type(e).__name__
    assert "ok" in outcomes.values()


def test_a_codec_memory_refusal_reaches_python_as_a_page_warning(fixtures: Fixtures) -> None:
    # Run 0003 H14: a JBIG2 image whose decoder would exceed its memory
    # budget is skipped with one "image: memory limit" warning; the page is
    # still recognised. The text path never decodes images, so it is clean.
    limits = pdf_extract.Limits(max_image_pixels=1_000_000)
    doc = pdf_extract.open(fixtures.path("jbig2_symbol_export"), limits=limits)
    engine = _native._scripted_engine([("text", "after the image")], dpi=36)
    tp = doc[0].ocr(engine)
    assert tp.source == "ocr"
    assert tp.text() == "after the image\n"
    assert len(tp.warnings) == 1, tp.warnings
    assert tp.warnings[0].startswith("image: memory limit: "), tp.warnings
    assert "max_image_pixels" in tp.warnings[0]
    assert doc[0].text_page().warnings == ()


def test_a_jpx_memory_refusal_reaches_python_as_a_page_warning(fixtures: Fixtures) -> None:
    # Run 0003 H15: the same for a JPEG 2000 image whose tile records alone
    # exceed its decoder's budget, here under the default limits.
    doc = pdf_extract.open(fixtures.path("jpx_tile_bomb"))
    engine = _native._scripted_engine([("text", "after the image")], dpi=36)
    tp = doc[0].ocr(engine)
    assert tp.source == "ocr"
    assert tp.text() == "after the image\n"
    assert len(tp.warnings) == 1, tp.warnings
    assert tp.warnings[0].startswith("image: memory limit: "), tp.warnings
    assert doc[0].text_page().warnings == ()


# --- real Tesseract -----------------------------------------------------------


@needs_helper
def test_page_ocr_with_the_default_engine(tmp_path: Path) -> None:
    page = pdf_extract.open(sample_pdf(tmp_path))[0]
    tp = page.ocr()
    assert tp.source == "ocr"
    assert tp.dpi == 300
    text = tp.text()
    assert text == SAMPLE_LINE + "\n"
    assert all(line.endswith("\n") for line in text.splitlines(keepends=True))
    assert "\n\n" not in text
    default = pdf_extract.default_ocr_engine()
    assert page.ocr(default).text() == text
    low = page.ocr(default, dpi=72)
    assert low.dpi == 72
    assert low.text() == text


@needs_helper
def test_default_engine_configuration_and_info() -> None:
    engine = pdf_extract.default_ocr_engine()
    tessdata = find_tessdata()
    assert tessdata is not None
    assert engine.tessdata == str(tessdata)
    assert engine.languages == ("eng",)
    assert engine.dpi == 300
    assert engine.max_instances == min(os.cpu_count() or 1, 4)
    assert engine.page_timeout == 60.0
    # Python's default engine is process-isolated (internal #53); Rust's is not.
    assert (engine.psm, engine.oem, engine.isolation) == ("auto", "lstm_only", "process")
    info = engine.info()
    assert info.name == "tesseract"
    assert info.version.startswith("5.")
    assert info.languages == ("eng",)
    assert info.tessdata == engine.tessdata
    digest = hashlib.sha256((tessdata / "eng.traineddata").read_bytes()).hexdigest()
    assert info.model_sha256 == {"eng": digest}
    assert info.linkage == "vendored"
    assert info.isolation == "process"
    assert repr(engine).startswith('<pdf_extract.OcrEngine languages="eng" dpi=300')
    assert repr(engine).endswith("isolation='process'>")
    with pytest.raises(ValueError, match="not a scripted engine"):
        _native._scripted_stats(engine)


@needs_tessdata
def test_engine_with_every_argument(tmp_path: Path) -> None:
    tessdata = find_tessdata()
    engine = pdf_extract.OcrEngine(
        languages=("eng",),
        tessdata=tessdata,
        dpi=150,
        max_instances=2,
        page_timeout=30.0,
        max_render_pixels=10_000_000,
        psm="single_block",
        oem="default",
        isolation="in_process",
    )
    assert (engine.dpi, engine.max_instances, engine.page_timeout) == (150, 2, 30.0)
    assert (engine.psm, engine.oem, engine.max_render_pixels) == ("single_block", "default", 10**7)
    tp = pdf_extract.open(sample_pdf(tmp_path))[0].ocr(engine)
    assert tp.dpi == 150
    assert tp.text() == SAMPLE_LINE + "\n"


@needs_helper
def test_an_empty_page_recognises_as_empty_string(tmp_path: Path) -> None:
    path = tmp_path / "blank.pdf"
    path.write_bytes(text_pdf([[]]))
    tp = pdf_extract.open(path)[0].ocr(pdf_extract.default_ocr_engine(), dpi=72)
    assert tp.text() == ""


# --- process isolation --------------------------------------------------------

PROCESS = """
    import os, sys
    import pdf_extract

    path = sys.argv[1]
    assert os.environ["PATH"] == ""
    page = pdf_extract.open(path)[0]
    engine = pdf_extract.OcrEngine(isolation="process", max_instances=1)
    assert engine.isolation == "process" and engine.info().isolation == "process"
    # Against in-process recognition, not another process engine (internal #53 made
    # OcrEngine() process-isolated).
    expected = page.ocr(pdf_extract.OcrEngine(isolation="in_process")).text()
    assert page.ocr(engine).text() == expected, page.ocr(engine).text()
    assert page.ocr(engine, dpi=72).text() == expected
    print("ok")
"""


@needs_helper
def test_process_isolation_end_to_end_with_an_empty_path(tmp_path: Path) -> None:
    assert HELPER.is_file()
    out = run_child(PROCESS, str(sample_pdf(tmp_path)), env=child_env(PATH=""))
    assert out.stdout.strip() == "ok"


ABORT = """
    import sys
    import pdf_extract

    page = pdf_extract.open(sys.argv[1])[0]
    engine = pdf_extract.OcrEngine(isolation="process", max_instances=1)
    for _ in range(2):  # the helper is respawned, and aborts again
        try:
            page.ocr(engine)
        except pdf_extract.OcrPageError as e:
            assert type(e) is pdf_extract.OcrPageError
            assert "helper exited: signal 6" in str(e), str(e)
        else:
            raise AssertionError("the injected abort did not fail the page")
    # The interpreter survived; write the verdict to a file, not fd 1.
    open(sys.argv[2], "w").write("ok")
"""


@needs_helper
def test_an_aborting_helper_fails_the_page_and_the_interpreter_survives(
    tmp_path: Path,
) -> None:
    verdict = tmp_path / "verdict"
    env = child_env(
        PDF_EXTRACT_OCR_HELPER_FAULT="abort-on-recognize",
        PDF_EXTRACT_OCR_HELPER_FAULT_ARMED="1",
    )
    proc = subprocess.run(
        [sys.executable, "-c", textwrap.dedent(ABORT), str(sample_pdf(tmp_path)), str(verdict)],
        env=env,
        capture_output=True,
        stdin=subprocess.DEVNULL,
        check=False,
        timeout=300,
    )
    assert proc.returncode == 0, proc.stderr
    assert verdict.read_text() == "ok"
    # Nothing reached fd 1 or 2 (O40): the helper's stderr is the null device.
    assert (proc.stdout, proc.stderr) == (b"", b"")
