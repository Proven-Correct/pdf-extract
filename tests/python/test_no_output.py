"""Nothing reaches file descriptors 1 or 2 (run spec T48; §4.11 item 7;
run 0002 O40; design spec §13.5).

A child interpreter runs with fd 1 and fd 2 redirected to files, at the
file-descriptor level, so output written by native code (Rust's panic hook,
a stray ``eprintln!``, hayro's logging if it were ever enabled) is caught as
well as Python-level prints. The child imports the package and runs open and
extract over every generated fixture (hostile, contract, page-tree, builder
samples), the encrypted fixtures, every error row, the panic hook and a
codec panic caught at the image-codec boundary (internal #76), then
records how many operations it ran in a report file; the test checks that
both captured files are empty and that the report is complete.

A second child does the same for OCR (run 0002 O40): a failing ``Init``
(where Tesseract would print ``Error opening data file``), a successful
recognition in process and through a helper, ``extract`` with OCR, and a
helper that aborts and is restarted.
"""

from __future__ import annotations

import json
import subprocess
import sys
import textwrap
from pathlib import Path

from ocrkit import needs_helper, sample_pdf
from pdfs import CORPUS_C, Fixtures, ccitt_image_pdf

CHILD = textwrap.dedent(
    r"""
    import json, sys
    from pathlib import Path

    import pdf_extract
    from pdf_extract import _native

    fixture_root, corpus_c, scratch, report_path = map(Path, sys.argv[1:5])
    outcomes = {}

    def record(label, fn):
        try:
            fn()
            outcomes[label] = "ok"
        except Exception as e:
            outcomes[label] = type(e).__name__

    def extract_all(path, **kwargs):
        doc = pdf_extract.open(path, **kwargs)
        doc.warnings
        for page in doc:
            try:
                tp = page.text_page()
                tp.text()
                tp.text(block_separator="blank_line")
                tp.warnings
            except pdf_extract.LimitExceededError:
                pass
        doc.close()

    for line in (fixture_root / "index.tsv").read_text().splitlines():
        rel, group = line.split("\t")
        if group == "big":
            # 300,000 pages: open and count only.
            record(rel, lambda: pdf_extract.open(fixture_root / rel).page_count)
        elif group == "slow":
            limits = pdf_extract.Limits(page_time_budget=0.2)
            record(rel, lambda: extract_all(fixture_root / rel, limits=limits))
        else:
            record(rel, lambda: extract_all(fixture_root / rel))
    # Prefixed: corpus-C names (e.g. form_self_recursion.pdf) repeat
    # generated fixture names, and the report is keyed by label.
    for path in sorted(corpus_c.glob("*.pdf")):
        record("corpus-c/" + path.name, lambda: extract_all(path))

    # Error rows.
    empty = scratch / "empty.pdf"
    empty.write_bytes(b"")
    valid = fixture_root / "valid.pdf"
    record("missing", lambda: pdf_extract.open(scratch / "missing.pdf"))
    record("directory", lambda: pdf_extract.open(scratch))
    record("empty", lambda: pdf_extract.open(empty))
    record("empty_bytes", lambda: pdf_extract.open_bytes(b""))
    tiny = pdf_extract.Limits(max_file_size=10)
    record("too_large", lambda: pdf_extract.open(valid, limits=tiny))
    record("too_many_pages", lambda: pdf_extract.open(valid, max_pages=1))
    record("objects", lambda: pdf_extract.open(valid, limits=pdf_extract.Limits(max_objects=1)))
    record("bad_limits", lambda: pdf_extract.Limits(max_objects=-1))
    record("bad_option", lambda: pdf_extract.open(valid)[0].text(unmapped="x"))
    record("index", lambda: pdf_extract.open(valid)[99])
    record("index_type", lambda: pdf_extract.open(valid)["a"])
    def closed():
        doc = pdf_extract.open(valid)
        doc.close()
        doc[0]
    record("closed", closed)
    record("ops", lambda: pdf_extract.open(
        fixture_root / "operator_flood_400000.pdf",
        limits=pdf_extract.Limits(max_ops_per_page=1000),
    )[0].text())
    record("panic", _native._panic_for_test)
    record("panic_detached", lambda: _native._panic_for_test(detached=True))

    # A codec panic caught at the image-codec boundary (internal #76): silent, a
    # warning instead of InternalError, and a later panic's InternalError
    # carries that panic's own message.
    codec_page = pdf_extract.open(scratch / "codec.pdf")[0]
    scripted = _native._scripted_engine(default=("text", "scripted"))
    def codec_panic():
        _native._arm_codec_panic_for_test()
        warnings = codec_page.ocr(scripted).warnings
        if not any(w.startswith("image: decode failed") for w in warnings):
            raise AssertionError(warnings)
    record("codec_panic", codec_panic)
    def panic_after_codec():
        # Resumed: skips the hook, so a stale recorded message would show.
        try:
            _native._panic_for_test(resumed=True)
        except pdf_extract.InternalError as e:
            if "deliberate panic for testing" not in str(e) or "codec" in str(e):
                raise AssertionError(str(e))
            return
        raise AssertionError("no InternalError")
    record("panic_after_codec", panic_after_codec)

    report_path.write_text(json.dumps(outcomes))
    """
)


def test_nothing_is_written_to_fd_1_or_2(tmp_path: Path, fixtures: Fixtures) -> None:
    script = tmp_path / "child.py"
    script.write_text(CHILD)
    report = tmp_path / "report.json"
    scratch = tmp_path / "scratch"
    scratch.mkdir()
    (scratch / "codec.pdf").write_bytes(ccitt_image_pdf())
    out_path, err_path = tmp_path / "fd1.txt", tmp_path / "fd2.txt"
    with out_path.open("wb") as out, err_path.open("wb") as err:
        proc = subprocess.run(
            [
                sys.executable,
                str(script),
                str(fixtures.root),
                str(CORPUS_C),
                str(scratch),
                str(report),
            ],
            stdout=out,
            stderr=err,
            stdin=subprocess.DEVNULL,
            check=False,
            timeout=600,
        )
    stdout, stderr = out_path.read_bytes(), err_path.read_bytes()
    assert proc.returncode == 0, stderr.decode(errors="replace")
    assert stdout == b""
    assert stderr == b""

    outcomes: dict[str, str] = json.loads(report.read_text())
    fixture_count = len(fixtures.index())
    assert len(outcomes) == fixture_count + len(list(CORPUS_C.glob("*.pdf"))) + 17
    expected_errors = {
        "missing": "DocumentNotFoundError",
        "directory": "OpenError",
        "empty": "EmptyFileError",
        "empty_bytes": "EmptyFileError",
        "too_large": "FileTooLargeError",
        "too_many_pages": "TooManyPagesError",
        "objects": "CorruptDocumentError",
        "bad_limits": "ValueError",
        "bad_option": "ValueError",
        "index": "IndexError",
        "index_type": "TypeError",
        "closed": "DocumentClosedError",
        "ops": "LimitExceededError",
        "panic": "InternalError",
        "panic_detached": "InternalError",
        "codec_panic": "ok",
        "panic_after_codec": "ok",
        "corpus-c/enc_aes_256_user.pdf": "PasswordRequiredError",
        "garbage_after_header.pdf": "CorruptDocumentError",
        "plain_text.pdf": "NotAPdfError",
        "header_at_1020.pdf": "NotAPdfError",
    }
    for label, expected in expected_errors.items():
        assert outcomes[label] == expected, (label, outcomes[label])
    # Every hostile fixture opened and extracted (or ran into a budget).
    for path, group in fixtures.index():
        rel = path.relative_to(fixtures.root).as_posix()
        if group in {
            "hostile",
            "slow",
            "big",
            "tree",
            "text",
            "ocr",
            "sample-builder",
            "sample-hostile",
        }:
            assert outcomes[rel] == "ok", (rel, outcomes[rel])


OCR_CHILD = textwrap.dedent(
    r"""
    import json, os, sys
    from pathlib import Path

    import pdf_extract

    sample, scratch, report_path = map(Path, sys.argv[1:4])
    outcomes = {}

    def record(label, fn):
        try:
            outcomes[label] = fn()
        except Exception as e:
            outcomes[label] = type(e).__name__

    broken = scratch / "broken"
    broken.mkdir()
    (broken / "eng.traineddata").write_bytes(b"")
    record("failing_init", lambda: pdf_extract.OcrEngine(tessdata=broken))
    record("missing_language", lambda: pdf_extract.OcrEngine(languages="eng+zzz"))
    page = pdf_extract.open(sample)[0]
    in_process = pdf_extract.OcrEngine(isolation="in_process")
    record("in_process", lambda: page.ocr(in_process).text())
    # The default engine (process-isolated, internal #53).
    record("default", lambda: page.ocr().text())
    record("extract", lambda: pdf_extract.extract(
        sample, ocr=pdf_extract.default_ocr_engine()).pages[0].source)
    helper = pdf_extract.OcrEngine(isolation="process", max_instances=1)
    record("process", lambda: page.ocr(helper).text())
    # Arm the helper's fault hook: helpers spawned from now on abort on
    # their first page, and each failure respawns one.
    os.environ["PDF_EXTRACT_OCR_HELPER_FAULT"] = "abort-on-recognize"
    os.environ["PDF_EXTRACT_OCR_HELPER_FAULT_ARMED"] = "1"
    aborting = pdf_extract.OcrEngine(isolation="process", max_instances=1)
    record("abort", lambda: page.ocr(aborting).text())
    record("abort_again", lambda: page.ocr(aborting).text())
    report_path.write_text(json.dumps({k: str(v) for k, v in outcomes.items()}))
    """
)


@needs_helper
def test_ocr_writes_nothing_to_fd_1_or_2(tmp_path: Path) -> None:
    script = tmp_path / "ocr_child.py"
    script.write_text(OCR_CHILD)
    report = tmp_path / "report.json"
    scratch = tmp_path / "scratch"
    scratch.mkdir()
    out_path, err_path = tmp_path / "fd1.txt", tmp_path / "fd2.txt"
    with out_path.open("wb") as out, err_path.open("wb") as err:
        proc = subprocess.run(
            [sys.executable, str(script), str(sample_pdf(tmp_path)), str(scratch), str(report)],
            stdout=out,
            stderr=err,
            stdin=subprocess.DEVNULL,
            check=False,
            timeout=600,
        )
    stdout, stderr = out_path.read_bytes(), err_path.read_bytes()
    assert proc.returncode == 0, stderr.decode(errors="replace")
    assert stdout == b""
    assert stderr == b""
    outcomes: dict[str, str] = json.loads(report.read_text())
    text = "Hello OCR world from pdf extract\n"
    assert outcomes == {
        "failing_init": "OcrUnavailableError",
        "missing_language": "OcrUnavailableError",
        "in_process": text,
        "default": text,
        "extract": "ocr",
        "process": text,
        "abort": "OcrPageError",
        "abort_again": "OcrPageError",
    }
