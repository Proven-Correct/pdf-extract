"""Corpus D: the image-only OCR quality corpus (run 0002 §3.8, Appendix C).

The vector source pages and their ground truth come from the Rust test kit
(``cargo run -p pdf-extract-testkit --example corpus_d_sources``). This
script rasterises each one with the **oracle's** renderer
(``get_pixmap(colorspace=csGRAY)``), so the scan never comes from the
rasteriser under test (D-9); optionally degrades it with Pillow/numpy (skew,
blur, JPEG); and
re-wraps it as one 8-bit grayscale image per page (FlateDecode, or
DCTDecode for the JPEG recipe) on a page with the source's MediaBox,
CropBox and ``/Rotate``. PyMuPDF is used strictly as a black box.

Commands (run from ``tools/oracle``)::

    uv run python mkcorpusd.py --fetch-model      # pinned tessdata_fast model into .cache/
    uv run python mkcorpusd.py                    # write corpus D + manifest + provenance
    uv run python mkcorpusd.py --verify           # oracle checks on the committed files
    uv run python mkcorpusd.py --verify --compare-pixels   # ... and regeneration is pixel-identical
    uv run python mkcorpusd.py --pin-built DIR    # pin the Rust-built noise variants in DIR
    uv run python mkcorpusd.py --verify --built DIR        # ... and check them through the oracle

The twelve noise variants are **not** made here (spec revision 5, §3.8): the
Rust test kit builds them at test time from the committed clean files
(``cargo run -p pdf-extract-testkit --example corpus_d_noise -- DIR``), with
integer noise whose bytes are identical on every host, and ``manifest.toml``
pins their SHA-256s in ``[[built]]`` rows. A Python re-implementation of the
noise could not be bit-identical and would verify only itself, so this script
only pins (``--pin-built``) and checks (``--verify --built``) the Rust output.

PyMuPDF randomises ``/ID`` on save, so a regenerated file has new bytes and
a new SHA-256 (the manifest and PROVENANCE rows are rewritten; rerun
``gen.py --ocr`` afterwards). Ground truth and pixel data do not change:
every step is deterministic, which
``--compare-pixels`` proves by regenerating into a scratch directory and
comparing the decoded image samples with the committed ones.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import subprocess
import sys
import tempfile
import tomllib
import urllib.request
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

import numpy as np
import numpy.typing as npt
import pymupdf
from PIL import Image, ImageFilter

REPO = Path(__file__).resolve().parents[2]
FIXTURES = REPO / "tests" / "fixtures"
CORPUS_D = FIXTURES / "corpus-d"
MANIFEST = FIXTURES / "manifest.toml"
PROVENANCE = FIXTURES / "PROVENANCE.md"
CACHE = Path(__file__).resolve().parent / ".cache"
MODEL_DIR = CACHE / "tessdata_fast"

# tessdata_fast eng.traineddata, pinned by git blob sha (spec D-17); the
# CI action .github/actions/tessdata installs the same file.
MODEL_BLOB = "bbef4675053b5b468cdb477053e28b1c698ba08e"
MODEL_SHA256 = "7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2"
MODEL_URL = f"https://api.github.com/repos/tesseract-ocr/tessdata_fast/git/blobs/{MODEL_BLOB}"

MANIFEST_BEGIN = "# corpus_d:begin (written by tools/oracle/mkcorpusd.py; do not edit)"
MANIFEST_END = "# corpus_d:end"
PROV_BEGIN = "<!-- mkcorpusd:begin -->"
PROV_END = "<!-- mkcorpusd:end -->"

SKEW_DEGREES = 1.5
BLUR_RADIUS = 1.2
JPEG_QUALITY = 75

Gray = npt.NDArray[np.uint8]


def skew(a: Gray) -> Gray:
    """+1.5 degrees (counter-clockwise) about the centre, bicubic, white fill."""
    img = Image.fromarray(a, mode="L")
    out = img.rotate(SKEW_DEGREES, resample=Image.Resampling.BICUBIC, fillcolor=255)
    return np.asarray(out, dtype=np.uint8)


def blur(a: Gray) -> Gray:
    """Gaussian blur, radius 1.2 px (at 300 dpi)."""
    img = Image.fromarray(a, mode="L")
    return np.asarray(img.filter(ImageFilter.GaussianBlur(BLUR_RADIUS)), dtype=np.uint8)


DEGRADE: dict[str, Callable[[Gray], Gray]] = {"skew": skew, "blur": blur}


@dataclass(frozen=True)
class Recipe:
    """One corpus-D file."""

    name: str  # file stem under corpus-d/
    source: str  # test-kit source page (corpus_d::sources name)
    subset: str  # clean | skew | blur | lowres | jpeg (quality.rs groups by it)
    dpi: int = 300
    jpeg: bool = False

    @property
    def generator(self) -> str:
        return f"tools/oracle/mkcorpusd.py::{self.subset}"


SOURCES = [
    "clean_body_09pt",
    "clean_body_10pt",
    "clean_body_11pt",
    "clean_body_12pt",
    "clean_two_column",
    "clean_heading_footnote",
    "clean_numbered_list",
    "clean_citations",
    "clean_one_line",
    "clean_rotate90",
    "clean_cropbox",
    "sparse_below_min_chars",
]


def recipes() -> list[Recipe]:
    """Appendix C's committed files, in catalogue order: 12 sources x 3 + 3 + 1.

    The noise variant of each source is built by the Rust test kit instead
    (see :func:`noise_names`)."""
    out = [Recipe(s, s, "clean") for s in SOURCES]
    for variant in ("skew", "blur"):
        out += [Recipe(f"{s}_{variant}", s, variant) for s in SOURCES]
    out += [
        Recipe("lowres_body_11pt_150", "clean_body_11pt", "lowres", dpi=150),
        Recipe("lowres_citations_150", "clean_citations", "lowres", dpi=150),
        Recipe("lowres_two_column_150", "clean_two_column", "lowres", dpi=150),
        Recipe("jpeg_body_11pt", "clean_body_11pt", "jpeg", jpeg=True),
    ]
    return out


def noise_names() -> list[str]:
    """The Rust-built noise variants (``corpus_d::noise_names``), catalogue order."""
    return [f"{s}_noise" for s in SOURCES]


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def git_blob_sha(data: bytes) -> str:
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def cmd_fetch_model() -> int:
    """Downloads the pinned model into the cache and checks both digests."""
    target = MODEL_DIR / "eng.traineddata"
    if not target.is_file():
        MODEL_DIR.mkdir(parents=True, exist_ok=True)
        req = urllib.request.Request(MODEL_URL, headers={"Accept": "application/vnd.github.raw"})
        with urllib.request.urlopen(req, timeout=120) as resp:
            data = resp.read()
        if git_blob_sha(data) != MODEL_BLOB:
            raise SystemExit("mkcorpusd: downloaded model has the wrong git blob sha")
        target.write_bytes(data)
    data = target.read_bytes()
    if git_blob_sha(data) != MODEL_BLOB or sha256_bytes(data) != MODEL_SHA256:
        raise SystemExit(f"mkcorpusd: {target} is not the pinned tessdata_fast model")
    print(f"{target} sha256 {MODEL_SHA256}")
    return 0


def dump_sources(dest: Path) -> None:
    """The test kit's vector pages and ground truth, into `dest`."""
    subprocess.run(
        [
            "cargo",
            "run",
            "--locked",
            "--quiet",
            "-p",
            "pdf-extract-testkit",
            "--example",
            "corpus_d_sources",
            "--",
            str(dest),
        ],
        cwd=REPO,
        check=True,
    )


def rasterise(src_pdf: Path, dpi: int) -> tuple[Gray, pymupdf.Rect, pymupdf.Rect, int]:
    """The source page's whole MediaBox, unrotated, as gray pixels.

    Rendering with rotation 0 over the MediaBox keeps every pixel of the
    page; the CropBox and ``/Rotate`` are then put back on the image page,
    so the renderer under test has to honour both (O1) to see what the
    oracle's reader sees.
    """
    doc = pymupdf.open(src_pdf)
    page = doc[0]
    rotation = int(page.rotation)
    page.set_rotation(0)
    mediabox, cropbox = pymupdf.Rect(page.mediabox), pymupdf.Rect(page.cropbox)
    page.set_cropbox(page.mediabox)
    pix = page.get_pixmap(dpi=dpi, colorspace=pymupdf.csGRAY, alpha=False)
    pixels = np.frombuffer(pix.samples, dtype=np.uint8).reshape(pix.h, pix.w).copy()
    doc.close()
    return pixels, mediabox, cropbox, rotation


def build(recipe: Recipe, sources: Path, out: Path) -> None:
    """Writes `out/<name>.pdf` and `out/<name>.txt` for one recipe."""
    pixels, mediabox, cropbox, rotation = rasterise(sources / f"{recipe.source}.pdf", recipe.dpi)
    if recipe.subset in DEGRADE:
        pixels = DEGRADE[recipe.subset](pixels)
    doc = pymupdf.open()
    page = doc.new_page(width=mediabox.width, height=mediabox.height)
    h, w = pixels.shape
    if recipe.jpeg:
        buf = io.BytesIO()
        Image.fromarray(pixels, mode="L").save(buf, format="JPEG", quality=JPEG_QUALITY)
        page.insert_image(page.rect, stream=buf.getvalue())
    else:
        pix = pymupdf.Pixmap(pymupdf.csGRAY, w, h, pixels.tobytes(), False)
        page.insert_image(page.rect, pixmap=pix)
    if cropbox != mediabox:
        page.set_cropbox(cropbox)
    if rotation:
        page.set_rotation(rotation)
    doc.save(out / f"{recipe.name}.pdf", garbage=4, deflate=True)
    doc.close()
    truth = (sources / f"{recipe.source}.txt").read_bytes()
    (out / f"{recipe.name}.txt").write_bytes(truth)


def generate(out: Path) -> None:
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="pdf-extract-corpus-d-") as tmp:
        dump_sources(Path(tmp))
        for r in recipes():
            build(r, Path(tmp), out)
            pymupdf.TOOLS.mupdf_warnings()


def built_pins() -> dict[str, str]:
    """``path -> sha256`` of the manifest's ``[[built]]`` rows."""
    manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
    return {row["path"]: row["sha256"] for row in manifest.get("built", [])}


def manifest_section(pins: dict[str, str]) -> str:
    lines = [MANIFEST_BEGIN, ""]
    for r in recipes():
        pdf, txt = f"corpus-d/{r.name}.pdf", f"corpus-d/{r.name}.txt"
        lines += [
            "[[fixture]]",
            f'path = "{pdf}"',
            f'sha256 = "{sha256_bytes((FIXTURES / pdf).read_bytes())}"',
            f'generator = "{r.generator}"',
            "compat_points = []",
            'gate = "ocr"',
            f'subset = "{r.subset}"',
            f'truth = "{txt}"',
            "",
            "[[fixture]]",
            f'path = "{txt}"',
            f'sha256 = "{sha256_bytes((FIXTURES / txt).read_bytes())}"',
            f'generator = "testkit::corpus_d::{r.source}"',
            "compat_points = []",
            'gate = "skip"',
            "",
        ]
    # The noise variants: pinned, never committed (spec §3.8 "Pinning").
    for name in noise_names():
        path = f"{name}.pdf"
        if path not in pins:
            continue
        source = name.removesuffix("_noise")
        lines += [
            "[[built]]",
            f'path = "{path}"',
            f'sha256 = "{pins[path]}"',
            'generator = "testkit::corpus_d::noise"',
            'gate = "ocr"',
            'subset = "noise"',
            f'source = "corpus-d/{source}.pdf"',
            f'truth = "corpus-d/{source}.txt"',
            "",
        ]
    lines.append(MANIFEST_END)
    return "\n".join(lines) + "\n"


def replace_between(text: str, begin: str, end: str, body: str) -> str:
    """`body` replaces the marked section (markers included), or is appended."""
    if begin in text:
        start, stop = text.index(begin), text.index(end) + len(end)
        return text[:start] + body.rstrip("\n") + text[stop:]
    return text.rstrip("\n") + "\n\n" + body


def describe(r: Recipe) -> str:
    what = {
        "clean": "clean scan",
        "skew": f"+{SKEW_DEGREES:g} deg bicubic rotation, white fill",
        "blur": f"Gaussian blur radius {BLUR_RADIUS:g} px",
        "lowres": "clean scan",
        "jpeg": f"clean scan, DCTDecode quality {JPEG_QUALITY}",
    }[r.subset]
    return f"`{r.source}` at {r.dpi} dpi, {what}"


def provenance_section(pins: dict[str, str]) -> str:
    rows = [PROV_BEGIN, "| File | SHA-256 | Recipe |", "|---|---|---|"]
    for r in recipes():
        rel = f"corpus-d/{r.name}.pdf"
        digest = sha256_bytes((FIXTURES / rel).read_bytes())
        rows.append(f"| `{rel}` | `{digest}` | {describe(r)} |")
    for name in noise_names():
        path = f"{name}.pdf"
        if path in pins:
            source = name.removesuffix("_noise")
            rows.append(
                f"| `built/{path}` | `{pins[path]}` | built at test time (not committed) by "
                f"`testkit::corpus_d::noise_variant` from `corpus-d/{source}.pdf`: "
                "Gaussian noise sigma 12, seed 20261003, unfiltered |"
            )
    rows.append(PROV_END)
    return "\n".join(rows) + "\n"


def write_sections(pins: dict[str, str]) -> None:
    """Rewrites the corpus-D blocks of the manifest and PROVENANCE.md."""
    MANIFEST.write_text(
        replace_between(
            MANIFEST.read_text(encoding="utf-8"),
            MANIFEST_BEGIN,
            MANIFEST_END,
            manifest_section(pins),
        ),
        encoding="utf-8",
    )
    PROVENANCE.write_text(
        replace_between(
            PROVENANCE.read_text(encoding="utf-8"), PROV_BEGIN, PROV_END, provenance_section(pins)
        ),
        encoding="utf-8",
    )


def cmd_pin_built(built: Path) -> int:
    """Pins the Rust-built noise variants found in `built`."""
    missing = [n for n in noise_names() if not (built / f"{n}.pdf").is_file()]
    if missing:
        raise SystemExit(f"mkcorpusd: {built} lacks {missing} (run the corpus_d_noise example)")
    pins = {f"{n}.pdf": sha256_bytes((built / f"{n}.pdf").read_bytes()) for n in noise_names()}
    write_sections(pins)
    for path, digest in pins.items():
        print(f"{digest}  {path}")
    print("pinned; now run gen.py --ocr --built and mkcorpusd.py --verify --built")
    return 0


def cmd_generate() -> int:
    pymupdf.TOOLS.mupdf_display_errors(False)
    # Regenerating the clean files leaves the noise pins valid: the variants
    # depend on the clean files' pixels and boxes, never on their bytes.
    pins = built_pins()
    for stale in CORPUS_D.glob("*"):
        if stale.is_file():
            stale.unlink()
    generate(CORPUS_D)
    write_sections(pins)
    total = sum(p.stat().st_size for p in CORPUS_D.iterdir())
    print(f"wrote {len(recipes())} files to {CORPUS_D} ({total / 1e6:.1f} MB)")
    return 0


def image_of(doc: pymupdf.Document) -> tuple[int, pymupdf.Pixmap]:
    images = doc[0].get_images(full=True)
    if len(images) != 1:
        raise ValueError(f"{len(images)} images on the page")
    xref = int(images[0][0])
    return xref, pymupdf.Pixmap(doc, xref)


def expected_size(mediabox: pymupdf.Rect, dpi: int) -> tuple[int, int]:
    irect = (mediabox * pymupdf.Matrix(dpi / 72, dpi / 72)).irect
    return irect.width, irect.height


def verify_file(r: Recipe, sources: Path) -> list[str]:
    """The oracle's view of one committed file matches its recipe."""
    path = CORPUS_D / f"{r.name}.pdf"
    problems: list[str] = []
    doc = pymupdf.open(path)
    src = pymupdf.open(sources / f"{r.source}.pdf")
    try:
        page, spage = doc[0], src[0]
        if doc.page_count != 1:
            problems.append(f"{doc.page_count} pages")
        if page.get_text("text") != "":
            problems.append("has a text layer")
        if page.rotation != spage.rotation:
            problems.append(f"rotation {page.rotation}, source {spage.rotation}")
        if page.cropbox != spage.cropbox or page.mediabox != spage.mediabox:
            problems.append("page boxes differ from the source")
        xref, pix = image_of(doc)
        want = expected_size(spage.mediabox, r.dpi)
        if (pix.width, pix.height) != want:
            problems.append(f"image {pix.width}x{pix.height}, expected {want}")
        if pix.n != 1 or pix.alpha:
            problems.append(f"image has {pix.n} components, alpha {pix.alpha}")
        flt = doc.xref_get_key(xref, "Filter")[1]
        if flt != ("/DCTDecode" if r.jpeg else "/FlateDecode"):
            problems.append(f"image filter {flt}")
        if doc.xref_get_key(xref, "BitsPerComponent")[1] != "8":
            problems.append("image is not 8 bits per component")
        truth = (CORPUS_D / f"{r.name}.txt").read_bytes()
        if truth != (sources / f"{r.source}.txt").read_bytes():
            problems.append("ground truth differs from the test kit's")
    except Exception as exc:  # the oracle's own failure is the observation
        problems.append(f"{type(exc).__name__}: {exc}")
    finally:
        doc.close()
        src.close()
    return problems


def samples(path: Path) -> bytes:
    doc = pymupdf.open(path)
    try:
        return bytes(image_of(doc)[1].samples)
    finally:
        doc.close()


def verify_built(name: str, built: Path, pins: dict[str, str]) -> list[str]:
    """One Rust-built noise variant: pinned bytes, and the oracle sees an
    image-only page with one unfiltered 8-bit gray image of the clean
    file's size, boxes and rotation."""
    path, clean_path = built / f"{name}.pdf", CORPUS_D / f"{name.removesuffix('_noise')}.pdf"
    if not path.is_file():
        return ["missing (run the corpus_d_noise example)"]
    problems: list[str] = []
    digest = sha256_bytes(path.read_bytes())
    pin = pins.get(f"{name}.pdf")
    if pin is None:
        problems.append("no [[built]] pin (run --pin-built)")
    elif digest != pin:
        problems.append(f"sha256 {digest}, pinned {pin}")
    doc, clean = pymupdf.open(path), pymupdf.open(clean_path)
    try:
        page, cpage = doc[0], clean[0]
        if doc.page_count != 1:
            problems.append(f"{doc.page_count} pages")
        if page.get_text("text") != "":
            problems.append("has a text layer")
        if page.rotation != cpage.rotation:
            problems.append(f"rotation {page.rotation}, clean {cpage.rotation}")
        if page.cropbox != cpage.cropbox or page.mediabox != cpage.mediabox:
            problems.append("page boxes differ from the clean file")
        xref, pix = image_of(doc)
        _, cpix = image_of(clean)
        if (pix.width, pix.height) != (cpix.width, cpix.height):
            problems.append(f"image {pix.width}x{pix.height}, clean {cpix.width}x{cpix.height}")
        if pix.n != 1 or pix.alpha:
            problems.append(f"image has {pix.n} components, alpha {pix.alpha}")
        if doc.xref_get_key(xref, "Filter")[0] != "null":
            problems.append("image stream is filtered")
        if doc.xref_get_key(xref, "BitsPerComponent")[1] != "8":
            problems.append("image is not 8 bits per component")
        if doc.xref_get_key(xref, "ColorSpace")[1] != "/DeviceGray":
            problems.append("image is not DeviceGray")
    except Exception as exc:  # the oracle's own failure is the observation
        problems.append(f"{type(exc).__name__}: {exc}")
    finally:
        doc.close()
        clean.close()
    return problems


def cmd_verify(compare_pixels: bool, built: Path | None) -> int:
    pymupdf.TOOLS.mupdf_display_errors(False)
    failures: list[str] = []
    manifest = tomllib.loads(MANIFEST.read_text(encoding="utf-8"))
    listed = {e["path"] for e in manifest.get("fixture", []) if e["path"].startswith("corpus-d/")}
    want = {f"corpus-d/{r.name}.{ext}" for r in recipes() for ext in ("pdf", "txt")}
    if listed != want:
        failures.append(f"manifest corpus-d rows differ from the recipes: {sorted(listed ^ want)}")
    with tempfile.TemporaryDirectory(prefix="pdf-extract-corpus-d-") as tmp:
        sources = Path(tmp) / "sources"
        dump_sources(sources)
        regen = Path(tmp) / "regen"
        if compare_pixels:
            generate(regen)
        for r in recipes():
            problems = verify_file(r, sources)
            if compare_pixels:
                old, new = CORPUS_D / f"{r.name}.pdf", regen / f"{r.name}.pdf"
                if old.read_bytes() == new.read_bytes():
                    problems.append("regenerated bytes are identical (expected a new /ID)")
                if samples(old) != samples(new):
                    problems.append("regenerated pixels differ")
                if (CORPUS_D / f"{r.name}.txt").read_bytes() != (
                    regen / f"{r.name}.txt"
                ).read_bytes():
                    problems.append("regenerated ground truth differs")
            pymupdf.TOOLS.mupdf_warnings()
            print(f"{'ok' if not problems else 'FAIL':4} {r.name} {'; '.join(problems)}")
            failures += [f"{r.name}: {p}" for p in problems]
    if built is not None:
        pins = built_pins()
        for name in noise_names():
            problems = verify_built(name, built, pins)
            pymupdf.TOOLS.mupdf_warnings()
            print(f"{'ok' if not problems else 'FAIL':4} built/{name} {'; '.join(problems)}")
            failures += [f"built/{name}: {p}" for p in problems]
    total = sum(p.stat().st_size for p in CORPUS_D.iterdir())
    print(f"{len(recipes())} committed files, {total / 1e6:.1f} MB; {len(failures)} problem(s)")
    return 1 if failures else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--fetch-model", action="store_true", help="download the pinned model")
    parser.add_argument("--verify", action="store_true", help="check the committed corpus")
    parser.add_argument(
        "--compare-pixels", action="store_true", help="with --verify: regenerate and compare"
    )
    parser.add_argument(
        "--built", type=Path, help="with --verify: also check the Rust-built noise variants here"
    )
    parser.add_argument(
        "--pin-built", type=Path, metavar="DIR", help="pin the noise variants built into DIR"
    )
    args = parser.parse_args()
    if (args.compare_pixels or args.built) and not args.verify:
        parser.error("--compare-pixels and --built need --verify")
    if args.fetch_model:
        return cmd_fetch_model()
    if args.pin_built:
        return cmd_pin_built(args.pin_built)
    if args.verify:
        return cmd_verify(args.compare_pixels, args.built)
    return cmd_generate()


if __name__ == "__main__":
    # A stale marker pair would make the section replacement eat the file.
    for path, begin, end in (
        (MANIFEST, MANIFEST_BEGIN, MANIFEST_END),
        (PROVENANCE, PROV_BEGIN, PROV_END),
    ):
        text = path.read_text(encoding="utf-8")
        if text.count(begin) > 1 or (begin in text) != (end in text):
            raise SystemExit(f"mkcorpusd: malformed markers in {path}")
    sys.exit(main())
