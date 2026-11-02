# pdf-extract

Thread-safe PDF text extraction in Rust, with a Python binding (`pdf_extract`).

## What it is

pdf-extract opens a PDF, counts its pages honestly (a lying `/Count` is
ignored and `max_pages` bounds the work), and returns each page's text layer in
the shape of PyMuPDF's `get_text("text")`. It exists to replace the part of
PyMuPDF that its first consumer uses, without PyMuPDF's AGPL license. The core is pure
Rust on a patched fork of the [hayro](https://github.com/LaurenzV/hayro) crates
(vendored under `vendor/`, including the JBIG2 and JPEG 2000 image decoders, whose
memory use is budgeted). Any input may be hostile: parsing, nesting,
decoding and interpretation are bounded, and every Python call is guarded
against panics. One `Document` can be shared between threads, and extraction
releases the GIL; free-threaded CPython 3.14t is supported.

See [`ARCHITECTURE.md`](ARCHITECTURE.md) for the map, invariants and landmines,
[`THEORY.md`](THEORY.md) for why it is built this way, and
[`docs/adr/`](docs/adr/README.md) for the decision log.

## Status

Under construction, nothing published. The text path (open, page index, text
extraction) works from Rust and Python and is checked against PyMuPDF 1.28.2 on
295 synthetic and 60 real-world fixtures (committed expectations; PyMuPDF is
not needed to run the tests). Pages without a text layer can be rendered and
recognised with a vendored Tesseract (OCR; from Python, in helper processes by default).
Fuzzing, sanitizers and loom model checking are in place (long fuzzing runs locally; CI runs them only on demand). Fuzzing depth is not a release gate, so callers must contain failures; see [Running on untrusted uploads](#running-on-untrusted-uploads). Release wheels are built, verified and
attached to GitHub Releases from tags; see [Installing a release](#installing-a-release). The known differences from PyMuPDF are listed in
[`ARCHITECTURE.md`](ARCHITECTURE.md#parity-residuals).

## Install

To build from a checkout instead of installing a release: You need `rustup` (the pinned
toolchain in `rust-toolchain.toml` installs itself on first use), a Python
supported by PyO3 0.29 as `python3` on `PATH` (or `PYO3_PYTHON`; 3.8 to 3.15; the package itself needs 3.13
or later), and for the Python package [`uv`](https://docs.astral.sh/uv/).
Building the OCR engine compiles the vendored Tesseract and Leptonica from
source, so you also need `cmake` and a C++17 compiler. OCR then needs language data at run time: install
`tesseract-ocr-eng` or point `TESSDATA_PREFIX` (or `OcrConfig.tessdata` /
`OcrEngine(tessdata=...)`) at a directory holding `eng.traineddata`; see
[Language data](#language-data). Alternatively, build against a distribution
Tesseract with `--no-default-features --features system-tesseract`
(needs `libtesseract-dev` and `libleptonica-dev`). Distribution builds of
Tesseract are often built with OpenMP and then oversubscribe the CPU when
several pages are recognised at once (`workers > 1`, or several threads sharing
an engine); setting `OMP_THREAD_LIMIT=1` in the environment should avoid
that, but it has not been measured here. The vendored build has OpenMP off.

```sh
uv sync                    # dev tools: maturin, pytest, mypy, ruff
tools/build/stage-helper.sh  # builds the OCR helper into python/pdf_extract/
uv run maturin develop     # builds the extension, installs it into .venv
```

`stage-helper.sh` needs `python3` (it reads the helper's path from cargo's JSON
output; `--profile <name>` replaces its default `--release`). Python OCR runs
Tesseract in that helper by default, so a source build without it gets
`OcrUnavailableError(reason="engine_not_built")` from `Page.ocr()`,
`default_ocr_engine()` and `OcrEngine()` (the message names the script); pass
`isolation="in_process"` to run without it.
`uv sync` removes the `maturin develop` install, so run `maturin develop` again
after it.

To build a release wheel, use `tools/build/build-wheel.sh` rather than
`maturin build`: it stages the helper and writes the wheel's CycloneDX SBOMs
(`.dist-info/sboms/`), which a bare `maturin build` omits.

```sh
tools/build/build-wheel.sh abi3                 # macOS arm64: cp313-abi3
tools/build/build-wheel.sh ft                   # macOS arm64: cp314t
tools/build/build-wheel.sh --docker x86_64 abi3 # Linux: manylinux_2_28, via Docker
uv run --no-sync python tools/build/verify_wheel.py \
    --platform linux-x86_64 --abi abi3 target/wheels/pdf_extract-*-manylinux_2_28_x86_64.whl
```

Linux wheels are built inside the pinned manylinux_2_28 image (`--docker`
needs Docker; build on the matching architecture, emulation is slow).
`verify_wheel.py` checks the helper, the licence files and the SBOMs; see
ARCHITECTURE.md, "Wheels and SBOM".

Free-threaded builds are supported from CPython 3.14t (PyO3 0.29 cannot build
for 3.13t). Run those with `PYTHON_GIL=0`.

## Installing a release

Releases are on this repository's GitHub Releases page: wheels for CPython
3.13 and later (abi3) and for free-threaded 3.14t, for manylinux_2_28 x86_64
and aarch64 and for macOS arm64, plus the sdist and `SHA256SUMS`. They are not
on PyPI. Anyone can download them; no credentials are needed.

Depend on a wheel by its URL. uv records the wheel's hash in `uv.lock`:

```sh
uv add "pdf-extract @ https://github.com/Proven-Correct/pdf-extract/releases/download/v0.1.1/pdf_extract-0.1.1-cp313-abi3-manylinux_2_28_x86_64.whl"
```

With pip, pin the hash from the release's `SHA256SUMS` in a requirements file
and install with `pip install --require-hashes -r requirements.txt`:

```text
pdf-extract @ https://github.com/Proven-Correct/pdf-extract/releases/download/v0.1.1/pdf_extract-0.1.1-cp313-abi3-manylinux_2_28_x86_64.whl --hash=sha256:<from SHA256SUMS>
```

A URL names one platform's wheel. To support several, give one line per
platform with an environment marker, for example
`; sys_platform == "linux" and platform_machine == "aarch64"`.

The wheel has no Python dependencies and needs no compiler or system
libraries; Tesseract and Leptonica are linked in, and the OCR helper ships
beside the extension. OCR still needs language data: install
`tesseract-ocr-eng` (Debian/Ubuntu) or set `TESSDATA_PREFIX`; see
[Language data](#language-data). Each wheel carries CycloneDX SBOMs in its
`.dist-info/sboms/`.

To cut a release, follow [`RELEASING.md`](RELEASING.md). In short: bump
`[workspace.package] version` in `Cargo.toml`, write
`docs/releases/v<version>.md`, merge, then push the tag `v<version>`. The
`Wheels` workflow reruns the licence checks, builds, verifies and smoke-tests
every wheel, installs the Linux ones on stock Ubuntu 24.04 (uv-managed CPython
3.13) and Ubuntu 26.04 (its own Python 3.14), and attaches the files to a
GitHub Release. `gh workflow run wheels.yml --ref <branch>` does the same
without releasing (the files become workflow artifacts).

## Build

```sh
cargo build --locked                   # whole workspace, including the extension
cargo build --release --locked -p pdf-extract-core
```

## Running the tests

Rust (unit, integration, parity gates C and B, doctests):

```sh
cargo test --workspace --locked
cargo test --locked -p pdf-extract-core --features render-support   # core's hooks for rendering/OCR
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked   # CI runs this too
cargo deny check                       # needs cargo-deny
tools/vendor/check-patches.sh          # vendor/ == pristine hayro + vendor/patches (needs network)
tools/vendor/test-check-patches.sh     # tests of the patch tooling (needs full history and the pristine tags)
tools/build/test-stage-helper.sh       # tests of stage-helper.sh (fake cargo)
tools/license/test-gen-notices.sh      # tests of gen-notices.sh's sort_used_by (bash and awk only)
git ls-files -z '*.sh' | xargs -0 uvx --from shellcheck-py==0.11.0.1 shellcheck   # CI lints every shell script
tools/license/gen-notices.sh           # rewrite THIRD_PARTY_NOTICES (needs cargo-about 0.9.2, jq, rustup, Python >= 3.11 as $PYTHON)
```

Hardening checks (need the pinned nightly in `fuzz/rust-toolchain.toml`; the `loom` run needs only the stable toolchain; details in `ARCHITECTURE.md`):

```sh
RUSTFLAGS="--cfg loom" LOOM_MAX_PREEMPTIONS=3 cargo test -p pdf-extract-core --test loom_close --release   # likewise -p pdf-extract-ocr --test loom_pool
tools/sanitizers/run.sh asan           # or tsan; instruments Tesseract too, slow
tools/fuzz/run.sh --minutes 10 open    # local fuzz session; needs cargo-fuzz
```

The OCR quality gate (corpus D; needs the pinned tessdata_fast model, which
`tools/oracle/mkcorpusd.py --fetch-model` downloads; `tools/oracle/README.md`
describes the corpus and how to regenerate it):

```sh
TESSDATA_PREFIX=$PWD/tools/oracle/.cache/tessdata_fast \
    cargo test --release -p pdf-extract-ocr --test quality -- --include-ignored --nocapture
```

`TESSDATA_PREFIX` matters even when a model is installed: without it the lookup
falls through to distribution directories (MacPorts, for one, installs the
combined set), and the gate then fails its model SHA-256 check; it does so
once, up front, naming the directory it found and the `TESSDATA_PREFIX` to set. Under `CI=1`
a missing model fails the gate instead of skipping it.

`--nocapture` makes the per-subset CER and word-F1 tables appear on stderr (libtest
hides them from a passing test otherwise); the same figures are in the JSON.
It writes `target/ocr-report.json` (per-page CER/WER, an order-insensitive word F1, latency, RSS) and
builds corpus D's twelve noise variants into `target/corpus-d/` (about
105 MB, regenerated when missing).

CI runs automatically only on pushes to `main`; pull requests are not run
unless asked (ADR 85). Run a branch with `gh workflow run ci.yml --ref <branch>`.
Run the checks above before pushing: a red `main` is the first CI signal for a
merged change.

### Language data

OCR needs an `eng.traineddata` (the library bundles none; the lookup order is
`OcrConfig.tessdata`, `TESSDATA_PREFIX`, then the usual distribution
directories). Pin **`tessdata_fast`**
([`tesseract-ocr/tessdata_fast`](https://github.com/tesseract-ocr/tessdata_fast),
Apache-2.0, 4.1 MB), SHA-256
`7d4322bd2a7749724879683fc3912cb542f19906c83bcc1a52132556427170b2`. On corpus D
it matched `tessdata_best` and the combined `tessdata` set on accuracy at
300 dpi and was 2.2 times faster per page than `tessdata_best` and 1.5 times faster than the combined set; see
[`docs/reference/tessdata-measurement.md`](docs/reference/tessdata-measurement.md)
(rerun with `tools/oracle/measure_tessdata.sh`). Check what an engine loaded
with `OcrEngine.info()` (`model_sha256`). Current Debian (tesseract-lang 1:4.1.0 and later) ships this
set; Ubuntu is not verified; MacPorts installs the combined set, which also works but is slower. The
CI quality gate's expectations are those of `tessdata_fast` whatever you
deploy.

Python (after `maturin develop`; the tests build their PDFs with the Rust test
kit):

```sh
uv run --no-sync pytest --cov=pdf_extract
uv run --no-sync ruff check python tests/python && uv run --no-sync mypy
```

The OS-thread-count checks run only on Linux, and CI's ubuntu jobs are the only
place they run. The Rust tests `crates/core/tests/extract_threads.rs` and
`thread_poll_panic.rs` are compiled out elsewhere, so a macOS run skips them.
`tests/python/test_extract.py` still runs on macOS; only its `/proc` assertions
are skipped. They check `extract(workers=...)`'s thread bound.

## Coverage

Line coverage of the Rust codebase (requires `cargo-llvm-cov`; `rust-toolchain.toml`
installs `llvm-tools-preview` automatically):

```sh
cargo llvm-cov --workspace --features pdf-extract-core/render-support --locked --no-report
cargo llvm-cov report --lcov --output-path lcov.info
cargo llvm-cov report --summary-only --ignore-filename-regex '(^|/)(vendor|crates/(testkit|python|tesseract-sys))/' --fail-under-lines 95
```

The first command runs the instrumented test suite, the second generates an lcov
report, and the third displays a summary of the gated crates
(`crates/core`, `crates/render`, `crates/ocr`; gating at 95% lines). The test kit, the
Tesseract FFI crate and vendored hayro are reported but not gated.

The reference oracle (PyMuPDF, used only as a black box to make encrypted
fixtures and expectation files) is a separate, dev-only project that nothing
else depends on; see [`tools/oracle/README.md`](tools/oracle/README.md).

## Usage

```python
import pdf_extract
from pdf_extract.testing import PdfBuilder

# Make a small PDF; in real use pass your own path or bytes.
b = PdfBuilder()
page = b.new_page()
page.insert_text((72, 100), "Hello, world", fontsize=12)
page.insert_text((72, 120), "Second line", fontsize=12)
pdf = b.to_bytes()

with pdf_extract.open_bytes(pdf, max_pages=500) as doc:
    print(doc.page_count, doc.is_encrypted)
    for page in doc:
        print(page.index, repr(page.text()))

# prints:
# 1 False
# 0 'Hello, world\nSecond line\n'
```

`pdf_extract.open("report.pdf", max_pages=500)` opens a path. `page.text()` takes
`block_separator="newline" | "blank_line"` and
`unmapped="cid" | "replacement" | "skip"`; `page.text_page()` returns an
immutable `TextPage` that also carries `warnings`. Resource limits are set with
`pdf_extract.Limits(...)` (`open(..., limits=...)`). Failures raise subclasses
of `pdf_extract.PdfExtractError` (`pdf_extract.errors`); a panic inside the
native code arrives as `InternalError`, never as a crash. A panic inside an
image codec (JPEG, JPEG 2000, JBIG2, CCITT) is caught where the codec is
called: the image is skipped with an `image: decode failed` warning and the
rest of the page is processed.

From Rust, `pdf-extract-core` and `pdf-extract-render` let panics propagate,
except that same image-codec panic, which becomes the warning instead of
unwinding into the caller. The library never installs or replaces the
process's panic hook: when it catches a codec panic, the application's hook
runs (Rust's default hook prints the message to stderr). Fuzzing builds
(`--cfg fuzzing`) compile the catch out.

### OCR and `extract()`

```python
engine = pdf_extract.OcrEngine(languages="eng", dpi=300, page_timeout=60.0)
# Tesseract runs in helper processes (isolation="process", the Python
# default), so a crash inside it fails one page instead of the interpreter.
# isolation="in_process" runs it in this process: no helper, but an abort
# inside Tesseract then ends the interpreter.
print(engine.info().model_sha256)       # which eng.traineddata was loaded

with pdf_extract.open("scan.pdf") as doc:
    print(doc[0].ocr(engine).text())     # recognise one page explicitly

result = pdf_extract.extract("report.pdf", ocr=engine, min_chars=50, workers=4)
print(result.ocr_status)                 # ok / partial / unavailable / ...
for p in result.pages:
    print(p.index, p.source, len(p.text))  # text, ocr, or text_fallback
```

**Isolation default (breaking change).** From Python, `OcrEngine()`,
`default_ocr_engine()` and `Page.ocr()` without an engine use process
isolation (ADR 0092); Rust's `OcrConfig::default()` stays in-process. A raster
cap (`max_render_pixels`) above 66,977,776 does not fit one helper frame and
now needs `isolation="in_process"`; measured overhead of the helper is 0.3 to
1.6 %.

`extract()` reads every page's text layer and recognises only pages with fewer
than `min_chars` characters, in page order and up to `max_ocr_pages`; a page
whose OCR fails keeps its text (`source == "text_fallback"`, `ocr_error` set)
and the document-level `ocr_status` says whether OCR was unavailable,
exhausted its budget, or partial. Before recognition, pages rendered at 144 dpi
or more get a cheap noise check: a raster dominated by isolated speckle (where
Tesseract can stall for tens of seconds producing garbage) is recognised at half
resolution instead, or skipped if it is still speckle. Either case is reported
in `warnings` (`page N: ocr: noise-dominated raster ...`); pass
`OcrEngine(noise_precheck=False)` to turn the check off. Without an `ocr=` engine the process-wide
`default_ocr_engine()` is used. OCR errors derive from `PdfExtractError`
(`OcrUnavailableError`, `OcrPageError`, `OcrTimeoutError`).

#### Forking and multiprocessing

A `fork()` copies only the calling thread, so a lock another thread held at
that instant (the first-use initialisation of the default engine or of
Tesseract, the engine's pool, the helper log) stays locked forever in the
child, which then deadlocks. Three rules avoid it:

1. Build every engine, the default one included, before forking; the child
   reuses it and rebuilds its pool.
2. Do not fork while another thread may be inside `Page.ocr()`,
   `OcrEngine(...)`, `default_ocr_engine()` or `extract()` with OCR.
3. In a threaded process prefer `posix_spawn`/`exec` to a bare `fork()`.

For Python `multiprocessing`, use the `spawn` or `forkserver` start method
(`multiprocessing.set_start_method("spawn")`). CPython 3.14 made
`forkserver` the default on POSIX; earlier versions still default to `fork`
on Linux. No `atfork` handler is installed; see the residuals in
[`ARCHITECTURE.md`](ARCHITECTURE.md).

From Rust (`crates/core`, not published):

```rust
use pdf_extract_core::{Document, OpenOptions, TextOptions};

let doc = Document::open("report.pdf", &OpenOptions::default())?;
for page in doc.pages() {
    println!("{}: {}", page.index(), page.text(&TextOptions::default())?);
}
```

## Running on untrusted uploads

Malformed and hostile input is bounded (see [Resource limits](docs/design-spec.md#10-resource-limits)),
and from Python, panics come back as `InternalError`. But the long fuzzing campaign was
waived as a release gate ([ADR 0093](docs/adr/0093-fuzzing-depth-is-not-a-release-gate.md)),
so some input may still crash the process (stack overflow, abort, out of
memory) or hang it. When the files come from users:

- Extract in a process you can afford to lose, such as a job-queue worker or
  a subprocess. Don't extract in a web worker that also serves requests.
- Enforce a wall-clock limit per document by killing that process. A Python
  timeout can't interrupt a call that is running, and the library's time
  limits are per page.
- Cap retries per document, quarantine a document that keeps failing, and log
  which document it was so the input can be reproduced and reported.

## Documents

- [`ARCHITECTURE.md`](ARCHITECTURE.md): map, invariants, landmines, CI jobs, residuals.
- [`THEORY.md`](THEORY.md): theory of operation and design rationale.
- [`docs/adr/`](docs/adr/README.md): one record per architectural decision.
- [`openspec/specs/`](openspec/specs/): the current behavioural contract, one spec per capability.
- [`docs/migrating-from-pymupdf.md`](docs/migrating-from-pymupdf.md): moving an application from PyMuPDF: call mapping, behaviour changes, containment, and an offline parity run before a single cut-over release.
- [`RELEASING.md`](RELEASING.md): the release checklist.
- [`docs/design-spec.md`](docs/design-spec.md): the product design specification.
- [`docs/specs/0001-text-path.md`](docs/specs/0001-text-path.md): the run specification for the text path.
- [`docs/specs/0002-ocr.md`](docs/specs/0002-ocr.md): the run specification for rendering, OCR and process isolation.
- [`docs/specs/0003-ocr-followons.md`](docs/specs/0003-ocr-followons.md): the run specification for the fork-hazard docs, codec memory budgets and noise pre-check.
- [`docs/reference/tessdata-measurement.md`](docs/reference/tessdata-measurement.md): language-data comparison on corpus D and the pinned recommendation.

## License

pdf-extract's own code is licensed under the GNU General Public License,
version 3 or (at your option) any later version (`GPL-3.0-or-later`); see
[`LICENSE`](LICENSE). Its dependencies stay permissively licensed, and every
license shipped with it is GPLv3-compatible. The vendored hayro crates under
`vendor/` keep their upstream license (`Apache-2.0 OR MIT`), and the vendored
Tesseract (Apache-2.0) and Leptonica (BSD-style) keep theirs; see
`vendor/PROVENANCE.md`. Notices for the third-party crates and data compiled
into the binary are in [`THIRD_PARTY_NOTICES`](THIRD_PARTY_NOTICES), which
`tools/license/gen-notices.sh` writes (CI checks it is current),
`crates/testkit/data/README-AFM.txt`, and, for the statically linked Rust
standard library, `tools/license/rust-std-COPYRIGHT.html` (the Rust project's
own notice, copied from the pinned toolchain).

If you distribute software that combines this library with your own code
(a consumer's distribution, for one), the GPL's source-availability obligations
apply to the combined work. Using it inside one organisation without
distributing it carries no such obligation.
