# Committed fuzz seeds

`unicode/` seeds the `unicode` target. Its inputs are a selector byte plus a
payload (see `pdf_extract_core::fuzz_api::unicode_fallbacks`), so no corpus
supplies them; each file's name starts with its selector:

- `0-*`: glyph names (Adobe Glyph List, `uniXXXX`, `uXXXX`);
- `1-*`: a 4-byte code and a name at the KP15 encoding positions;
- `2-*`: a policy byte and a 4-byte code for the unmapped-glyph policy;
- `3-*`: a 2-byte character budget and a PDF text string (UTF-16BE/LE with
  BOM, UTF-8 with BOM, PDFDocEncoding);
- `4-*`: text for the control-character sanitiser.

The other targets need no committed seeds: the document targets read corpus
B and C, and the `cmap` target's seeds come from
`tools/fuzz/seed-cmaps.py` and the fuzz crate's `synthetic_cmaps` binary.
