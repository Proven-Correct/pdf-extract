"""The open contract of run spec §4.8, through Python (§4.11 item 1).

One test per contract row, with the exception classes of design spec §9.2.
The Rust twin is ``crates/core/tests/contract.rs``; the fixtures are the same
generator output (see ``conftest.py``). Warnings are compared by their text,
``"<category>: <detail>"``, which is what Python sees.
"""

from __future__ import annotations

import os
import socket
import sys
import tempfile
import threading
from collections.abc import Callable
from pathlib import Path

import pytest

import pdf_extract
from pdf_extract import (
    CorruptDocumentError,
    DocumentNotFoundError,
    EmptyFileError,
    FileTooLargeError,
    Limits,
    NotAPdfError,
    OpenError,
    PasswordRequiredError,
    PdfExtractError,
    TooManyPagesError,
    UnsupportedEncryptionError,
)

from pdfs import Fixtures, assemble_pdf, corpus_c, set_xref_offset

POSIX = sys.platform != "win32"


def has_warning(doc: pdf_extract.Document, prefix: str) -> bool:
    return any(w.startswith(prefix) for w in doc.warnings)


def write(tmp_path: Path, name: str, data: bytes) -> Path:
    path = tmp_path / name
    path.write_bytes(data)
    return path


def test_missing_path_is_document_not_found(tmp_path: Path) -> None:
    missing = tmp_path / "does-not-exist.pdf"
    with pytest.raises(DocumentNotFoundError) as info:
        pdf_extract.open(missing)
    err = info.value
    assert isinstance(err, FileNotFoundError)
    assert isinstance(err, OpenError)
    assert isinstance(err, PdfExtractError)
    assert err.filename == str(missing)
    assert err.errno == 2
    # Also catchable the way callers already handle missing files.
    with pytest.raises(FileNotFoundError):
        pdf_extract.open(str(missing))


def test_directory_is_open_error_with_os_error_cause(tmp_path: Path) -> None:
    with pytest.raises(OpenError) as info:
        pdf_extract.open(tmp_path)
    err = info.value
    assert type(err) is OpenError
    assert not isinstance(err, FileNotFoundError)
    assert "not a regular file" in str(err)
    assert str(tmp_path) in str(err)
    assert isinstance(err.__cause__, OSError)


@pytest.mark.skipif(not POSIX, reason="device files are POSIX")
def test_device_is_open_error() -> None:
    with pytest.raises(OpenError) as info:
        pdf_extract.open("/dev/null")
    assert type(info.value) is OpenError
    assert isinstance(info.value.__cause__, OSError)


@pytest.mark.skipif(not POSIX, reason="UNIX sockets and FIFOs are POSIX")
def test_socket_and_fifo_are_refused_without_blocking() -> None:
    # Short directory: socket paths are limited to ~104 bytes.
    with tempfile.TemporaryDirectory(dir="/tmp") as d:
        sock_path = os.path.join(d, "s.pdf")
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.bind(sock_path)
            with pytest.raises(OpenError, match="not a regular file"):
                pdf_extract.open(sock_path)

        fifo = os.path.join(d, "f.pdf")
        os.mkfifo(fifo)
        outcome: list[BaseException | None] = []

        def attempt() -> None:
            try:
                pdf_extract.open(fifo)
                outcome.append(None)
            except BaseException as e:
                outcome.append(e)

        t = threading.Thread(target=attempt, daemon=True)
        t.start()
        t.join(10)
        if t.is_alive():
            # Release the blocked reader so the process can exit, then fail.
            with open(fifo, "wb"):
                pass
            pytest.fail("open() blocked on a FIFO")
        assert len(outcome) == 1
        assert isinstance(outcome[0], OpenError)
        assert type(outcome[0]) is OpenError


@pytest.mark.skipif(not POSIX or os.geteuid() == 0, reason="needs a non-root POSIX user")
def test_unreadable_file_is_open_error_caused_by_permission_error(
    tmp_path: Path, fixtures: Fixtures
) -> None:
    path = write(tmp_path, "locked.pdf", fixtures.bytes("valid"))
    path.chmod(0)
    try:
        with pytest.raises(OpenError) as info:
            pdf_extract.open(path)
    finally:
        path.chmod(0o600)
    assert type(info.value) is OpenError
    assert isinstance(info.value.__cause__, PermissionError)


@pytest.mark.skipif(not POSIX, reason="symlinks")
def test_symlink_to_a_regular_file_opens(tmp_path: Path, fixtures: Fixtures) -> None:
    target = write(tmp_path, "target.pdf", fixtures.bytes("valid"))
    link = tmp_path / "link.pdf"
    link.symlink_to(target)
    assert pdf_extract.open(link).page_count == 2


def test_too_large_buffer_and_file(tmp_path: Path, fixtures: Fixtures) -> None:
    data = fixtures.bytes("valid")
    limits = Limits(max_file_size=100)
    with pytest.raises(FileTooLargeError, match=f"{len(data)} bytes"):
        pdf_extract.open_bytes(data, limits=limits)
    with pytest.raises(FileTooLargeError):
        pdf_extract.open(write(tmp_path, "big.pdf", data), limits=limits)
    # Exactly at the limit opens.
    exact = Limits(max_file_size=len(data))
    assert pdf_extract.open_bytes(data, limits=exact).page_count == 2


def test_header_straddling_byte_1024_is_not_a_pdf(fixtures: Fixtures) -> None:
    with pytest.raises(NotAPdfError):
        pdf_extract.open_bytes(fixtures.bytes("header_at_1020"))
    assert pdf_extract.open_bytes(fixtures.bytes("header_at_1019")).page_count == 1


def test_direct_kid_beyond_position_4096_is_not_indexed() -> None:
    kids = "<< /MediaBox [0 0 10 10] >> " * 4098
    data = assemble_pdf(["<< /Type /Catalog /Pages 2 0 R >>", f"<< /Type /Pages /Kids [{kids}] >>"])
    doc = pdf_extract.open_bytes(data)
    assert doc.page_count == 4097
    assert len(doc.warnings) == 1
    assert doc.warnings[0].startswith("page tree: direct kid beyond position limit")


def test_a_thousand_duplicate_leaves_coalesce_into_one_warning() -> None:
    kids = "3 0 R " * 1000
    data = assemble_pdf(
        [
            "<< /Type /Catalog /Pages 2 0 R >>",
            f"<< /Type /Pages /Kids [{kids}] /Count 1000 >>",
            "<< /Type /Page /Parent 2 0 R >>",
        ]
    )
    doc = pdf_extract.open_bytes(data)
    assert doc.page_count == 1
    assert len(doc.warnings) == 1
    assert doc.warnings[0].startswith("page tree: node revisited")
    assert doc.warnings[0].endswith(" (\u00d7999)")


def test_zero_bytes_is_empty(tmp_path: Path) -> None:
    with pytest.raises(EmptyFileError):
        pdf_extract.open(write(tmp_path, "empty.pdf", b""))
    with pytest.raises(EmptyFileError):
        pdf_extract.open_bytes(b"")


def test_header_then_garbage_is_corrupt(fixtures: Fixtures) -> None:
    with pytest.raises(CorruptDocumentError):
        pdf_extract.open_bytes(fixtures.bytes("garbage_after_header"))


def test_plain_text_named_pdf_is_not_a_pdf(tmp_path: Path, fixtures: Fixtures) -> None:
    with pytest.raises(NotAPdfError):
        pdf_extract.open(write(tmp_path, "plain.pdf", fixtures.bytes("plain_text")))


@pytest.mark.parametrize("name", ["valid.txt", "valid_noext", "valid.pdf"])
def test_valid_pdf_with_any_name_opens(tmp_path: Path, fixtures: Fixtures, name: str) -> None:
    doc = pdf_extract.open(write(tmp_path, name, fixtures.bytes("valid")))
    assert doc.page_count == 2
    assert not doc.is_encrypted
    assert doc.warnings == ()


def test_header_offset_500_opens_and_1500_is_not_a_pdf(fixtures: Fixtures) -> None:
    assert pdf_extract.open_bytes(fixtures.bytes("header_at_500")).page_count == 1
    # Q6: the oracle opens this one; F2 keeps the window at 1024 bytes.
    with pytest.raises(NotAPdfError):
        pdf_extract.open_bytes(fixtures.bytes("header_at_1500"))


@pytest.mark.parametrize("alg", ["rc4_40", "rc4_128", "aes_128", "aes_256"])
def test_encrypted_owner_opens_and_user_needs_a_password_at_open(alg: str) -> None:
    doc = pdf_extract.open(corpus_c(f"enc_{alg}_owner.pdf"))
    assert doc.is_encrypted
    assert doc.page_count == 1
    assert doc[0].text() == f"Encrypted {alg} owner\n"

    # Raised by open itself, never deferred to page access.
    with pytest.raises(PasswordRequiredError):
        pdf_extract.open(corpus_c(f"enc_{alg}_user.pdf"))
    with pytest.raises(PasswordRequiredError):
        pdf_extract.open_bytes(corpus_c(f"enc_{alg}_user.pdf").read_bytes())


@pytest.mark.parametrize(
    "name",
    [
        "broken_startxref",
        "truncated_tail_8",
        "truncated_tail_20",
        "truncated_tail_60",
        "truncated_tail_120",
        "truncated_tail_200",
    ],
)
def test_broken_xref_opens_repaired(fixtures: Fixtures, name: str) -> None:
    doc = pdf_extract.open_bytes(fixtures.bytes(name))
    assert doc.page_count == 1
    assert "xref: repaired" in doc.warnings


def test_wrong_offset_of_a_page_tree_node_is_repaired_during_the_walk(
    fixtures: Fixtures,
) -> None:
    # tree_flat_2: 1 font, 2 catalog, 3 root, then content/leaf pairs; leaf 1
    # is object 5. Point it at the catalog.
    intact = fixtures.bytes("tree_flat_2")
    catalog_at = intact.index(b"\n2 0 obj") + 1
    doc = pdf_extract.open_bytes(set_xref_offset(intact, 5, catalog_at))
    assert doc.page_count == 2
    assert "xref: repaired" in doc.warnings


def test_catalog_without_pages_is_corrupt(fixtures: Fixtures) -> None:
    with pytest.raises(CorruptDocumentError):
        pdf_extract.open_bytes(fixtures.bytes("catalog_without_pages"))


def test_empty_kids_has_no_pages(fixtures: Fixtures) -> None:
    for name in ["empty_kids", "deep_page_tree_0"]:
        doc = pdf_extract.open_bytes(fixtures.bytes(name))
        assert doc.page_count == 0
        assert len(doc) == 0
        assert list(doc) == []


def test_lying_count_is_ignored(fixtures: Fixtures) -> None:
    assert pdf_extract.open_bytes(fixtures.bytes("tree_lying_count_5")).page_count == 100


@pytest.mark.parametrize("name", ["tree_duplicate_leaf", "tree_cycle"])
def test_duplicate_leaf_and_cycle_yield_one_page_with_a_warning(
    fixtures: Fixtures, name: str
) -> None:
    doc = pdf_extract.open_bytes(fixtures.bytes(name))
    assert doc.page_count == 1
    assert doc.warnings[0].startswith("page tree: node revisited")


@pytest.mark.parametrize("name", ["tree_typeless_leaf", "tree_direct_dict_kid"])
def test_typeless_leaf_and_direct_dict_kid_are_pages(fixtures: Fixtures, name: str) -> None:
    doc = pdf_extract.open_bytes(fixtures.bytes(name))
    assert doc.page_count == 2
    assert doc.warnings == ()


def test_deep_page_tree_200_has_its_page(fixtures: Fixtures) -> None:
    doc = pdf_extract.open_bytes(fixtures.bytes("deep_page_tree_200"))
    assert doc.page_count == 1
    assert doc.warnings == ()


def test_deep_page_tree_3000_is_cut_at_the_depth_limit(fixtures: Fixtures) -> None:
    doc = pdf_extract.open_bytes(fixtures.bytes("deep_page_tree_3000"))
    assert doc.page_count == 0  # KP23: the oracle finds the page
    assert any(
        w.startswith("page tree: node") and "deeper than 256 skipped" in w for w in doc.warnings
    ), doc.warnings
    assert not has_warning(doc, "page tree: brute-force")


def test_depth_limit_is_configurable_and_inclusive(fixtures: Fixtures) -> None:
    limits = Limits(max_page_tree_depth=10)
    assert (
        pdf_extract.open_bytes(fixtures.bytes("deep_page_tree_10"), limits=limits).page_count == 1
    )
    assert (
        pdf_extract.open_bytes(fixtures.bytes("deep_page_tree_11"), limits=limits).page_count == 0
    )


def test_max_pages_stops_the_walk(fixtures: Fixtures) -> None:
    data = fixtures.bytes("tree_fanout_100x2")
    with pytest.raises(TooManyPagesError) as info:
        pdf_extract.open_bytes(data, max_pages=10)
    assert (info.value.limit, info.value.at_least) == (10, 11)
    assert isinstance(info.value, OpenError)
    # Exactly at the limit opens; no limit opens all 10,000.
    assert pdf_extract.open_bytes(fixtures.bytes("tree_flat_10"), max_pages=10).page_count == 10
    assert pdf_extract.open_bytes(data).page_count == 10_000
    with pytest.raises(TooManyPagesError):
        pdf_extract.open(fixtures.path("tree_flat_10"), max_pages=9)


def test_max_objects_is_a_corrupt_document_naming_the_limit(fixtures: Fixtures) -> None:
    for name in ["valid", "tree_flat_3"]:
        with pytest.raises(CorruptDocumentError, match="objects"):
            pdf_extract.open_bytes(fixtures.bytes(name), limits=Limits(max_objects=3))


def test_max_objects_is_rechecked_after_a_repair_during_the_walk() -> None:
    bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R >>",
        *(f"<< /Filler {i} >>" for i in range(200)),
    ]
    full = assemble_pdf(bodies)
    # Keep the free head + objects 1-3, with object 3 pointing at object 1,
    # so the walk's lookup of the leaf repairs the table from a scan.
    xref_at = full.rindex(b"\nxref\n") + 1
    entries = full.index(b"\n", full.index(b"\n", xref_at) + 1) + 1
    entry = [full[entries + 20 * n : entries + 20 * n + 20] for n in range(3)]
    table = b"xref\n0 4\n" + entry[0] + entry[1] + entry[2] + entry[1]
    data = (
        full[:xref_at]
        + table
        + f"trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n".encode()
    )
    assert "xref: repaired" in pdf_extract.open_bytes(data).warnings
    with pytest.raises(CorruptDocumentError, match="objects"):
        pdf_extract.open_bytes(data, limits=Limits(max_objects=10))


def test_deep_nesting_in_the_page_dictionary_opens_without_pages(fixtures: Fixtures) -> None:
    doc = pdf_extract.open_bytes(fixtures.bytes("deep_nesting_in_page_dict_200000"))
    assert doc.page_count == 0
    assert has_warning(doc, "page tree: node unreadable"), doc.warnings
    assert has_warning(doc, "object: nesting limit"), doc.warnings


def test_dangling_pages_reference_opens_without_pages() -> None:
    data = assemble_pdf(["<< /Type /Catalog /Pages 9 0 R >>", "<< /Producer (x) >>"])
    doc = pdf_extract.open_bytes(data)
    assert doc.page_count == 0
    assert has_warning(doc, "page tree: node unreadable"), doc.warnings
    assert has_warning(doc, "page tree: brute-force scan"), doc.warnings


def test_page_out_of_range_is_index_error(fixtures: Fixtures) -> None:
    doc = pdf_extract.open_bytes(fixtures.bytes("valid"))
    assert doc[1].index == 1
    with pytest.raises(IndexError, match="out of range"):
        doc[2]
    with pytest.raises(IndexError):
        doc[2**70]


def test_every_open_error_is_a_pdf_extract_error(tmp_path: Path, fixtures: Fixtures) -> None:
    attempts: list[Callable[[], object]] = [
        lambda: pdf_extract.open(tmp_path / "missing.pdf"),
        lambda: pdf_extract.open(tmp_path),
        lambda: pdf_extract.open_bytes(b""),
        lambda: pdf_extract.open_bytes(fixtures.bytes("plain_text")),
        lambda: pdf_extract.open_bytes(fixtures.bytes("garbage_after_header")),
        lambda: pdf_extract.open(corpus_c("enc_aes_256_user.pdf")),
        lambda: pdf_extract.open_bytes(fixtures.bytes("valid"), limits=Limits(max_file_size=1)),
        lambda: pdf_extract.open_bytes(fixtures.bytes("tree_flat_10"), max_pages=1),
    ]
    for attempt in attempts:
        with pytest.raises(PdfExtractError):
            attempt()


def test_unsupported_encryption_is_its_own_error() -> None:
    data = assemble_pdf(
        [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] >>",
            "<< /Filter /Standard /V 9 /R 9 /O <00> /U <00> /P -4 >>",
        ]
    ).replace(b"/Root 1 0 R >>", b"/Root 1 0 R /Encrypt 4 0 R /ID [<01> <01>] >>")
    with pytest.raises(UnsupportedEncryptionError, match="not supported") as info:
        pdf_extract.open_bytes(data)
    assert isinstance(info.value, OpenError)


@pytest.mark.skipif(not POSIX, reason="POSIX errno and path semantics")
def test_os_error_cause_carries_errno_and_filename(tmp_path: Path, fixtures: Fixtures) -> None:
    regular = write(tmp_path, "file.pdf", fixtures.bytes("valid"))
    through = regular / "x.pdf"  # a path through a regular file: ENOTDIR
    with pytest.raises(OpenError) as info:
        pdf_extract.open(through)
    cause = info.value.__cause__
    assert isinstance(cause, NotADirectoryError)
    assert cause.errno is not None
    assert cause.strerror is not None
    assert "os error" not in cause.strerror
    assert cause.filename == str(through)


def test_not_found_filename_round_trips_surrogate_escapes_everywhere(tmp_path: Path) -> None:
    # The path does not exist, so no filesystem ever sees the odd byte.
    missing = str(tmp_path / "gone-\udcff.pdf") if POSIX else str(tmp_path / "gone.pdf")
    with pytest.raises(DocumentNotFoundError) as info:
        pdf_extract.open(missing)
    assert info.value.filename == missing
