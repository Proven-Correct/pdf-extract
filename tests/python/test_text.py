"""Text extraction through Python: output shape (design spec §13.5; run spec
§4.11 item 6), options, TextPage, and the hostile pages of §4.10 (outcome
class and warning kind)."""

from __future__ import annotations

import time

import pytest

import pdf_extract
from pdf_extract import LimitExceededError, Limits, PageError

from pdfs import Fixtures


def shape_ok(text: str) -> None:
    """The compat shape: empty, or every line ends in a newline and no line
    is empty (no fixture here maps a glyph to LF)."""
    if text:
        assert text.endswith("\n"), repr(text)
        assert "\n\n" not in text, repr(text)


def test_output_shape_over_every_builder_sample(fixtures: Fixtures) -> None:
    samples = [path for path, group in fixtures.index() if group == "sample-builder"]
    assert len(samples) >= 5
    for path in samples:
        doc = pdf_extract.open(path)
        for page in doc:
            text = page.text()
            shape_ok(text)
            assert text == page.text_page().text()
            blank = page.text(block_separator="blank_line")
            # Blank-line mode only adds newlines between blocks.
            assert blank.replace("\n", "") == text.replace("\n", "")
            assert blank.count("\n") >= text.count("\n")


def test_empty_page_is_empty_string(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(fixtures.path("empty_page"))
    assert doc[0].text() == ""
    assert doc[0].text_page().text(block_separator="blank_line") == ""


def test_two_blocks(fixtures: Fixtures) -> None:
    page = pdf_extract.open(fixtures.path("two_blocks"))[0]
    assert page.text() == "Line one.\nline two.\nBlock two.\n"
    assert page.text(block_separator="newline") == "Line one.\nline two.\nBlock two.\n"
    assert page.text(block_separator="blank_line") == "Line one.\nline two.\n\nBlock two.\n"
    tp = page.text_page()
    assert tp.text() == "Line one.\nline two.\nBlock two.\n"
    assert tp.text(block_separator="blank_line") == "Line one.\nline two.\n\nBlock two.\n"
    assert tp.source == "content"
    assert tp.dpi is None
    assert tp.warnings == ()


def test_unmapped_options_are_accepted(fixtures: Fixtures) -> None:
    page = pdf_extract.open(fixtures.path("valid"))[0]
    for unmapped in ("cid", "replacement", "skip"):
        assert page.text(unmapped=unmapped) == "One\n"
        assert page.text_page(unmapped=unmapped).text() == "One\n"


def test_unmapped_policies_reach_the_extractor(fixtures: Fixtures) -> None:
    # builder_type3 shows codes 41 42 00 01 09 20 0A C8 in a Type 3 font
    # whose glyph names (g65, square, g0, ...) do not map, except `space`.
    page = pdf_extract.open(fixtures.path("samples/builder_type3"))[0]
    # Raw codes, with NUL and C0 other than TAB/LF/CR made U+FFFD (KP6, D-10);
    # code 10 is a raw LF, which ends the line.
    assert page.text(unmapped="cid") == "AB\ufffd\ufffd\t \n\u00c8\n"
    assert page.text() == page.text(unmapped="cid")
    assert page.text(unmapped="replacement") == "\ufffd" * 5 + " " + "\ufffd" * 2 + "\n"
    assert page.text_page(unmapped="skip").text() == " \n"


@pytest.mark.parametrize(
    ("kwargs", "match"),
    [
        ({"block_separator": "blank"}, "block_separator must be 'newline' or 'blank_line'"),
        ({"unmapped": "raw"}, "unmapped must be 'cid', 'replacement' or 'skip'"),
    ],
)
def test_bad_option_values_are_value_errors(
    fixtures: Fixtures, kwargs: dict[str, str], match: str
) -> None:
    page = pdf_extract.open(fixtures.path("valid"))[0]
    with pytest.raises(ValueError, match=match):
        page.text(**kwargs)  # type: ignore[arg-type]


def test_bad_option_types_and_positional_use_are_type_errors(fixtures: Fixtures) -> None:
    page = pdf_extract.open(fixtures.path("valid"))[0]
    with pytest.raises(TypeError):
        page.text(block_separator=1)  # type: ignore[arg-type]
    with pytest.raises(TypeError):
        page.text("newline")  # type: ignore[call-arg]
    with pytest.raises(ValueError, match="unmapped"):
        page.text_page(unmapped="bogus")  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="block_separator"):
        page.text_page().text(block_separator="x")  # type: ignore[arg-type]


# --- Hostile pages (§4.10 rows, outcome class + warning kind) -------------


def warnings_of(
    fixtures: Fixtures, name: str, limits: Limits | None = None
) -> tuple[str, tuple[str, ...]]:
    page = pdf_extract.open(fixtures.path(name), limits=limits)[0]
    tp = page.text_page()
    return tp.text(), tp.warnings


def test_deep_content_keeps_before(fixtures: Fixtures) -> None:
    text, warnings = warnings_of(fixtures, "deep_nesting_in_content_200000")
    assert text == "Before\n"
    assert any(w.startswith("object: nesting limit") for w in warnings), warnings


@pytest.mark.parametrize(
    ("name", "expected"),
    [
        ("deep_nesting_in_resources_200000", "Res\n"),
        ("deep_nesting_in_type4_function_200000", "Fn\n"),
    ],
)
def test_deep_resources_and_functions(fixtures: Fixtures, name: str, expected: str) -> None:
    text, warnings = warnings_of(fixtures, name)
    assert text == expected
    assert any(w.startswith("object: nesting limit") for w in warnings), warnings


def test_flate_bomb_over_the_stream_limit(fixtures: Fixtures) -> None:
    text, warnings = warnings_of(
        fixtures, "flate_bomb_50mib", Limits(max_stream_decoded_bytes=16 * 2**20)
    )
    assert text == ""
    assert any(w.startswith("stream: decoded size limit") for w in warnings), warnings


def test_form_self_recursion_and_nesting_cap(fixtures: Fixtures) -> None:
    text, warnings = warnings_of(fixtures, "form_self_recursion")
    assert text == "Rec\n"
    assert any(w.startswith("form:") for w in warnings), warnings

    text, warnings = warnings_of(fixtures, "nested_forms_60")
    assert text.splitlines() == [f"L{i}" for i in range(1, 51)]
    assert any(w.startswith("interpreter:") for w in warnings), warnings

    text, warnings = warnings_of(fixtures, "nested_forms_49")
    assert len(text.splitlines()) == 49
    assert warnings == ()


def test_operator_cap_raises_limit_exceeded(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(
        fixtures.path("operator_flood_400000"), limits=Limits(max_ops_per_page=200_000)
    )
    with pytest.raises(LimitExceededError) as info:
        doc[0].text()
    err = info.value
    assert isinstance(err, PageError)
    assert err.limit_name == "ops_per_page"
    assert err.value == 200_000
    with pytest.raises(LimitExceededError):
        doc[0].text_page()
    # Without the cap the same page extracts.
    assert pdf_extract.open(fixtures.path("operator_flood_400000"))[0].text() == "Start\nEnd\n"


def test_time_budget_raises_limit_exceeded_promptly(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(fixtures.path("form_fanout_22"), limits=Limits(page_time_budget=0.5))
    start = time.monotonic()
    with pytest.raises(LimitExceededError) as info:
        doc[0].text()
    elapsed = time.monotonic() - start
    assert info.value.limit_name == "page_time_budget"
    assert info.value.value == 500  # milliseconds
    assert elapsed < 0.5 + 2.0, elapsed


def test_character_cap_returns_a_partial_page(fixtures: Fixtures) -> None:
    text, warnings = warnings_of(fixtures, "valid", Limits(max_chars_per_page=1))
    assert text == "O\n"
    assert any(w.startswith("text: character limit reached") for w in warnings), warnings


def test_lazy_repair_is_reported_on_the_document_not_the_page(fixtures: Fixtures) -> None:
    doc = pdf_extract.open(fixtures.path("misoffset_40_6_10"))
    assert not doc.warnings
    pages = [p.text_page() for p in doc]
    assert [tp.text() for tp in pages] == [f"Page {i}\n" for i in range(1, 41)]
    assert doc.warnings == ("xref: repaired",)
    assert all("xref" not in w for tp in pages for w in tp.warnings)
