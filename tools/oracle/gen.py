"""Writes oracle expectation JSON for fixture files (spec §3.6, §4.13).

Runs PyMuPDF 1.28.2 strictly as a black box: every input is opened **from
disk** by path (the oracle treats unsaved in-memory documents differently,
spec Appendix A.3), its page count and each page's ``get_text("text")`` are
recorded, and nothing else is consulted. MuPDF/PyMuPDF source is never read.
The library and its tests never import this project; CI never installs
PyMuPDF and reads the committed JSON instead.

Usage (from ``tools/oracle``)::

    uv run python gen.py ../../tests/fixtures/corpus-c
    uv run python gen.py --out /private/dir --root /private/corpus /private/corpus

Every regular file under each input directory (recursively, dotfiles
skipped, any extension: the contract fixtures include ``valid.txt`` and
``valid_noext``) gets ``<out>/<sha256>.json``. ``--prune`` deletes JSON files
in ``<out>`` whose hash matches none of the inputs, so a regenerated corpus
leaves no stale expectations; give every corpus that shares ``<out>`` when
pruning. With ``--ocr`` and no ``--built``, a prune that would delete a built
variant's expectation is refused.

OCR (run 0002 §3.8)::

    uv run python gen.py --ocr --tessdata .cache/tessdata_fast \
        --built ../../target/corpus-d ../../tests/fixtures/corpus-d

``--ocr`` takes only the ``*.pdf`` inputs (corpus D's ``.txt`` ground truth
needs no expectation), writes ``schema: 2`` and adds an ``ocr`` block:
``{"dpi": 72, "language": "eng", "tessdata_sha256", "pages": [{"index",
"text", "sha256"}]}`` from ``get_textpage_ocr(full=True, dpi=72,
tessdata=<dir>)``. The model must be the pinned tessdata_fast
``eng.traineddata`` (spec D-17); any other file is refused. ``--built <dir>``
adds corpus D's noise variants, which the Rust test kit builds at test time
(``corpus_d_noise`` example): every ``[[built]]`` row of ``manifest.toml``
names a file in ``<dir>`` whose SHA-256 must equal the row's pin, or the
whole run stops before writing anything; their ``source`` is
``built/<path>``.

Schema (``schema: 1``): ``sha256``/``size`` of the input; ``source``, its
path relative to ``--root``; ``oracle`` versions and platform; ``open``
(``ok``, ``exception`` class name when opening raised, ``needs_pass``,
``is_encrypted``); ``page_count``; and per page ``index``, ``text``,
``sha256`` (of the UTF-8 text) and ``error`` (``"<class>: <message>"`` when
loading or extracting the page raised, e.g. for a user password). A text over
1 MiB is stored as its hash only (``text: null``).
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import platform
import sys
import tomllib
from pathlib import Path
from typing import Any

import pymupdf

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / "tests" / "fixtures"
SCHEMA = 1
SCHEMA_OCR = 2
OCR_DPI = 72
# tessdata_fast eng.traineddata (spec D-17): the one model the committed OCR
# expectations and the CI gate are valid for.
MODEL_SHA256 = "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2"
# Texts above this many UTF-8 bytes are stored by hash only (spec §3.6).
MAX_INLINE_TEXT = 1 << 20


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def oracle_info() -> dict[str, str]:
    """Versions and platform of this oracle run."""
    mac = platform.mac_ver()[0]
    system = f"macOS-{mac}" if mac else f"{platform.system()}-{platform.release()}"
    return {
        "pymupdf": str(pymupdf.VersionBind),
        "mupdf": str(pymupdf.VersionFitz),
        "python": platform.python_version(),
        "platform": f"{system}-{platform.machine()}",
        "generated": datetime.date.today().isoformat(),
    }


def describe(exc: BaseException) -> str:
    return f"{type(exc).__name__}: {exc}"


def page_record(doc: Any, index: int) -> dict[str, Any]:
    """One page's text, or the error loading or extracting it raised."""
    try:
        text = str(doc[index].get_text("text"))
    except Exception as exc:  # the oracle's own failure is the observation
        return {"index": index, "text": None, "sha256": None, "error": describe(exc)}
    encoded = text.encode("utf-8")  # raises on lone surrogates: never silently mangle
    return {
        "index": index,
        "text": text if len(encoded) <= MAX_INLINE_TEXT else None,
        "sha256": sha256_hex(encoded),
        "error": None,
    }


def ocr_block(doc: Any, tessdata: Path, model_sha256: str) -> dict[str, Any]:
    """The oracle's 72-dpi OCR of every page (spec §3.8)."""
    pages = []
    for index in range(int(doc.page_count)):
        page = doc[index]
        tp = page.get_textpage_ocr(full=True, language="eng", dpi=OCR_DPI, tessdata=str(tessdata))
        text = str(page.get_text("text", textpage=tp))
        pages.append({"index": index, "text": text, "sha256": sha256_hex(text.encode("utf-8"))})
    return {
        "dpi": OCR_DPI,
        "language": "eng",
        "tessdata_sha256": model_sha256,
        "pages": pages,
    }


def expectation(
    path: Path,
    root: Path,
    info: dict[str, str],
    ocr: tuple[Path, str] | None = None,
    source: str | None = None,
) -> dict[str, Any]:
    data = path.read_bytes()
    record: dict[str, Any] = {
        "schema": SCHEMA_OCR if ocr else SCHEMA,
        "sha256": sha256_hex(data),
        "size": len(data),
        "source": source or path.resolve().relative_to(root.resolve()).as_posix(),
        "oracle": info,
    }
    try:
        doc = pymupdf.open(path)
    except Exception as exc:
        record["open"] = {
            "ok": False,
            "exception": type(exc).__name__,
            "needs_pass": False,
            "is_encrypted": False,
        }
        record["page_count"] = 0
        record["pages"] = []
        return record
    try:
        record["open"] = {
            "ok": True,
            "exception": None,
            "needs_pass": bool(doc.needs_pass),
            "is_encrypted": bool(doc.is_encrypted),
        }
        count = int(doc.page_count)
        record["page_count"] = count
        record["pages"] = [page_record(doc, i) for i in range(count)]
        if ocr:
            record["ocr"] = ocr_block(doc, *ocr)
    finally:
        doc.close()
    return record


def inputs(dirs: list[Path]) -> list[Path]:
    """Every regular, non-hidden file under `dirs`, in a stable order."""
    files: list[Path] = []
    for d in dirs:
        if not d.is_dir():
            raise SystemExit(f"gen.py: {d} is not a directory")
        for p in sorted(d.rglob("*")):
            rel = p.relative_to(d)
            if (
                p.is_file()
                and not p.is_symlink()
                and not any(part.startswith(".") for part in rel.parts)
            ):
                files.append(p)
    return files


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("dirs", nargs="+", type=Path, help="fixture directories")
    parser.add_argument(
        "--out", type=Path, default=FIXTURES / "expectations", help="expectation directory"
    )
    parser.add_argument(
        "--root", type=Path, default=FIXTURES, help="`source` paths are relative to this"
    )
    parser.add_argument("--prune", action="store_true", help="delete stale expectations")
    parser.add_argument("--ocr", action="store_true", help="add the 72-dpi OCR block (schema 2)")
    parser.add_argument("--tessdata", type=Path, help="with --ocr: directory of eng.traineddata")
    parser.add_argument(
        "--built", type=Path, help="with --ocr: corpus D's built noise variants (pinned)"
    )
    args = parser.parse_args(argv)
    if args.ocr != (args.tessdata is not None) or (args.built and not args.ocr):
        parser.error("--ocr needs --tessdata; --built needs --ocr")
    if args.prune and args.ocr and not args.built:
        # Without --built the built noise variants are not inputs, so --prune
        # would treat their expectations as stale. Refuse only when such an
        # expectation is really in --out: a private-scans run prunes an
        # unrelated directory and must keep working without --built.
        pins = tomllib.loads((FIXTURES / "manifest.toml").read_text(encoding="utf-8")).get(
            "built", []
        )
        doomed = [r["path"] for r in pins if (args.out / f"{r['sha256']}.json").is_file()]
        if doomed:
            parser.error(
                f"--prune without --built would delete the expectation of built/{doomed[0]} "
                f"and {len(doomed) - 1} other(s); pass --built <dir> too"
            )

    ocr: tuple[Path, str] | None = None
    if args.ocr:
        model = args.tessdata / "eng.traineddata"
        if not model.is_file():
            raise SystemExit(f"gen.py: {model} does not exist (mkcorpusd.py --fetch-model)")
        digest = sha256_hex(model.read_bytes())
        if digest != MODEL_SHA256:
            raise SystemExit(f"gen.py: {model} is {digest}, not the pinned tessdata_fast model")
        ocr = (args.tessdata.resolve(), digest)

    files = inputs(args.dirs)
    if args.ocr:
        files = [p for p in files if p.suffix == ".pdf"]
    # Built files: (path on disk, `source` value). Every one must carry the
    # bytes its [[built]] row pins, or nothing is written: an expectation
    # keyed by the pin must describe exactly those bytes.
    built: list[tuple[Path, str]] = []
    if args.built:
        manifest = tomllib.loads((FIXTURES / "manifest.toml").read_text(encoding="utf-8"))
        rows = manifest.get("built", [])
        if not rows:
            raise SystemExit("gen.py: manifest.toml has no [[built]] rows")
        for row in rows:
            path = args.built / row["path"]
            if not path.is_file():
                raise SystemExit(f"gen.py: {path} missing (run the corpus_d_noise example)")
            digest = sha256_hex(path.read_bytes())
            if digest != row["sha256"]:
                raise SystemExit(
                    f"gen.py: {path} is {digest}, but manifest.toml pins {row['sha256']}; "
                    "re-pin with mkcorpusd.py --pin-built or rebuild"
                )
            built.append((path, f"built/{row['path']}"))
    # Validate everything before running the oracle or writing a file, so a
    # bad invocation never leaves the expectations half-regenerated.
    root = args.root.resolve()
    outside = [p for p in files if not p.resolve().is_relative_to(root)]
    if outside:
        raise SystemExit(f"gen.py: {outside[0]} is not under --root {args.root}")
    digests: dict[str, Path] = {}
    for path in files + [p for p, _ in built]:
        digest = sha256_hex(path.read_bytes())
        if digest in digests:
            raise SystemExit(f"gen.py: {path} has the same bytes as {digests[digest]}")
        digests[digest] = path

    pymupdf.TOOLS.mupdf_display_errors(False)
    info = oracle_info()
    args.out.mkdir(parents=True, exist_ok=True)
    written: set[str] = set()
    jobs: list[tuple[Path, str | None]] = [(p, None) for p in files] + [(p, s) for p, s in built]
    for path, source in jobs:
        record = expectation(path, args.root, info, ocr, source)
        pymupdf.TOOLS.mupdf_warnings()  # drain, so one file's warnings never reach the next
        name = f"{record['sha256']}.json"
        written.add(name)
        text = json.dumps(record, indent=2, ensure_ascii=False) + "\n"
        (args.out / name).write_text(text, encoding="utf-8")
        pages = record["page_count"]
        status = "ok" if record["open"]["ok"] else record["open"]["exception"]
        print(f"{record['source']}: {status}, {pages} page(s)")
    if args.prune:
        for stale in sorted(args.out.glob("*.json")):
            if stale.name not in written:
                stale.unlink()
                print(f"pruned {stale.name}")
    print(f"wrote {len(written)} expectation(s) to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
