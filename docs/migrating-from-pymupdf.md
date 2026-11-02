# Migrating from PyMuPDF

This guide is for an application that extracts text with PyMuPDF
(`import fitz`) and wants to move to pdf-extract: same text shape, no AGPL,
thread-safe, OCR built in. It covers the call mapping, the behaviour that
changes, how to run extraction safely on uploaded files, and a rollout that
proves parity on your own documents before you switch, without running
PyMuPDF in production.

Every Python example below runs in the test suite
(`tests/python/test_migration_guide.py`). In them, `pdf_path` is a PDF file
and `old_pages` is the list of page texts your current PyMuPDF code returned
for it.

## 1. Call mapping

| PyMuPDF | pdf_extract |
|---|---|
| `import fitz` | `import pdf_extract` (prints nothing) |
| `fitz.open(path)` | `pdf_extract.open(path, max_pages=...)` |
| `with doc:` | `with doc:` |
| `doc.page_count` | `doc.page_count` (counts real pages, not the declared `/Count`) |
| `for page in doc` | `for page in doc` |
| `page.get_text("text")` | `page.text()` |
| `page.get_textpage_ocr(full=True, language="eng")` | `page.ocr(engine)`, with `engine = pdf_extract.OcrEngine(languages="eng")` |
| `page.get_text("text", textpage=tp)` | `tp.text()` |
| `RuntimeError` from the OCR call (Tesseract missing) | `OcrUnavailableError`, still a `RuntimeError`, raised when the engine is built |
| any other exception from OCR | `OcrPageError` |
| tests: `fitz.open()`, `new_page`, `insert_text`, `save` | `pdf_extract.testing.PdfBuilder` |
| tests: `page.get_textpage()` | `page.text_page()` |
| tests: `mock.patch.object(fitz.Page, "get_textpage_ocr", ...)` | `mock.patch.object(pdf_extract.Page, "ocr", ...)` |

A typical loop that OCRs only the pages without a usable text layer:

```python
import pdf_extract

def extract_pages(path, engine):
    texts = []
    with pdf_extract.open(path, max_pages=500) as doc:
        for page in doc:
            text = page.text()
            if len(text.strip()) < 50 and engine is not None:
                try:
                    text = page.ocr(engine).text()
                except pdf_extract.OcrPageError:
                    text = ""
            texts.append(text)
    return texts

pages = extract_pages(pdf_path, engine=None)
assert len(pages) == len(old_pages)
```

`pdf_extract.extract(path)` does the same in one call, with the routing
threshold, a per-document OCR budget and optional parallelism built in.

## 2. Behaviour changes

| Change | Default | To match PyMuPDF | Effect |
|---|---|---|---|
| OCR resolution 72 → 300 dpi | 300 | `OcrEngine(dpi=72)` | better OCR text, slower OCR |
| Missing language data means OCR is unavailable | always | n/a | an "OCR unavailable" state you can see, instead of silent empty text |
| A user-password PDF fails at open with `PasswordRequiredError` | always | n/a | a clear error instead of an empty document |
| `page_count` counts real pages, not `/Count` | always | n/a | a file that lies about its size cannot get past a page limit |
| Unmapped codes that would be NUL, control characters or surrogates become U+FFFD | always | n/a | no storage errors from stray control characters |
| Nothing printed to stdout or stderr | always | n/a | quieter logs |
| Blank line between blocks | off | `block_separator="newline"` (the default) | an opt-in improvement for paragraph detection |
| Different rasteriser and Tesseract build | always | n/a | OCR text differs slightly; measure it with character error rate |

## 3. Contain failures on untrusted input

The library bounds malformed and hostile input (resource limits, panics
surfaced as `InternalError`), but no PDF library can promise that no input
will ever crash or hang its process. When the files come from users:

- Extract in a process you can afford to lose: a job-queue worker or a
  subprocess, never a web worker that also serves requests.
- Enforce a wall-clock limit per document by killing that process. A Python
  timeout cannot interrupt a call in progress, and the library's own time
  limits are per page.
- Cap retries per document, quarantine a document that keeps failing, and log
  which document it was.

One child process per document, killed at the deadline:

```python
import json
import subprocess
import sys

WORKER = """
import json, sys
import pdf_extract
with pdf_extract.open(sys.argv[1], max_pages=500) as doc:
    print(json.dumps([page.text() for page in doc]))
"""

def extract_contained(path, timeout_s=120):
    try:
        done = subprocess.run(
            [sys.executable, "-c", WORKER, str(path)],
            capture_output=True, text=True, timeout=timeout_s,
        )
    except subprocess.TimeoutExpired:      # the child has been killed
        return None, "timeout"
    if done.returncode != 0:               # crashed, or raised
        return None, f"exit {done.returncode}"
    return json.loads(done.stdout), "ok"

texts, status = extract_contained(pdf_path)
assert status == "ok"
assert len(texts) == len(old_pages)
```

The caller records `status` per attempt and quarantines a document after,
say, three failures.

## 4. Roll out: measure offline, then cut over once

Don't run PyMuPDF beside pdf-extract in production. A "shadow" period keeps
the dependency you are migrating away from in every release until the switch,
and its comparison work competes with real extractions for their time limits.
Measure parity offline instead, on a copy of the documents your application
already holds, then replace PyMuPDF in one release.

### Measure

On an internal machine that serves no users, put a snapshot of your stored
PDFs in a directory. You need a checkout of this repository with its pinned
Rust toolchain (`rustup`) and uv: the run builds pdf-extract from source,
including the vendored Tesseract, and the oracle tooling is a separate,
development-only uv project that is never shipped. Then run:

```text
tools/oracle/run_private_corpus.sh /path/to/snapshot
```

It extracts every page with PyMuPDF and with pdf-extract, scores each pair
with the library's parity metrics (`pdf_extract_core::parity`), and writes its
output to a directory outside the repository (by default under
`~/.cache/pdf-extract-private/`; it refuses one inside the checkout). Treat
that directory like the snapshot itself:

- `expectations/` holds PyMuPDF's full page text for every document;
- `report.json` holds lengths and scores and no page text, but it names every
  file by its path in the snapshot, so share it only if file names are not
  sensitive.

The per-page metrics, all on text after `str.strip()`:

- **routing agreement**: both sides have at least 50 characters, or both
  fewer (the usual "does this page need OCR" decision);
- **similarity**: `1 − Levenshtein distance / length of the longer text`, in
  characters;
- **exact match**;
- **line-count delta**.

The gates:

| Metric | Gate |
|---|---|
| Opens (PyMuPDF opened it ⇒ pdf-extract opens it) | 100 % |
| Routing agreement | ≥ 99.9 % of pages |
| Similarity, median | ≥ 0.999 |
| Similarity, 1st percentile | ≥ 0.98 |
| Exact match, line-count delta | reported, not gated |

Add your own downstream check too: whatever your application derives from the
text (paragraphs, search snippets) should come out the same on at least 99 %
of pages. Your application probably stores PyMuPDF's output already, so this
check needs no PyMuPDF at all: re-extract the same documents with pdf-extract
(use `OcrEngine(dpi=72)` and the default output options, which match
PyMuPDF), apply your own post-processing, and compare with what you stored.

### Cut over

When the gates pass, ship one release that replaces the PyMuPDF calls (section
1), adds the containment of section 3, and drops `pymupdf` from your
dependencies. The deployment then needs only Tesseract's language data
(`tesseract-ocr-eng` on Debian and Ubuntu). Port your tests to
`pdf_extract.testing`. Rolling back means returning to the previous release.

### Improve

Move OCR to 300 dpi (`OcrEngine()`'s default). Separately, decide whether
`block_separator="blank_line"` helps your paragraph detection. Measure both
with the same offline run and downstream check before shipping them.
