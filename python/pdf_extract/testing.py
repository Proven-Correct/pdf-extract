"""pdf_extract.testing: build small PDFs for tests (run spec §3.5, T49).

A thin layer over the Rust test kit (``pdf-extract-testkit``), so a fixture
built here is byte for byte the one the Rust tests build with the same
calls. Pages use PyMuPDF's coordinate convention: ``insert_text`` takes the
baseline origin of the first glyph, measured from the page's top-left corner
with y growing downwards. Text is set in the standard-14 Helvetica with
WinAnsiEncoding, not embedded; a character outside WinAnsi raises
``ValueError``. :func:`text_width` gives a run's advance from Adobe's AFM
metrics, so fixtures can place runs exactly::

    from pdf_extract.testing import PdfBuilder, text_width

    doc = PdfBuilder()
    page = doc.new_page()                 # A4: 595 x 842 pt
    page.insert_text((72, 100), "Alpha")
    gap = text_width("Alpha") + 0.3 * 11  # a 0.3 em gap at 11 pt
    page.insert_text((72 + gap, 100), "Beta")
    data = doc.to_bytes()

Builders may be shared between threads; ``to_bytes``, ``save`` and
``text_width`` release the GIL. Importing :mod:`pdf_extract` does not import
this module.
"""

from __future__ import annotations

from ._native import PageBuilder, PdfBuilder, text_width

__all__ = ["PageBuilder", "PdfBuilder", "text_width"]
