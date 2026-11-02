"""The exception hierarchy and the panic boundary (run spec T44, T45; §4.11
items 2 and 3; design spec §9.2)."""

from __future__ import annotations

import inspect
import pickle
import subprocess
import sys
import textwrap
import threading

import pytest

import pdf_extract
from pdf_extract import _native, errors

from pdfs import ccitt_image_pdf


def all_classes() -> list[type[BaseException]]:
    out = []
    for name in errors.__all__:
        obj = getattr(errors, name)
        assert inspect.isclass(obj), name
        assert issubclass(obj, BaseException), name
        out.append(obj)
    return out


def test_all_lists_every_class_defined_in_the_module() -> None:
    defined = {
        name
        for name, obj in vars(errors).items()
        if inspect.isclass(obj) and obj.__module__ == errors.__name__
    }
    assert defined == set(errors.__all__)
    assert len(errors.__all__) == 18


def test_only_ocr_unavailable_is_a_runtime_error() -> None:
    runtime = [cls for cls in all_classes() if issubclass(cls, RuntimeError)]
    assert runtime == [errors.OcrUnavailableError]


def test_every_class_is_an_exception_under_pdf_extract_error() -> None:
    for cls in all_classes():
        assert issubclass(cls, Exception), cls
        assert issubclass(cls, errors.PdfExtractError), cls


def test_internal_error_is_not_base_exception_only() -> None:
    assert issubclass(errors.InternalError, Exception)
    assert not issubclass(errors.InternalError, (KeyboardInterrupt, SystemExit, GeneratorExit))


@pytest.mark.parametrize(
    ("cls", "bases"),
    [
        (errors.OpenError, (errors.PdfExtractError,)),
        (errors.DocumentNotFoundError, (errors.OpenError, FileNotFoundError)),
        (errors.EmptyFileError, (errors.OpenError,)),
        (errors.NotAPdfError, (errors.OpenError,)),
        (errors.CorruptDocumentError, (errors.OpenError,)),
        (errors.PasswordRequiredError, (errors.OpenError,)),
        (errors.UnsupportedEncryptionError, (errors.OpenError,)),
        (errors.FileTooLargeError, (errors.OpenError,)),
        (errors.TooManyPagesError, (errors.OpenError,)),
        (errors.DocumentClosedError, (errors.PdfExtractError,)),
        (errors.PageError, (errors.PdfExtractError,)),
        (errors.LimitExceededError, (errors.PageError,)),
        (errors.OcrError, (errors.PdfExtractError,)),
        (errors.OcrUnavailableError, (errors.OcrError, RuntimeError)),
        (errors.OcrPageError, (errors.OcrError,)),
        (errors.OcrTimeoutError, (errors.OcrPageError,)),
        (errors.InternalError, (errors.PdfExtractError,)),
    ],
)
def test_tree_matches_the_design_spec(
    cls: type[BaseException], bases: tuple[type[BaseException], ...]
) -> None:
    assert cls.__bases__ == bases


def test_ocr_page_errors_are_not_runtime_errors() -> None:
    assert not issubclass(errors.OcrPageError, RuntimeError)
    assert not issubclass(errors.OcrTimeoutError, RuntimeError)


def test_package_reexports_every_error() -> None:
    for name in errors.__all__:
        assert getattr(pdf_extract, name) is getattr(errors, name)
        assert name in pdf_extract.__all__


def test_attribute_carrying_errors_pickle_round_trip() -> None:
    cases: list[BaseException] = [
        errors.TooManyPagesError("too many", 10, 11),
        errors.LimitExceededError("over", "ops_per_page", 200_000),
        errors.OcrUnavailableError(
            "no tessdata",
            reason="no_tessdata",
            tessdata="/x",
            languages=["eng"],
            searched=["/a", "/b"],
        ),
        errors.OcrPageError("render failed"),
        errors.OcrPageError("over", "image_pixels", 5),
        errors.OcrTimeoutError("too slow", 60.0),
        errors.DocumentNotFoundError(2, "No such file or directory", "/missing.pdf"),
        errors.InternalError("boom"),
    ]
    for err in cases:
        copy = pickle.loads(pickle.dumps(err))
        assert type(copy) is type(err)
        assert str(copy) == str(err)
        assert vars(copy) == vars(err)
    too_many = errors.TooManyPagesError("too many", 10, 11)
    assert (too_many.limit, too_many.at_least, str(too_many)) == (10, 11, "too many")
    ocr = errors.OcrUnavailableError("x")
    assert (ocr.reason, ocr.tessdata, ocr.languages, ocr.searched) == ("unknown", None, (), ())


def test_ocr_page_errors_carry_their_attributes() -> None:
    limit = errors.OcrPageError("m", "image_pixels", 5)
    assert (str(limit), limit.limit_name, limit.value) == ("m", "image_pixels", 5)
    plain = errors.OcrPageError("m")
    assert (plain.limit_name, plain.value) == (None, None)
    timeout = errors.OcrTimeoutError("m", 60.0)
    assert (str(timeout), timeout.timeout) == ("m", 60.0)
    assert (timeout.limit_name, timeout.value) == (None, None)
    assert isinstance(timeout, errors.OcrPageError)
    copy = pickle.loads(pickle.dumps(timeout))
    assert type(copy) is errors.OcrTimeoutError
    assert (str(copy), copy.timeout, copy.limit_name, copy.value) == ("m", 60.0, None, None)
    copy_limit = pickle.loads(pickle.dumps(limit))
    assert (copy_limit.limit_name, copy_limit.value) == ("image_pixels", 5)


def test_panic_becomes_internal_error() -> None:
    with pytest.raises(pdf_extract.InternalError) as info:
        _native._panic_for_test()
    assert "deliberate panic for testing" in str(info.value)
    assert "src/lib.rs" in str(info.value)  # the hook records the location


def test_panic_while_detached_becomes_internal_error() -> None:
    with pytest.raises(
        pdf_extract.InternalError, match=r"deliberate panic for testing \(detached\)"
    ):
        _native._panic_for_test(detached=True)


def test_panic_is_caught_by_except_exception_and_is_never_panic_exception() -> None:
    caught: type[BaseException] | None = None
    try:
        _native._panic_for_test()
    except Exception:
        exc_type = sys.exc_info()[0]
        assert exc_type is not None
        caught = exc_type
    assert caught is pdf_extract.InternalError
    assert "PanicException" not in {cls.__name__ for cls in caught.__mro__}


def test_panic_in_a_worker_thread_does_not_kill_it() -> None:
    results: list[str] = []

    def worker() -> None:
        for _ in range(3):
            try:
                _native._panic_for_test(detached=True)
            except Exception as e:
                results.append(type(e).__name__)
        results.append("alive")

    t = threading.Thread(target=worker)
    t.start()
    t.join()
    assert results == ["InternalError", "InternalError", "InternalError", "alive"]


def scripted_ocr_of_codec_page(*, armed: bool) -> list[str]:
    """OCRs the CCITT page (rendered for real, recognition scripted) with the
    codec panic hook armed or not, and returns the page's warnings."""
    engine = _native._scripted_engine(default=("text", "scripted"))
    page = pdf_extract.open_bytes(ccitt_image_pdf())[0]
    if armed:
        _native._arm_codec_panic_for_test()
    return list(page.ocr(engine).warnings)


def test_a_codec_panic_during_ocr_is_an_image_warning() -> None:
    # internal #76: caught at the codec boundary, so no InternalError.
    warnings = scripted_ocr_of_codec_page(armed=True)
    decode_failures = [w for w in warnings if w.startswith("image: decode failed")]
    # Exactly one warning for the one image, reported once: no repeat-count
    # suffix (" (\N{MULTIPLICATION SIGN}N)").
    assert len(decode_failures) == 1, warnings
    assert "\N{MULTIPLICATION SIGN}" not in decode_failures[0], warnings


def test_without_the_codec_hook_the_page_has_no_image_warning() -> None:
    assert scripted_ocr_of_codec_page(armed=False) == []


@pytest.mark.parametrize("resumed", [False, True])
def test_a_panic_after_a_caught_codec_panic_carries_its_own_message(resumed: bool) -> None:
    # `resumed` skips the panic hook (as a worker's panic re-raised by
    # `extract` does), so only the guard's own bookkeeping keeps the caught
    # codec panic's recorded message out of the error.
    scripted_ocr_of_codec_page(armed=True)
    with pytest.raises(pdf_extract.InternalError) as info:
        _native._panic_for_test(resumed=resumed)
    assert "deliberate panic for testing" in str(info.value)
    assert "codec panic injected" not in str(info.value)


def test_a_resumed_panic_reports_its_payload() -> None:
    with pytest.raises(
        pdf_extract.InternalError, match=r"deliberate panic for testing \(resumed\)"
    ):
        _native._panic_for_test(resumed=True)


def test_exception_classes_are_cached_at_import() -> None:
    # Deleting (or breaking) pdf_extract.errors after import must not turn a
    # raise into a process abort: the classes were captured at import.
    code = textwrap.dedent(
        """
        import sys
        import pdf_extract
        import pdf_extract.errors as e
        cls, internal = e.EmptyFileError, e.InternalError
        del e.EmptyFileError, e.InternalError
        sys.modules["pdf_extract.errors"] = None
        try:
            pdf_extract.open_bytes(b"")
        except cls:
            pass
        try:
            pdf_extract._native._panic_for_test()
        except internal:
            pass
        print("ok")
        """
    )
    out = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, check=False)
    assert out.returncode == 0, out.stderr
    assert out.stdout == "ok\n"
    assert out.stderr == ""
