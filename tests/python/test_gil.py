"""GIL release, free-threading and import cost (run spec T42, T43; §4.11
items 8 and 9; design spec §8.4).

The GIL tests run a native call in one thread while a second thread does
pure-Python work and records the longest gap between its iterations. A call
that holds the GIL (a missing ``py.detach``) stalls that thread for the whole
call, so the extraction, OCR and engine tests assert that gaps stay under
MAX_GAP_S, the interpreter's switch-interval scale. The open test is the
exception: both variants free the Document under the GIL (only the buffer
copy is specific to the bytes variant), so the call holds it by design for
part of its run, and scheduler noise on shared runners adds to any gap.
There the bound is relative to the measured duration of the call
(``assert_no_whole_run_hold``), which still fails a call that holds the GIL
throughout. The extraction is a ~25M-operator page cut off by
``page_time_budget``, so its duration is fixed by the budget rather than by
the machine's speed.

The counter measures CPU starvation as well as GIL holds: on an
oversubscribed machine (load ~53 on 16 cores, internal #160) the OS can deschedule
the counter thread for 100 ms or more while the native call holds nothing.
The absolute-bound tests therefore run up to three trials and fail only when
two exceed MAX_GAP_S (``assert_gap_bound_in_two_of_three``; it stops after
two passes or two failures). That keeps the 50 ms sensitivity: a missing
``py.detach`` stalls every trial, so it still fails, in two. It absorbs one
starved trial, not sustained starvation: a machine loaded enough to starve
the counter in two of three trials still fails the test, and that is the
accepted residual. The duration guards stay hard assertions in every trial,
not retried: they check the work was long enough for a hold to show.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import sysconfig
import threading
import time
from collections.abc import Callable
from pathlib import Path

import pytest

import pdf_extract
from pdf_extract import Isolation

from ocrkit import needs_helper, needs_tessdata, text_pdf
from pdfs import Fixtures

FREE_THREADED = bool(sysconfig.get_config_var("Py_GIL_DISABLED"))
BUDGET_S = 1.0
MAX_GAP_S = 0.050


def assert_no_whole_run_hold(longest: float, duration: float) -> None:
    """Fails only when the counter's longest gap is about as long as the work
    itself, which is what a GIL held for the whole call looks like (the gap is
    roughly ``duration``). A GIL-held phase shorter than half the call is
    accepted, and scheduler noise of up to ``MAX_GAP_S`` on top of it.

    The allowance is added, not a floor: ``max(MAX_GAP_S, duration / 2)`` was
    always ``duration / 2`` for the callers' duration guard, so the noise
    allowance never applied and a call that got faster (about 100 ms) would
    have been held to about 50 ms, the scale of the noise. Added, the bound
    stays above ``MAX_GAP_S`` however short the call.

    A whole-call hold leaves a gap of ``duration - e`` (``e`` is the time
    before the worker takes the GIL), and that fails only when
    ``duration >= 2 * MAX_GAP_S + 2 * e``. The callers' guard of
    ``duration > 2 * MAX_GAP_S`` therefore does not make a whole-call hold
    fail "exactly when" it passes: it fails reliably for durations
    comfortably above 100 ms and only marginally just over it. The open test
    runs at about 140 ms on a fast machine, which a guard of
    ``3 * MAX_GAP_S`` would reject, so the guard stays and the thin margin
    is accepted; the absolute-bound tests (extraction, OCR, engine) have no
    such margin.
    """
    bound = duration / 2 + MAX_GAP_S
    assert longest < bound, (
        f"counter stalled for {longest * 1000:.1f} ms "
        f"(bound {bound * 1000:.1f} ms, work took {duration * 1000:.1f} ms)"
    )


def longest_stall_during(work: Callable[[], None]) -> tuple[float, float]:
    """Runs ``work()`` on a thread while this thread counts; returns the
    longest gap between two counter iterations and the work's duration.

    The clock starts before the worker does and nothing here waits for the
    worker to begin: a wait would itself absorb a stall (this thread would
    only wake once a GIL-holding extraction had finished).
    """
    done = threading.Event()
    duration = 0.0

    def run() -> None:
        nonlocal duration
        t0 = time.perf_counter()
        try:
            work()
        finally:
            duration = time.perf_counter() - t0
            done.set()

    worker = threading.Thread(target=run)
    longest = 0.0
    last = time.perf_counter()
    worker.start()
    while not done.is_set():
        now = time.perf_counter()
        longest = max(longest, now - last)
        last = now
        sum(range(100))  # a little Python-only work per iteration
    worker.join()
    return longest, duration


TRIALS = 3
FAILURES_ALLOWED = 1


def assert_gap_bound_in_two_of_three(measure: Callable[[], float]) -> None:
    """Runs ``measure`` (one trial, returning the counter's longest gap) up to
    ``TRIALS`` times and fails only if more than ``FAILURES_ALLOWED`` trials
    reach ``MAX_GAP_S``. It stops as soon as the outcome is decided: after
    two passes, or after two failures.

    A held GIL stalls every trial, so a regression still fails; one trial
    lost to CPU starvation of the counter thread (internal #160) does not. An
    ``AssertionError`` raised inside ``measure`` (a duration guard) is not
    caught: only the gap bound is retried.
    """
    passes_needed = TRIALS - FAILURES_ALLOWED
    gaps: list[float] = []
    failures = 0
    while failures <= FAILURES_ALLOWED and len(gaps) - failures < passes_needed:
        longest = measure()
        gaps.append(longest)
        if longest >= MAX_GAP_S:
            failures += 1
    assert failures <= FAILURES_ALLOWED, (
        f"counter stalled past {MAX_GAP_S * 1000:.0f} ms in {failures} of "
        f"{len(gaps)} trials (gaps: {', '.join(f'{g * 1000:.1f}' for g in gaps)} ms)"
    )


def fake_measure(gaps: list[float]) -> tuple[Callable[[], float], list[float]]:
    """A ``measure`` that returns ``gaps`` in order; the second value is the
    list of gaps actually consumed, so a test can check early stopping."""
    remaining = list(gaps)
    consumed: list[float] = []

    def measure() -> float:
        gap = remaining.pop(0)
        consumed.append(gap)
        return gap

    return measure, consumed


OVER = MAX_GAP_S * 2
UNDER = MAX_GAP_S / 10


@pytest.mark.parametrize(
    ("gaps", "runs"),
    [
        ([UNDER, UNDER, OVER], 2),  # two passes decide it; the third never runs
        ([OVER, UNDER, UNDER], 3),  # one starved trial is absorbed
        ([UNDER, OVER, UNDER], 3),
    ],
)
def test_gap_retry_passes_with_at_most_one_stalled_trial(gaps: list[float], runs: int) -> None:
    measure, consumed = fake_measure(gaps)
    assert_gap_bound_in_two_of_three(measure)
    assert len(consumed) == runs


@pytest.mark.parametrize(
    ("gaps", "runs"),
    [
        ([OVER, OVER, UNDER], 2),  # two failures decide it
        ([UNDER, OVER, OVER], 3),
        ([OVER, UNDER, OVER], 3),
        ([MAX_GAP_S, MAX_GAP_S, UNDER], 2),  # the bound itself is a failure
    ],
)
def test_gap_retry_fails_with_two_stalled_trials(gaps: list[float], runs: int) -> None:
    measure, consumed = fake_measure(gaps)
    with pytest.raises(AssertionError, match=r"in 2 of \d trials"):
        assert_gap_bound_in_two_of_three(measure)
    assert len(consumed) == runs


def test_gap_retry_does_not_retry_a_duration_guard() -> None:
    calls = 0

    def measure() -> float:
        nonlocal calls
        calls += 1
        raise AssertionError("duration guard")

    with pytest.raises(AssertionError, match="duration guard"):
        assert_gap_bound_in_two_of_three(measure)
    assert calls == 1


@pytest.mark.parametrize("operation", ["text", "text_page"])
def test_extraction_releases_the_gil(fixtures: Fixtures, operation: str) -> None:
    doc = pdf_extract.open(
        fixtures.path("form_fanout_22"), limits=pdf_extract.Limits(page_time_budget=BUDGET_S)
    )
    page = doc[0]

    def work() -> None:
        with pytest.raises(pdf_extract.LimitExceededError):
            getattr(page, operation)()

    def measure() -> float:
        longest, duration = longest_stall_during(work)
        # The extraction really ran for the budget, so a held GIL would show.
        assert duration >= BUDGET_S * 0.9, duration
        return longest

    assert_gap_bound_in_two_of_three(measure)


def test_whole_run_hold_bound_allows_noise_on_top_of_half() -> None:
    # Half of a 150 ms call held (75 ms) plus 45 ms of noise passes. Under the
    # old max(MAX_GAP_S, duration / 2) the bound was 75 ms and this 120 ms gap
    # failed. A hold of the whole call still fails.
    assert_no_whole_run_hold(longest=0.120, duration=0.150)
    with pytest.raises(AssertionError, match="counter stalled"):
        assert_no_whole_run_hold(longest=0.150, duration=0.150)


@pytest.mark.parametrize("source", ["path", "bytes"])
def test_open_releases_the_gil(fixtures: Fixtures, source: str) -> None:
    # A 300,000-page tree: the page-tree walk is the work open() does, and
    # one walk lasts well over the stall bound.
    #
    # Known residual (internal #149): the whole-call-hold check is only weakly
    # sensitive here. The open takes about 145 ms against a bound of
    # duration / 2 + 50 ms (about 122 ms), so a GIL held throughout fails
    # only if the worker takes the GIL within about 20 ms. Looping the
    # open does not help (each held call stalls under the enlarged bound);
    # only a larger single-call fixture would allow a 3 * MAX_GAP_S guard,
    # and its generation time and size were judged not worth it.
    path = fixtures.path("big_tree_300000")
    data = path.read_bytes()

    def work() -> None:
        doc = pdf_extract.open(path) if source == "path" else pdf_extract.open_bytes(data)
        assert doc.page_count == 300_000

    longest, duration = longest_stall_during(work)
    assert duration > 2 * MAX_GAP_S, duration
    assert_no_whole_run_hold(longest, duration)


# Both modes: in process the GIL is released around Tesseract itself; with
# helpers (the Python default, internal #53) around rendering and the pipe wait.
ISOLATIONS = [
    pytest.param("in_process", marks=needs_tessdata, id="in_process"),
    pytest.param("process", marks=needs_helper, id="process"),
]


@pytest.mark.parametrize("isolation", ISOLATIONS)
def test_ocr_releases_the_gil(tmp_path: Path, isolation: Isolation) -> None:
    # A dense page: rendering at 300 dpi and recognising 40 lines takes far
    # longer than the stall bound (O21).
    path = tmp_path / "dense.pdf"
    path.write_bytes(
        text_pdf(
            [[f"Line {i:02d}: the quick brown fox jumps over the lazy dog" for i in range(40)]]
        )
    )
    page = pdf_extract.open(path)[0]
    engine = pdf_extract.OcrEngine(max_instances=1, isolation=isolation)

    def work() -> None:
        assert "Line 39" in page.ocr(engine).text()

    def measure() -> float:
        longest, duration = longest_stall_during(work)
        assert duration > 2 * MAX_GAP_S, duration
        return longest

    assert_gap_bound_in_two_of_three(measure)


@pytest.mark.parametrize("isolation", ISOLATIONS)
def test_engine_construction_releases_the_gil(isolation: Isolation) -> None:
    # Construction loads the model, hashes it and runs one recognition (in
    # this process, or inside the first helper).
    def work() -> None:
        pdf_extract.OcrEngine(max_instances=1, isolation=isolation)

    assert_gap_bound_in_two_of_three(lambda: longest_stall_during(work)[0])


def child_env() -> dict[str, str]:
    """Environment for a child interpreter that decides about the GIL on its
    own (no PYTHON_GIL override) and imports the package fresh."""
    env = dict(os.environ)
    env.pop("PYTHON_GIL", None)
    env.pop("COV_CORE_SOURCE", None)
    return env


@pytest.mark.skipif(not FREE_THREADED, reason="needs a free-threaded CPython (3.14t+)")
def test_import_keeps_the_gil_disabled() -> None:
    # Without PYTHON_GIL=0, CPython re-enables the GIL when it imports an
    # extension that does not declare free-threading support; ours does
    # (gil_used = false), so it stays off.
    code = "import sys, pdf_extract; print(sys._is_gil_enabled())"
    out = subprocess.run(
        [sys.executable, "-c", code], env=child_env(), capture_output=True, text=True, check=True
    )
    assert out.stdout.strip() == "False", out.stderr
    assert "GIL" not in out.stderr


@pytest.mark.skipif(not FREE_THREADED, reason="needs a free-threaded CPython (3.14t+)")
def test_gil_is_disabled_in_this_process() -> None:
    assert sys._is_gil_enabled() is False


def test_import_does_no_work_and_starts_no_threads() -> None:
    code = (
        "import threading, pdf_extract\n"
        "assert threading.active_count() == 1, threading.enumerate()\n"
        "print('ok')\n"
    )
    out = subprocess.run(
        [sys.executable, "-X", "importtime", "-c", code],
        env=child_env(),
        capture_output=True,
        text=True,
        check=True,
    )
    assert out.stdout.strip() == "ok"
    # -X importtime lines: "import time: self [us] | cumulative | name".
    cumulative = None
    for line in out.stderr.splitlines():
        m = re.match(r"import time:\s+\d+\s+\|\s+(\d+)\s+\|\s?(\s*)(\S+)$", line)
        if m and m.group(3) == "pdf_extract":
            cumulative = int(m.group(1))
    assert cumulative is not None, out.stderr
    assert cumulative < 50_000, f"import pdf_extract took {cumulative} us"
