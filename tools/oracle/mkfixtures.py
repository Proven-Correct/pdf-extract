"""Fixture tooling that needs the reference implementation (spec §1.7 T51, §4.3).

Runs PyMuPDF 1.28.2 strictly as a black box: it calls the public API and
observes the results. MuPDF/PyMuPDF source is never read, copied or
translated, and nothing in the library, its tests or CI imports this project.

Commands (run from anywhere; paths resolve against the repository root):

    uv run python mkfixtures.py enc           # write the 8 encrypted fixtures
    uv run python mkfixtures.py --verify      # oracle must not raise on generated files
    uv run python mkfixtures.py --afm-check   # regenerate the Helvetica width vector

``enc`` exists because the Rust test kit never encrypts. It rewrites the
fixtures, their ``manifest.toml`` entries and their PROVENANCE.md rows.
MuPDF draws a fresh random file identifier and encryption key per save, so
each run produces different bytes (and SHA-256s) for the same content.

``--verify`` builds every test-kit sample with
``cargo run --example dump_fixtures`` and runs each through the oracle (open,
``page_count``, ``get_text`` on every page), comparing with the expectation the
dump records; then it opens every committed fixture in the manifest and, where
an oracle expectation is committed (``gen.py``), checks the outcome still
matches it. Exit status 1 if anything raised unexpectedly or disagreed.

``--afm-check`` measures ``get_text_length`` for every WinAnsi printable
character and a few strings and writes the result as a test vector that
``crates/testkit/tests/afm_oracle.rs`` compares with ``text_width``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import re
import subprocess
import sys
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pymupdf

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / "tests" / "fixtures"
CORPUS_C = FIXTURES / "corpus-c"
MANIFEST = FIXTURES / "manifest.toml"
PROVENANCE = FIXTURES / "PROVENANCE.md"
AFM_VECTOR = REPO / "crates" / "testkit" / "tests" / "data" / "helvetica_widths_oracle.tsv"

ENC_GENERATOR = "tools/oracle/mkfixtures.py::enc"
ENC_BEGIN = "<!-- mkfixtures:enc:begin -->"
ENC_END = "<!-- mkfixtures:enc:end -->"

# (fixture tag, PyMuPDF encryption constant name)
ENC_METHODS = [
    ("rc4_40", "PDF_ENCRYPT_RC4_40"),
    ("rc4_128", "PDF_ENCRYPT_RC4_128"),
    ("aes_128", "PDF_ENCRYPT_AES_128"),
    ("aes_256", "PDF_ENCRYPT_AES_256"),
]
OWNER_PASSWORD = "owner-secret"
USER_PASSWORD = "user-secret"


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def enc_text(tag: str, kind: str) -> str:
    """The single line of text in an encrypted fixture (what extraction yields)."""
    return f"Encrypted {tag} {kind}"


@dataclass
class ManifestEntry:
    path: str
    sha256: str
    generator: str
    compat_points: list[str]
    gate: str

    def block(self) -> str:
        points = ", ".join(f'"{p}"' for p in self.compat_points)
        return (
            "[[fixture]]\n"
            f'path = "{self.path}"\n'
            f'sha256 = "{self.sha256}"\n'
            f'generator = "{self.generator}"\n'
            f"compat_points = [{points}]\n"
            f'gate = "{self.gate}"\n'
        )


def upsert_manifest(entries: list[ManifestEntry]) -> None:
    """Updates the sha256 of existing entries (matched by path) and appends new
    ones. Other entries and comments are left byte-for-byte alone, so units that
    add fixtures by hand never see this tool reformat their work."""
    text = MANIFEST.read_text(encoding="utf-8") if MANIFEST.exists() else ""
    for e in entries:
        pattern = re.compile(
            r'(path = "' + re.escape(e.path) + r'"\nsha256 = ")[0-9a-f]*(")', re.MULTILINE
        )

        def replace(m: re.Match[str], sha: str = e.sha256) -> str:
            return m.group(1) + sha + m.group(2)

        text, n = pattern.subn(replace, text)
        if n == 0:
            if text and not text.endswith("\n\n"):
                text = text.rstrip("\n") + "\n\n"
            text += e.block()
    MANIFEST.write_text(text, encoding="utf-8")


def rewrite_provenance_section(rows: list[str]) -> None:
    """Replaces the generated table between the enc markers in PROVENANCE.md."""
    text = PROVENANCE.read_text(encoding="utf-8")
    start, end = text.index(ENC_BEGIN), text.index(ENC_END)
    header = "| File | SHA-256 | Command | Content |\n|---|---|---|---|\n"
    body = header + "".join(r + "\n" for r in rows)
    text = text[: start + len(ENC_BEGIN)] + "\n" + body + text[end:]
    PROVENANCE.write_text(text, encoding="utf-8")


def cmd_enc() -> int:
    CORPUS_C.mkdir(parents=True, exist_ok=True)
    entries: list[ManifestEntry] = []
    rows: list[str] = []
    for tag, const in ENC_METHODS:
        for kind in ("owner", "user"):
            name = f"enc_{tag}_{kind}.pdf"
            path = CORPUS_C / name
            doc = pymupdf.open()
            page = doc.new_page()  # A4-ish default page; content is one line
            page.insert_text((72, 72), enc_text(tag, kind), fontsize=11)
            doc.save(
                path,
                encryption=getattr(pymupdf, const),
                owner_pw=OWNER_PASSWORD,
                # Owner-only files have an empty user password: anyone may open
                # them, and readers must decrypt transparently (spec T6).
                user_pw=USER_PASSWORD if kind == "user" else "",
                permissions=-1,
            )
            doc.close()
            digest = sha256_file(path)
            rel = f"corpus-c/{name}"
            entries.append(ManifestEntry(rel, digest, ENC_GENERATOR, [], "exact"))
            user = f'user_pw="{USER_PASSWORD}"' if kind == "user" else 'user_pw=""'
            rows.append(
                f"| `{rel}` | `{digest}` | `cd tools/oracle && uv run python mkfixtures.py enc` "
                f'(`save(encryption={const}, owner_pw="{OWNER_PASSWORD}", {user})`) '
                f'| one page, "{enc_text(tag, kind)}" (Helvetica 11 pt at 72,72) |'
            )
            print(f"wrote {rel} {digest}")
    upsert_manifest(entries)
    rewrite_provenance_section(rows)
    return 0


@dataclass
class Expectation:
    pages: int
    load_raises: bool


def parse_expectation(token: str) -> Expectation:
    parts = token.split(";")
    pages = int(parts[0].removeprefix("pages="))
    return Expectation(pages, "load-raises" in parts[1:])


@dataclass
class OracleResult:
    pages: int
    needs_pass: bool
    error: str | None
    texts: list[str]

    def excerpt(self) -> str:
        """First 60 characters of the document text, for the log: the text is
        not gated here (unit 13 owns text expectations) but is printed so a
        surprising extraction is visible to whoever runs --verify."""
        return repr("".join(self.texts)[:60])


def oracle_run(path: Path) -> OracleResult:
    """Opens `path` and extracts every page; records the first page error.
    Open failures propagate."""
    doc = pymupdf.open(path)
    try:
        if doc.needs_pass:
            # User-password files: page access raises by design (spec A.6);
            # opening and counting is the whole check.
            return OracleResult(doc.page_count, True, None, [])
        texts: list[str] = []
        for i in range(doc.page_count):
            try:
                texts.append(doc[i].get_text("text"))
            except Exception as exc:  # the oracle's own errors are the observation
                error = f"page {i}: {type(exc).__name__}: {exc}"
                return OracleResult(doc.page_count, False, error, texts)
        return OracleResult(doc.page_count, False, None, texts)
    finally:
        doc.close()


EXPECTATIONS = FIXTURES / "expectations"


def load_expectation(sha256: str) -> dict[str, Any] | None:
    """The committed oracle expectation for a fixture hash, if any."""
    path = EXPECTATIONS / f"{sha256}.json"
    if not path.exists():
        return None
    record: dict[str, Any] = json.loads(path.read_text(encoding="utf-8"))
    return record


def expectation_problems(expected: dict[str, Any], r: OracleResult) -> list[str]:
    """Where a fresh oracle run disagrees with the committed expectation."""
    problems = []
    if not expected["open"]["ok"]:
        problems.append("opened, but the expectation records an open failure")
    if r.pages != expected["page_count"] or r.needs_pass != expected["open"]["needs_pass"]:
        problems.append(
            f"pages={r.pages} needs_pass={r.needs_pass}, expectation "
            f"pages={expected['page_count']} needs_pass={expected['open']['needs_pass']}"
        )
    errored = [p["index"] for p in expected["pages"] if p["error"] is not None]
    # oracle_run stops at the first failing page, which is then page len(texts).
    first_error = len(r.texts) if r.error is not None else None
    if not r.needs_pass and first_error != (errored[0] if errored else None):
        problems.append(f"page error {r.error!r}, expectation errors on pages {errored}")
    if r.error is None and not r.needs_pass:
        for page, text in zip(expected["pages"], r.texts, strict=True):
            digest = hashlib.sha256(text.encode("utf-8")).hexdigest()
            if page["text"] != text and page["sha256"] != digest:
                problems.append(f"page {page['index']} text differs from the expectation")
    return problems


def cmd_verify() -> int:
    pymupdf.TOOLS.mupdf_display_errors(False)
    failures: list[str] = []
    with tempfile.TemporaryDirectory(prefix="pdf-extract-dump-") as tmp:
        subprocess.run(
            [
                "cargo",
                "run",
                "--locked",
                "--quiet",
                "-p",
                "pdf-extract-testkit",
                "--example",
                "dump_fixtures",
                "--",
                tmp,
            ],
            cwd=REPO,
            check=True,
        )
        index = (Path(tmp) / "index.tsv").read_text(encoding="utf-8").splitlines()
        if not index:
            failures.append("dump_fixtures wrote an empty index.tsv")
        for line in index:
            name, kind, token = line.split("\t")
            exp = parse_expectation(token)
            try:
                r = oracle_run(Path(tmp) / name)
            except Exception as exc:
                failures.append(f"{name}: open raised {type(exc).__name__}: {exc}")
                continue
            ok = (
                r.pages == exp.pages
                and (r.error is not None) == exp.load_raises
                and not r.needs_pass
            )
            print(
                f"{'ok' if ok else 'FAIL':4} {kind:7} {name}: pages={r.pages} "
                f"error={r.error} text={r.excerpt()}"
            )
            if not ok:
                failures.append(f"{name}: expected {token}, got {r}")
            pymupdf.TOOLS.mupdf_warnings()  # drain per file

    # Committed fixtures, driven by the manifest so every listed file (in any
    # directory) is opened. A fixture with a committed expectation
    # (tools/oracle/gen.py) must still behave as recorded there: some open
    # failures and page errors are the expected oracle behaviour (garbage,
    # non-PDFs, a page-tree cycle), and a changed outcome means the
    # expectations are stale.
    manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
    for entry in manifest.get("fixture", []):
        rel = entry["path"]
        expected = load_expectation(entry["sha256"])
        try:
            r = oracle_run(FIXTURES / rel)
        except Exception as exc:
            if (
                expected is not None
                and not expected["open"]["ok"]
                and expected["open"]["exception"] == type(exc).__name__
            ):
                print(f"ok   fixture {rel}: open raised {type(exc).__name__}, as recorded")
            else:
                failures.append(f"{rel}: open raised {type(exc).__name__}: {exc}")
            continue
        if expected is not None:
            problems = expectation_problems(expected, r)
        else:
            problems = [] if r.error is None else [r.error]
        match = re.fullmatch(r"enc_(.+)_(owner|user)\.pdf", Path(rel).name)
        if match:
            tag, kind = match.groups()
            if r.needs_pass != (kind == "user") or r.pages != 1:
                problems.append(f"needs_pass={r.needs_pass} pages={r.pages}")
            want = [enc_text(tag, "owner") + "\n"] if kind == "owner" else []
            if r.texts != want:
                problems.append(f"text {r.texts!r}, expected {want!r}")
        failures += [f"{rel}: {p}" for p in problems]
        print(
            f"{'FAIL' if problems else 'ok':4} fixture {rel}: pages={r.pages} "
            f"needs_pass={r.needs_pass} text={r.excerpt()}"
        )
        pymupdf.TOOLS.mupdf_warnings()

    if failures:
        print("\nverify failed:", *failures, sep="\n  ")
        return 1
    print("\nverify: all files behaved as expected")
    return 0


def win_ansi_printable() -> list[str]:
    """WinAnsi printable characters, derived independently of the Rust table
    via Python's cp1252 codec (identical for every code WinAnsi defines)."""
    chars: list[str] = []
    for code in [*range(0x20, 0x7F), *range(0x80, 0x100)]:
        try:
            chars.append(bytes([code]).decode("cp1252"))
        except UnicodeDecodeError:
            continue  # 0x81, 0x8D, 0x8F, 0x90, 0x9D are undefined
    return chars


RUN_ORIGIN_X = 10.0


def rendered_widths(rows: list[tuple[str, float]]) -> list[float]:
    """Measures, per row, how far the oracle advances the pen over the text
    when rendering it: each run is drawn on its own line from a hand-written
    PDF with a WinAnsi Helvetica font, followed by a sentinel 'X'; the width is
    the X's rawdict origin minus the run origin."""
    lines = []
    for i, (text, size) in enumerate(rows):
        payload = (text + "X").encode("cp1252").hex().upper()
        lines.append(f"BT /H {size!r} Tf {RUN_ORIGIN_X} {20 + 30 * i} Td <{payload}> Tj ET")
    content = "\n".join(lines).encode("ascii")
    height = 40 + 30 * len(rows)
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 3000 {height}] "
        "/Resources << /Font << /H 4 0 R >> >> /Contents 5 0 R >>".encode("ascii"),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        f"<< /Length {len(content)} >>\nstream\n".encode("ascii") + content + b"\nendstream",
    ]
    pdf = bytearray(b"%PDF-1.7\n")
    offsets = []
    for n, body in enumerate(objects, start=1):
        offsets.append(len(pdf))
        pdf += f"{n} 0 obj\n".encode("ascii") + body + b"\nendobj\n"
    xref = len(pdf)
    pdf += f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode("ascii")
    pdf += b"".join(f"{o:010} 00000 n \n".encode("ascii") for o in offsets)
    pdf += f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\n".encode("ascii")
    pdf += f"startxref\n{xref}\n%%EOF\n".encode("ascii")

    doc = pymupdf.open("pdf", bytes(pdf))
    page = doc[0]
    # Map each run's baseline (page space, y up) to the sentinel's x origin.
    # rawdict uses y-down coordinates, so convert back with the page height.
    sentinel_x: dict[int, float] = {}
    raw = page.get_text("rawdict")
    for block in raw["blocks"]:
        for line in block.get("lines", []):
            for span in line["spans"]:
                for ch in span["chars"]:
                    x, y = ch["origin"]
                    row = round((height - y - 20) / 30)
                    if ch["c"] == "X":
                        sentinel_x[row] = max(sentinel_x.get(row, x), x)
    doc.close()
    missing = [i for i in range(len(rows)) if i not in sentinel_x]
    if missing:
        raise RuntimeError(f"rendered width: no sentinel found for rows {missing}")
    return [sentinel_x[i] - RUN_ORIGIN_X for i in range(len(rows))]


def cmd_afm_check() -> int:
    rows: list[tuple[str, float]] = [(c, 11.0) for c in win_ansi_printable()]
    ascii_all = "".join(chr(c) for c in range(0x20, 0x7F))
    rows += [
        ("Alpha", 11.0),
        ("Beta", 22.0),
        ("Hello world", 11.0),
        ("The quick brown fox jumps over the lazy dog", 7.0),
        ("The quick brown fox jumps over the lazy dog", 11.0),
        ("The quick brown fox jumps over the lazy dog", 22.0),
        (ascii_all, 11.0),
        ("".join(win_ansi_printable()), 11.0),
    ]
    out = [
        "# Helvetica advance widths measured with the oracle "
        f"(pymupdf {pymupdf.VersionBind}), python {platform.python_version()}, "
        f"{platform.platform()}.",
        "# Generated by: tools/oracle/mkfixtures.py --afm-check. Do not edit.",
        "# Columns: code points (hex, space-separated) <TAB> fontsize <TAB>",
        "#   get_text_length(fontname='helv') in pt <TAB> rendered advance in pt.",
        "# 'rendered' is where the oracle places the glyph after the run when it",
        "# renders the run from a WinAnsi-encoded standard Helvetica font (rawdict",
        "# origin of a sentinel 'X' minus the run origin). get_text_length only",
        "# knows Latin-1 code points and returns 278 units for the 0x80-0x9F block,",
        "# which is not the width the oracle renders (see afm_oracle.rs).",
    ]
    rendered = rendered_widths(rows)
    for (text, size), drawn in zip(rows, rendered, strict=True):
        width = pymupdf.get_text_length(text, fontname="helv", fontsize=size)
        cps = " ".join(f"{ord(c):04X}" for c in text)
        out.append(f"{cps}\t{size!r}\t{width!r}\t{drawn!r}")
    AFM_VECTOR.parent.mkdir(parents=True, exist_ok=True)
    AFM_VECTOR.write_text("\n".join(out) + "\n", encoding="utf-8")
    print(f"wrote {AFM_VECTOR.relative_to(REPO)} ({len(rows)} rows)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("command", nargs="?", choices=["enc"], help="generate fixtures")
    parser.add_argument(
        "--verify", action="store_true", help="run generated files through the oracle"
    )
    parser.add_argument("--afm-check", action="store_true", help="regenerate the width vector")
    args = parser.parse_args()
    if not (args.command or args.verify or args.afm_check):
        parser.error("nothing to do: give 'enc', --verify or --afm-check")
    status = 0
    if args.command == "enc":
        status |= cmd_enc()
    if args.afm_check:
        status |= cmd_afm_check()
    if args.verify:
        status |= cmd_verify()
    return status


if __name__ == "__main__":
    sys.exit(main())
