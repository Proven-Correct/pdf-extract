#!/usr/bin/env python3
"""Smoke test of an *installed* pdf-extract wheel (design D6/D7; packaging
spec "Every built wheel is verified before release", "Wheels install on the
production OS image").

Run it with the interpreter of a fresh environment the wheel was installed
into, from outside the checkout:

    python -I tools/build/smoke.py [--expect-isolation process|in_process]
                                   [--gil-off-check]

It imports nothing from the repository: the test PDF comes from
``pdf_extract.testing.PdfBuilder``, which ships in the wheel. It checks:

1. the package imported is not the development tree;
2. the OCR helper sits beside the extension and is executable;
3. text extraction returns the line written;
4. ``Page.ocr()`` with no engine (the default engine) recognises the line's
   words, and that engine runs in the expected isolation mode;
5. with ``--gil-off-check``, a free-threaded interpreter still has the GIL
   disabled after the import.

The default engine finds its English model the usual way (``TESSDATA_PREFIX``
or the known install directories), so the caller provides one: the tessdata
action in CI, the distribution's ``tesseract-ocr-eng`` in the production
image. Exit status 0 on success; on failure, the first failing check is
printed to stderr.
"""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

# The checkout this script sits in, if any: in the production-image test it
# is copied into the container on its own.
_HERE = Path(__file__).resolve()
REPO = (
    _HERE.parents[2]
    if len(_HERE.parents) > 2 and (_HERE.parents[2] / "Cargo.toml").is_file()
    else None
)
LINE = "Seven quick brown foxes jump over the lazy dog"
#: Words that must survive recognition (case-insensitive). Short and common
#: words are left out: they are the ones a model may merge or split.
WORDS = ("quick", "brown", "foxes", "jump", "lazy")


class SmokeFailure(Exception):
    pass


def check(condition: bool, message: str) -> None:
    if not condition:
        raise SmokeFailure(message)


def run(expect_isolation: str, gil_off_check: bool) -> list[str]:
    import pdf_extract  # noqa: PLC0415 - after argument parsing, on purpose
    from pdf_extract.testing import PdfBuilder  # noqa: PLC0415

    report = []
    package = Path(pdf_extract.__file__).resolve().parent
    check(
        REPO is None or package != (REPO / "python" / "pdf_extract").resolve(),
        f"imported the development tree ({package}), not an installed wheel",
    )
    report.append(f"package {pdf_extract.__version__} at {package}")

    helper = package / "pdf-extract-ocr-helper"
    check(helper.is_file(), f"no OCR helper at {helper}")
    check(os.access(helper, os.X_OK), f"OCR helper is not executable: {helper}")
    report.append("helper present and executable")

    builder = PdfBuilder()
    builder.new_page().insert_text((72, 120), LINE, fontsize=24)
    with pdf_extract.open_bytes(builder.to_bytes()) as doc:
        page = doc[0]
        text = page.text()
        check(LINE in text, f"text extraction returned {text!r}")
        report.append("text extraction ok")

        engine = pdf_extract.default_ocr_engine()
        check(
            engine.isolation == expect_isolation,
            f"default engine isolation is {engine.isolation!r}, expected {expect_isolation!r}",
        )
        recognised = page.ocr().text().lower()
        missing = [w for w in WORDS if w not in recognised]
        check(not missing, f"OCR missed {missing} in {recognised!r}")
        report.append(f"ocr ok ({engine.isolation}, tessdata {engine.info().tessdata})")

    if gil_off_check:
        check(not sys._is_gil_enabled(), "the GIL is enabled after importing pdf_extract")
        report.append("GIL stays disabled")
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Smoke-test an installed pdf-extract wheel.")
    parser.add_argument("--expect-isolation", choices=("process", "in_process"), default="process")
    parser.add_argument("--gil-off-check", action="store_true")
    args = parser.parse_args(argv)
    try:
        for line in run(args.expect_isolation, args.gil_off_check):
            print(f"smoke: {line}")
    except SmokeFailure as e:
        print(f"smoke: FAILED: {e}", file=sys.stderr)
        return 1
    print("smoke: all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
