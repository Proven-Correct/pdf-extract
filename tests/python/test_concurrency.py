"""Concurrency through the Python binding (run spec §4.12, T38-T40; design
spec §13.3).

- **Stress**: 16 threads x 200 extractions, on one shared 40-page document
  and on 16 separate documents, compared with single-threaded output.
- **Shared document**: threads extracting the same page and different pages
  of one ``Document``, including a document whose xref is repaired lazily by
  whichever thread first needs the broken object.
- **Close during use**: a closer thread closes the document at a seeded
  random moment while 8 workers extract; every call returns the right text
  or raises ``DocumentClosedError``, over 500 rounds.
- **Shared iterator**: threads consuming one ``iter(doc)`` see every page
  exactly once overall.
- **OCR stress** (run 0002 §4.11): 16 threads x 20 pages through one engine
  with ``max_instances=4``, with Tesseract and with the scripted backend
  (which also proves the pool's bound), compared with single-threaded output.

Determinism: nothing asserts on timing. Each assertion holds for every
interleaving; the schedule only changes how much contention a run sees.
Random choices are seeded per round, so a failure names a reproducible
configuration. The suite runs unchanged on free-threaded 3.14t with
``PYTHON_GIL=0`` (CI's ``python-freethreaded`` job), where the threads
really run in parallel.
"""

from __future__ import annotations

import random
import threading
import time
from collections.abc import Callable, Iterator
from itertools import islice

import pytest

import pdf_extract
from pdf_extract import Document, DocumentClosedError, Isolation, Page, _native
from pdf_extract.testing import PdfBuilder

from ocrkit import needs_helper, needs_tessdata
from pdfs import Fixtures, run_threads

THREADS = 16
CALLS = 200

#: Extraction variants a worker cycles through; each runs a detached native
#: path (Page.text, Page.text_page + TextPage.text) with different options.
Extract = Callable[[Page], str]
VARIANTS: list[Extract] = [
    lambda p: p.text(),
    lambda p: p.text(block_separator="blank_line", unmapped="replacement"),
    lambda p: p.text_page(unmapped="skip").text(block_separator="blank_line"),
]

WORDS = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta"]
WORDS += ["eta", "theta", "iota", "kappa", "lambda", "mu"]


def report_document(pages: int, seed: int) -> bytes:
    """A document whose pages each carry ~15 runs in two blocks, at varied
    sizes and positions, so a page's text is several lines long and differs
    from every other page's."""
    rng = random.Random(seed)
    builder = PdfBuilder()
    for n in range(pages):
        page = builder.new_page()
        page.insert_text((72, 60), f"Report {seed} page {n + 1}", fontsize=16)
        y = 100.0
        for line in range(14):
            size = rng.choice([9.0, 10.0, 11.0, 12.0])
            words = " ".join(rng.choice(WORDS) for _ in range(rng.randint(3, 9)))
            page.insert_text((72 + rng.randint(0, 20), y), f"{line}: {words}", fontsize=size)
            y += size * 1.25 + (40 if line == 6 else 0)  # a block gap mid-page
    return builder.to_bytes()


def expected_texts(doc: Document) -> list[list[str]]:
    """``[page][variant]`` single-threaded output."""
    return [[v(doc[i]) for v in VARIANTS] for i in range(len(doc))]


def test_stress_one_shared_document_16_threads_x_200() -> None:
    doc = pdf_extract.open_bytes(report_document(40, seed=1))
    expected = expected_texts(doc)

    def work(t: int) -> list[str]:
        # Thread t walks the pages from its own offset and stride, so
        # threads hit the same page at the same time and different pages.
        return [VARIANTS[k % 3](doc[(t * 7 + k * (1 + t % 3)) % 40]) for k in range(CALLS)]

    for t, got in enumerate(run_threads(THREADS, work)):
        want = [expected[(t * 7 + k * (1 + t % 3)) % 40][k % 3] for k in range(CALLS)]
        assert got == want, f"thread {t}"


def mixed_documents(fixtures: Fixtures) -> list[tuple[str, bytes]]:
    """16 different documents: builder samples covering every font kind and
    page feature, a lazily repaired xref, a 10-page tree, and kit-built
    reports."""
    names = [
        "samples/builder_sizes_modes",
        "samples/builder_winansi_all",
        "samples/builder_crop_rotate",
        "samples/builder_type1_encodings",
        "samples/builder_type3",
        "samples/builder_type0",
        "samples/builder_forms",
        "samples/builder_annotations",
        "samples/builder_ocg_actualtext",
        "samples/builder_multi_page",
        "misoffset_40_6_10",
        "tree_flat_10",
        "two_blocks",
    ]
    docs = [(name, fixtures.bytes(name)) for name in names]
    docs += [(f"report_{seed}", report_document(5, seed)) for seed in range(2, 5)]
    assert len(docs) == THREADS
    return docs


def test_stress_16_separate_documents_16_threads_x_200(fixtures: Fixtures) -> None:
    docs = mixed_documents(fixtures)
    # Expected output from documents of their own, so the threaded documents
    # start cold (no lazy repair done yet).
    expected = [expected_texts(pdf_extract.open_bytes(data)) for _, data in docs]

    def work(t: int) -> list[str]:
        doc = pdf_extract.open_bytes(docs[t][1])
        n = len(doc)
        return [VARIANTS[k % 3](doc[k % n]) for k in range(CALLS)]

    for t, got in enumerate(run_threads(THREADS, work)):
        n = len(expected[t])
        assert got == [expected[t][k % n][k % 3] for k in range(CALLS)], docs[t][0]


@pytest.mark.parametrize("name", ["report", "misoffset_40_6_10"])
def test_shared_document_same_and_different_pages(fixtures: Fixtures, name: str) -> None:
    data = report_document(40, seed=5) if name == "report" else fixtures.bytes(name)
    reference = pdf_extract.open_bytes(data)
    expected = expected_texts(reference)
    count = len(expected)
    # misoffset_40_6_10: page 6's object sits at a wrong offset, so the
    # first thread to load it repairs the xref while the others look up
    # objects. A fresh document per round re-runs that race.
    for _ in range(10):
        doc = pdf_extract.open_bytes(data)

        def work(t: int, doc: Document = doc) -> list[tuple[int, str]]:
            out = []
            for k in range(40):
                # Even threads all read page 6, odd threads spread out.
                i = 6 if t % 2 == 0 else (t * 5 + k) % count
                out.append((i, VARIANTS[(t + k) % 3](doc[i])))
            return out

        for t, got in enumerate(run_threads(8, work)):
            assert got == [(i, expected[i][(t + k) % 3]) for k, (i, _) in enumerate(got)], t
        assert doc.warnings == reference.warnings


def test_close_during_use_500_rounds() -> None:
    data = report_document(40, seed=6)
    expected = [p.text() for p in pdf_extract.open_bytes(data)]
    workers = 8
    # Backstop only: a worker stops at the first call started after close()
    # returned, so it never gets near this many calls before the close.
    max_calls = 100_000

    def run_round(round_no: int) -> int:
        """One round; returns the number of successful extractions."""
        delay = random.Random(round_no).uniform(0.0, 0.004)
        doc = pdf_extract.open_bytes(data)
        # Taken before close(): a TextPage keeps answering after it, a Page
        # handle stays valid but its extraction then raises.
        kept = doc[0].text_page()
        pages = [doc[t] for t in range(workers)]
        # Set once close() has returned (even if it raised). A call that
        # starts after that must raise DocumentClosedError; one in flight
        # across the close may finish either way. This also bounds every
        # worker: if close() regressed to a no-op, the first call after the
        # event fails the test instead of looping forever.
        close_returned = threading.Event()

        def work(t: int) -> int:
            if t == workers:  # the closer
                time.sleep(delay)
                try:
                    doc.close()
                finally:
                    close_returned.set()
                return 0
            for done in range(max_calls):
                after_close = close_returned.is_set()
                # Alternate the held Page and a fresh doc[i] (which itself
                # raises once the document is closed).
                i = (t + 3 * done) % len(expected)
                try:
                    text = pages[t].text() if done % 2 else doc[i].text()
                except DocumentClosedError:
                    return done
                assert not after_close, f"round {round_no}: worker {t} extracted after close()"
                assert text == (expected[t] if done % 2 else expected[i]), (round_no, t)
            pytest.fail(f"round {round_no}: worker {t} never saw the close")

        # Every worker returns only through DocumentClosedError; any other
        # exception (or a wrong text) is re-raised by run_threads.
        extracted = sum(run_threads(workers + 1, work))
        assert doc.closed
        assert len(doc) == 40  # page_count stays readable
        assert kept.text() == expected[0]
        doc.close()  # a second close is harmless
        with pytest.raises(DocumentClosedError):
            pages[0].text()
        return extracted

    # The rounds did real work before the close (over 500 rounds; no single
    # round has to).
    assert sum(run_round(r) for r in range(500)) > 0


@pytest.mark.parametrize("threads", [2, 4])
def test_shared_iterator_hands_out_each_page_once(fixtures: Fixtures, threads: int) -> None:
    doc = pdf_extract.open(fixtures.path("tree_fanout_100x2"))
    assert len(doc) == 10_000
    for _ in range(5):
        it = iter(doc)

        def work(_: int, it: Iterator[Page] = it) -> list[int]:
            # Bounded: an iterator that never stopped would fail the count
            # check below instead of hanging.
            return [page.index for page in islice(it, 10_001)]

        seen = [i for part in run_threads(threads, work) for i in part]
        assert sorted(seen) == list(range(10_000))
        with pytest.raises(StopIteration):
            next(it)


def test_shared_iterator_while_extracting() -> None:
    # Slower consumers: each page handed out is also extracted, so the
    # threads interleave on every page rather than in long runs.
    doc = pdf_extract.open_bytes(report_document(60, seed=7))
    expected = [p.text() for p in doc]
    it = iter(doc)

    def work(_: int) -> list[tuple[int, str]]:
        # Bounded like the test above.
        return [(page.index, page.text()) for page in islice(it, len(expected) + 1)]

    got = sorted(pair for part in run_threads(2, work) for pair in part)
    assert got == list(enumerate(expected))


OCR_PAGES = 20
OCR_INSTANCES = 4


def small_pages_doc() -> Document:
    """20 small pages (360 x 60 pt), each one distinct line at 20 pt: quick
    to render and recognise."""
    b = PdfBuilder()
    for i in range(OCR_PAGES):
        b.new_page(360, 60).insert_text((10, 40), f"Page {i:02d} of the stress run", fontsize=20)
    return pdf_extract.open_bytes(b.to_bytes())


def ocr_stress(engine: pdf_extract.OcrEngine) -> None:
    doc = small_pages_doc()
    expected = [page.ocr(engine).text() for page in doc]
    assert len(set(expected)) == OCR_PAGES  # every page is told apart

    def work(t: int) -> list[str]:
        # Each thread starts at a different page, so the threads contend
        # for the pool on different pages at once.
        order = [(t + i) % OCR_PAGES for i in range(OCR_PAGES)]
        got = {i: doc[i].ocr(engine).text() for i in order}
        return [got[i] for i in range(OCR_PAGES)]

    for result in run_threads(THREADS, work):
        assert result == expected


@pytest.mark.parametrize(
    "isolation",
    [
        pytest.param("in_process", marks=needs_tessdata, id="in_process"),
        pytest.param("process", marks=needs_helper, id="process"),
    ],
)
def test_ocr_stress_16_threads_x_20_pages_with_tesseract(isolation: Isolation) -> None:
    # Both pools: in-process instances, and helpers (the Python default, internal #53).
    engine = pdf_extract.OcrEngine(dpi=150, max_instances=OCR_INSTANCES, isolation=isolation)
    ocr_stress(engine)


def test_ocr_stress_16_threads_x_20_pages_scripted() -> None:
    # Every recognition sleeps briefly, so the threads queue for instances;
    # the scripted text ignores the page, so all pages read the same and the
    # comparison is with the single-threaded run.
    engine = _native._scripted_engine(
        default=("sleep", 0.002, ("text", "scripted")), max_instances=OCR_INSTANCES
    )
    doc = small_pages_doc()

    def work(t: int) -> list[str]:
        return [doc[(t + i) % OCR_PAGES].ocr(engine).text() for i in range(OCR_PAGES)]

    for result in run_threads(THREADS, work):
        assert result == ["scripted\n"] * OCR_PAGES
    stats = _native._scripted_stats(engine)
    assert stats["calls"] == THREADS * OCR_PAGES
    assert 1 <= stats["max_in_flight"] <= OCR_INSTANCES
    assert stats["creates"] <= OCR_INSTANCES
