"""``pdf_extract.extract`` (run 0002 §4.11 item 4; O22-O25; design spec
§12.4): unit 10's scenarios through the scripted engine, the shared
sparse-rule table, the result types and the thread rules.

The test document has 40 pages; every page holds one line of more than 50
characters except pages 3, 7 and 12, which hold only ``pN`` and so are
sparse. Scripted steps are consumed one per recognition, in the order the
pages are recognised.

The OS-thread assertions of the thread-rule tests read ``/proc`` and so run
on Linux only; elsewhere (macOS, the usual dev machine) those tests check the
Python-level thread count and the scripted backend's in-flight count alone,
and the OS-level bound (internal #55 C4) is exercised by CI's ubuntu jobs. There is no
macOS port, deliberately.
"""

from __future__ import annotations

import json
import os
import sys
import threading
import time
from collections.abc import Callable
from pathlib import Path

import pytest

import pdf_extract
from pdf_extract import _native
from pdf_extract.testing import PdfBuilder

from ocrkit import SAMPLE_LINE, needs_helper, sample_pdf, text_pdf
from pdfs import REPO, assemble_pdf

SPARSE = (3, 7, 12)
DENSE = "The quick brown fox jumps over the lazy dog, page {i:02d}, twice."


@pytest.fixture(scope="module")
def doc40(tmp_path_factory: pytest.TempPathFactory) -> Path:
    pages = [[f"p{i}"] if i in SPARSE else [DENSE.format(i=i)] for i in range(40)]
    path = tmp_path_factory.mktemp("extract") / "forty.pdf"
    path.write_bytes(text_pdf(pages))
    return path


def test_text_pass_matches_page_text_for_any_worker_count(doc40: Path) -> None:
    one = pdf_extract.extract(doc40)
    four = pdf_extract.extract(doc40, workers=4)
    doc = pdf_extract.open(doc40)
    assert one.page_count == four.page_count == doc.page_count == len(one.pages) == 40
    assert isinstance(one.pages, tuple)
    assert one.pages is one.pages  # built once
    for a, b, page in zip(one.pages, four.pages, doc, strict=True):
        assert (a.index, a.text, a.source, a.ocr_error) == (b.index, b.text, b.source, b.ocr_error)
        assert a.index == page.index
        assert a.text == page.text()
        assert a.source == "text"
        assert a.ocr_error is None
    assert one.ocr_status == four.ocr_status == "not_needed"
    assert one.warnings == ()


def test_sparse_pages_beyond_the_budget_are_never_attempted(doc40: Path) -> None:
    engine = _native._scripted_engine([("text", "first"), ("text", "second")])
    r = pdf_extract.extract(doc40, ocr=engine, max_ocr_pages=2)
    assert [(p.index, p.source, p.text) for p in r.pages if p.source != "text"] == [
        (3, "ocr", "first\n"),
        (7, "ocr", "second\n"),
    ]
    assert (r.pages[12].source, r.pages[12].ocr_error, r.pages[12].text) == ("text", None, "p12\n")
    assert r.ocr_status == "budget_exhausted"
    assert _native._scripted_stats(engine)["calls"] == 2


def test_with_four_workers_the_same_pages_are_recognised(doc40: Path) -> None:
    engine = _native._scripted_engine(default=("text", "recognised"), max_instances=4)
    r = pdf_extract.extract(doc40, ocr=engine, max_ocr_pages=2, workers=4)
    assert {p.index for p in r.pages if p.source == "ocr"} == {3, 7}
    assert r.pages[12].source == "text"
    assert r.ocr_status == "budget_exhausted"
    assert _native._scripted_stats(engine)["calls"] == 2


def test_a_failed_page_falls_back_to_its_text_layer(doc40: Path) -> None:
    engine = _native._scripted_engine(
        [("text", "three"), ("recognition", "scripted failure"), ("text", "twelve")]
    )
    r = pdf_extract.extract(doc40, ocr=engine)
    p7 = r.pages[7]
    assert (p7.source, p7.text) == ("text_fallback", "p7\n")
    assert p7.ocr_error == "recognition failed: scripted failure"
    assert (r.pages[3].text, r.pages[12].text) == ("three\n", "twelve\n")
    assert r.ocr_status == "partial"


def test_an_empty_result_falls_back_without_an_error(doc40: Path) -> None:
    r = pdf_extract.extract(doc40, ocr=_native._scripted_engine())
    for i in SPARSE:
        assert (r.pages[i].source, r.pages[i].ocr_error, r.pages[i].text) == (
            "text_fallback",
            None,
            f"p{i}\n",
        )
    assert r.ocr_status == "ok"


def test_unavailable_ocr_stops_after_the_first_failure(doc40: Path) -> None:
    engine = _native._scripted_engine(fail_creates=1)
    r = pdf_extract.extract(doc40, ocr=engine)
    messages = {r.pages[i].ocr_error for i in SPARSE}
    assert len(messages) == 1
    message = messages.pop()
    assert message is not None
    assert "scripted creation failure" in message
    assert all(r.pages[i].source == "text_fallback" for i in SPARSE)
    assert r.ocr_status == "unavailable"
    assert _native._scripted_stats(engine)["calls"] == 0


def test_timeouts_are_page_failures(doc40: Path) -> None:
    engine = _native._scripted_engine(default=("timeout",), page_timeout=0.5)
    r = pdf_extract.extract(doc40, ocr=engine, max_ocr_pages=1)
    assert r.pages[3].ocr_error == "OCR exceeded the page timeout of 500ms"
    # BudgetExhausted outranks Partial (O23).
    assert r.ocr_status == "budget_exhausted"


@pytest.mark.parametrize("workers", [1, 4])
def test_a_panic_in_a_worker_is_internal_error(doc40: Path, workers: int) -> None:
    engine = _native._scripted_engine(default=("panic", "scripted worker panic"), max_instances=4)
    with pytest.raises(pdf_extract.InternalError, match="scripted worker panic"):
        pdf_extract.extract(doc40, ocr=engine, workers=workers)


def test_a_text_pass_limit_yields_an_empty_page_and_a_warning(tmp_path: Path) -> None:
    path = tmp_path / "busy.pdf"
    path.write_bytes(text_pdf([[DENSE.format(i=0)], [f"line {i}" for i in range(40)]]))
    r = pdf_extract.extract(path, limits=pdf_extract.Limits(max_ops_per_page=50))
    assert (r.pages[1].text, r.pages[1].source, r.pages[1].ocr_error) == ("", "text", None)
    assert r.pages[0].text == DENSE.format(i=0) + "\n"
    assert len(r.warnings) == 1
    assert r.warnings[0].startswith("page 1: limit ops_per_page exceeded")


def test_options_are_passed_through(doc40: Path, tmp_path: Path) -> None:
    # min_chars=0: nothing is sparse, so the engine is never asked.
    engine = _native._scripted_engine()
    r = pdf_extract.extract(doc40, ocr=engine, min_chars=0)
    assert r.ocr_status == "not_needed"
    assert _native._scripted_stats(engine)["calls"] == 0
    # max_ocr_pages=0: every sparse page is beyond the budget.
    r = pdf_extract.extract(doc40, ocr=engine, max_ocr_pages=0)
    assert r.ocr_status == "budget_exhausted"
    with pytest.raises(pdf_extract.TooManyPagesError):
        pdf_extract.extract(doc40, max_pages=10)
    with pytest.raises(pdf_extract.DocumentNotFoundError):
        pdf_extract.extract(tmp_path / "missing.pdf")
    two_blocks = tmp_path / "blocks.pdf"
    b = PdfBuilder()
    page = b.new_page()
    page.insert_text((72, 72), "first block")
    page.insert_text((72, 400), "second block")
    b.save(two_blocks)
    blank = pdf_extract.extract(two_blocks, block_separator="blank_line")
    assert blank.pages[0].text == pdf_extract.open(two_blocks)[0].text(block_separator="blank_line")
    assert "\n\n" in blank.pages[0].text


@pytest.mark.parametrize(
    ("kwargs", "match"),
    [
        ({"workers": 0}, "workers must be at least 1"),
        ({"min_chars": -1}, "min_chars must not be negative"),
        ({"max_ocr_pages": -1}, "max_ocr_pages must not be negative"),
        ({"block_separator": "none"}, "block_separator must be"),
        ({"max_pages": -1}, "max_pages must not be negative"),
    ],
)
def test_bad_arguments_are_value_errors(doc40: Path, kwargs: dict[str, object], match: str) -> None:
    with pytest.raises(ValueError, match=match):
        pdf_extract.extract(doc40, **kwargs)  # type: ignore[arg-type]


def test_reprs(doc40: Path) -> None:
    r = pdf_extract.extract(doc40)
    assert repr(r) == "<pdf_extract.Extraction page_count=40 ocr_status='not_needed' warnings=0>"
    assert repr(r.pages[3]) == "<pdf_extract.PageResult index=3 source='text' chars=3>"
    for cls in (pdf_extract.Extraction, pdf_extract.PageResult):
        assert cls.__module__ == "pdf_extract"
    with pytest.raises(AttributeError):
        r.pages[0].text = "x"  # type: ignore[misc]


def test_sparse_rule_matches_the_shared_table() -> None:
    # The same 12 rows crates/core/src/extract_tests.rs asserts against the
    # Rust rule (spec 4.10).
    table = json.loads(
        (REPO / "crates" / "testkit" / "tests" / "data" / "sparse_rule.json").read_text()
    )
    rows = table["rows"]
    assert len(rows) == 12
    for row in rows:
        assert (len(row["text"].strip()) < row["min_chars"]) is row["sparse"], row["name"]


def thread_count() -> int | None:
    """The process's OS thread count (Linux only)."""
    status = Path("/proc/self/status")
    if not status.exists():
        return None
    for line in status.read_text().splitlines():
        if line.startswith("Threads:"):
            return int(line.split()[1])
    return None


def settled_thread_count() -> int | None:
    """``thread_count()`` once it stops changing (10 ms apart, up to ~1 s):
    a thread joined by an earlier test can stay listed in ``/proc`` briefly
    after the join returns (and on 3.11 ``join`` returns before the OS
    thread has exited)."""
    last = thread_count()
    for _ in range(100):
        time.sleep(0.01)
        now = thread_count()
        if now == last:
            return now
        last = now
    return last


def eventually(check: Callable[[], bool]) -> bool:
    """Whether ``check()`` holds within about a second (polled every 10 ms),
    for counts read right after threads were joined."""
    for _ in range(100):
        if check():
            return True
        time.sleep(0.01)
    return check()


def thread_ids() -> frozenset[int] | None:
    """The process's OS thread ids (Linux only)."""
    try:
        return frozenset(int(t) for t in os.listdir("/proc/self/task"))
    except FileNotFoundError:
        return None


class ThreadSampler:
    """One thread that records the process's thread ids continuously from
    ``start()`` until ``stop()``.

    It is the only thread this harness adds, and its first sample is taken
    after it is running, so that sample is the idle set (the process's
    threads plus the sampler) to which samples during the call compare.
    Only the idle set, the union of everything seen and the largest
    per-sample excess are kept: a pass is sampled thousands of times, and
    those are all the assertions need.
    """

    def __init__(self) -> None:
        self.idle: frozenset[int] | None = None
        self._seen: frozenset[int] = frozenset()
        self._peak_extra = 0
        self._first = threading.Event()
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run)

    def _run(self) -> None:
        while True:
            ids = thread_ids()
            if ids is not None:
                if self.idle is None:
                    self.idle = ids
                self._seen |= ids
                self._peak_extra = max(self._peak_extra, len(ids - self.idle))
            self._first.set()
            if self._stop.is_set():
                return
            # No fixed period: a short pass must not fall between samples.
            # sleep(0) only yields: the GIL on a regular build (the call
            # under test runs detached, so it is not slowed by this loop
            # holding it), the CPU on a free-threaded one, where the loop
            # busy-spins on its own core.
            time.sleep(0)

    def start(self) -> None:
        self._thread.start()
        self._first.wait()

    def stop(self) -> None:
        self._stop.set()
        self._thread.join()

    def peak_extra(self) -> int:
        """The most threads listed at once beyond the idle set."""
        return self._peak_extra

    def spawned(self) -> int:
        """Distinct threads seen during the call that were not there idle."""
        assert self.idle is not None
        return len(self._seen - self.idle)


def slow_pages_pdf(pages: int) -> bytes:
    """``pages`` A4 pages sharing one content stream of 500,000 no-op
    operators (``q Q``) and no text: each page takes milliseconds even in a
    release build, so a text pass over them lasts long enough for the
    sampler to see its workers (the fixture of
    crates/core/tests/extract_threads.rs, with more operators per page).
    """
    content = b"q Q " * 250_000
    kids = " ".join(f"{4 + i} 0 R" for i in range(pages))
    bodies: list[str | bytes] = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        f"<< /Type /Pages /Kids [{kids}] /Count {pages} >>",
        b"<< /Length %d >>\nstream\n" % len(content) + content + b"\nendstream",
    ]
    bodies += ["<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 3 0 R >>"] * pages
    return assemble_pdf(bodies)


@pytest.fixture(scope="module")
def slow40(tmp_path_factory: pytest.TempPathFactory) -> Path:
    path = tmp_path_factory.mktemp("slow") / "slow40.pdf"
    path.write_bytes(slow_pages_pdf(40))
    return path


@pytest.fixture(scope="module")
def sparse12(tmp_path_factory: pytest.TempPathFactory) -> Path:
    """12 pages, every one sparse, so the OCR pass has work for every
    worker."""
    path = tmp_path_factory.mktemp("sparse") / "sparse12.pdf"
    path.write_bytes(text_pdf([[f"p{i}"] for i in range(12)]))
    return path


@pytest.mark.parametrize("ocr", [False, True], ids=["text", "ocr"])
@pytest.mark.parametrize("workers", [1, 4])
def test_worker_threads(slow40: Path, sparse12: Path, workers: int, ocr: bool) -> None:
    # extract() runs one pass on `workers - 1` scoped threads plus the
    # caller, then (with OCR) a second pass the same way. Within a pass all
    # threads are spawned before any is joined, so a joined thread still
    # listed in /proc cannot inflate that pass's peak. Across passes it can:
    # text-pass threads may linger while the OCR pass spawns its own (the
    # Rust test in crates/core/tests/extract_threads.rs makes the same
    # split). So the /proc peak is bounded on the text pass alone (slow
    # pages, so the pass lasts long enough for the sampler to see its
    # workers), and the OCR pass is bounded directly by the scripted backend's count of
    # concurrent recognitions, which holds on every platform. Its pool
    # (max_instances=8) is larger than any worker count here, so the pool
    # is not what limits it, and all 12 pages are sparse, so every worker
    # has pages to take.
    engine = _native._scripted_engine(default=("sleep", 0.02, ("empty",)), max_instances=8, dpi=36)
    before_py = threading.active_count()
    baseline = settled_thread_count()
    sampler = ThreadSampler()
    sampler.start()
    try:
        if ocr:
            r = pdf_extract.extract(sparse12, ocr=engine, workers=workers)
            assert r.ocr_status == "ok"
        else:
            r = pdf_extract.extract(slow40, workers=workers)
            assert r.page_count == 40
    finally:
        sampler.stop()
    if ocr:
        stats = _native._scripted_stats(engine)
        assert stats["calls"] == 12
        if workers == 1:
            assert stats["max_in_flight"] == 1, stats
        else:
            assert 1 < stats["max_in_flight"] <= workers, stats
    # Nothing is left behind once the joined threads have gone; the counts
    # are polled because they may lag the joins by a moment.
    assert eventually(lambda: threading.active_count() == before_py), threading.enumerate()
    if baseline is None:
        return  # no /proc: the Python-level count is all there is
    assert eventually(lambda: thread_count() == baseline), (thread_count(), baseline)
    # The sampler is the one thread added before the call.
    assert sampler.idle is not None
    assert len(sampler.idle) == baseline + 1, sampler.idle
    if workers == 1:
        assert sampler.spawned() == 0, sampler.spawned()
        return
    passes = 2 if ocr else 1
    assert sampler.peak_extra() > 0, sampler.peak_extra()
    assert sampler.spawned() <= passes * (workers - 1), sampler.spawned()
    if not ocr:
        assert sampler.peak_extra() <= workers - 1, sampler.peak_extra()


@needs_helper
def test_extract_with_tesseract(tmp_path: Path) -> None:
    engine = pdf_extract.default_ocr_engine()
    r = pdf_extract.extract(sample_pdf(tmp_path), ocr=engine)
    (page,) = r.pages
    assert (page.source, page.text, page.ocr_error) == ("ocr", SAMPLE_LINE + "\n", None)
    assert r.ocr_status == "ok"


def test_module_names_are_reexported() -> None:
    for name in (
        "OcrEngine",
        "OcrInfo",
        "default_ocr_engine",
        "extract",
        "PageResult",
        "Extraction",
        "Isolation",
        "PageSegMode",
        "EngineMode",
        "PageSource",
        "OcrStatus",
    ):
        assert name in pdf_extract.__all__, name
        assert hasattr(pdf_extract, name), name
    assert sys.modules["pdf_extract._native"].extract is pdf_extract.extract
