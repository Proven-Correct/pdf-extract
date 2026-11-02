"""The Python test kit, ``pdf_extract.testing`` (run spec T49, §3.5, §4.12
item 1; design spec §12.3).

The golden test replays one call script (``builder_golden.tsv``, shared with
the Rust example that writes the fixtures) through ``pdf_extract.testing``
and requires the same bytes and the same ``text_width`` bits as the Rust test
kit. The remaining tests pin the Python-facing contract: PyMuPDF's
coordinate convention, ``ValueError`` for anything the kit refuses, ``save``
and its ``OSError``s, and sharing one builder between threads.
"""

from __future__ import annotations

import struct
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

import pytest

import pdf_extract
from pdf_extract import testing
from pdf_extract.testing import PageBuilder, PdfBuilder, text_width

from pdfs import GOLDEN_SCRIPT, Fixtures, run_threads


@dataclass
class GoldenScript:
    """``builder_golden.tsv``, parsed: documents built with the Python kit
    and the expected ``text_width`` rows."""

    docs: dict[str, PdfBuilder] = field(default_factory=dict)
    widths: list[tuple[str, float, str]] = field(default_factory=list)


def replay_golden_script() -> GoldenScript:
    """Replays the shared script through ``pdf_extract.testing``.

    Mirrors ``golden()`` in ``crates/testkit/examples/pytest_fixtures.rs``:
    the text is the last tab-separated field and may be absent when empty;
    numbers are parsed as ``float`` and narrowed by the binding exactly as
    the Rust side narrows its ``f64``.
    """
    script = GoldenScript()
    current: PdfBuilder | None = None
    page: PageBuilder | None = None
    for lineno, raw in enumerate(GOLDEN_SCRIPT.read_text(encoding="utf-8").splitlines(), 1):
        if not raw or raw.startswith("#"):
            continue
        fields = raw.split("\t", 5)
        match fields:
            case ["doc", name]:
                current = script.docs[name] = PdfBuilder()
                page = None
            case ["page"]:
                assert current is not None, lineno
                page = current.new_page()
            case ["page", w, h]:
                assert current is not None, lineno
                page = current.new_page(float(w), float(h))
            case ["text", x, y, size, mode, *rest]:
                assert page is not None, lineno
                text = rest[0] if rest else ""
                page.insert_text(
                    (float(x), float(y)), text, fontsize=float(size), render_mode=int(mode)
                )
            case ["width", size, *rest]:
                text = rest[0] if rest else ""
                script.widths.append((size, text_width(text, float(size)), text))
            case _:
                pytest.fail(f"builder_golden.tsv:{lineno}: cannot parse {raw!r}")
    return script


def test_golden_documents_are_byte_identical_to_the_rust_test_kit(fixtures: Fixtures) -> None:
    script = replay_golden_script()
    rust_docs = sorted(p.stem for p in (fixtures.root / "golden").glob("*.pdf"))
    assert sorted(script.docs) == rust_docs
    assert len(rust_docs) >= 5
    for name, builder in script.docs.items():
        assert builder.to_bytes() == fixtures.bytes(f"golden/{name}"), name


def test_golden_text_widths_match_the_rust_test_kit_bit_for_bit(fixtures: Fixtures) -> None:
    script = replay_golden_script()
    rust_rows = (fixtures.root / "golden" / "widths.tsv").read_text(encoding="utf-8").splitlines()
    assert len(rust_rows) == len(script.widths) >= 5
    for row, (size, width, text) in zip(rust_rows, script.widths, strict=True):
        rust_size, rust_bits, *rest = row.split("\t", 2)
        assert (rust_size, rest[0] if rest else "") == (size, text)
        # The binding returns the kit's f32 widened to a float, so packing
        # it back into an f32 is exact.
        assert struct.pack(">f", width).hex() == rust_bits, (size, text)


def single_page(*runs: tuple[tuple[float, float], str]) -> str:
    """Builds one A4 page with ``runs`` at 11 pt and returns its text."""
    builder = PdfBuilder()
    page = builder.new_page()
    for point, text in runs:
        page.insert_text(point, text)
    return pdf_extract.open_bytes(builder.to_bytes())[0].text()


@pytest.mark.parametrize(("height", "pdf_y"), [(842, b"742"), (200, b"100"), (100.5, b"0.5")])
def test_origin_is_top_left_with_y_growing_downwards(height: float, pdf_y: bytes) -> None:
    # (72, 100) from the top-left is PDF user-space y = height - 100.
    builder = PdfBuilder()
    builder.new_page(300, height).insert_text((72, 100), "x")
    assert b" 1 0 0 1 72 " + pdf_y + b" Tm " in builder.to_bytes()


def test_runs_on_one_baseline_form_one_line() -> None:
    # Extraction follows content order; y decides only which runs share a
    # line (a 0.5 em gap is a space, not a break).
    right = 72 + text_width("left") + 0.5 * 11
    assert single_page(((72, 100), "left"), ((right, 100), "right")) == "left right\n"
    assert single_page(((72, 300), "lower"), ((72, 100), "upper")) == "lower\nupper\n"


def test_text_width_places_runs_exactly() -> None:
    # The space rule (strict > 0.149 em) sees exactly the gap text_width
    # leaves: 0.1 em joins, 0.3 em gets a space.
    end = 72 + text_width("Alpha")
    assert single_page(((72, 100), "Alpha"), ((end + 0.1 * 11, 100), "Beta")) == "AlphaBeta\n"
    assert single_page(((72, 100), "Alpha"), ((end + 0.3 * 11, 100), "Beta")) == "Alpha Beta\n"


def test_text_width_defaults_and_scaling() -> None:
    assert text_width("Hello world") == text_width("Hello world", 11)
    assert text_width("Hello world", 22) == pytest.approx(2 * text_width("Hello world"))
    assert text_width("") == 0.0
    # AFM: 'W' is 944 units.
    assert text_width("W", 1000) == 944.0


def test_winansi_text_round_trips() -> None:
    text = "Café — “quoted” €5 ½ ß ÿ Ÿ œ ‰"
    assert single_page(((40, 100), text)) == text + "\n"


@pytest.mark.parametrize("text", ["日本", "tab\there", "line\nbreak", "\x00", "Ā"])
def test_characters_outside_winansi_raise_value_error_and_draw_nothing(text: str) -> None:
    builder = PdfBuilder()
    page = builder.new_page()
    before = builder.to_bytes()
    with pytest.raises(ValueError, match="WinAnsiEncoding"):
        page.insert_text((72, 72), text)
    assert builder.to_bytes() == before
    with pytest.raises(ValueError, match="WinAnsiEncoding"):
        text_width(text)


@pytest.mark.parametrize("mode", [-1, 8, 255, 256, 2**70])
def test_render_mode_outside_0_to_7_raises_value_error(mode: int) -> None:
    page = PdfBuilder().new_page()
    with pytest.raises(ValueError, match=f"text rendering mode {mode} is outside 0-7"):
        page.insert_text((72, 72), "x", render_mode=mode)


@pytest.mark.parametrize("mode", range(8))
def test_every_render_mode_is_accepted(mode: int) -> None:
    builder = PdfBuilder()
    builder.new_page().insert_text((72, 72), "Mode", render_mode=mode)
    assert f" {mode} Tr ".encode() in builder.to_bytes()


INF = float("inf")
NAN = float("nan")


@pytest.mark.parametrize(
    ("point", "fontsize"),
    [((NAN, 72), 11), ((72, INF), 11), ((72, 72), NAN), ((72, 72), -INF), ((1e39, 72), 11)],
)
def test_non_finite_numbers_raise_value_error(point: tuple[float, float], fontsize: float) -> None:
    # 1e39 is finite as a float but not as the kit's f32.
    with pytest.raises(ValueError, match="finite"):
        PdfBuilder().new_page().insert_text(point, "x", fontsize=fontsize)


def test_text_width_rejects_a_non_finite_size() -> None:
    with pytest.raises(ValueError, match="finite"):
        text_width("x", NAN)


@pytest.mark.parametrize(("width", "height"), [(NAN, 842), (595, INF), (1e39, 842)])
def test_non_finite_page_size_raises_value_error(width: float, height: float) -> None:
    builder = PdfBuilder()
    with pytest.raises(ValueError, match="page size must be finite"):
        builder.new_page(width, height)
    assert repr(builder) == "<pdf_extract.testing.PdfBuilder pages=0>"


def test_wrong_argument_types_raise_type_error() -> None:
    page = PdfBuilder().new_page()
    with pytest.raises(TypeError):
        page.insert_text([72, 72], "x")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        page.insert_text((72, 72), b"x")  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        page.insert_text((72, 72), "x", 11)  # type: ignore[call-arg]
    with pytest.raises(TypeError):
        PageBuilder()


def test_page_builders_write_to_their_own_page() -> None:
    builder = PdfBuilder()
    first = builder.new_page()
    second = builder.new_page(612, 792)
    second.insert_text((72, 72), "second")
    first.insert_text((72, 72), "first")
    doc = pdf_extract.open_bytes(builder.to_bytes())
    assert [p.text() for p in doc] == ["first\n", "second\n"]
    assert repr(second) == "<pdf_extract.testing.PageBuilder index=1>"
    assert repr(builder) == "<pdf_extract.testing.PdfBuilder pages=2>"


def test_a_page_builder_keeps_its_document_alive() -> None:
    page = PdfBuilder().new_page()
    page.insert_text((72, 72), "still here")  # the builder object is gone


def test_empty_builder_is_a_document_without_pages() -> None:
    assert pdf_extract.open_bytes(PdfBuilder().to_bytes()).page_count == 0


def test_output_is_deterministic() -> None:
    def build() -> bytes:
        builder = PdfBuilder()
        builder.new_page().insert_text((72, 72), "Same", fontsize=9.5, render_mode=3)
        return builder.to_bytes()

    assert build() == build()


def test_save_writes_to_bytes(tmp_path: Path) -> None:
    builder = PdfBuilder()
    builder.new_page().insert_text((72, 72), "saved")
    builder.save(tmp_path / "a.pdf")
    builder.save(str(tmp_path / "b.pdf"))
    assert (tmp_path / "a.pdf").read_bytes() == builder.to_bytes()
    assert (tmp_path / "b.pdf").read_bytes() == builder.to_bytes()
    assert pdf_extract.open(tmp_path / "a.pdf")[0].text() == "saved\n"


def test_save_raises_the_matching_os_error(tmp_path: Path) -> None:
    missing = tmp_path / "no-such-dir" / "a.pdf"
    with pytest.raises(FileNotFoundError) as err:
        PdfBuilder().save(missing)
    assert err.value.filename == str(missing)
    with pytest.raises(IsADirectoryError):
        PdfBuilder().save(tmp_path)


def test_one_builder_shared_between_threads() -> None:
    # Frozen classes over a mutex: concurrent new_page/insert_text calls
    # must neither lose nor mix up runs. Each thread owns its pages.
    builder = PdfBuilder()

    def work(t: int) -> None:
        for k in range(25):
            builder.new_page().insert_text((72, 72), f"T{t} P{k}")

    run_threads(8, work)
    doc = pdf_extract.open_bytes(builder.to_bytes())
    texts = sorted(page.text() for page in doc)
    assert texts == sorted(f"T{t} P{k}\n" for t in range(8) for k in range(25))


def test_module_surface() -> None:
    assert testing.__all__ == ["PageBuilder", "PdfBuilder", "text_width"]
    assert PdfBuilder.__module__ == "pdf_extract.testing"
    assert PageBuilder.__module__ == "pdf_extract.testing"


def test_importing_pdf_extract_does_not_load_the_test_kit() -> None:
    code = "import sys, pdf_extract; print('pdf_extract.testing' in sys.modules)"
    out = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, check=True)
    assert out.stdout.strip() == "False"
