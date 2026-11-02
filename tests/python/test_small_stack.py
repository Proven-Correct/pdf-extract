"""Extraction on 512 KiB thread stacks (run spec §2.3, §4.12 item 6).

``pdfs.on_small_stack`` runs each extraction on a Python thread whose stack
it has measured to be 512 KiB (the ``threading.stack_size`` setting alone
is not enough on Linux, internal issue #70), and the Rust code it calls runs on that
stack. Every generated fixture (hostile, page-tree, contract, text, golden
and the builder samples, which carry the KP features) and every committed
corpus-C file is opened and extracted on such a thread. The only requirement is that the
process survives: any ``PdfExtractError`` is an acceptable outcome except
``InternalError`` (a caught panic, i.e. a bug). A stack overflow would abort
the whole interpreter, so a crash here shows up as the test run dying.

Corpus C is read by glob, so the KP fixtures unit 13 commits are covered
without touching this file.
"""

from __future__ import annotations

import sys
import threading
from functools import partial
from pathlib import Path

import pytest

import pdf_extract
from pdf_extract import InternalError, PdfExtractError

from ocrkit import text_pdf
from pdfs import CORPUS_C, SMALL_STACK, Fixtures, on_small_stack, stack_fits, thread_stack_size

#: Every group of the generated fixture index; a new group must be added
#: here (``test_every_generated_group_is_covered``).
GROUPS = [
    "contract",
    "tree",
    "hostile",
    "text",
    "golden",
    "sample-builder",
    "sample-hostile",
    "big",
    "slow",
    "ocr",
]
#: Pages extracted from the front of each document (plus the last page):
#: enough for every fixture's interesting pages, bounded for the
#: 300,000-page tree.
FRONT_PAGES = 50


def limits_for(group: str) -> pdf_extract.Limits:
    # The "slow" fixture is a ~25M-operator form fan-out; an operator cap
    # ends it after the nesting it exists for has been reached.
    if group == "slow":
        return pdf_extract.Limits(max_ops_per_page=500_000)
    return pdf_extract.Limits()


def extract_everything(path: Path, limits: pdf_extract.Limits) -> str:
    """Opens ``path`` and extracts its pages; returns an outcome label."""
    try:
        doc = pdf_extract.open(path, limits=limits)
    except InternalError:
        raise
    except PdfExtractError as e:
        return f"open: {type(e).__name__}"
    with doc:
        count = len(doc)
        indices = sorted({*range(min(count, FRONT_PAGES)), count - 1} - {-1})
        failures = 0
        for i in indices:
            try:
                doc[i].text()
                doc[i].text_page(unmapped="replacement").text(block_separator="blank_line")
            except InternalError:
                raise
            except PdfExtractError:
                failures += 1
        return f"pages={count} failed={failures}"


def run_files(files: list[Path], limits: pdf_extract.Limits) -> dict[str, str]:
    # One small-stack thread per file, so each starts from an empty stack.
    return {f.name: on_small_stack(partial(extract_everything, f, limits)) for f in files}


@pytest.mark.parametrize("group", GROUPS)
def test_generated_fixtures_on_512_kib_stacks(fixtures: Fixtures, group: str) -> None:
    files = [path for path, g in fixtures.index() if g == group]
    assert files, f"no generated fixtures in group {group!r}"
    outcomes = run_files(files, limits_for(group))
    assert len(outcomes) == len(files)


def test_corpus_c_on_512_kib_stacks() -> None:
    files = sorted(CORPUS_C.glob("*.pdf"))
    assert files, "corpus C is empty"
    outcomes = run_files(files, pdf_extract.Limits())
    assert len(outcomes) == len(files)


def test_every_generated_group_is_covered(fixtures: Fixtures) -> None:
    assert {g for _, g in fixtures.index()} == set(GROUPS)


def test_the_harness_sets_and_restores_the_stack_size(monkeypatch: pytest.MonkeyPatch) -> None:
    # `threading.stack_size()` read from inside the harness thread is the
    # process-wide setting, which the harness restores as soon as the thread
    # has started; whether the thread reads it before or after the restore is
    # a race (lost every time on free-threaded 3.14). So check the calls the
    # harness makes (one set-and-restore pair per thread it starts), and the
    # thread's real stack.
    before = threading.stack_size()
    calls: list[int] = []
    real = threading.stack_size

    def recording(*size: int) -> int:
        calls.extend(size)
        return real(*size)

    with monkeypatch.context() as patch:
        patch.setattr(threading, "stack_size", recording)
        measured = on_small_stack(thread_stack_size)

    assert calls
    assert calls == [SMALL_STACK, before] * (len(calls) // 2)
    assert threading.stack_size() == before
    assert_small_stack(measured)


def test_a_cached_rust_worker_stack_is_not_reused(tmp_path: Path) -> None:
    # Internal issue #70: Rust's `extract(workers=...)` threads run on 2 MiB stacks;
    # once they exit, glibc keeps their stacks and serves a later thread
    # from them when no free cached stack fits better, accepting up to 4x
    # the request, so a thread asking for ~512 KiB can be given 2 MiB (what
    # the Linux runners measured). A free cached stack of exactly the
    # requested size wins over that, and every earlier harness thread left
    # one of SMALL_STACK, so this test asks for a size no other thread
    # uses: then only the oversized Rust stacks are on offer.
    unique = SMALL_STACK + 16 * 1024
    path = tmp_path / "eight.pdf"
    path.write_bytes(text_pdf([[f"page {i} of eight"] for i in range(8)]))
    for _ in range(3):
        assert pdf_extract.extract(path, workers=8).page_count == 8
    assert_small_stack(on_small_stack(thread_stack_size, unique), unique)


def assert_small_stack(measured: int | None, stack_size: int = SMALL_STACK) -> None:
    """``measured``, a harness thread's stack, is the ``stack_size`` it asked
    for: exact on Linux, a few KiB more on macOS (512 KiB measures as
    524,288 + 12,288 bytes). Only Linux and macOS can be asked."""
    platform: str = sys.platform
    if platform == "darwin" or platform.startswith("linux"):
        assert measured is not None
        assert stack_fits(measured, stack_size), f"harness thread stack is {measured} bytes"
