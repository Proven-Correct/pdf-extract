#!/usr/bin/env python3
"""Seeds for the ``cmap`` fuzz target (design D8 "Seeds").

Writes every CMap stream found in the corpus PDFs (ToUnicode maps and
embedded encoding CMaps) into one directory, each file named by the SHA-1 of
its content as libFuzzer names corpus entries, so re-running is idempotent
and merges with an existing corpus.

The PDF scan is deliberately shallow: it finds top-level ``stream`` bodies by
their keyword, inflates FlateDecode ones, and keeps those that look like a
CMap. Streams inside object streams and other filters are missed; seeds
only need to be representative, and the fuzzer grows the rest. Corpus C is
hand-built, so almost all of its streams are top-level.

The corpus streams are almost all ``bfchar`` ToUnicode maps, so the target's
other seeds are hand-written text CMaps covering the rest of the grammar
(CID and notdef ranges, ``bfrange``, wide and mixed codespaces, ``usecmap``);
the fuzz crate's ``synthetic_cmaps`` binary writes them, checking that each
one parses, and ``tools/fuzz/run.sh`` runs both.

Usage: seed-cmaps.py OUT_DIR [--corpus DIR]...
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
import zlib
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DEFAULT_CORPORA = [REPO / "tests" / "fixtures" / "corpus-c"]

# A stream keyword ends its dictionary and is followed by an end of line.
STREAM = re.compile(rb">>\s*stream(\r\n|\n|\r)")
ENDSTREAM = re.compile(rb"\r?\n?endstream")
# Operators only a CMap program contains.
CMAP_MARKERS = (b"begincmap", b"beginbfchar", b"beginbfrange", b"begincidrange", b"usecmap")


def stream_bodies(pdf: bytes) -> list[tuple[bytes, bytes]]:
    """The (dictionary, raw body) of every top-level stream in ``pdf``."""
    out = []
    for m in STREAM.finditer(pdf):
        start = m.end()
        end = ENDSTREAM.search(pdf, start)
        if end is None:
            continue
        dict_start = pdf.rfind(b"obj", 0, m.start())
        out.append((pdf[max(dict_start, 0) : m.start()], pdf[start : end.start()]))
    return out


def decoded(dictionary: bytes, body: bytes) -> bytes | None:
    """``body`` with FlateDecode undone, or ``None`` for other filters or a
    stream that does not inflate."""
    if b"/Filter" not in dictionary:
        return body
    if b"/FlateDecode" not in dictionary or b"/DecodeParms" in dictionary:
        return None
    try:
        # decompressobj tolerates trailing garbage and a missing end marker.
        return zlib.decompressobj().decompress(body)
    except zlib.error:
        return None


def is_cmap(data: bytes) -> bool:
    return any(marker in data for marker in CMAP_MARKERS)


def cmaps_in(pdf: bytes) -> list[bytes]:
    found = []
    for dictionary, body in stream_bodies(pdf):
        data = decoded(dictionary, body)
        if data is not None and is_cmap(data):
            found.append(data)
    return found


def write_seed(out: Path, data: bytes) -> bool:
    """Writes ``data`` under its SHA-1; ``False`` if it was already there."""
    path = out / hashlib.sha1(data).hexdigest()
    if path.exists():
        return False
    path.write_bytes(data)
    return True


def seed_from_corpora(out: Path, corpora: list[Path]) -> int:
    """Writes the CMap streams of every file in ``corpora``; returns how many
    distinct ones were found."""
    seen = set()
    for corpus in corpora:
        if not corpus.is_dir():
            raise SystemExit(f"seed-cmaps: no corpus directory {corpus}")
        for path in sorted(p for p in corpus.iterdir() if p.is_file()):
            for data in cmaps_in(path.read_bytes()):
                seen.add(hashlib.sha1(data).hexdigest())
                write_seed(out, data)
    return len(seen)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("out", type=Path, help="directory to write seeds into")
    parser.add_argument(
        "--corpus",
        type=Path,
        action="append",
        help="directory of PDFs to scan (repeatable; default: corpus C)",
    )
    args = parser.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    streams = seed_from_corpora(args.out, args.corpus or DEFAULT_CORPORA)
    print(f"cmap seeds: {streams} CMap streams -> {args.out}")
    if streams == 0:
        print("no CMap seeds found", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
