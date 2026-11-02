# Run 0003: OCR follow-ons (fork hazard docs, codec memory budget, noise pre-check)

**Status:** signed off 2026-10-05 (every §7 question takes its proposed default; unit 06 included) · **Date:** 2026-10-05 · **Lands on:** `main` (after internal commit `a8481ce`)
**Inputs:** `docs/specs/0002-ocr.md` (cited as 0002 §n, O-rows and D-rows by id), `docs/specs/0001-text-path.md` (0001 §n), `docs/adr/` (ADR n), the run-0002 end-of-run discovery report (items A1–A4, B4–B16), internal issues #51, #52, #54, #55.

This document specifies the follow-ons to run 0002 that the user decided on
2026-10-05: **A1** the fork-deadlock hazard stays documented rather than
defended, and the documentation is strengthened; **A2** the H13 image memory
bound is extended into the JBIG2 and JPEG 2000 decoders; **A4** a cheap
pre-check on the rendered raster stops Tesseract from stalling on near-blank,
noisy pages. **A3** (flipping the default isolation to `Process`) is not in
this run (§2.1). A fifth, separately droppable unit folds in the small
in-spec defects B4, B5, B6, B9 and B16 of the discovery report, each re-verified
against `main` (§1.4, Appendix B.4); it is marked **PROPOSED — user to confirm**.

Everything here was checked against the sources named in Appendix B (the
vendored fork, the registry copies of `hayro-jbig2` 0.3.0 and `hayro-jpeg2000`
0.3.5, the vendored Tesseract 5.5.3, the crates under `crates/`) or measured
(Appendix A) with a throwaway probe over corpus D on the development machine.
Where a number comes from that probe it says so; the probe is not part of
the deliverable.

Requirement ids carry the prefix **F** (follow-on run). Fork changes continue
run 0002's numbering at **H14**; patch files continue at `0023`. Branches are
`ocr3/NN-slug`.

---

## Revision log

**2026-10-05.** First draft.

**2026-10-05.** Signed off: Q1–Q9 take their proposed defaults; unit 06 is in the run.

**2026-10-05.** Write-back from unit 01: F2 now states the verified 3.14 change (`fork` → `forkserver` on POSIX platforms; macOS has defaulted to `spawn` since 3.8).

**2026-10-05.** Write-back from unit 03. (1) The committed patch
series carried mixed `[PATCH n/N]` subject totals from incremental exports
(`01/19`…`19/19`, `20/20`, `21/21`, `22/22`); even the original
`export-patches.sh` rewrote those headers, so "byte-identical" was never true
of the headers. Unit 03 runs the export once and commits the normalised
subjects (range 1 counts `n/22` within its range and stays stable as range 2
grows); the invariant is now: patch **bodies** byte-identical, headers
normalised once in unit 03, regeneration byte-identical thereafter. (2)
cargo-about orders path crates before registry crates, so vendoring moves
`hayro-jbig2`/`hayro-jpeg2000` up in Part 1's MIT "Used by" list; the
regenerated `THIRD_PARTY_NOTICES` is committed in unit 03 and must be clean
thereafter. Changed: F10, §3.2 (two bullets), §3.6 `deny` row, §4.3 items 3
and 4.

**2026-10-05.** Write-back from unit 04 (H14), verified against T.88 §6.5.10
and the vendored decoder. (1) §4.4's symbol-export bomb was unsatisfiable as
written: T.88's export flags are run lengths that partition the dictionary's
input + new symbol indices, so one dictionary exports each symbol at most
once. The nearest shape is two dictionaries — one creates the 8 MiB symbol
and exports it once (a legitimate copy), the next names that dictionary
65,535 times and exports its whole input list — and the stream is 74 KB, not
"a few hundred bytes". The RSS bound becomes 24 MiB (one legitimate copy;
measured 18 MB macOS / 9 MB Linux, 61 MB when refusing copy by copy) plus a
requested-heap bound from a counting allocator (< 17 MiB for this case,
< 1 MiB for the 40,000² halftone), which is what proves "before the
allocation". (2) Export decodes every run length and budget-checks the total
copy size before the first clone; the spec states no error order, and on
malformed export flags the count-mismatch error may now precede an
out-of-range error (both `ImageDecode`) — recorded in §4.4. (3) F9's
"legitimate file refused" gains its first concrete instance in §2.2:
refinement/aggregation dictionaries re-charge the all-symbols list and the
uniform Huffman table per aggregated symbol (≈ 44 B × N²/2), so about 5,000
Huffman-coded aggregate symbols in one dictionary are refused at the default
cap. (4) §4.5 had the same flaw in mirror image: a JPEG 2000 `Ok` leg that
"decodes to the same pixels" needs a codestream with one tile-part per tile
(90,000 for a 1 × 1-tiled 300 × 300 image), which patching a committed
file's `SIZ` cannot produce; the refusal legs stay header-patched and the
`Ok` legs now use test-kit codestreams or committed encoder output.
Changed: §2.2 (codec row), F9, §4.4 item 1, §4.5 item 1.

**2026-10-05.** Write-back from the unit 05 review (PASS), each item checked
against the vendored decoder and the unit's tests. (1) The default JPEG 2000
budget is 64 × 256,000,000 = 16.4 GB (15.3 GiB), not "16 GiB". (2) The
multiplier's margin at the cap is **1.5×**, measured on OpenJPEG output
(512²: gray 15, RGB 33, RGB in 128² tiles 17, RGB with 8 resolutions and
16 × 16 code-blocks 37, four components 42 B/px), not the derived 4×/1.8×; the
extrapolated 16-component case is unreachable because hayro-jpeg2000 accepts
at most four components (`TooManyChannels` before any decode), so A.5's
"revisit Q1" trigger does not fire. (3) A tile-count overflow surfaces as
`ImageTooLarge` → `ImageDecode`, not `ImageMemoryLimit` (§2.2 now matches
§4.5). (4) H15 charges reused buffers by capacity growth
(`MemoryBudget::reserve`/`reserve_exact`; reuse is free), so charged bytes
track allocated capacity (unit 05 asserts within 10 % of peak heap) and the
"`Vec` growth up to 2×" caveat no longer applies to what H15 charges; the
uncharged stable-sort scratch (≈ half the precinct list) stays within the
uncounted allowance; a reused `DecoderContext`'s leftover capacity is not
re-charged (hayro-syntax always decodes with a fresh context). (5) A.5
records the 1 × 1 code-block cost, 242.1 B/px, and the `Ok`-leg constant
`PRECINCT_1_BYTES_PER_PIXEL = 256`: such files are refused above about a
quarter of `max_image_pixels`, a non-default encoder geometry. Changed: F5,
F7, §2.2 (codec row), §4.5, Appendix A.5.

**2026-10-05.** Landing order: the PR stack is 01 → 03 → 04 → 05 → 02 → 06 (02 and 06 are independent of the codec chain, §5), so the critical path lands without waiting on 02. Unit numbers are unchanged.

**2026-10-05.** Write-back from unit 02; **changes signed-off behaviour,
re-sign-off needed** (§7, Q10). The implementer found that F11's rule
"`OtsuClass::None` ⇒ `Speckle`" skips real text: a page whose dark side is
25–75 % of the raster (an inverted banner, a figure, a shaded half) stays
`None` after the downsample and was skipped by default, where Tesseract reads
it in 0.5–2 s; and that `BackgroundDark ⇒ Clear` let an inverted noise page
stall for 27 s. Both were reproduced and the implementer's amendment measured
(Appendix A.6, 19 new rasters): the isolation test now runs on **Tesseract's
own foreground side** for every non-`Empty` class, and the class never
decides by itself. On corpus D (52 pages × 72/150/300 dpi) the old and new
rules agree everywhere. Also: §4.2's "equals the run-0002 result recorded in
unit 12's report" was unsatisfiable (no per-page text is committed; OCR text
is never compared exactly across hosts) and is replaced by an on/off
byte-equality check within one run; §2.3 lists the newly measured residuals
(correlated noise, photo pages); `MIN_DPI_FOR_PRECHECK` is `precheck::MIN_DPI`
everywhere. Changed: F11, F12, F13, F14, F16, F17, §2.3, §3.3 (`Stats`),
§4.2, §7 (Q10), Appendix A.2 (margins), new A.6.


**2026-10-05.** Q10 signed off by the user: the revised F11 rule (foreground-side isolation for every non-`Empty` class) is accepted as proposed.

**2026-10-05.** Write-back from the unit 02 review (PASS), each item checked
against the code: §2.1's inverted-page non-goal contradicted revision (3)
and is reworded; §3.3 names `Stats::verdict()`; §4.2's rectangle pin states
its raster size (on an A4 raster the rectangle is under `MIN_FOREGROUND`);
the test PDFs are assembled by `testkit::corpus_d::scan_pdf` with a page box
a thousandth of a point short of the exact 300-dpi size so the renderer maps
pixels 1:1; §3.6/§4.2 name `ocr-report-precheck-off.json`, its
`reference_0002` block, the `adversarial` block and the `foreground_dark`
field; F14/§3.1 state the `Extraction` note's detail and display. Changed:
§2.1, §3.1, §3.3, §3.6, F14, §4.2.

**2026-10-21.** Wording write-backs from the 2026-10-21 bug-fix pass; no
behaviour change, and each edited passage carries its own dated note. F21 and
§2.2 (descriptor sweep): the bound is exclusive, so descriptors *at or above*
`MAX_SWEEP` stay unmarked, and the sweep stops at a plausible
`sysconf(_SC_OPEN_MAX)`, else the soft `RLIMIT_NOFILE`. F6 and the §2.2
JBIG2/JPEG 2000 row: allocations "linear in the input, bounded by
`max_file_size`" is not true of the referred-to segment list (up to 2 GiB per
segment header, bounded by `max_stream_decoded_bytes`) or of the MMR vectors
of JBIG2's embedded `hayro-ccitt` decode (ADR 0079 addendum). §2.3 and A.6: the
2.2 s correlated-noise case is a single report with no fixture. §4.5/A.5: since
H16 a 256-component palette image is refused at the header, so it no longer
reaches plane charging.

---

## Contents

1. [Behavioural requirements](#1-behavioural-requirements)
2. [Non-goals and bounded claims](#2-non-goals-and-bounded-claims)
3. [Interface changes](#3-interface-changes)
4. [Verification oracle](#4-verification-oracle)
5. [Work units](#5-work-units)
6. [Decisions that need records](#6-decisions-that-need-records)
7. [Open questions for the user](#7-open-questions-for-the-user)
- [Appendix A: Measurement evidence](#appendix-a-measurement-evidence)
- [Appendix B: Facts verified against source](#appendix-b-facts-verified-against-source)

---

## 1. Behavioural requirements

### 1.1 Fork hazard: documented, not defended [A1; 0002 §2.4; O11, O12, O30]

The user decided: no `atfork` handlers. The behaviour of the code does not
change; what changes is that every place a caller looks before forking says
the same, complete thing. The hazard, stated once here and quoted by the
documents below:

> A `fork()` while another thread of the process is (a) inside the first
> `default_ocr_engine()` construction, (b) inside the process-wide Tesseract
> initialisation (`pxt_global_init` under its `OnceLock`), (c) holding an
> engine's pool lock (checking out, creating or returning an instance), or
> (d) holding the process-isolation client's `LOG`/`TEST_ENV` lock or the
> tessdata candidate override lock, leaves that lock held forever in the
> child. The child's next OCR call deadlocks. O11/O30 rebuild the pool
> *after* the child's first checkout succeeds; they cannot help a child that
> never gets that far. A fork while no thread is inside the OCR crates is
> safe: the child's first OCR call pays one `Init` (in process) or one helper
> spawn (process isolation) and continues.

| ID | Requirement |
|---|---|
| F1 | **Rust docs.** The crate-level documentation of `pdf-extract-ocr` gains a `# Forking` section carrying the hazard statement and the three rules: build every engine (the default one included) before forking; do not fork while another thread may be inside `Page::ocr`, `OcrEngine::tesseract`/`default_ocr_engine` or `Document::extract` with `ocr` set; prefer `posix_spawn`/`exec` over a bare `fork` in a threaded process. `default_ocr_engine`, `OcrEngine`, the `pool` module, `isolation::client` (`LOG`, `TEST_ENV`), `tessdata` (`OVERRIDE`) and `pdf-extract-tesseract-sys` (`GLOBAL_INIT`) each carry a one-paragraph cross-reference naming which lock they hold and linking to that section. `cargo doc -D warnings` stays green. |
| F2 | **Python docs.** `OcrEngine`, `default_ocr_engine` and `extract` in `_native.pyi` get docstrings (today `OcrEngine`'s says nothing about forking and the other two have none) stating the hazard in Python terms: `os.fork()` and `multiprocessing` with the `fork` start method; the safe patterns are the `spawn` or `forkserver` start method, or forking before the first OCR call (an engine built before the fork is reused by the child, O11/O30); the note that CPython 3.14 changed the default start method from `fork` to `forkserver` on POSIX platforms (verified against the 3.14 `multiprocessing` docs and quoted with their exact wording). The pyclass doc in `crates/python/src/ocr.rs` is the same text (it is what `OcrEngine.__doc__` shows). `stubtest` stays green. |
| F3 | **README and pickup docs.** The README's OCR section gets a short "Forking and multiprocessing" paragraph (today one sentence) with the three rules and the `spawn`/`forkserver` recommendation; `ARCHITECTURE.md` names every lock of the hazard statement (the `OVERRIDE` and `GLOBAL_INIT` locks are missing today) in its invariants and keeps the residual bullet; `THEORY.md` records *why* no `atfork` handler is installed (a `OnceLock` mid-initialisation has no safe reset; releasing a pool lock in the child would hand it an instance another thread was using; a library installing process-wide `pthread_atfork` handlers into a host process is itself a global side effect) in one paragraph. |
| F4 | **Spec write-back for B7.** 0002 O12 is amended in place (a wording correction, not a behaviour change): an `Err` from the first construction is stored and returned forever; a **panic** inside it propagates to the caller (ADR 35) and leaves the `OnceLock` empty, so the next call constructs again. The `default_ocr_engine` docs say the same. No code changes (Appendix B.4, B7). |

### 1.2 Codec memory budget: H13 extended into hayro-jbig2 and hayro-jpeg2000 [A2; 0002 O4, §2.2, D-8; ADR 62]

**Where the crates live (Appendix B.2).** `hayro-jbig2` 0.3.0 and
`hayro-jpeg2000` 0.3.5 are exact crates.io pins (`crates/core/Cargo.toml`,
"not used directly"), consumed only by the vendored `hayro-syntax` through its
`images` feature; they are not vendored, not patched and not forked.
Changing them means **vendoring them the way the three patched hayro crates
are vendored** (ADR 3): a pristine import under `vendor/hayro-jbig2` and
`vendor/hayro-jpeg2000`, two rows in `vendor/PROVENANCE.md` (which `import.sh`
already selects by its `^| hayro-` pattern), two `[patch.crates-io]` entries,
a second pristine tag, and a change to `export-patches.sh`, whose crate list
is hard-coded and whose single base cannot express "three crates patched
since tag 1, two more pristine since tag 2" (§3.2). A git dependency is not
an option (`deny.toml` `[sources] allow-git = []`). This extends ADR 3's
"vendor only what we patch" rule and contradicts its *title* ("only the three
patched hayro crates"); §6 records the superseding note. ADR 55 (hayro itself
pinned, not vendored) is unaffected.

**Budget semantics.** One cumulative byte budget per decode call, derived
from the existing `DecodeLimits.max_image_pixels` (no new `Limits` field, Q1):

| ID | Requirement |
|---|---|
| F5 | **Derivation.** For an image stream decoded under `DecodeLimits { max_image_pixels: Some(P) }`, the JBIG2 decoder runs with a budget of `JBIG2_BUDGET_BYTES_PER_PIXEL × P` bytes and the JPEG 2000 decoder with `JPX_BUDGET_BYTES_PER_PIXEL × P` bytes, where the two constants live in hayro-syntax's `filter/mod.rs` with the values **2** and **64** (Appendix A.5 derives them: a sane JBIG2 page needs at most about 0.5 B per page pixel in total, a sane JPEG 2000 image about 9 B per pixel per component, so 2 leaves a 4× margin at the pixel cap for JBIG2 and 64 a **1.5×** margin for the costliest legitimate JPEG 2000 geometry measured — four components at 42 B/px; gray 15, RGB 33, tiled RGB 17, RGB with 8 resolutions and 16 × 16 code-blocks 37 (unit 05, OpenJPEG output) — with a far larger margin for ordinary images, since the cost is per *image* pixel and the cap is 256 M of them; the hostile shapes cost hundreds of bytes per pixel or are unbounded). With the default `Limits` (256,000,000 pixels) the budgets are 512 MB and 16.4 GB (15.3 GiB). `max_image_pixels: None` means no budget, as today. |
| F6 | **What is counted (JBIG2).** Every heap allocation whose size is a function of file-declared values, charged *before* it is made: every `Bitmap` (page, generic/refinement/text/halftone region, symbol, height-class collective, aggregation, pattern collective, each stored intermediate region) at `stride × height × 4` bytes, **including clones** (symbol export clones a `Bitmap` per exported symbol); the halftone gray-scale value plane (`4 × gw × gh`), its two bit planes and the HSKIP bitmap; the shifted-pattern table; the `SymbolIdDecoder` context table (`1 << code_len` entries); the uniform Huffman table lines; the per-text-region refinement bitmaps; the `Vec<&Bitmap>` and cloned-table collections built from `referred_to_segments` (8 bytes per entry, tables at their size). The list of sites is Appendix B.2; a reviewer checks it against the fork diff. Not counted: fixed-size context tables (≤ 2¹⁶ entries), allocations linear in the input bytes (segment headers, referred-to lists, custom Huffman lines) — these are bounded by the stream, which is bounded by `max_file_size`. *Revised 2026-10-21*: that does not hold for the referred-to segment list, whose `Vec::with_capacity` in `segment.rs` is sized from the declared 29-bit count rather than from bytes read (about 32 × the retention bytes, up to 2 GiB for one segment header; transient and uncharged, run in `Image::new_embedded` before any budget exists, and bounded by `max_stream_decoded_bytes` since the stream can be filter output, not by `max_file_size`); and the MMR coder inside a JBIG2 generic region (`hayro-ccitt`, not vendored) keeps O(width) vectors uncharged (width at most 65,535: about 1–2 MiB per region, transient). ADR 0079 addendum 2026-10-21. |
| F7 | **What is counted (JPEG 2000).** The per-tile records (`num_tiles × (size_of::<Tile>() + num_components × size_of::<ComponentInfo>() + the two per-component `Vec`s)`), charged once from the header before any `Tile` is built, which also replaces the unchecked `num_x_tiles × num_y_tiles` multiplication with a checked one (overflow is a refusal); the per-component channel planes (`4 × w × h` each, for every component the decoder will keep, including the palette-indexed case that skips channel validation today); the per-tile coefficient buffer; the per-tile structural records (sub-band, precinct, tag-tree node, code-block and `Layer` entries), charged as they are pushed; the code-block decode buffers and the IDWT buffer; the output buffer. Buffers and lists the decoder reuses across tiles and code-blocks are charged by **capacity growth** (`MemoryBudget::reserve`/`reserve_exact`: reuse of capacity already paid for is free), so the bytes charged track the bytes allocated (unit 05 asserts the two within 10 % of each other with a counting allocator). Not counted: marker/box parsing allocations linear in the input; the stable sort's scratch space over the precinct list (about half that list's size); and, for a caller that reuses a `DecoderContext` across decodes, the capacity left over from the previous decode — hayro-syntax never does, it decodes with a fresh context every time. |
| F8 | **Refusal.** A charge that would take the cumulative total over the budget fails the decode *before* the allocation it governs: hayro-jbig2 returns `DecodeError::Overflow(OverflowError::MemoryBudget)`, hayro-jpeg2000 `DecodeError::Validation(ValidationError::MemoryBudget)`. hayro-syntax maps exactly those variants to a new `DecodeFailure::ImageMemoryLimit` and bumps a new thread-local `image_memory_limit_hits()` (same `std`-only pattern as `image_pixel_limit_hits()`); every other codec error stays `ImageDecode`. hayro-interpret's single `ImageDecodeFailure` site maps `ImageMemoryLimit` to a new `InterpreterWarning::ImageMemoryLimit`; core and the render crate surface it as `WarningKind::ImageMemoryLimit` (category `image`, text `image: memory limit`), sampled in the same signal window as the pixel-limit counter (the larger of the thread-local delta and the sink count, as for `ImagePixelLimit`). A refused image draws nothing; the page is otherwise rendered and recognised. A refusal is counted once per image, as a memory-limit warning, never as a decode failure and never as a pixel-limit warning (Q2). |
| F9 | **Budget is cumulative and per call.** The total is reset at the start of every `decode` call (JBIG2 globals are charged within the decode that uses them) and never refunded when a buffer is dropped. Consequence, stated not hidden: a legitimate file whose transient allocations over one decode exceed the budget is refused (F8), which the margins of F5 make rare for sane encoders (Appendix A.5) and which `Limits.max_image_pixels` lets a caller raise. The one measured instance (unit 04) is in §2.2: thousands of refinement/aggregation symbols in one JBIG2 dictionary, whose per-symbol re-charge of the reference list and Huffman table is quadratic. |
| F10 | **Unchanged behaviour.** Images under the cap decode byte-identically (the corpus-B and corpus-C parity gates, the render golden hashes and the H13 tests of 0002 §4.4 pass unchanged). CCITT and DCT are untouched (both already bounded, 0002 Appendix B.4). The patch series 0001–0022 is unchanged in substance after the vendoring: every patch **body** is byte-identical; the `[PATCH n/N]` subject headers, which the committed series carried with mixed totals from incremental exports, are normalised once by unit 03's single export and are byte-identical on every regeneration thereafter (the review of the vendoring commit is the two pristine trees, the tooling and that one header normalisation). |

### 1.3 Noise pre-check before Tesseract [A4; 0002 §2.4 near-blank stall; O16, O18, O19, O23, O34]

**The mechanism, measured (Appendix A.1–A.4).** Tesseract binarises the
whole raster with one global Otsu threshold (`thresholding_method = 0`,
`OtsuThreshold` over the full rectangle; Appendix B.3). On a page that holds
little real ink the between-class variance is maximised by splitting the
*noise* instead of separating text from paper: on `sparse_below_min_chars_noise`
at 300 dpi the threshold is 245, 21.7 % of the page becomes foreground, the
foreground is 1.09 million connected components of which 987 thousand are
three pixels or fewer, and layout analysis spends 16–25 s producing garbage
(CER 0.93). The same page box-filtered 2 × 2 to 150 dpi has threshold 147,
0.06 % foreground, and is read exactly in 22 ms. A pure-noise blank page costs
46 s at 300 dpi and still 5.5 s of garbage after one downsample, where Otsu
finds no convincing foreground class at all. Pages with ordinary amounts of
text are unaffected by the same noise (every other noise variant: foreground
mostly dark, isolated-pixel fraction 0.000, 0.9–2.2 s). Gray text with no dark
pixel at all reads perfectly (64 ms), so a rule on "dark pixels" would misfire;
the two signals that separate the pathological class are Otsu's own
foreground verdict and the **isolation fraction** of the foreground.

| ID | Requirement |
|---|---|
| F11 | **The statistic.** `precheck::classify(img: &GrayImage) -> Verdict` computes, on the raster that would go to the backend: the 256-bin histogram; the Otsu threshold `t`, `H` and `ω₀ = Σ_{i≤t} h[i]` **by the same algorithm as Tesseract's `OtsuStats`** (maximise `ω₀·ω₁·(μ₀−μ₁)²` over `t ∈ 0..=254`, first maximum wins, `ω₀` counts pixels `≤ t`); Tesseract's class verdict (`Empty` if `ω₀ ∈ {0, H}`; `ForegroundDark` if `ω₀ < 0.25 H`; `BackgroundDark` if `ω₀ > 0.75 H`; `None` otherwise); and **Tesseract's foreground side** — pixels `≤ t` when `ω₀ < H/2` (its `hi_value = 1`), pixels `> t` otherwise — which is the side Tesseract binarises as ink for every class, the "best of the bad lot" it keeps for `None` included (Appendix B.3). `p` is that side's share of the raster (`ω₀/H` or `1 − ω₀/H`). For every class other than `Empty`, when `p ≥ MIN_FOREGROUND = 0.01`, the isolation fraction `iso` = share of foreground-side pixels with no foreground-side 4-neighbour (missing neighbours at the edges count as background). The verdict is **`Speckle`** iff the class is not `Empty`, `p ≥ 0.01` and `iso ≥ ISOLATION_FACTOR × (1 − p)⁴` with `ISOLATION_FACTOR = 0.5` (`(1 − p)⁴` is the isolation fraction of independent random pixels at density `p`; connected ink gives ≈ 0). Otherwise **`Clear`**; `Empty` is always `Clear`. **The class never decides by itself** (revision (3)): a `None` class — a dark region of 25–75 % of the raster, as an inverted banner, a figure or a shaded half produces — is `Clear` when its foreground is connected, and a `BackgroundDark` class — an inverted page — is `Speckle` when its light side is noise. Margins measured (Appendix A.2, A.6): every noise-dominated raster, of either polarity, is ≥ 2× above the line at 300 dpi; every text, banner, figure, slide and photo page is at isolation `0.000`–`0.016` against lines of 0.03 or more; corpus D at 72, 150 and 300 dpi is unchanged by the revision. The function is pure, deterministic and allocates nothing. |
| F12 | **Where it runs.** In `OcrEngine::ocr`, after rendering and the pre-recognition budget check and before the backend call (so for every backend — `TesseractBackend` in process, the process-isolation client, `ScriptedBackend` — and, under `Isolation::Process`, in the **parent**, which still loads no Tesseract and no Leptonica, O29). It runs only when `cfg.noise_precheck` is true (default; §3.3) and the rendered resolution `dpi'` is at least `precheck::MIN_DPI = 144`; below that the raster goes to the backend unchanged (a 72-dpi downsample would destroy text; Appendix A.2 shows the 72-dpi renders are never speckle-dominated in corpus D). Its time counts against `page_timeout` like rendering does (O16). |
| F13 | **Remedy: downsample, re-check, else skip.** On `Speckle`: the raster is box-filtered 2 × 2 (each output pixel the rounded mean of a 2 × 2 block; `⌊w/2⌋ × ⌊h/2⌋`, a trailing odd row or column dropped; if either side is < 2 the step is skipped and the raster goes to the backend as is), and `classify` runs again. If the result is `Clear`, the downsampled raster is recognised with `dpi = dpi' / 2` (integer division) and the result is a normal OCR `TextPage` whose `dpi()` is `dpi' / 2` and whose coordinates are mapped with the exact pixel factor 2 (page points = `px × 2 × 72 / dpi'`, y flipped), so `Block::bbox()` keeps its meaning. If the result is still `Speckle`, recognition is **skipped**: the result is an empty OCR `TextPage` (`source = Ocr`, `dpi() = dpi'`, `text() == ""`). Exactly one downsample step is ever taken. The second verdict is F11's rule again, on the half-resolution raster's own Otsu split and foreground side (an inverted noise page, `BackgroundDark` at 300 dpi, is `None` at 150 dpi and still speckle; Appendix A.6). |
| F14 | **Warnings.** A downsampled page carries `WarningKind::OcrNoiseDownsampled` (category `ocr`; text `ocr: noise-dominated raster recognised at 150 dpi (rendered at 300)`); a skipped page carries `WarningKind::OcrNoiseSkipped` (`ocr: noise-dominated raster skipped (no convincing foreground at 300 or 150 dpi)`; "convincing foreground" here means a connected one — the foreground side was speckle at both resolutions). Both are `TextPage` warnings (O19 is amended: recognition now has two warning kinds of its own). `Document::extract` appends, after the page-limit warnings and in page order, one `Warning` per affected page whose detail is the whole page-prefixed text, `page N: ocr: <text>` (`N` 0-based = `PageResult.index`), and whose `Display` prints that detail unchanged for the two `OcrNoise*` kinds (the page's own `TextPage` warning of the same kind displays as `ocr: <text>`; the `PageLimit` shape), so an `extract()` caller can see why a page is empty or at half resolution (Q6). A skipped page is otherwise the existing empty-OCR outcome of O23: `source = TextFallback`, `ocr_error = None`, counts as a success for `OcrStatus::Ok`. |
| F15 | **Cost bound.** `classify` plus the box filter on an A4 raster at 300 dpi (8.7 MP) takes under 50 ms in release on the development machine (measured 25–35 ms for an unoptimised version plus 2.5 ms for the filter; Appendix A.4), i.e. under 5 % of a clean page's recognition time; reported by the corpus-D job, never gated (ADR 13). |
| F16 | **No regression on corpus D.** With the pre-check on, at 300 dpi no corpus-D page other than `sparse_below_min_chars_noise` carries an `ocr:` warning, and at 72 dpi none does (the floor of F12). The realistic adversarial pages of Appendix A.6 built from corpus-D renders (a dark banner, an inverted upper half, a shaded box, a flat gray figure, a gray upper half, a photo-like band, a dark slide, an inverted page, each with and without noise) are `Clear` at 300 dpi and are recognised in full; the inverted noise page is skipped and the inverted `sparse_below_min_chars_noise` page is downsampled and read exactly (§4.2). The O34 gates (a)–(c) pass with margins at least those of 0002 §4.12; the noise subset's mean CER at 300 dpi falls (the one page goes from 0.93 to ≈ 0). `sparse_below_min_chars_noise` at 300 dpi: `dpi() == 150`, one `OcrNoiseDownsampled` warning, CER ≤ 0.05 against its ground truth, and the page completes in under 5 s (a functional stall-vs-fixed assertion with a 50× margin over the fixed time and a 3× one under the stall, the same kind as 0002 §4.7's "within 1 s"; the millisecond figure stays report-only). |
| F17 | **Opt-out and parity.** `OcrConfig { noise_precheck: false }` (Python `OcrEngine(noise_precheck=False)`) restores run-0002 behaviour: on every corpus-D page the pre-check never fired on, the `TextPage` with the pre-check off is byte-identical to the one with it on (the switch is inert there), and on `sparse_below_min_chars_noise` at 300 dpi it reproduces the stall (CER ≈ 0.93, `dpi() == 300`, no `ocr:` warning); run 0002's per-page text is not committed and OCR text is never compared exactly across hosts (0002 §2.4), so "exactly" means within one run on one host. `InProcess` and `Process` engines give identical results with the pre-check on (the unit-09 parity test extended to the noise pages). |

### 1.4 Small defects of the discovery report — PROPOSED, user to confirm [B4, B5, B6, B9, B16]

Each item was re-verified against `main` before being specified (Appendix
B.4). B7 turned out to be a documentation matter and is F4. All five are one
work unit (§5, unit 06), last in the stack so it can be dropped without a
rebase.

| ID | Requirement |
|---|---|
| F18 | **B4 — process-global Tesseract parameters are refused.** `Instance::with_variables` (the only path by which variables reach `Init`; `OcrConfig` and Python expose none) refuses, with `ErrorKind::InvalidArgument` naming the variable, any name that resolves in Tesseract's `GlobalParams()` (the shim answers `pxt_is_global_param(name)` by searching the four typed global vectors, so the set is Tesseract's own, not a list that drifts) and every per-instance parameter that (a) opens a file named by or derived from the variable, (b) selects a training, dump or box mode, or (c) changes which models Init loads (e.g. `tessedit_ocr_engine_mode`, `tessedit_init_config_only`, `tessedit_load_sublangs`), from a Rust-side list (`REFUSED_INSTANCE_PARAMS`) re-audited at every Tesseract upgrade; parameters outside these classes are residuals, not defects (widened from the original two file writers by internal #65 A1–A3, ADR-0082 addendum). `("debug_file", "")`, which today reverts every instance's output to stderr (Appendix B.4), is the pinned negative case; `("tessedit_do_invert", "0")` still works. |
| F19 | **B5 — language list grammar.** The sys-crate validator accepts exactly `[A-Za-z0-9_]+(\+[A-Za-z0-9_]+)*`: `"+"`, `"eng+"`, `"+eng"`, `"eng++fra"` are `InvalidArgument` before any file is touched (today `"+"` silently loads `eng` and `"eng+"` loads it twice, Appendix B.4). The ocr crate, the helper and the Python binding already reject these; their tests gain the same four cases so all three layers are pinned to one grammar. |
| F20 | **B6 — every requested language must have loaded.** After a successful `Init`, `pxt_init` compares the requested list with `GetLoadedLanguagesAsVector()`; a requested language that is absent fails `Init` with `PXT_ERR_INIT` and the message `language "<x>" did not load`, which the engine reports as `OcrUnavailable { reason: InitFailed }` (the same reason a 0-byte first language gives today, 0002 §4.8). Requested ⊆ loaded, not equality (`tessedit_load_sublangs` may add languages). This covers the in-process engine and the helper through the one shim. `info().languages` is unchanged (it is the configured list, now guaranteed loaded). |
| F21 | **B9 — descriptor sweep never fails open.** In `pipe.rs`, when `sysconf(_SC_OPEN_MAX)` returns a value that is negative, not above `first`, or above `MAX_SWEEP = 1,048,576`, the sweep bound becomes `getrlimit(RLIMIT_NOFILE).rlim_cur` clamped to `[first + 1, MAX_SWEEP]`, and `RLIM_INFINITY` or a failed `getrlimit` clamps to `MAX_SWEEP`; the loop is never empty. The Linux `close_range` fast path is unchanged. The bound computation is a pure function of its inputs with a unit test over the four cases; the existing `report-fds` helper test still shows exactly fds 0, 1, 2. Descriptors at or above `MAX_SWEEP` are not marked (residual, §2.3; *revised 2026-10-21*: "above" was wrong, the bound is exclusive, so `MAX_SWEEP` itself is unmarked too). |
| F22 | **B16 — the `no_tessdata` hint names what was tried.** The detail keeps its tested prefix `no directory holds <lang>.traineddata` and continues with: `; the tessdata setting <P> lacks it` when an explicit directory was given; `; TESSDATA_PREFIX=<P> and <P>/tessdata lack it` when the variable was set; `; set the tessdata option or TESSDATA_PREFIX` only when neither was; `searched` is unchanged. The wording names "the tessdata option" so it reads correctly from Rust (`OcrConfig.tessdata`) and Python (`OcrEngine(tessdata=...)`). The process-isolation path shares the one `resolve` and needs no change. |

---

## 2. Non-goals and bounded claims

### 2.1 Out of this run

- **A3 — flipping the default `isolation` to `Process`** (internal issue #53): deferred to Phase 3 with wheel packaging, by user decision. D-7/ADR 61 stand.
- **`atfork` handlers** of any kind, and any test that reproduces the fork deadlock in CI (the deadlock was reproduced during units 08–09; a committed test would hang on failure).
- **Pre-processing of every page** (denoising, deskew, binarisation, adaptive thresholding). The pre-check touches only rasters it classifies `Speckle`; 0002 §2.1's non-goal is narrowed, not dropped (§2.4). Tesseract's `thresholding_method` (Otsu, adaptive Otsu, Sauvola) stays at its default and is not exposed.
- **Leptonica in the pre-check.** The vendored Leptonica has everything needed (`pixOtsuAdaptiveThreshold`, `pixConnComp`, …), but calling it would load native code into the parent under `Isolation::Process` (O29) and put the check inside the backend rather than in front of it. The check is pure Rust in `pdf-extract-ocr` (`forbid(unsafe_code)`).
- **Connected-component analysis** in the pre-check (0.3–1 s per 300-dpi page in the probe; the isolation fraction gives the same decision for a one-pass cost).
- **Inverted (dark-background) pages as a class of their own**: the pre-check treats them by the same rule as upright ones (F11 tests whichever side Tesseract binarises as ink; Appendix A.6 measures an inverted body page, a dark slide and inverted noise), but nothing is done to re-polarise them for Tesseract — an inverted page is recognised as Tesseract recognises inverted pages today.
- **A memory budget for CCITT, DCT or hayro's RGBA conversion** (bounded by H13 already), **CPU time inside a decoder** (still bounded only by `page_time_budget` between operators; 0002 §2.2), **stack depth inside the codecs**, and the `unsafe` in `fearless_simd` paths the two codec crates compile under hayro-syntax's `unsafe` feature.
- **Upstreaming** H14/H15 (patches are exported, not sent; 0001 §2.1); upstream `hayro-jpeg2000` 0.4.0 exists and is not compatible with hayro-syntax 0.7.2, so the patches target 0.3.5. Issue E4 of the discovery report (reporting the ceilings upstream) stays a user action.
- **Other discovery-report items** (B1–B3, B8, B10–B15, B17–B27, C*, D*) — not in scope; internal #55/#56 remain open.
- **Windows** (as 0002).

### 2.2 Trust boundary

Each new input-accepting component: where malformed input is caught, what it
becomes, and what is not defended.

| Boundary | What is caught | Converted to | Not defended |
|---|---|---|---|
| **JBIG2 / JPEG 2000 stream → decoder** (H14, H15) | Any sequence of counted allocations (F6, F7) whose cumulative size would exceed the budget; the palette-indexed component count. A tile-count overflow (`num_x_tiles × num_y_tiles` past `u32`) is caught too, but as hayro-jpeg2000's `ImageTooLarge`, which stays `ImageDecode` (§4.5) | `DecodeFailure::ImageMemoryLimit` → `InterpreterWarning::ImageMemoryLimit` → `WarningKind::ImageMemoryLimit`; the image is skipped | **Uncounted allocations** (fixed context tables; everything linear in the input, bounded by `max_file_size` (*revised 2026-10-21*: except the referred-to segment list of a JBIG2 segment header, up to 2 GiB and bounded by `max_stream_decoded_bytes`, and the MMR vectors of JBIG2's embedded `hayro-ccitt` decode, about 1–2 MiB per region; see F6 and ADR 0079); JPEG 2000's sort scratch over the precinct list, about half that list); **`Vec` growth**: JBIG2's charged `Vec`s may reserve up to 2× the charged size (the claim is on charged bytes, so peak ≤ 2 × budget + uncounted), while H15 charges capacity growth itself, so for JPEG 2000 charged ≈ allocated; **a legitimate file over the budget is refused** (F9; concrete instance: a JBIG2 symbol dictionary that builds its symbols by refinement/aggregation re-charges the all-symbols reference list and the uniform Huffman table for every aggregated symbol, ≈ 44 B × N²/2 in all, so a dictionary of about 5,000 Huffman-coded aggregate symbols is refused at the default `max_image_pixels` — a shape no known encoder emits, since aggregation is used for a handful of composite symbols, not thousands); **time**: a decoder that allocates little but loops long is bounded only by `page_time_budget`, checked between operators; **the existing residual** that one operator is not interruptible. The budget is enforced by code inside the two vendored crates, so a future rebase of either crate must carry the patches or the claim lapses (`check-patches.sh` fails on drift, not on a forgotten rebase). |
| **Rendered raster → pre-check** (F11–F13) | Nothing to catch: the input is our own renderer's bytes. Adversarial *content* can only steer the verdict | A `Speckle` verdict → downsample or skip, with a warning | **A page can be made to skip OCR** by covering it in speckle (denial of recognition, not of service); **misclassification** (§2.3); the verdict depends on the whole raster, so one noisy region on an otherwise blank page decides for the page. |
| **Variable names → `Instance::with_variables`** (F18) | Process-global parameter names; per-instance file, training/dump/box and model-selection parameter names | `ErrorKind::InvalidArgument` | **A model's own `lang.config`** is applied by Tesseract with no constraint and can set anything, `debug_file` included (model-trust residual of 0002 §2.2, unchanged); per-instance debug toggles are still accepted (deliberately: 0002 §3.4). |
| **Language string → `Instance::new`** (F19) | Empty segments | `ErrorKind::InvalidArgument` | Nothing else new; the file-system rules of O9 are unchanged. |
| **`Init` result → F20** | A requested language Tesseract silently dropped | `OcrUnavailable { InitFailed }` | A model that loads but is corrupt past the header (Tesseract's problem, as before). |
| **`sysconf` → descriptor sweep** (F21) | −1, 0, absurd values | a clamped bound | Descriptors numbered at or above 1,048,576 (`MAX_SWEEP`; the sweep bound is exclusive) in the parent are inherited (a process would need that many open files; recorded). **On a host without `close_range`** (Linux before 5.11, non-Linux), the sweep stops at `sysconf(_SC_OPEN_MAX)` when that is plausible (above `first` and at most `MAX_SWEEP`), else at the soft `RLIMIT_NOFILE` clamped to `[first + 1, MAX_SWEEP]` (*revised 2026-10-21*), so a descriptor the parent still holds at or above a soft limit it lowered afterwards stays inheritable (the `sweep_bound` docstring). |

### 2.3 Approximate checks: residuals, by input class

The pre-check (F11) is an approximation of "Tesseract will stall and
produce garbage on this raster". Measured classes (Appendix A.2–A.3) and
their outcomes; nothing is claimed beyond them:

- **Detected and recovered**: a near-blank page with real text under i.i.d. gray noise of σ ≈ 12 at 300 dpi, in either polarity (the inverted page too); the same with a light-gray background (`200` + noise) and one 63-character line. Each reads exactly after the downsample.
- **Detected and skipped (text lost, where Tesseract produced garbage before)**: a blank page under noise, in either polarity; a near-blank page whose text is light gray (`145`) *and* under noise; a 30-character or 63-character line on a `200` or `128` background under noise. For each, the un-checked outcome was 14–46 s of garbage, and the half-resolution raster still gave 2–6 s of garbage; the loss is of text Tesseract did not read anyway.
- **Not detected, correctly** (Appendix A.2, A.6): every text page of corpus D, clean or degraded, at 72, 150 and 300 dpi; gray text without noise; body text in gray under noise (isolation 0.12 against a line at 0.35, read with a few junk tokens, 2.2 s); a page with a dark banner, an inverted upper half, a shaded box, a flat gray figure, a gray upper half, a photo-like band, a dark slide with light text, or fully inverted — each with and without noise, each read in full (0.5–2.2 s); any page where the foreground side is under 1 % of the raster (a page with a little dust goes to Tesseract unchanged; a blank page costs 39 ms there).
- **Not detected, and not meant to be**: a page with no text whose content is not noise — a full-page photo or gradient (`None` class, isolation 0.016) — goes to Tesseract, which returns a few hundred characters of junk in under a second. The pre-check is a stall guard, not a content filter; `extract()`'s `min_chars` routing and the caller's own judgement are what decide whether such text is kept.
- **Correlated noise is not speckle.** Noise whose grain is larger than one pixel at the rendered resolution — a noisy 150-dpi scan rendered at 300 dpi, or any noise field upscaled 2 × — has a connected foreground (isolation 0.000) and is `Clear`. Measured: the noisy sparse page scanned at 150 and rendered at 300 reads exactly in 57 ms (no stall to guard against); a 2 × 2-block noise field with no text costs 7.0 s of garbage at 300 dpi (its own half-resolution raster is speckle, but the rule only looks there after a first `Speckle`). One further case was reported once (internal #65 A6; a single report, and no fixture or measurement reproduces it, so it is not in Appendix A.6 beyond a row marked as such): a correlated-noise page that reached Tesseract as `Clear` returned empty output after 2.2 s, so the cost is not only time spent on garbage; the page is lost silently, with no `OcrNoiseSkipped` warning. Bounded by `page_timeout`; not defended (user decision: document, report only; the pre-check's rule is unchanged, and no gate on correlation is added).
- **Misclassified, by construction**: a page whose real content is itself speckle-like at the 2 × 2 scale (halftone screens, stippled drawings, dense dotted leaders, very small type at low rendered resolution) is `Speckle` and will be downsampled; if it stays `Speckle` after the downsample it is skipped. Such pages are not in corpus D and the rule has no test on them. A page with large faint text under heavy noise may be skipped where a human could read it. A page with noise σ much larger than 12 and a lot of text is unmeasured.
- **Resolution floor**: below `dpi' = 144` nothing is done; the 72-dpi stall exposure was never observed (`ms@72` for the pathological page is 12 ms) but is not excluded for other inputs.
- **Thresholds are constants** (`0.01`, `0.5`, `144`), chosen from Appendix A with the stated margins; they are not tuned per model, language or `psm`, and the data behind them is one synthetic corpus plus 29 synthetic rasters.

The memory budget (F5–F9) is exact on what it counts and silent on what it
does not (§2.2 first row). Its constants are derived from one page shape per
codec (Appendix A.5); a legitimate encoder that needs more than 2 B per page
pixel (JBIG2) or 64 B per pixel (JPEG 2000) in counted allocations is refused.

### 2.4 Relation to earlier records

- 0002 §2.1 ("no deskew, binarisation or denoising of our own before recognition") is narrowed: a resolution halving on rasters classified `Speckle` is the one pre-processing step, and it is opt-out. §6 records it.
- 0002 §2.4's "near-blank noisy page" residual is removed for the measured class and replaced by §2.3's residuals; the "JBIG2 and JPEG 2000 memory" residual is replaced by §2.2's first row; the fork residual stays, reworded by F1–F3.
- 0002 O2/O18: `TextPage.dpi` now reports the resolution of the raster the backend saw, which equals `dpi'` except on a downsampled page.
- 0002 O19 (no recognition warning kinds) is amended by F14.
- ADR 62's consequence paragraph (the two residuals and the "offered follow-on") is superseded by F5–F9; ADR 3's "only the three" by the vendoring of two more patched crates; ADR 35 is unchanged (F4 states its consequence for `default_ocr_engine`). No requirement here contradicts an Accepted record without saying so in §6.

---

## 3. Interface changes

### 3.1 `pdf-extract-core`

```rust
// warning.rs
pub enum WarningKind { /* existing */ ImageMemoryLimit, OcrNoiseDownsampled, OcrNoiseSkipped }
// category(): ImageMemoryLimit → "image"; OcrNoise* → "ocr"
// Display: "image: memory limit"; "ocr: noise-dominated raster recognised at D/2 dpi (rendered at D)";
//          "ocr: noise-dominated raster skipped (no convincing foreground at D or D/2 dpi)"
// extract.rs: Extraction.warnings = document warnings ++ page-limit warnings ++ per-page OCR notes
//          (OcrNoise* whose detail is the whole "page N: ocr: <text>" and whose Display prints it unchanged; page order)
// limits.rs: Limits.max_image_pixels doc gains the two derived budgets (F5)
```

Python sees warnings as strings; `_native.pyi` is unchanged for these (its
docstrings mention the new texts).

### 3.2 Vendored fork: new crates and changes H14, H15

**Vendoring `hayro-jbig2` 0.3.0 and `hayro-jpeg2000` 0.3.5** (together 852 KB
on disk, 8,720 + 8,999 lines of Rust; both `Apache-2.0 OR MIT`, both
`forbid(unsafe_code)` apart from the `fearless_simd` feature paths), by the
ADR 3 mechanics:

- `vendor/PROVENANCE.md` gains two `| hayro-jbig2 | 0.3.0 | <sha256> | 2026-10-.. |` / `| hayro-jpeg2000 | 0.3.5 | … |` rows (selected by `import.sh`'s existing `^| hayro-` filter, so a one-row addition is the whole manifest change); `.cargo-checksum.json` is deleted as for the other three; `[patch.crates-io]` gains the two path entries; `crates/core/Cargo.toml` keeps its `=` pins (they now resolve onto the vendored paths; `cargo tree` shows one copy of each). Features resolve as today (`std`, `simd`; `image` off).
- **Pristine tag `vendor/hayro-pristine-2`** on the import commit (the three older crates are already patched at that commit). `tools/vendor/export-patches.sh` learns the two-range shape: patches `0001–0022` from `vendor/hayro-pristine-1..vendor/hayro-pristine-2^` over the three old paths, then `0023+` from `vendor/hayro-pristine-2..HEAD` over all five paths, numbering continuous, and the script's hard-coded path list becomes a variable per range. `check-patches.sh` is unchanged in logic (fresh import of every `^| hayro-` row, apply every patch in name order, `diff -r`); its `--numstat` empty-match guard and `GIT_CEILING_DIRECTORIES` stay. `test-check-patches.sh` gains the two-range case. **Invariant** (F10): the 22 existing patch bodies are byte-identical after the change; their `[PATCH n/N]` subjects are normalised once (range 1 counts `n/22` within its range, so they stay stable as range 2 grows) and every later regeneration is byte-identical.
- `tools/license/gen-notices.sh` Part 3 locates hayro-jpeg2000's `assets/` through `cargo metadata`, which now reports the vendored path; cargo-about orders path crates before registry crates, so the two crates move up in Part 1's MIT "Used by" list: the regenerated `THIRD_PARTY_NOTICES` is committed in unit 03 (no notice text is added or removed) and must be clean on every regeneration thereafter. `license.rs` asserts both crates' `LICENSE-MIT` sections as it does for the other three. `cargo deny` sees no new licences.
- ARCHITECTURE's landmine ("never edit `vendor/hayro-*` without exporting") already covers the new trees; the two-range export is added to it.

**Fork changes.** Numbering continues from H13; patches from `0023`.

| ID | Change | Where |
|---|---|---|
| **H14** | **JBIG2 memory budget** (F5, F6, F8, F9). `hayro_jbig2::DecoderContext` gains `pub memory_budget: Option<u64>`; `Image::decode_with` installs a `MemoryBudget { remaining }` handle carried **by reference as a parameter** (no thread-local) to every counted site of Appendix B.2 — `Bitmap::new_with`/`reinitialize` take `&MemoryBudget`, `Bitmap`'s derived `Clone` is replaced by `clone_within(&MemoryBudget) -> Result<Bitmap>` used by symbol export, and the non-`Bitmap` sites charge explicitly. New `OverflowError::MemoryBudget`. `Image::decode` (no context) is unchanged and unbudgeted. hayro-syntax `filter/jbig2.rs` calls `decode_with` with the budget from `max_pixels` (`JBIG2_BUDGET_BYTES_PER_PIXEL = 2`), maps `Overflow(MemoryBudget)` to `DecodeFailure::ImageMemoryLimit` + `note_image_memory_limit_hit()`; `filter/mod.rs` gains the constants, the thread-local, `image_memory_limit_hits()`; `object/stream.rs` gains the `DecodeFailure` variant; hayro-interpret `x_object.rs` maps it to `InterpreterWarning::ImageMemoryLimit` (new variant in `interpret/mod.rs`). Size ≈ 150 lines in hayro-jbig2, ≈ 40 in hayro-syntax/-interpret. Patches `0023` (jbig2) and `0024` (syntax + interpret). | `vendor/hayro-jbig2/src/{lib,bitmap,decode/*,symbol,pattern,gray_scale,halftone,text,huffman_table,symbol_id_decoder,error}.rs`; `vendor/hayro-syntax/src/{filter/mod.rs,filter/jbig2.rs,object/stream.rs,lib.rs}`; `vendor/hayro-interpret/src/{x_object.rs,interpret/mod.rs}` |
| **H15** | **JPEG 2000 memory budget** (F5, F7, F8, F9). `hayro_jpeg2000::DecodeSettings` gains `pub max_memory_bytes: Option<u64>` (it is `Copy` and already travels with the `Image`); a `MemoryBudget` handle lives in `DecoderContext` for the decode and is charged at: the end of header parsing (tile-record estimate from `num_tiles()`, which becomes checked arithmetic), `DecoderContext::reset` (channel planes), `build_decompositions` (coefficient buffer, then each structural push), the code-block and IDWT buffers, the output buffer. New `ValidationError::MemoryBudget`. hayro-syntax `filter/jpx.rs` passes `max_memory_bytes = max_pixels × JPX_BUDGET_BYTES_PER_PIXEL (64)` in both `DecodeSettings` it builds and maps `Validation(MemoryBudget)` as H14 does. Size ≈ 120 lines in hayro-jpeg2000, ≈ 20 in hayro-syntax. Patch `0025`. | `vendor/hayro-jpeg2000/src/{lib,error,decode,j2c/codestream,j2c/tile,j2c/build,j2c/bitplane,idwt}.rs`; `vendor/hayro-syntax/src/filter/jpx.rs` |

Each H-change is its own commit with a test in `crates/core/tests/`
(hayro-level observables only, ADR 15) and an exported patch.

### 3.3 `pdf-extract-ocr`

```rust
// config.rs
pub struct OcrConfig { /* existing */ pub noise_precheck: bool /* true */ }
// precheck.rs (new; forbid(unsafe_code) like the crate)
pub mod precheck {
    pub const MIN_DPI: u32 = 144;
    pub const MIN_FOREGROUND: f64 = 0.01;
    pub const ISOLATION_FACTOR: f64 = 0.5;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Verdict { Clear, Speckle }
    /// `foreground` is the share of Tesseract's foreground side (`ω₀/H` when `foreground_dark`, else `1 − ω₀/H`);
    /// `isolation` is that side's isolation fraction, `Some` for every class but `Empty` when `foreground ≥ MIN_FOREGROUND`.
    #[derive(Clone, Copy, Debug, PartialEq)] pub struct Stats { pub threshold: u8, pub class: OtsuClass, pub foreground_dark: bool, pub foreground: f64, pub isolation: Option<f64> }
    impl Stats { pub fn verdict(&self) -> Verdict; }   // the F11 rule over these statistics; classify() and the quality report use it
    #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum OtsuClass { Empty, ForegroundDark, BackgroundDark, None }
    pub fn stats(img: &GrayImage) -> Stats;          // F11, Tesseract's OtsuStats + verdict
    pub fn classify(img: &GrayImage) -> Verdict;     // stats(img).verdict()
    pub fn downsample2(img: &GrayImage) -> Option<GrayImage>;   // None when a side is < 2
}
// engine.rs: OcrEngine::ocr applies F12–F14 between the budget check and the backend call;
//            build_page takes the pixel scale (1 or 2) and the warnings to append.
```

`Stats` is public so the quality test can report it per page (report-only)
and so a user can diagnose a verdict; the constants are documented, not
configurable (Q4).

### 3.4 `pdf-extract-tesseract-sys` (unit 06, PROPOSED)

- `Instance::with_variables`: refuses process-global and file-writing names (F18) through a new shim entry `pxt_is_global_param(const char* name, char* err, size_t err_len) -> int` (1 global, 0 instance-level, error code on failure), referenced by a sys test as `shim_symbols.rs` requires; the two file-writing names are a Rust-side constant.
- Language validator: the grammar of F19.
- `pxt_init`: the loaded-language check of F20 (`GetLoadedLanguagesAsVector`), error text `language "<x>" did not load`.
- `pipe.rs`: `sweep_bound(sysconf: i64, rlimit: Option<u64>, first: c_int) -> c_int` (pure, tested) feeding `mark_cloexec_from`; `MAX_SWEEP = 1 << 20`.

### 3.5 Python (normative stub additions)

```python
class OcrEngine:
    def __init__(self, *, ..., isolation: Isolation = "in_process",
                 noise_precheck: bool = True) -> None: ...   # new keyword; ValueError never (a bool)
    @property
    def noise_precheck(self) -> bool: ...
```

`_scripted_engine(...)` (test only) gains the same keyword so the engine-level
behaviour is testable from Python without Tesseract. Docstrings per F2.
`OcrUnavailableError` messages change wording per F22 (the tested prefix is
kept). No new exception classes, literals or error attributes.

### 3.6 CI and tooling

| Job | Change | Gating |
|---|---|---|
| `vendor-patches` | `check-patches.sh` now imports five hayro rows and applies 25 patches; `test-check-patches.sh` run by hand gains the two-range case | yes |
| `deny` | `gen-notices.sh` regenerated and the result committed in unit 03 (Part 1 ordering only); `git diff --exit-code THIRD_PARTY_NOTICES` clean thereafter | yes |
| `corpus-d` | `quality.rs` gains the F16 assertions and reports `precheck::Stats` and the pre-check time per page in `ocr-report.json` (`precheck_ms`, `precheck_verdict`) | CER gates yes; new per-page pins yes; times report-only |

| `deny` | `gen-notices.sh` regenerated; `git diff --exit-code THIRD_PARTY_NOTICES` must be clean (no text change expected) | yes |
| `corpus-d` | `quality.rs` gains the F16 assertions and reports `precheck::Stats` (with `foreground_dark`) and the pre-check time per page in `ocr-report.json` (`precheck_ms`, `precheck_verdict`, an `adversarial` block for the A.6 pins); with `PDF_EXTRACT_NOISE_PRECHECK=0` it writes `ocr-report-precheck-off.json` with a `reference_0002` block instead | CER gates yes; new per-page pins yes; times report-only |
| `rust`, `coverage` | new tests; the 95 % gate still covers `crates/{core,render,ocr}` (the new `precheck.rs` is in it) | yes |
| `python` | new kwarg/getter tests, docstring pins, `stubtest` | yes |

Timeouts unchanged. No new environment variable.

---

## 4. Verification oracle

"Done" for each unit means its listed items pass on CI. Items are stated
before implementation; a unit may add tests, never remove these.

### 4.1 Unit 01 — fork hazard documentation (F1–F4)

- `cargo doc --workspace --no-deps -D warnings` green; the rendered `pdf_extract_ocr` crate docs contain a `Forking` heading; a doc test–free grep test in `crates/ocr/tests/docs.rs` asserts that the doc comments of `default_ocr_engine`, the `pool` module, `isolation/client.rs` (`LOG`, `TEST_ENV`), `tessdata.rs` (`OVERRIDE`) and `tesseract-sys/src/lib.rs` (`GLOBAL_INIT`) each contain the word `fork` and a link to the crate-level section (the test reads the source files, as `shim_symbols.rs` does).
- `tests/python/test_ocr_docs.py`: `"fork" in OcrEngine.__doc__`, in `default_ocr_engine.__doc__` and in `extract.__doc__`; each names `spawn`; `stubtest` green.
- README has a "Forking and multiprocessing" heading; ARCHITECTURE's fork bullet names `OVERRIDE` and `GLOBAL_INIT`; THEORY has the "why no atfork" paragraph; wordlist test green.
- 0002 O12 amended per F4 (the only change to 0002 in this unit); `docs/specs/0002-ocr.md` revision log gains an entry.

### 4.2 Unit 02 — noise pre-check (F11–F17)

- **`precheck::stats`** against the probe's values (Appendix A.2, A.6, computed by the same algorithm): a 2480 × 3509 white raster → `Empty`; the same with `testkit::corpus_d::add_noise(seed 20261003)` → `ForegroundDark`, `foreground_dark`, `threshold == 246`, `foreground` within ±0.005 of 0.2417, `isolation` within ±0.02 of `(1 − p)⁴`; its `downsample2` → `None` class, `foreground` ≈ 0.36, `isolation` ≈ 0.165; the **inverted** noise field → `BackgroundDark`, `!foreground_dark`, `threshold == 8`, `foreground` ≈ 0.242, `isolation` ≈ 0.331 (the light side, not the dark one, whose isolation is 0.003); a 400 × 200 white raster with a black 200 × 20 rectangle (5 % of the raster; on an A4-sized raster the same rectangle is under `MIN_FOREGROUND` and gives `isolation == None`) → `ForegroundDark`, `isolation == Some(0.0)`; the rectangle at value 200 → the same; the rectangle plus noise → `isolation < 0.1`; a raster whose upper 45 % is black and lower part white with a few black words → `None`, `foreground_dark`, `isolation == Some(0.0)`; a 1 × 1 raster → `Empty`. Otsu agreement: for 20 random histograms the threshold equals a direct port of `OtsuStats` kept in the test.
- **`classify`**: `Speckle` for the noise field, for its `downsample2`, for the inverted noise field and its `downsample2`, for `render_gray(sparse_below_min_chars_noise, 300)` and for its inversion; `Clear` for every other corpus-D page at 300 dpi and for every corpus-D page at 72 dpi (the test iterates the 40 committed files plus the 12 built noise variants, needs no tessdata, runs in the `rust` job); `Clear` for `downsample2(render(sparse_below_min_chars_noise, 300))` and for the `downsample2` of its inversion. **Realistic adversarial pages** (Appendix A.6), each built in the test from a 300-dpi corpus-D render by a pixel transform (the recipes are in A.6) and each `Clear` both as built and after `downsample2`: the heading page with its top 8 % inverted (banner); the body page with its top 45 % inverted; the same plus noise; the body page with rows 30–60 % on a 200 background (shaded box); rows 25–75 % on 200 plus noise; rows 40–70 % replaced by flat 128 (figure); the top half flat 128, with and without noise; rows 40–70 % replaced by a gradient plus noise (photo band); the full-page gradient plus noise (no text; `None`, isolation ≤ 0.02); the body page remapped to background 40 / text 220 (slide), with and without noise; the body page inverted. The sparse page and the one-line page on a 128 background plus noise are `Speckle` as built and after `downsample2`.
- **`downsample2`**: dimensions `⌊w/2⌋ × ⌊h/2⌋`; a 2 × 2 block `[0, 255, 255, 255]` → 191; `None` for `1 × n`; the result of a 2480 × 3509 render is byte-identical to the probe's box filter on three pixels sampled by position.
- **Engine, `ScriptedBackend`, no Tesseract**: on the built `sparse_below_min_chars_noise` page at 300 dpi the backend's `last_image()` is `(1240, 1754)` and `last_request().dpi == 150`; the returned `TextPage.dpi() == Some(150)`, carries exactly one `OcrNoiseDownsampled` warning, and a scripted word at pixel box `[100, 100, 200, 120]` maps to the page points it would have at 150 dpi; on a test page whose render is the pure-noise field (a one-page PDF assembled by `testkit::corpus_d::scan_pdf` from the noise field, as `noise_variant` assembles its files, with a page box of `px × 0.24 − 0.001` pt a side — a thousandth of a point short of the exact 300-dpi size, because the renderer's `ceil` would otherwise add a column, and a merely close box such as A4's resamples the image and smooths the noise away; the 300-dpi render then maps pixels 1:1, pinned by a round trip in which fewer than 0.1 % of pixels change) the backend is **not called** (`calls()` unchanged), the result is empty with one `OcrNoiseSkipped` warning and `dpi() == Some(300)`; with `noise_precheck: false` the backend sees `(2480, 3509)` at 300 and the page has no `ocr:` warning; with `dpi = Some(100)` on the noise page the backend sees the full raster (floor); the pre-check time is inside `page_timeout` (a 1 ms timeout on the noise page is `Timeout`, never a backend call).
- **`extract()`**: a scripted document whose page 3 renders as the noise field → `pages[3].source == TextFallback`, `ocr_error == None`, `ocr_status == Ok`, and `Extraction.warnings` ends with one `OcrNoiseSkipped` whose detail starts `page 3: `; a downsampled page likewise adds one `OcrNoiseDownsampled` row; with `workers = 4` the rows are in page order.
- **Tesseract (CI model), `quality.rs`**: F16 verbatim — at 300 dpi only `sparse_below_min_chars_noise` carries an `ocr:` warning (`dpi() == 150`, CER ≤ 0.05, elapsed < 5 s); at 72 dpi no page does; O34 (a)–(c) pass; the report gains `precheck_ms` and `precheck_verdict` per page and resolution, the full `Stats` (`foreground_dark` included) per page, and an `adversarial` block with the pins of the A.6 pages below; the unit-09 parity check over the noise pages at 300 dpi gives byte-identical text in `Process` and `InProcess`; `PDF_EXTRACT_NOISE_PRECHECK=0` (a switch of the quality test only, not a library variable) runs the whole pass with `noise_precheck: false`; in gate mode (switch unset) the clean subset at 300 dpi is recognised a second time with `noise_precheck: false` and every page's `to_text()` must equal the pre-check-on text byte for byte (the pre-check is inert on them), and the switch-off run writes `target/ocr-report-precheck-off.json`, whose `reference_0002` block carries 0002 §4.12's subset CER means beside the measured ones so a reviewer can see that switch-off reproduces them and the 0.93 CER of the stalling page (report-only: the numbers are host-dependent, 0002 D-12). **Tesseract pins for the A.6 pages** (one-page PDFs assembled by `testkit::corpus_d::scan_pdf` from the transformed rasters, with the 1:1 page box above; CI model): the inverted-upper-half body page at 300 dpi yields at least 2,500 non-whitespace characters with `dpi() == 300` and no `ocr:` warning (today's rule skipped it); the gray-figure page likewise at least 1,400; the inverted `sparse_below_min_chars_noise` page yields `dpi() == 150`, one `OcrNoiseDownsampled` warning and CER ≤ 0.05 against the sparse page's ground truth, in under 5 s; the inverted noise field yields `""`, one `OcrNoiseSkipped` warning, in under 5 s (the un-checked outcome is 27 s).
- **Python**: `OcrEngine(noise_precheck=False).noise_precheck is False`; default `True`; `_scripted_engine(noise_precheck=...)` reproduces the downsample and skip cases through `page.ocr` (`TextPage.dpi == 150`, warning text contains `ocr: noise-dominated`); `extract()` warnings tuple contains the `page N: ocr:` string; `stubtest`, `mypy --strict` on `typing_consumer.py`, `panic_guard.rs` with its floor raised by the new getter; `pdf_extract` coverage ≥ 95 %.
- Coverage of `crates/ocr` ≥ 95 % with `precheck.rs` included; the silence test (fd 1/2) re-run over a downsampled and a skipped page.

### 4.3 Unit 03 — vendoring hayro-jbig2 and hayro-jpeg2000 pristine (§3.2)

- `vendor/hayro-jbig2` and `vendor/hayro-jpeg2000` equal `cargo vendor`'s output for the two PROVENANCE rows minus `.cargo-checksum.json` (`import.sh` into a temp dir, `diff -r`); the package SHA-256s in the rows match crates.io's; the tag `vendor/hayro-pristine-2` points at the import commit.
- `cargo tree -p pdf-extract-core` shows one copy each of `hayro-jbig2 0.3.0` and `hayro-jpeg2000 0.3.5`, both at `vendor/` paths; `cargo build --workspace --locked` with `CARGO_NET_OFFLINE=true` succeeds; the lockfile change is exactly the removal of the two `source = registry` lines plus checksum lines.
- `export-patches.sh` regenerates `vendor/patches/hayro/0001–0022` with **bodies byte-identical** to the committed files; unit 03 runs it once and commits the normalised `[PATCH n/N]` subjects (the only lines that change), after which regeneration is **byte-identical** (asserted by `git diff --exit-code` in `test-check-patches.sh`); `check-patches.sh` green with the five rows and the two-range series; `test-check-patches.sh` passes its new two-range case and its existing silently-skipped-patch case.
- `gen-notices.sh` regenerated and the result committed in unit 03 (the two crates move up in Part 1's "Used by" ordering; hayro-jpeg2000's asset notices are found at the vendored path; no notice text added or removed); `git diff --exit-code THIRD_PARTY_NOTICES` clean thereafter; `license.rs` asserts the two crates' `LICENSE-MIT` sections; `cargo deny check` green; wordlist green (the two trees are under `vendor/`, excluded).
- Render goldens, corpus-B/C parity and the H13 tests unchanged (nothing in the two crates changed).

### 4.4 Unit 04 — H14 JBIG2 memory budget (F5, F6, F8, F9, F10)

- **hayro-jbig2 level** (`crates/core/tests/h14_jbig2_budget.rs`, hand-made streams from `testkit::jbig2`, a few hundred bytes each except the export bomb below, decoded through `Image::new_embedded` + `decode_with` with `DecoderContext { memory_budget: Some(B) }`): the **symbol-export bomb** — T.88 §6.5.10's export flags are run lengths over the dictionary's input + new symbols, so one dictionary exports each of its symbols at most once; the bomb is therefore two dictionaries: one declares an 8,192 × 8,192 symbol and exports it once (a legitimate 8 MiB copy), the next names that dictionary 65,535 times in its referred-to list (T.88 allows duplicates) and exports its whole 65,535-entry input list; 74 KB of stream, 65,535 one-byte references — → `Err(Overflow(MemoryBudget))` with `B = 64 MiB`, refused as a batch before the first of the 65,535 copies: the test's peak RSS grows by less than **24 MiB** over the call in a child process (the symbol, its one legitimate copy, the 512 KiB name list; measured 18 MB on macOS, 9 MB on Linux; refusing copy by copy instead touches five more copies first, 61 MB measured), and, with a counting global allocator in the test binary, the bytes **requested** from the heap during the call are under **17 MiB** (the two bitmaps plus the list; this is the assertion that proves "charged before allocated" — RSS alone cannot tell a refused charge from a lazily-faulted allocation); the same stream with `memory_budget: None` is **not** run (it would allocate ≈ 512 GiB) — instead the control is a 64 × 64 symbol exported 4 times, `Ok` under `B = 1 MiB` and `Err(MemoryBudget)` under `B = 8 KiB`; a halftone region whose gray-scale grid is 40,000 × 40,000 → `MemoryBudget` under `B = 1 GiB` before any plane is allocated (peak RSS growth < 16 MiB in the child; requested heap < 1 MiB); three intermediate generic regions of 4,096² each → `Ok` under `B = 8 MiB` (3 × 2 MiB), `MemoryBudget` under `B = 4 MiB`; a text region referring to the same dictionary 1,000 times → `MemoryBudget` under a budget smaller than `8 × 1000 × symbols`; a page bitmap of 1,000 × 1,000 → `Ok` under `B = 128 KiB` and `MemoryBudget` under `B = 100 KiB` (so the page bitmap is counted); every `Ok` case decodes to the same pixels with `memory_budget: None` (pixel equality over the hayro `Decoder` sink). **Error order on malformed export flags** (write-back): export now decodes every run length and budget-checks the total copy size before the first clone, so a run-length total that disagrees with `SDNUMEXSYMS` is reported before a symbol index out of range would have been; both are `DecodeError::Symbol` → `ImageDecode`, and this spec states no order between them (F8 fixes only which variants become `ImageMemoryLimit`). The requested-heap assertions are mutation-checked: moving a charge after its allocation makes them fail.
- **hayro-syntax level**: an embedded JBIG2 image stream of the first shape under `DecodeLimits { max_image_pixels: Some(1_000_000) }` → `Err(DecodeFailure::ImageMemoryLimit)`, `image_memory_limit_hits()` + 1, `image_pixel_limit_hits()` unchanged; the H13 cap still fires first for a 20,000 × 20,000 page (`ImagePixelLimit`, memory hits unchanged); a malformed stream is still `ImageDecode` with neither counter moving; `JBIG2_BUDGET_BYTES_PER_PIXEL == 2`.
- **hayro-interpret level** (the unit-04 test device): `interpret_page` over a page drawing that XObject raises `InterpreterWarning::ImageMemoryLimit` once, `ImagePixelLimit` and `ImageDecodeFailure` zero times, no pixels reach the device; an inline JBIG2 image is not possible (JBIG2 is not an inline filter) — stated, not tested.
- **core / render / Python**: `WarningKind::ImageMemoryLimit.category() == "image"`, text `image: memory limit`; `render_gray` of the page carries one such warning and the OCR `TextPage` too; the text path (which never decodes images) is unchanged — `the_text_path_never_decodes_images` extended with this fixture; the warning string reaches Python through `TextPage.warnings`.
- Corpus B/C parity, render goldens, 0002 §4.4 tests: unchanged. Patches `0023`, `0024` exported; `check-patches.sh` green.

### 4.5 Unit 05 — H15 JPEG 2000 memory budget (F5, F7, F8, F9, F10)

- **hayro-jpeg2000 level** (`crates/core/tests/h15_jpx_budget.rs`, codestreams built by patching the `SIZ`/`COD` of a committed minimal file, as unit 04 did): `SIZ` with `XTsiz = YTsiz = 1` over 4,000 × 4,000 (16 M tiles) → `Err(Validation(MemoryBudget))` under `max_memory_bytes: Some(64 MiB)`, refused from the header before any `Tile` is built — proven as in §4.4 by peak RSS growth under 16 MiB in a child process **and** requested heap under 1 MiB with the counting allocator (the body need not be consistent: `tile::parse` is never reached); the `Ok` control is a 1 × 1-tiled image **the test kit can actually emit** — `tile::parse` needs one tile-part per tile, so a 300 × 300 image at 1 × 1 would need 90,000 tile-parts and cannot come from patching a committed file's `SIZ`: either a test-kit codestream of a few thousand tiles whose tile-parts carry no packets (`SOT`/`SOD` only), which decodes to a flat image, or a small committed file produced once by an external encoder and recorded in `tests/fixtures/PROVENANCE.md`; unit 05 chooses, records, and asserts `Ok` under 64 MiB with pixels equal to `None`; a `SIZ` whose `num_x_tiles × num_y_tiles` overflows `u32` → `MemoryBudget` (or `ImageTooLarge`), never a wrap (a debug-build run of the test proves no panic); a subsampled image (`XRsiz = YRsiz = 255`) whose shrunk size passes the 60,000 ceiling but whose reference grid is 1 × 1-tiled → refused from the header; `COD` with precinct exponent 1 at every resolution over a 2,048² image (1 × 1 code-blocks) → `MemoryBudget` under a budget of `16 × pixels` (the refusal leg; a patched `COD` over an inconsistent body suffices because the structural records are charged as they are built, before any packet is read), and the `Ok` leg at the measured threshold — the structural cost is 242.1 B/px, above 64, so the test's constant is `PRECINCT_1_BYTES_PER_PIXEL = 256` and the `Ok` leg is `256 × pixels` — on codestreams that actually decode (the committed OpenJPEG `precinct1_gray_128.j2k` and the test kit's 256² shape, both measured at 242.1); a `num_x_tiles × num_y_tiles` overflow is `ImageTooLarge` → `ImageDecode` (never `ImageMemoryLimit`), as the item above says; a JP2 with a `pclr` box and 256 components → the channel planes are charged (refused under a small budget before allocation) [*note 2026-10-21*: since H16 (patch 0026, internal #63) a 256-component image, palette or not, is refused at the header with `TooManyChannels` (`ImageDecode`) and never reaches plane charging; plane charging is exercised by `the_channel_planes_of_a_palette_image_are_charged` with a one-component palette image, ADR 0079 addendum 2026-10-12]; every `Ok` case is pixel-identical with `None`.
- **hayro-syntax level**: `jpx.rs` passes `max_memory_bytes = 64 × max_pixels` in both `Image::new` calls (asserted through a stream that is refused at `max_image_pixels: Some(100_000)` and accepted at `Some(10_000_000)`); `ImagePixelLimit` still fires first for a 20,000 × 20,000 `SIZ`; counters as in §4.4.
- hayro-interpret, core, render as §4.4 with a JPX XObject. Patch `0025` exported; `check-patches.sh` green; `JPX_BUDGET_BYTES_PER_PIXEL == 64`.

### 4.6 Unit 06 — B-items (F18–F22) — PROPOSED

- **F18**: `Instance::with_variables(.., &[("debug_file", "")])` → `Err(InvalidArgument)` naming `debug_file`, and the silence test's child (which runs a recognition after that refusal) still captures nothing on fd 1/2; `("log_level", "0")`, `("dotproduct", "native")`, `("tessedit_write_images", "1")`, `("tessedit_write_params_to_file", "x")` refused; `("tessedit_do_invert", "0")` accepted (existing test); `pxt_is_global_param` referenced from a test (`shim_symbols.rs`).
- **F19**: `bad_language_names_never_reach_tesseract` gains `"+"`, `"eng+"`, `"+eng"`, `"eng++fra"` (sys), and the ocr `config_tests.rs` and `tests/python/test_ocr.py` gain the same four strings with their layer's error (`InitFailed` / `ValueError`).
- **F20**: `crates/ocr/tests/construction.rs`: a directory with a valid `eng` and a 0-byte `deu`, `languages = ["eng", "deu"]` → `OcrUnavailable { reason: InitFailed, detail contains "deu" }`; `["eng"(0 bytes), "deu"]` → `InitFailed` (today it silently runs on `deu`); the same two cases under `Isolation::Process`; the existing single-language 0-byte test unchanged.
- **F21**: `sweep_bound(-1, Some(256), 3) == 256`, `sweep_bound(-1, None, 3) == MAX_SWEEP`, `sweep_bound(2, Some(1024), 3) == 1024`, `sweep_bound(1 << 40, Some(1 << 40), 3) == MAX_SWEEP`, `sweep_bound(1024, _, 3) == 1024`; the `report-fds` helper test still shows exactly `0 1 2`; the Linux `close_range` path untouched (the test that counts a helper's fds runs on both OSes).
- **F22**: `tessdata_tests.rs`: with `tessdata: Some(P)` and no model → detail contains `no directory holds eng.traineddata` and `the tessdata setting` and `P`; with `TESSDATA_PREFIX=P` (override) → contains `TESSDATA_PREFIX=P`; with neither → contains `set the tessdata option or TESSDATA_PREFIX`; `tests/python/test_ocr.py:115` still passes (prefix unchanged); Python message contains `TESSDATA_PREFIX=` when the variable was set.
- Coverage: `crates/ocr` ≥ 95 %; `tesseract-sys` reported.

### 4.7 Cross-cutting (every unit)

As 0002 §4.15: `forbid(unsafe_code)` in `core`, `render`, `ocr`, `testkit`; file headers; no file over 2,000 lines outside `vendor/`; `cargo deny`, patch series in sync, notices regenerated when the graph changes; wordlist green over specs and ADRs; adversarial review before merge with these targets in particular — a JBIG2 or JPEG 2000 allocation site that bypasses the budget (the reviewer greps the two vendored crates for `vec!`, `with_capacity`, `resize`, `clone()` and matches each against F6/F7's list), a raster that defeats `classify` in either direction (the reviewer constructs one and the result is recorded under §2.3 if it does), and a parameter name that slips past F18.

---

## 5. Work units

Stack order; each PR is independently reviewable and CI-green. Tier:
**workhorse** = code; **mid** = docs/CI/config. Sizes exclude vendored
imports and fixtures.

| NN | Branch | Contents | Oracle | Tier | Depends on | Size |
|---|---|---|---|---|---|---|
| 01 | `ocr3/01-fork-docs` | F1–F4: `pdf-extract-ocr` crate-level `Forking` section and per-lock cross-references; `tesseract-sys` `GLOBAL_INIT` note; pyclass/pyi docstrings; README paragraph; ARCHITECTURE/THEORY lines; 0002 O12 write-back; `docs.rs` and `test_ocr_docs.py` pins | §4.1 | mid | — | ~150 |
| 02 | `ocr3/02-noise-precheck` | F11–F17: core `WarningKind::{OcrNoiseDownsampled, OcrNoiseSkipped}` + `extract()` per-page notes; ocr `precheck.rs`, `OcrConfig.noise_precheck`, engine integration, coordinate scale in `build_page`; testkit: pure-noise page builder; Python kwarg/getter/stub/`_scripted_engine`; `quality.rs` F16 pins and report fields; tests | §4.2 | workhorse | — | ~700 |
| 03 | `ocr3/03-vendor-jbig2-jpx` | §3.2 vendoring: pristine import of the two crates, PROVENANCE rows, `[patch]` entries, tag `vendor/hayro-pristine-2`, two-range `export-patches.sh`, `test-check-patches.sh` case, notices regenerated, `license.rs` rows, ARCHITECTURE landmine line | §4.3 | workhorse | — | ~120 (+ 852 KB vendored) |
| 04 | `ocr3/04-hayro-h14-jbig2-budget` | H14; hayro-syntax `ImageMemoryLimit` + counter + constants; hayro-interpret warning variant; core/render/python `WarningKind::ImageMemoryLimit`; `h14_jbig2_budget.rs`; patches 0023, 0024 | §4.4 | workhorse | 03 | ~450 |
| 05 | `ocr3/05-hayro-h15-jpx-budget` | H15; `jpx.rs` settings; `h15_jpx_budget.rs`; patch 0025 | §4.5 | workhorse | 04 | ~350 |
| 06 | `ocr3/06-b-fixes` **PROPOSED** | F18–F22: `pxt_is_global_param` + denylist, language grammar, loaded-language check, `sweep_bound`, tessdata hint; tests at all three layers | §4.6 | workhorse | — | ~250 |

01, 02, 03 and 06 are independent of each other; 04 depends on 03 and 05 on
04. Critical path 03 → 04 → 05. Unit 06 is last in the stack so dropping it
is a no-op for the others. The docs/ADR update at the end of the run (ADR
graph, README/ARCHITECTURE/THEORY) is handled outside this
table; §6 lists what it must record.

---

## 6. Decisions that need records

To be recorded as ADRs at the end of the run (one record each; supersede,
never rewrite).

| # | Decision | Rationale | Affects |
|---|---|---|---|
| D-1 | The fork-deadlock hazard is documented, not defended: no `atfork` handlers (user decision, 2026-10-05) | A `OnceLock` mid-initialisation has no safe reset; a library must not install process-wide handlers into a host; the safe patterns (`spawn`/`forkserver`, build before fork) are cheap | new record; 0002 §2.4 stays |
| D-2 | Codec-internal memory is bounded by a cumulative per-decode byte budget derived from `max_image_pixels` with per-codec multipliers (2, 64), enforced inside the vendored codec crates, refused as a distinct `ImageMemoryLimit` warning | One existing knob; the multipliers encode each codec's legitimate per-pixel need with a 4× margin; a distinct kind keeps `ImagePixelLimit` honest (as D-8 kept `ImageDecodeFailure` honest) | ADR 62's decision stands; its consequence paragraph (the two residuals, the "offered follow-on") is overtaken — recorded as a new record plus an addendum on 0062 pointing at it |
| D-3 | `hayro-jbig2` 0.3.0 and `hayro-jpeg2000` 0.3.5 are vendored because they are now patched; a second pristine tag and a two-range patch export | ADR 3's rule (vendor only what we patch) applied; its title's "three" is no longer true | new record with `supersedes: 0003` (and the C-tree addendum carried over), or an addendum to 0003 — the end-of-run update decides; this spec says the title is superseded |
| D-4 | The noise pre-check: Tesseract's own global Otsu replicated in Rust plus the isolation-fraction rule; downsample once, re-check, else skip; default on, opt-out per engine; floor 144 dpi; constants not configurable | Measured (Appendix A): the mechanism is Otsu splitting noise; the rule has ≥ 2× / ≥ 20× margins on the data; downsampling recovers real text where skip would lose it and the re-check turns pure noise into a skip | narrows 0002 §2.1's "no pre-processing" non-goal; amends O2/O18/O19 |
| D-5 | `Extraction.warnings` carries per-page OCR notes in the `page N:` shape | An `extract()` caller otherwise cannot tell a skipped page from a blank one | extends ADR 70's consequence list |
| D-6 | Process-global Tesseract parameters are refused by asking Tesseract (`GlobalParams()`), not by a list | The set is Tesseract's and changes with versions | new record (PROPOSED unit) |
| D-7 | A panic in the first `default_ocr_engine()` construction is not cached (ADR 35's consequence) | Documented, not changed | addendum to ADR 35 |
| D-8 | A3 stays deferred to Phase 3 | User decision | note in the run's end-of-run report; no record |

---

## 7. Open questions for the user

Only questions whose answer changes the work. Each has the default the units
follow unless told otherwise.

| # | Question | Proposed default |
|---|---|---|
| Q1 | Budget shape: derived from `max_image_pixels` with the constants 2 B/px (JBIG2) and 64 B/px (JPEG 2000), no new `Limits` field — or a new `Limits.max_image_decode_bytes` (one value for both codecs, exposed in Python)? | **Derived, no new field.** One knob already bounds image memory and the multipliers are codec facts, not policy; a field would need a default that is wrong for one codec or the other. |
| Q2 | Refusal surfaced as a **distinct** `WarningKind::ImageMemoryLimit` (`image: memory limit`), or reuse `ImagePixelLimit` so no public enum grows? | **Distinct.** Same plumbing cost either way (a variant in three crates); callers can tell "declared too big" from "the codec's working set was refused", and `ImagePixelLimit` keeps meaning what 0002 says. |
| Q3 | Pre-check **default on** (changes `Page.ocr` results on noise-dominated pages for every existing caller) or default off (opt-in)? | **On.** Every page it changes produced garbage in 16–46 s before; the opt-out exists for a deployment that disagrees. |
| Q4 | Config surface: one boolean `noise_precheck` (Rust field, Python kwarg + getter), constants fixed — or expose the thresholds? | **One boolean.** The thresholds come from one corpus and have wide margins; exposing them invites tuning without data and triples the test matrix. |
| Q5 | Below 144 dpi a `Speckle` raster is left alone. Alternative: skip it (never downsample below 72). | **Leave alone.** No stall was observed at 72 dpi (12 ms) and skipping would silently drop low-resolution OCR. |
| Q6 | `Extraction.warnings` gains `page N: ocr: …` rows for downsampled/skipped pages (D-5)? | **Yes**, same shape as `PageLimit`; strings only, no Python API change. |
| Q7 | Include unit 06 (B4, B5, B6, B9, B16) in this run? And B7 as a wording fix only (F4), no `catch_unwind`? | **Include**; **B7 wording only** (a `catch_unwind` there would be a local exception to ADR 35 for a case nobody has observed). |
| Q8 | The F16 "under 5 s" assertion on one page: keep as a functional stall-vs-fixed check (50× margin), or make it report-only like every other timing? | **Keep.** It distinguishes a 16 s stall from a 0.1 s fix, the kind of bound 0002 §4.7/§4.9 already use; the raw millisecond stays report-only. |
| Q9 | Vendoring adds 852 KB and ~17.7 k lines of upstream Rust under `vendor/`, reviewed only as pristine imports plus three patches. Accept? | **Accept.** It is the only route (`allow-git = []`), the same mechanics as the three existing crates, and the alternative is leaving the residual. |
| Q10 | **Revision (3), after sign-off.** F11's rule is changed from "`None` class ⇒ `Speckle`, `BackgroundDark` ⇒ `Clear`" to "isolation of Tesseract's own foreground side, for every non-`Empty` class". Consequences visible to a caller: pages with a large dark region (banner, figure, shaded or inverted half, slide) are **no longer skipped** — they reach Tesseract and are read; inverted noisy pages are **now** downsampled or skipped instead of stalling; corpus D and every previously measured raster keep their verdicts; `precheck::Stats` gains `foreground_dark` and `foreground`/`isolation` now describe the foreground side; a page with no text and non-noise content (photo, gradient) is no longer skipped and may yield a few hundred junk characters. Accept the revised rule? | **Accept.** The signed-off rule lost real text on ordinary layouts (Appendix A.6: 1,200–3,000 characters per page), which violates the design goal that no page Tesseract reads usefully is skipped by default; the revised rule has the same margins on every stall case and adds the inverted ones. |

---

## Appendix A: Measurement evidence

Probe: a throwaway cargo project in the session scratchpad depending on
`pdf-extract-core`, `-render`, `-tesseract-sys` and `-testkit` by path with the
workspace's `[patch.crates-io]`, release profile, run 2026-10-05 on macOS 14
arm64 (M3 Max) against the committed corpus D plus the 12 noise variants
built by `testkit::corpus_d::build_noise`, and the MacPorts combined
`eng.traineddata` (which unit 13 measured to behave like `tessdata_fast` on
the pathological page: 16.4 s vs 15.8 s). Rasters come from
`pdf_extract_render::render_gray`; Tesseract is driven through
`pdf_extract_tesseract_sys::Instance` (`set_image`, `set_source_resolution`,
`recognize`, `blocks`), one instance, pages one at a time. Times are single
runs; treat ±20 % as noise. Statistics are the ones F11 defines, computed by
the probe's own implementation of Tesseract's `OtsuStats`.

### A.1 The pathological page and its treatments

| Raster | `t` | class | `p` | iso | mean run | Tesseract | Output |
|---|---|---|---|---|---|---|---|
| `sparse_below_min_chars_noise` rendered at 300 | 245 | fg-dark | 0.217 | 0.376 | 1.28 px | **24,981 ms** | garbage (1.09 M components, 987 k ≤ 3 px) |
| same, rendered at 150 | 151 | fg-dark | 0.0007 | 0.000 | 2.96 | 22 ms | `Exhibit 12: Notice of Filings.` (exact) |
| same at 300, box-filtered 2 × 2 (`downsample2`) | 147 | fg-dark | 0.0006 | 0.000 | 2.86 | 21–22 ms | exact |
| same at 300, binarised at a fixed 128 | — | — | — | — | — | 57 ms | exact |
| white 2480 × 3509 | — | empty | 0 | — | — | 39 ms | `""` |
| pure noise σ 12 on white, 2480 × 3509 | 246 | fg-dark | 0.242 | 0.331 | 1.32 | **46,558 ms** | garbage |
| pure noise, `downsample2` (1240 × 1754) | 249 | **none** | 0.363 | 0.165 | 1.57 | 5,516 ms | garbage |
| pure noise, downsampled twice (620 × 877) | 250 | none | 0.486 | 0.070 | 1.95 | — | — |
| pure noise σ 6 (halved deviates) at 300 | 250 | fg-dark | 0.217 | — | — | 24,716 ms | garbage |
| `sparse_below_min_chars` (clean) at 300 | 137 | fg-dark | 0.0006 | 0.000 | 5.37 | 58–64 ms | exact |
| `clean_one_line_noise` at 300 (63 chars) | 135 | fg-dark | 0.0013 | 0.000 | 5.30 | 76 ms | exact |
| `clean_body_11pt_noise` at 300 | 135 | fg-dark | 0.078 | 0.000 | 5.43 | 2,132 ms | correct |

Reading: Otsu's between-class variance is won by the text/paper split as
soon as the page holds a few thousand dark pixels (63 characters suffice;
30 do not). Downsampling 2 × 2 reduces σ to ≈ 6 *and* the pixel count by 4;
σ 6 alone at full resolution still stalls, so the gain is in the raster
Tesseract has to segment, not only in the noise amplitude. Fixed-threshold
binarisation would also work for this page but is not the remedy chosen
(§1.3): it changes every pixel of a page it misclassifies, where a
downsample is a resolution change with a re-check behind it.

### A.2 Corpus-D sweep: the statistic on every page

At **300 dpi** (52 pages): every page except `sparse_below_min_chars_noise`
has class fg-dark, `p` between 0.0006 (`sparse_*`) and 0.102 (`clean_body_12pt_blur`),
`t` between 135 and 177, mean run 4.5–7.2 px and **isolation 0.000**. The
pathological page: `t = 245`, `p = 0.217`, isolation `0.376` against the
F11 line `0.5 × (1 − 0.217)⁴ = 0.188` (2.0× above). Under revision (3)'s
rule the sweep is unchanged: every page is `ForegroundDark` with the dark
side as foreground, so `p` and the isolation are the same numbers, and the
old and new rules agree on all 52 × 3 renders and on the 20 `downsample2`
rasters (verified by running both side by side, A.6).

At **72 dpi** (52 pages): all fg-dark, `t` 169–195, `p` 0.0008–0.124, mean
run 1.8–2.7 px, isolation `0.001–0.013` (the four `sparse_*` pages are the
highest at 0.009–0.013; the line for them is `0.5 × (1 − 0.0008)⁴ ≈ 0.498`,
38× above). No page is `Speckle`; the floor of F12 makes this moot but the
data shows the rule itself would not fire.

At **150 dpi** and for `downsample2` of the 300-dpi render (the 12 noise
pages, the 4 `sparse_*`, the 4 `clean_one_line_*`): all fg-dark, isolation
`0.000`, including `sparse_below_min_chars_noise` (`p = 0.0007`, run 2.96 at
150; `p = 0.0006`, run 2.86 for `downsample2`).

Synthetic rasters at 2480 × 3509 (text pages are the 300-dpi renders of the
named corpus files, transformed):

| Raster | `t` | class | `p` | iso | line `0.5(1−p)⁴` | verdict | Tesseract | Output |
|---|---|---|---|---|---|---|---|---|
| sparse page, ink lightened to 200 (no dark pixel) | 229 | fg-dark | 0.0006 | 0.000 | 0.499 | Clear | 64 ms | exact |
| sparse page, ink 145 | 204 | fg-dark | 0.0006 | 0.000 | 0.499 | Clear | 59 ms | exact |
| one-line page, ink 200 | 229 | fg-dark | 0.0013 | 0.000 | 0.497 | Clear | 78 ms | exact |
| body page, ink 200 | 229 | fg-dark | 0.078 | 0.000 | 0.361 | Clear | 2,132 ms | correct |
| sparse page, ink 145 + noise | 246 | fg-dark | 0.242 | 0.330 | 0.165 | **Speckle** | 45,970 ms | garbage |
| body page, ink 200 + noise | 228 | fg-dark | 0.0885 | 0.123 | 0.345 | Clear | 2,197 ms | correct with a few junk tokens |
| sparse page on background 200 + noise | 199 | **none** | 0.484 | 0.071 | — | **Speckle** | 28,080 ms | garbage |
| one-line page on background 200 + noise | 199 | **none** | 0.485 | 0.071 | — | **Speckle** | 40,132 ms | garbage |
| body page on background 200 + noise | 110 | fg-dark | 0.079 | 0.000 | 0.359 | Clear | 2,140 ms | correct |
| body page on background 200 | 108 | fg-dark | 0.079 | 0.000 | 0.359 | Clear | 2,130 ms | correct |

### A.3 The remedy on the hard synthetic cases

| Raster | after `downsample2` | verdict | action | Tesseract | Output |
|---|---|---|---|---|---|
| sparse page, ink 145 + noise | `t` 249, none, `p` 0.364, iso 0.163 | Speckle | **skip** | (would be 4,208 ms of garbage) | `""` |
| sparse page on bg 200 + noise | `t` 199, none, `p` 0.460 | Speckle | **skip** | (5,955 ms of garbage) | `""` |
| one-line page on bg 200 + noise | `t` 120, fg-dark, `p` 0.0014, iso 0.000 | Clear | recognise at 150 | 39 ms | exact |
| pure noise | none, `p` 0.363 | Speckle | **skip** | (5,528 ms of garbage) | `""` |
| `sparse_below_min_chars_noise` | fg-dark, `p` 0.0006, iso 0.000 | Clear | recognise at 150 | 21 ms | exact |

### A.4 Cost of the check

On the 300-dpi `clean_body_11pt` raster: histogram + Otsu + binarise +
isolation pass + string formatting, 24.9 ms per call (five-run mean); on the
noise page 34.5 ms; `downsample2` 2.5 ms. The probe's isolation pass is a
straightforward per-pixel loop over a `Vec<bool>`; a bit-packed row scan
should halve it. F15's bound of 50 ms has that headroom.

### A.5 Deriving the budget multipliers

- **JBIG2.** A page of `P` pixels: page bitmap `P/8` B; regions normally tile the page, `≈ P/8` in total; symbol dictionaries of text pages hold a few thousand symbols of a few hundred bytes each (single-digit MB); refinement is rare. Total ≈ `0.3–0.5 B × P`. The multiplier **2** gives ≥ 4× margin at the pixel cap and, for an ordinary 8.7 MP scan, about 60×. The hostile shapes (Appendix B.2): 512 MiB per region/symbol bitmap regardless of `P`, a 16 GiB gray-scale plane from 8 header bytes, 65,535 clones of one symbol — all refused at `2 × P`.
- **JPEG 2000.** A `P`-pixel image with `c` components: channel planes `4c·P`, one tile's coefficient buffer up to `4c·P` for a single-tile image, output `c·P`, structural records at 64 × 64 code-blocks ≈ negligible. Total ≈ `9c·P` ≈ 36 B/px for RGBA by derivation. **Measured by unit 05** (OpenJPEG 2.5.4 output, 512 × 512, charged bytes per image pixel): gray 15, RGB 33, RGB in 128 × 128 tiles 17, RGB with 8 resolutions and 16 × 16 code-blocks 37, four components 42 — so the multiplier **64** gives a **1.5×** margin for the costliest legitimate geometry at the pixel cap (the derivation's 1.8× was for the single-tile RGBA case; four untied components cost more) and a far larger one below it. More than four components cannot reach the budget: hayro-jpeg2000 accepts gray, RGB, RGB + alpha and CMYK only and refuses the rest with `ValidationError::TooManyChannels` before decoding, so the 16-component case this paragraph once extrapolated (≈ 170 B/px) is unreachable and A.5's "revisit Q1" trigger does not fire. The hostile shapes are refused: 1 × 1 tiles (≥ 100 B per reference-grid pixel in `Tile` records alone, on a grid up to 255² × larger than the pixel count through subsampling), 1 × 1 code-blocks, and a palette-indexed file with thousands of components (*note 2026-10-21*: refused since H16 at the header with `TooManyChannels`, not by the budget; more than 255 components never reach it). **1 × 1 code-blocks** (precinct exponent 1 at every resolution: each coefficient above the lowest resolution gets its own precinct, code-block, layer record and two one-node tag trees) cost **242.1 B/px**, measured both on OpenJPEG output (`precinct1_gray_128.j2k`, committed with its provenance) and on the test kit's 256² codestream; §4.5's `Ok` leg uses `PRECINCT_1_BYTES_PER_PIXEL = 256`, and such a file is refused at the default budget once it exceeds about a quarter of `max_image_pixels` — a geometry no encoder picks by default (OpenJPEG writes it only when told, at 12× the file size of the default geometry), recorded here rather than widened for.

---

## Appendix B: Facts verified against source

All paths relative to the repository root unless noted; registry paths under
`~/.cargo/registry/src/index.crates.io-*/`. Checked 2026-10-05 against `main`
at internal commit `a8481ce`.

### B.1 The run-0002 code this run touches

- `crates/ocr/src/engine.rs:199` `OcrEngine::ocr`; `:201-202` deadline from `start`; `:204-217` `render_gray`; `:220-222` pre-recognition budget check; `:223-228` `OcrRequest` and `backend.recognize`; `:233-238`/`:249-288` `build_page` (pixel → point scale `72 / dpi`, y flipped) into `TextPageBuilder` (`crates/core/src/text/ocr.rs:53-164`); `:310-314` `impl PageRecognizer`; `:316-328` `default_ocr_engine` over `static DEFAULT: OnceLock<Result<OcrEngine, OcrUnavailable>>`.
- `crates/ocr/src/config.rs:117-146` `OcrConfig` (fields, defaults `:148-163`, `validate` `:184-216`, `MIN_DPI = 1`, `MAX_DPI = 2400`); `crates/python/src/ocr.rs:236-290` `OcrEngine.__new__` kwargs; getters `:292-330`; `_scripted_engine` `:524-566`; `_scripted_stats` `:568-587`.
- `crates/render/src/lib.rs:50-58` `GrayImage` (8-bit, row-major, stride = width); `geometry.rs:43-71` `dpi'` and dimensions; `gray.rs:23-36` luminance.
- `crates/core/src/extract.rs:118-144` `PageResult`/`Extraction` (no stats field anywhere; `grep -i stats` finds only hayro's `InterpretStats`, the Python `_scripted_stats` hook, `fork_resets()` and `RESOLUTIONS`); `:399-408`/`:444-449` empty-OCR → `TextFallback`, `ocr_error = None`; `warning.rs:34-82` `WarningKind`, categories `:88-105`, `PageLimit` display `:116-118`.
- `crates/ocr/src/isolation/client.rs:390-428` the parent sends the raw raster (`width, height, dpi, psm, deadline_ms` + bytes); `helper.rs:264-290` rebuilds `GrayImage` and calls `recognize`; nothing else per page, so an engine-level step runs in the parent. `GLOBAL_INIT_CALLS` (`tesseract-sys/src/lib.rs:209`) is what proves the parent loads nothing.
- `crates/testkit/src/ocr.rs:204-236` `ScriptedBackend` records `last_request` and `last_image (w, h)` and ignores pixels; `calls()` `:145`.
- Corpus D: `crates/testkit/src/corpus_d/mod.rs:376-387` `sparse()` = one 11 pt line `Exhibit 12: Notice of Filings.` (30 chars) on A4; the committed PDF embeds a 2480 × 3509 gray image; `noise.rs:81` `add_noise`, `:145` `noise_variant`, `:279` `build_noise`, `NOISE_SEED = 20_261_003`; `crates/ocr/tests/quality.rs:56-62` gates, `:209-224` `recognise` (times `engine.ocr`), `:236-456` the test, `:239-247` `PDF_EXTRACT_TESSDATA` measurement mode, `:398-433` report schema; no test bounds a normal recognition's latency (ADR 13); functional bounds exist at `construction.rs:23` (< 2 s), `process.rs:164` (< 3 s), `:266`, `:295`.
- Locks of the fork hazard: `engine.rs:325` `DEFAULT`; `pool.rs:50-55` `state: Mutex<State>` + `Condvar` (per engine), `:185-197` `check_pid`; `isolation/client.rs:84` `TEST_ENV`, `:92-95` `LOG`; `tessdata.rs:48` `OVERRIDE: RwLock<Option<Candidates>>`; `tesseract-sys/src/lib.rs:204` `GLOBAL_INIT: OnceLock<Result<(), Error>>`. Current fork text: `crates/python/src/ocr.rs:22-27`, `:209-215`, `:454-458`; `pool.rs:16-25`; `backend.rs:131-140`; `README.md:190-195`; `ARCHITECTURE.md:75, 122, 172, 176, 189`; `THEORY.md:360-362, 385-386`; 0002 §2.4 `:245, :247`. `_native.pyi:145` (`OcrEngine` docstring: no fork text), `:199` (`default_ocr_engine`: no docstring). Tests: `crates/ocr/tests/fork.rs`, `process_fork.rs`, `tests/python/test_ocr_fork.py`, `pool_tests.rs:142, 189` — none reproduces the deadlock.

### B.2 hayro-jbig2 0.3.0 and hayro-jpeg2000 0.3.5

- **Provenance.** `Cargo.lock`: both `source = "registry+https://github.com/rust-lang/crates.io-index"`; `hayro-syntax 0.7.2` and `hayro-interpret 0.7.0` have no `source` (path). `crates/core/Cargo.toml:27-37` pins `=0.3.0` / `=0.3.5` with `default-features = false` ("not used directly"; 0.4 is incompatible with hayro-syntax 0.7.2). Consumer: `vendor/hayro-syntax/Cargo.toml:33-39, 46-47, 55-56, 78-86` (`images` → `dep:`; `std`; `unsafe` → `simd`). Resolved features: `std`, `simd`; `image` off. A `hayro-jpeg2000-0.4.0` directory sits in the registry cache, unused. Workspace `Cargo.toml:103-106` patches only the three; `vendor/patches/hayro/` holds `0001`–`0022`. `tools/vendor/import.sh:25` selects `| hayro-` rows; `export-patches.sh:33` hard-codes the three paths, single base `vendor/hayro-pristine-1`; `check-patches.sh` is crate-agnostic (`:40-44` numstat guard, `:26` ceiling, `:57` diff). No patch-count floor exists anywhere (ADR 2's "19-patch series" is prose, already stale at 22). `tools/license/gen-notices.sh:33-41` locates `hayro-cmap` and `hayro-jpeg2000` through `cargo metadata` ("not vendored, so this finds their registry copies"). Sizes: 416 KB / 436 KB on disk; 8,720 / 8,999 lines of `src/*.rs`; `hayro-jpeg2000-0.3.5/assets/` holds three ICC profiles, `LICENSE.txt`, `README.md` (already in Part 3 of the notices).
- **H13 plumbing in hayro-syntax.** `filter/mod.rs:41-52` `DecodeLimits`; `:135-139` thread-local `IMAGE_PIXEL_LIMIT_HITS`, `:147-157` `image_pixel_limit_hits()`, `:159-162` `note_image_pixel_limit_hit()`, `:169-181` `check_image_pixels`; `:249-288` `Filter::apply` passes `max_pixels` to `dct::decode` (268), `ccitt::decode` (270), `jbig2::decode` (272), `jpx::decode` (274). `object/stream.rs:226-230` declared check, `:262-267` reported check, `:315-331` `DecodeFailure` (incl. `ImagePixelLimit`). `filter/jbig2.rs:24-28` globals + `Image::new_embedded`, `:34-38` header check, `:72-74`/`:97-99` `decode` through the `hayro_jbig2::Decoder` trait, every error → `ImageDecode`; `filter/jpx.rs:37-42` header-only `Image::new` with `target_resolution: None`, `:51` second `Image::new`, `:82` `decode()`, errors → `ImageDecode` via `.ok()?`. hayro-interpret `x_object.rs:462-470` maps `ImagePixelLimit` → `InterpreterWarning::ImagePixelLimit` (`interpret/mod.rs:146-150`), else `ImageDecodeFailure`. Core: `limits.rs:49, 62`; `open.rs:114-119`; `text/extract.rs:79-90, 215-225, 240-281`; `warning.rs:76, 102`; render `signals.rs:27-52, 163-175`. Public codec entry points: `hayro_jbig2::Image::decode_with(&self, decoder, &mut DecoderContext)` (`lib.rs:195`; `DecoderContext` pub, `Default`, no limit fields, `:26-30`; `ScratchBuffers` `:33-35` is passed to generic/refinement/text/halftone/pattern but **not** to `symbol::decode`, `lib.rs:351-357`); `hayro_jpeg2000::Image::new(data, &DecodeSettings)` (`lib.rs:106-126`, `Copy`, no limit field), `decode()`/`decode_into(buf, &mut DecoderContext)` (`lib.rs:216`; `DecoderContext` `decode.rs:101-107`, `Default`).
- **hayro-jbig2 allocation sites** (`#![forbid(unsafe_code)]`). Ceiling: `bitmap.rs:15` `MAX_DIMENSION = u16::MAX`; `Bitmap::new_with` `:57-79` (`vec![word; stride*height]` at 70, `Word = u32`), `reinitialize` `:81-103`; ≈ 512 MiB per bitmap, no aggregate cap; `Bitmap` derives `Clone` (`:29`); the arithmetic decoder pads input with 0xFF (`arithmetic_decoder.rs:281-291`), so size is not tied to data length. Sites: page bitmap `lib.rs:687` (page info, **bounded by H13**); generic region `decode/generic.rs:17-23`; refinement `generic_refinement.rs:22-28` (fit check at 50 runs *after* the allocation); text region `text.rs:29-35` (`MAX_INSTANCES = 10_000`, `:95-99`), per-instance refinement `text.rs:565`; halftone region `halftone.rs:23-29`, HSKIP `:224-227` (grid ≤ 65535 checked at `:97-99`); gray-scale values `gray_scale.rs:186-188` `vec![0_u32; width*height]` (**≤ 16 GiB from 8 header bytes**), planes `:190-191`; pattern collective `pattern.rs:26-30`, patterns `:78, 87`, shifted patterns `:136-138` (≈ 512 MiB); symbol dictionary `symbol.rs:41, 409` (`MAX_SYMBOLS = u16::MAX`, `:815-817`), symbols `:109, 257, 625`, height-class collective `:589, 615`, aggregation `:389-395`, export `:679` + **`.clone()` per exported symbol** `:693-698`; stored intermediate regions `lib.rs:314, 440, 479, 492`, `store_region` `:614-616` (unbounded count); `Vec<&Bitmap>` collects `lib.rs:325-331, 362-368, 415-421` and cloned Huffman tables `:334-340, 373-379, 424-430` (duplicate references allowed: `segment.rs:203-220` requires only `referred < segment_number`); `SymbolIdDecoder` `symbol_id_decoder.rs:17-20` `vec![ctx; 1 << code_len]` (`code_len` from `num_symbols = symbols.len() as u32`, `lib.rs:381`, truncating; up to 4 GiB); uniform Huffman `huffman_table.rs:140-145`, `:68-78, 112`; custom tables `:215-248` (linear in data); code lengths `text.rs:684`; context tables `generic.rs:46`, `generic_refinement.rs:75`, `text.rs:76`, `gray_scale.rs:122`, `pattern.rs:61`, `symbol.rs:483-484`, `integer_decoder.rs:19` (≤ 2¹⁶, fixed). Errors: `error.rs:6-26` `DecodeError { Parse, Format, Segment, Huffman, Region, Template, Symbol, Overflow(OverflowError), Unsupported }`, `OverflowError` `:124-138` (`BitmapDimension`, `SymbolCount`); `bail!`/`?` out of `decode_segments` (`lib.rs:260`).
- **hayro-jpeg2000 allocation sites.** Ceilings: `j2c/codestream.rs:568-574` `MAX_DIMENSIONS = 60000` on `image_width()/image_height()` — which are **shrunk** by component subsampling `XRsiz/YRsiz` (u8, up to 255; `:507-516`, `:631-646`), so the reference grid and the tile count can be 255× larger per side than what H13 and the ceiling see; `num_tiles()` `:490-504` is an unchecked `u32` multiply; `MAX_LAYER_COUNT = 32`, `MAX_RESOLUTION_COUNT = 32`, `MAX_PRECINCT_EXPONENT = 31` (`:12-14`), `MAX_BITPLANE_COUNT = 32` (`segment.rs:14`). Sites: header `codestream.rs:52-53, 603` (linear in `Csiz`); **per-tile records** `tile.rs:135-137` one `Tile` per `num_tiles()`, each cloning `header.component_infos` (`:78-121`), built in `decode.rs:36` (not in `Image::new`); channel planes `decode.rs:110-125` `SimdBuffer::zeros(w*h)` f32 per component (channel validation `lib.rs:297-323` is skipped when a `pclr` box is present); output `lib.rs:196-200`; per-tile coefficients `build.rs:19-31`; structural records `build.rs:60-72` (sub-bands), `:115-177` (precincts, two `TagTree`s each), `:199-240` (one `CodeBlock` per code-block plus `num_layers ≤ 32` `Layer`s; code-block size `min(xcb, PPx−1)` can be 1 × 1 at `r > 0` when `PPx = 1`, `tile.rs:650-680`); code-block buffers `bitplane.rs:425-440`; IDWT `idwt.rs:224`; JP2 boxes `jp2/pclr.rs:18, 33-36`, `cdef.rs:12`, `cmap.rs:11`. Errors: `error.rs` `DecodeError { Format, Marker, Tile, Validation(ValidationError), Decoding, Color }`.
- **Post-decode checks today**: only `object/stream.rs:262-267` (reported size vs cap). Nothing compares decoded length with `w × h × c` in hayro-syntax; hayro-interpret `fix_image_length` (`x_object.rs:922-948`) pads at most one row or truncates.

### B.3 Tesseract thresholding (vendored 5.5.3)

- `src/ccmain/tesseractclass.cpp:84` `thresholding_method` default 0 (Otsu); `src/api/baseapi.cpp:2022-2026` → `thresholder_->ThresholdToPix`; `src/ccmain/thresholder.cpp:284-303` → `OtsuThresholdRectToPix` over `rect_left_, rect_top_, rect_width_, rect_height_` (the whole image after `SetImage`); `src/ccstruct/otsuthr.cpp:35-91` `OtsuThreshold`: per channel `HistogramRect` + `OtsuStats`; empty when `best_omega_0 ∈ {0, H}`; `hi_value = best_omega_0 < H × 0.5`; convincing foreground when `best_omega_0 < 0.25 H` (`hi_values = 1`: pixel `> threshold` is background) or background when `> 0.75 H`; otherwise "no thresholding information" and the best of the bad lot is used. `OtsuStats` (`:116-150`): `H`, `mu_T`; maximise `sig_sq_B` over `t ∈ [0, 254]` with `omega_0` accumulating `histogram[t]` (so `omega_0` counts pixels `≤ t`), strict `>` so the first maximum wins. F11 replicates exactly this.

### B.4 The B-items, re-verified (`main` at internal commit `a8481ce`)

- **B4 — still valid, narrower than reported.** No denylist. `Instance::with_variables` (`crates/tesseract-sys/src/lib.rs:296-365`) is reachable only from the sys crate's own tests (`tests/instance.rs:66-74, 119-128`): `OcrConfig` has no variables field (`crates/ocr/src/config.rs:118-146`), Python no kwarg (`crates/python/src/ocr.rs:237-264`), `TesseractFactory::create` and the helper pass `&[]` (`crates/ocr/src/tesseract.rs:74`, `isolation/helper.rs:249`). Path: `lib.rs:296-365` → `shim.cpp:131-139` `Init(..., &names, &values, set_only_non_debug_params = false)` → `vendor/tesseract/src/ccmain/tessedit.cpp:118-136` `ParamUtils::SetParam` → `ccutil/params.cpp:81-126` looks in `GlobalParams()` first, so every namespace-scope `*_VAR` (≈ 200 across 21 files) is writable; `tprintf.cpp:37` `debug_file`, `:53-71` `get_debugfp` falls back to stderr when empty, and `SetParam` applies an empty value then returns (`params.cpp:86-91`), so `("debug_file", "")` reverts output to stderr and `""` then a path redirects it; also racy on a `std::string`. Other globals of note: `log_level` (`tprintf.cpp:35`), `dotproduct` (`arch/simddetect.cpp:88`), `equationdetect_save_*` (write files), `stream_filelist`, `document_title`, `curl_*`, `classify_font_name` (`api/baseapi.cpp:93-121`); per-instance file writers `tessedit_write_params_to_file` (`tesseractclass.cpp:139`, `tessedit.cpp:138-139`) and `tessedit_write_images` (`:391`). `set_only_non_debug_params = true` would block only names containing `debug`/`display` (`params.h:139`). A model's `lang.config` is applied with `SET_PARAM_CONSTRAINT_NONE` (`tessedit.cpp:113-116`). `ParamUtils::FindParam` (`params.h:74-80`) searches a global vector then the member vector; `GlobalParams()` `params.h:339`.
- **B5 — still valid in the sys crate only; the report's "tesseract-sys / ocr" is wrong about ocr.** `lib.rs:315-324` accepts any string of `[A-Za-z0-9_+]` (so `+`, `eng+`, `+eng`, `eng++fra`); Tesseract (`tessedit.cpp:265-289`) skips leading/repeated `+`, a trailing `+` yields an empty code, and `tessedit.cpp:82` substitutes `eng` for an empty language — `"+"` loads `eng`, `"eng+"` loads it twice. The ocr crate (`config.rs:174-176, 189-192`, per element, `+` disallowed → `InitFailed`), the helper (`helper.rs:231-238`) and Python (`crates/python/src/ocr.rs:128-147`, `ValueError`) already reject empty segments. Tests: `tests/instance.rs:109-116` (no `+` cases), `config_tests.rs:74-81` (`"", "eng+deu", "+"`), `tests/python/test_ocr.py:62-65` (`"eng+"`).
- **B6 — still valid, worse than described.** `shim.cpp:138-144` fails only on a non-zero `Init`; Tesseract returns −1 only if *no* language loaded (`tessedit.cpp:360-363`); a failed sub-language is `tprintf`'d (silenced) and dropped (`:350-353`); a failed **primary** promotes the next language (`:342-349`), so `["eng"(0 B), "deu"]` runs on `deu`. `GetLoadedLanguagesAsVector` exists (`api/baseapi.cpp:385-394`) and is not exposed. The presence check `tessdata.rs:116-118` is `metadata().is_file()` (no size), applied at `:134` and `:160-174`; `info().languages` is the configured list (`tesseract.rs:235`, `client.rs:593`). Only the single-language case is tested (`construction.rs:118-130`, `InitFailed` with Tesseract's message).
- **B7 — still valid; expected `OnceLock` behaviour; handled as documentation (F4).** `engine.rs:324-328` `get_or_init(|| OcrEngine::tesseract(OcrConfig::default())).as_ref()`; an `Err` is stored (O12 holds); a panic propagates and leaves the cell empty (`OnceLock` does not poison, unlike `Once`); nothing on the path catches panics (`tesseract.rs:200-253`, `client::build`); Python `crates/python/src/ocr.rs:459-469` → `guard.rs:65-88` `InternalError`, next call retries. Tests cover errors only (`default_engine_failure.rs:11-30`, `default_engine.rs`, `test_ocr.py:120-131`). ADR 35 says panics propagate.
- **B9 — still valid, exactly as described.** `pipe.rs:216-230`: `c_int::try_from(-1i64)` succeeds, `.max(first)` gives `first`, the range is empty, nothing is marked; `unwrap_or(c_int::MAX)` covers only values above `i32::MAX` (and a huge positive value would run up to 2³¹ `fcntl` calls in the child). Linux `:198-215` tries `close_range(3, u32::MAX, CLOSE_RANGE_CLOEXEC)` first and falls through only on kernels before 5.11 or under seccomp; every other Unix always sweeps; `closefrom` unused; called from `cloexec_extra_fds` (`:183-194`) via `client.rs:248` `pre_exec`. No test of `mark_cloexec_from`.
- **B16 — still valid, and wrong for Python too.** `tessdata.rs:147-156` formats `no directory holds {first}.traineddata (set OcrConfig.tessdata or TESSDATA_PREFIX)` regardless of `explicit`/`candidates.prefix`; `searched` (`:131-139`, `Candidates::list` `:70-79`) lists explicit, `$P`, `$P/tessdata`, the known directories; `TESSDATA_PREFIX` read at `:53-58` (empty = unset, `:83-85`); the process path shares `resolve` (`client.rs:567`). Tests matching text: `tests/python/test_ocr.py:115` (`"no directory holds eng.traineddata" in str(e)`), `tessdata_tests.rs:127` (`contains("eng.traineddata")`); reason-only checks elsewhere. No document quotes the message.

### A.6 Revision (3): realistic adversarial pages and the foreground-side rule

Same probe, same machine, the pinned `tessdata_fast` model this time (the
one unit 02 pins). Each raster is a 300-dpi corpus-D render (2480 × 3509)
transformed per pixel as the recipe says; "+ noise" adds the test kit's
σ-12 field with a fresh seed. `side` is Tesseract's foreground side (dark
when `ω₀ < H/2`); `p` and `iso` are that side's share and isolation; "old"
is the signed-off F11, "new" is revision (3). Tesseract was run on every
raster as built (`RAW`) and, where the new rule said `Speckle`, on its
`downsample2` at 150 dpi. Characters are non-whitespace.

| Raster (recipe) | `t` | class | side | `p` | iso | line | old | new | Tesseract RAW @300 | after `downsample2` @150 |
|---|---|---|---|---|---|---|---|---|---|---|
| banner: heading page, top 8 % inverted | 131 | fg-dark | dark | 0.140 | 0.000 | 0.273 | Clear | Clear | 957 ms, 2,424 chars, correct | — |
| body page, top 45 % inverted | 127 | **none** | dark | 0.444 | 0.000 | 0.048 | **Speckle → skip** | Clear | 1,772 ms, 3,011 chars, correct | — |
| same + noise | 127 | none | dark | 0.444 | 0.000 | 0.048 | Speckle → skip | Clear | 1,740 ms, 3,005, correct | — |
| shaded box: body, rows 30–60 % on bg 200 | 130 | fg-dark | dark | 0.081 | 0.000 | 0.357 | Clear | Clear | 1,168 ms, 3,009, correct | — |
| shaded 50 % on bg 200 + noise | 125 | fg-dark | dark | 0.081 | 0.000 | 0.356 | Clear | Clear | 1,187 ms, 3,009, correct | — |
| gray figure: body, rows 40–70 % flat 128 | 184 | **none** | dark | 0.350 | 0.000 | 0.089 | **Speckle → skip** | Clear | 688 ms, 1,658, correct | — |
| half gray: top 50 % flat 128, text below | 188 | **none** | light | 0.465 | 0.000 | 0.041 | **Speckle → skip** | Clear | 529 ms, 1,211, correct | — |
| half gray + noise | 186 | none | light | 0.465 | 0.000 | 0.041 | Speckle → skip | Clear | 514 ms, 1,211, correct | — |
| photo band: body, rows 40–70 % gradient + noise | 162 | fg-dark | dark | 0.247 | 0.010 | 0.161 | Clear | Clear | 728 ms, 1,660, correct | — |
| full-page gradient + noise, no text | 129 | none | light | 0.500 | 0.016 | 0.031 | Speckle → skip | Clear | 740 ms, 213 chars of junk | — |
| slide: body remapped to bg 40 / text 220 | 122 | bg-dark | light | 0.079 | 0.000 | 0.360 | Clear | Clear | 2,123 ms, 3,009, correct | — |
| slide + noise | 122 | bg-dark | light | 0.079 | 0.000 | 0.361 | Clear | Clear | 2,146 ms, 3,009, correct | — |
| body page inverted | 116 | bg-dark | light | 0.079 | 0.000 | 0.360 | Clear | Clear | 2,113 ms, 3,009, correct | — |
| noise field inverted (blank) | 8 | bg-dark | light | 0.242 | **0.331** | 0.165 | **Clear → 26.7 s stall** | Speckle | 26,712 ms, 11,043 chars of garbage | `t` 5, none, `p` 0.387, iso 0.141 > 0.070 → **skip** (would be 2,417 ms of garbage) |
| `sparse_below_min_chars_noise` inverted | 9 | bg-dark | light | 0.217 | **0.376** | 0.188 | **Clear → 14 s garbage** | Speckle | 14,034 ms, 7,624 garbage | bg-dark, `p` 0.0007, iso 0.000 → **recognise**: 21 ms, exact |
| sparse page on bg 128 + noise | 127 | none | dark | 0.484 | 0.071 | 0.035 | Speckle | Speckle | 29,884 ms, garbage | none, iso 0.086 > 0.043 → **skip** (3,790 ms of garbage) |
| one-line page on bg 128 + noise | 127 | none | dark | 0.485 | 0.071 | 0.035 | Speckle | Speckle | 18,751 ms, garbage | none, iso 0.086 > 0.043 → **skip** (3,313 ms of garbage) |
| correlated: `sparse_below_min_chars_noise` rendered at 150, upscaled 2 × nearest | 151 | fg-dark | dark | 0.0007 | 0.000 | 0.499 | Clear | Clear | 57 ms, exact | — |
| correlated: noise field `downsample2`d then upscaled 2 × nearest (no text) | 249 | none | dark | 0.363 | 0.000 | 0.082 | Speckle → skip | Clear | 6,964 ms, 2,942 chars of garbage | (its `downsample2` is speckle under both rules, iso 0.165, but is never consulted after a `Clear`) |
| *reported once (internal #65 A6, 2026-10-21 note): a correlated-noise page that reached Tesseract as `Clear`; no fixture, statistic not measured* | — | — | — | — | — | — | Clear (as reported) | Clear | 2.2 s, empty output, no `OcrNoiseSkipped` warning | — |

Reading. The signed-off rule's two class shortcuts were both wrong in the
field: `None` is the normal class of any page with a large dark region, and
its foreground is connected (isolation 0.000) exactly like text; `Background-
Dark` is the normal class of an inverted noise page, whose *light* side is as
isolated (0.33–0.38) as the dark side of the upright one. Testing the
isolation of whichever side Tesseract will treat as ink fixes both without
touching any corpus-D verdict, and the margins are the same as before: ≥ 2×
above the line for every stall case, ≤ 0.016 absolute for every page with
content. The two residuals it leaves are listed in §2.3: non-noise pages
without text (a photo) go to Tesseract and yield junk in under a second, and
correlated (multi-pixel) noise is connected and passes — 7 s of garbage for
a blocky noise field, no stall for the realistic case (a noisy 150-dpi scan
rendered at 300, 57 ms, exact).
