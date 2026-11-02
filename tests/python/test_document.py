"""Document, Page handles, Limits and the close protocol (run spec §3.5;
§4.11 items 4 and 5; T2, T16, T40, T46)."""

from __future__ import annotations

import array
import gc
import importlib.metadata
import threading
import tomllib
from collections.abc import Iterator
from pathlib import Path
from unittest import mock

import pytest

import pdf_extract
from pdf_extract import (
    Document,
    DocumentClosedError,
    Limits,
    Page,
    TextPage,
)

from pdfs import REPO, Fixtures


@pytest.fixture
def forty(fixtures: Fixtures) -> Iterator[Document]:
    doc = pdf_extract.open(fixtures.path("forty_pages"))
    yield doc
    doc.close()


# --- Indexing and iteration (T46) -------------------------------------------


def test_negative_index_and_len(forty: Document) -> None:
    assert len(forty) == forty.page_count == 40
    assert forty[-1].index == 39
    assert forty[-40].index == 0
    assert forty[-1].text() == "Page 40\n"
    with pytest.raises(IndexError):
        forty[-41]
    with pytest.raises(IndexError):
        forty[len(forty)]
    with pytest.raises(IndexError):
        forty[-(2**70)]


@pytest.mark.parametrize("bad", ["a", 1.0, None, slice(0, 2), [0]])
def test_non_integer_index_is_type_error(forty: Document, bad: object) -> None:
    with pytest.raises(TypeError, match="page indices must be integers"):
        forty[bad]  # type: ignore[index]


def test_index_accepts_objects_with_dunder_index(forty: Document) -> None:
    class Three:
        def __index__(self) -> int:
            return 3

    assert forty[Three()].index == 3  # type: ignore[index]
    assert forty[True].index == 1  # bool is an int, as for lists


def test_iteration_yields_every_page_once_in_order(forty: Document) -> None:
    pages = list(forty)
    assert len(pages) == forty.page_count
    assert [p.index for p in pages] == list(range(40))
    assert all(isinstance(p, Page) for p in pages)


def test_iterator_protocol(forty: Document) -> None:
    it = iter(forty)
    assert iter(it) is it
    assert it.__length_hint__() == 40  # type: ignore[attr-defined]
    assert next(it).index == 0
    assert it.__length_hint__() == 39  # type: ignore[attr-defined]
    rest = list(it)
    assert len(rest) == 39
    with pytest.raises(StopIteration):
        next(it)


def test_shared_iterator_hands_each_page_out_once(forty: Document) -> None:
    it = iter(forty)
    seen: list[int] = []
    lock = threading.Lock()

    def consume() -> None:
        for page in it:
            with lock:
                seen.append(page.index)

    threads = [threading.Thread(target=consume) for _ in range(4)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert sorted(seen) == list(range(40))


# --- Context manager and close (T40) ---------------------------------------


def test_context_manager_closes_on_exception_and_reraises(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(fixtures.path("valid"))
    page = doc[0]

    def body() -> None:
        with doc as entered:
            assert not entered.closed
            raise ZeroDivisionError

    with pytest.raises(ZeroDivisionError):
        body()
    assert doc.closed
    with pytest.raises(DocumentClosedError):
        page.text()


def test_context_manager_returns_the_document_and_closes(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(fixtures.path("valid"))
    with doc as entered:
        assert entered is doc
    assert doc.closed
    # Returns None (falsy), so an exception is never suppressed.
    assert doc.__exit__(ValueError, ValueError("x"), None) is None  # type: ignore[func-returns-value]


def test_close_protocol(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(fixtures.path("broken_startxref"))
    page = doc[0]
    text_page = page.text_page()
    before = page.text()
    doc.close()
    doc.close()  # harmless
    assert doc.closed
    # Facts answer after close.
    assert doc.page_count == 1
    assert len(doc) == 1
    assert not doc.is_encrypted
    assert "xref: repaired" in doc.warnings
    # Operations raise.
    with pytest.raises(DocumentClosedError):
        page.text()
    with pytest.raises(DocumentClosedError):
        page.text_page()
    with pytest.raises(DocumentClosedError):
        doc[0]
    # Handles and results obtained before still answer.
    assert page.index == 0
    assert text_page.text() == before
    assert text_page.warnings == ()
    # Iterating a closed document yields handles whose operations raise.
    handles = list(doc)
    assert [p.index for p in handles] == [0]
    with pytest.raises(DocumentClosedError):
        handles[0].text()


def test_closed_document_is_freed(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(fixtures.path("valid"))
    text_page = doc[0].text_page()
    del doc
    gc.collect()
    assert text_page.text() == "One\n"


# --- open / open_bytes inputs (T2) ------------------------------------------


def test_open_accepts_str_and_pathlike(fixtures: Fixtures) -> None:
    path = fixtures.path("valid")
    assert pdf_extract.open(str(path)).page_count == 2
    assert pdf_extract.open(Path(path)).page_count == 2

    class Custom:
        def __fspath__(self) -> str:
            return str(path)

    assert pdf_extract.open(Custom()).page_count == 2


def test_open_rejects_non_paths() -> None:
    with pytest.raises(TypeError):
        pdf_extract.open(42)  # type: ignore[arg-type]


def test_open_bytes_accepts_bytes_bytearray_memoryview(fixtures: Fixtures) -> None:
    data = fixtures.bytes("valid")
    for buf in (data, bytearray(data), memoryview(data), memoryview(bytearray(data))):
        assert pdf_extract.open_bytes(buf)[1].text() == "Two\n"


def test_open_bytes_copies_and_keeps_no_reference(fixtures: Fixtures) -> None:
    buf = bytearray(fixtures.bytes("valid"))
    doc = pdf_extract.open_bytes(buf)
    # Resizing would fail if a buffer export were still held.
    buf.clear()
    buf.extend(b"garbage")
    assert doc[0].text() == "One\n"


def test_open_bytes_non_contiguous_memoryview(fixtures: Fixtures) -> None:
    data = fixtures.bytes("valid")
    doubled = bytes(b for byte in data for b in (byte, 0))
    view = memoryview(doubled)[::2]
    assert not view.contiguous
    assert pdf_extract.open_bytes(view).page_count == 2


@pytest.mark.parametrize("bad", ["%PDF-1.7", 42, None, [1, 2]])
def test_open_bytes_rejects_non_buffers(bad: object) -> None:
    with pytest.raises(TypeError):
        pdf_extract.open_bytes(bad)  # type: ignore[arg-type]


def test_max_pages_must_not_be_negative(fixtures: Fixtures) -> None:
    with pytest.raises(ValueError, match="max_pages"):
        pdf_extract.open(fixtures.path("valid"), max_pages=-1)
    with pytest.raises(pdf_extract.TooManyPagesError):
        pdf_extract.open_bytes(fixtures.bytes("valid"), max_pages=0)


# --- Limits -----------------------------------------------------------------


def test_limits_defaults_match_the_spec() -> None:
    limits = Limits()
    assert limits.max_file_size == 512 * 2**20
    assert limits.max_page_tree_depth == 256
    assert limits.max_objects == 10_000_000
    assert limits.max_stream_decoded_bytes == 256 * 2**20
    assert limits.max_ops_per_page == 10_000_000
    assert limits.page_time_budget == 30.0
    assert limits.max_chars_per_page == 2_000_000
    assert limits.max_image_pixels == 256_000_000
    assert repr(limits).startswith("Limits(max_file_size=536870912,")
    assert repr(limits).endswith("max_image_pixels=256000000)")


def test_limits_round_trip_and_are_read_only() -> None:
    limits = Limits(
        max_file_size=1,
        max_page_tree_depth=2,
        max_objects=3,
        max_stream_decoded_bytes=4,
        max_ops_per_page=5,
        page_time_budget=0.25,
        max_chars_per_page=6,
        max_image_pixels=7,
    )
    assert (
        limits.max_file_size,
        limits.max_page_tree_depth,
        limits.max_objects,
        limits.max_stream_decoded_bytes,
        limits.max_ops_per_page,
        limits.page_time_budget,
        limits.max_chars_per_page,
        limits.max_image_pixels,
    ) == (1, 2, 3, 4, 5, 0.25, 6, 7)
    with pytest.raises(AttributeError):
        limits.max_objects = 7  # type: ignore[misc]
    assert Limits(page_time_budget=2).page_time_budget == 2.0


def test_limits_are_keyword_only() -> None:
    with pytest.raises(TypeError):
        Limits(1)  # type: ignore[call-arg]


@pytest.mark.parametrize(
    ("kwargs", "match"),
    [
        ({"max_file_size": -1}, "max_file_size must not be negative"),
        ({"max_page_tree_depth": 2**32}, "max_page_tree_depth is too large"),
        ({"max_objects": -5}, "max_objects"),
        ({"max_stream_decoded_bytes": 2**64}, "too large"),
        ({"max_ops_per_page": -1}, "max_ops_per_page"),
        ({"max_chars_per_page": -1}, "max_chars_per_page"),
        ({"max_image_pixels": -1}, "max_image_pixels must not be negative"),
        ({"max_image_pixels": 2**64}, "max_image_pixels is too large"),
        ({"page_time_budget": -1.0}, "page_time_budget"),
        ({"page_time_budget": float("nan")}, "page_time_budget"),
        ({"page_time_budget": float("inf")}, "page_time_budget"),
    ],
)
def test_limits_reject_out_of_range_values(kwargs: dict[str, float], match: str) -> None:
    with pytest.raises(ValueError, match=match):
        Limits(**kwargs)  # type: ignore[arg-type]


def test_limits_reject_wrong_types() -> None:
    with pytest.raises(TypeError):
        Limits(max_objects="many")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        Limits(max_objects=1.5)  # type: ignore[arg-type]


def test_limits_apply_to_the_document(fixtures: Fixtures) -> None:
    limits = Limits(max_ops_per_page=1000)
    doc = pdf_extract.open_bytes(fixtures.bytes("operator_flood_400000"), limits=limits)
    with pytest.raises(pdf_extract.LimitExceededError):
        doc[0].text()


# --- Classes ----------------------------------------------------------------


def test_classes_live_in_pdf_extract_and_are_not_constructible() -> None:
    for cls in (Document, Page, TextPage, Limits):
        assert cls.__module__ == "pdf_extract"
    for cls in (Document, Page, TextPage):
        with pytest.raises(TypeError):
            cls()


def test_reprs(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(fixtures.path("valid"))
    assert repr(doc) == "<pdf_extract.Document page_count=2 closed=False>"
    assert repr(doc[1]) == "<pdf_extract.Page index=1>"
    assert repr(doc[0].text_page()) == "<pdf_extract.TextPage source='content' warnings=0>"
    doc.close()
    assert repr(doc) == "<pdf_extract.Document page_count=2 closed=True>"


def test_classes_are_patchable_with_mock(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(fixtures.path("valid"))
    with mock.patch.object(pdf_extract.Page, "text", return_value="patched") as fake:
        assert doc[0].text() == "patched"
        fake.assert_called_once()
    assert doc[0].text() == "One\n"
    with mock.patch.object(
        pdf_extract.Document, "page_count", new=mock.PropertyMock(return_value=7)
    ):
        assert doc.page_count == 7
    assert doc.page_count == 2
    with mock.patch.object(pdf_extract.TextPage, "text", return_value="x"):
        assert doc[0].text_page().text() == "x"


def test_version() -> None:
    parts = pdf_extract.__version__.split(".")
    assert len(parts) == 3
    assert all(p.isdigit() for p in parts)


def test_package_version_is_the_workspace_version() -> None:
    # pyproject.toml declares the version dynamic, so maturin copies the
    # workspace version into the distribution metadata. The OCR helper's
    # handshake compares crate versions (all the workspace version), so the
    # installed package, the extension and the helper cannot disagree.
    with (REPO / "Cargo.toml").open("rb") as f:
        workspace = tomllib.load(f)["workspace"]["package"]["version"]
    assert importlib.metadata.version("pdf-extract") == workspace
    assert pdf_extract.__version__ == workspace


def test_open_bytes_casts_wide_memoryviews(fixtures: Fixtures) -> None:
    data = fixtures.bytes("valid")
    data += b" " * (-len(data) % 4)
    assert pdf_extract.open_bytes(memoryview(data).cast("I")).page_count == 2
    wide = array.array("I", data)
    assert pdf_extract.open_bytes(memoryview(wide)).page_count == 2


def test_open_bytes_rejects_non_contiguous_wide_memoryviews(fixtures: Fixtures) -> None:
    data = fixtures.bytes("valid")
    data += b" " * (-len(data) % 8)
    view = memoryview(data).cast("I")[::2]
    with pytest.raises(TypeError, match="could not view this memoryview as bytes"):
        pdf_extract.open_bytes(view)


def test_errors_from_dunder_index_propagate(forty: Document) -> None:
    class Bad:
        def __index__(self) -> int:
            raise ZeroDivisionError("from __index__")

    with pytest.raises(ZeroDivisionError, match="from __index__"):
        forty[Bad()]  # type: ignore[index]
