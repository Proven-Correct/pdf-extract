# Theory of operation

This document is for an engineer about to change pdf-extract. It explains why
the system has the shape it has. The enumerated map, the invariants and the
landmines are in [`ARCHITECTURE.md`](ARCHITECTURE.md) and are not repeated here;
the full record of each decision, with the alternatives that lost, is in
[`docs/adr/`](docs/adr/README.md) (cited "ADR n"). Sources for the rationale
below are those records, the run specification
([`docs/specs/0001-text-path.md`](docs/specs/0001-text-path.md), "spec §n") and
the design specification ([`docs/design-spec.md`](docs/design-spec.md)). Where a
reason was never written down, this document says so instead of supplying one.
What the system currently promises, as opposed to why, is stated per capability
in [`openspec/specs/`](openspec/specs/). Those specs were backfilled from the run
specifications, which now serve as their history (ADR 84).

## Orientation

Three facts shape everything.

First, the library has to behave like something it may not look at. The product
exists because PyMuPDF is AGPL; its first consumer's downstream code depends on the exact
shape of PyMuPDF's `get_text("text")`, so the replacement must reproduce that
shape without reading, copying or translating any MuPDF code (ADR 5). The only
way to learn that shape is to run PyMuPDF as a black box and observe it. So the
project is, at its core, a measurement problem: probe the oracle, fit a rule to
the observations, encode the rule, and keep the observations as regression
data.

Second, every PDF is hostile. The library is called on uploads, from many
threads, inside a Python process that must not crash. Any limit that can be
exceeded by crafted input (nesting, page count, decoded size, operator count,
time, characters) has an explicit bound, and every bound has a defined outcome:
a skipped subtree plus a warning, a hard failure of one stream, or a typed
error (ADR 18, 20, 24, 27).

Third, the foundation is someone else's code. The core is pure Rust on the
hayro crates (ADR 1), chosen after the exclusions recorded in that ADR, but which
as released could not meet the robustness requirements. The project therefore carries a small, reviewable
patch series against three of them (ADR 2, 3). Much of the design is about
where to put a change: in the fork when only hayro can see the problem, in core
when core can own it.

If you hold only those three ideas, you can place most new code: a rule that
came from watching the oracle goes in the text path with a fixture and an
expectation; a bound goes in the place that can enforce it and reports through a
warning or limit error; something hayro cannot express goes in the fork with its
own commit and patch.

## How it is structured

The component map is in ARCHITECTURE.md. The relationships that matter:

**Open and the text path are separate pipelines joined by a small record.** Open
(`source`, `open`, `index`) turns bytes into a `Document` and a page index. It
never builds page objects: a `PageRef` is just enough to rebuild a hayro `Page`
on demand. The text path runs only inside a page operation: it rebuilds the
page, interprets it into `CharRecord`s through a hayro `Device`, then assembles
lines. The two meet only at `Loaded`, the immutable bundle an operation clones.
That boundary exists because of the honest-page-count requirement (ADR 22): an
open that built pages would cost seconds and gigabytes on a hostile 1M-page file.

**The device and the assembler are deliberately two stages.** The device answers
"which characters, in what content order, at what geometry, with what Unicode";
the assembler answers "which of those belong on one line". The design
specification states the split (ADR 4) but does not record why it was drawn there.
What can be observed is that Unicode questions (ToUnicode, glyph names, CID
fonts) and layout questions (gaps, rises, backward moves) have separate probe
evidence and separate residual lists. The stream of records is the contract
between the stages, with form and annotation boundaries carried as separate
markers (ADR 32).

**The fork sits underneath and is mostly invisible above core.** Core's contact
with it is a short list: lazy pages, budgets and cancellation, per-glyph facts,
a few thread-local counters, and the `Device` hooks. Everything else in hayro is
used as published. The design specification's exit condition (rebase the fork on
every adopted hayro release) is why the surface is worth keeping small.

**The Python binding is thin.** It converts types, releases the GIL, and turns
panics into `InternalError`; behaviour lives in core. Why it is kept that thin is
not recorded beyond the panic split in ADR 35.

## Core ideas

**The oracle is a black box, and its answers are data.** Expected outputs are
checked-in JSON keyed by content hash, so CI never needs PyMuPDF (ADR 6). Two
gates use it differently. Gate C is exact: small synthetic fixtures, one or more
per compat point, where any drift is a regression; where the library is
deliberately different the fixture is pinned to our own snapshot and must keep
differing. Gate B is statistical: real files from hayro-tests and PDFBox (ADR 7)
scored by similarity, because real fonts and layouts will never match
perfectly. The numbers in the specification (median 0.98, p10 0.90) are initial
thresholds; the measured corpus-B results sit well above them, and tightening
them is a specification change, not a code change.

**Compat points (KP) are the unit of knowledge.** Each KPn is a question about
the oracle ("what suppresses a synthetic space?") with a probed answer and a
fixture that pins it. When you meet a surprising behaviour, the right move is a
new KP, a probe, a fixture and either an exact row or a documented divergence.

**Deliberate differences are first-class.** Not every oracle behaviour is worth
copying: it emits Tr 4-6 text twice (ADR 53), emits NUL (ADR 50), counts duplicate
page-tree leaves twice and raises on cycles (ADR 23). Each such difference is a
named KP, pinned as a divergence fixture, and listed in the residuals. The
design spec's requirement K2 is that every difference is either configurable back
to parity or listed; an unrecorded difference is a bug.

**Bounded by construction, with a known outcome.** Recursion is replaced by
iteration (the page walk) or capped (object nesting at 256, nested interpretation
at 50). Work is capped by a cooperative budget that hayro checks every 4,096
operators; it cannot interrupt a single codec call, so the JBIG2 decoder carries
its own bounds on time (see "Why decoder time is bounded by guards", below).
Memory is capped by file size, per-stream decode size, 16 bytes per
page, and a character cap. The object-nesting
bound is a fixed constant because the reader that enforces it has no access to
per-document settings, and a process-wide knob would let documents interfere
(ADR 19).

**State crosses a boundary only through thread-locals or `Arc` clones.** Because
hayro reports some events where no context exists (a content operand is parsed by
a bare reader), the fork counts them in thread-locals and core samples them
around each synchronous operation. Because operations run concurrently on one
document, per-document counters would charge one operation's events to another;
thread-locals do not. This is why "stay synchronous between the two samples" is
an invariant.

## Design decisions and tradeoffs

Only the decisions whose reasoning is needed to change code safely are
summarised; the ADRs have the rest.

**Why hayro, and why a fork (ADR 1, 2, 3).** MuPDF/PyMuPDF (AGPL) and Poppler
(GPL) are excluded by the licensing requirement. PDFium was rejected on
concurrency: it is not thread-safe even across documents, so `pdfium-render`
serialises every call behind one global mutex, which fails the one requirement
the project cares most about, and it needs a prebuilt C++ binary. `lopdf` loads
eagerly, repairs little and has no interpreter or fonts. Writing a parser was
rejected because robust handling of broken real-world files takes years. hayro
was selected on conditions, because probing found it opens a 1M-page file in 8.9
s and 1.49 GB (PyMuPDF: 64 ms, because it trusts `/Count`), has no nesting cap
(stack overflow at 200,000 levels), no decode cap, no work budget, and a lazy
xref repair that panics under concurrency. The fork carries the fixes. Only the
crates we patch are vendored (ADR 3's rule): the three original hayro crates and,
since run 0003, the `hayro-jbig2` and `hayro-jpeg2000` codecs, whose decoders
needed memory budgets. `hayro-cmap` and `hayro-ccitt` are not vendored (ADR 80);
the bug fuzzing found in `hayro-cmap` (internal #121) is handled by a gate in the vendored caller
(H24, below). A git dependency was not an option (`deny.toml` forbids
git sources), so the codecs follow the same mechanics, with a second pristine
tag and a patch export in two ranges. The diff stays reviewable and the unpatched
leaves reproduce from `Cargo.lock`; CI re-applies the patches to a fresh import
and fails on drift. The design specification states the exit condition: if
upstream declines a blocking change the fork becomes permanent and is rebased on
each adopted release with the corpus gates run on every rebase. Upstream pull
requests were not part of this run.

**Why PyMuPDF only as a black box (ADR 5, 6).** The reason for replacing it is
its license, so reading its source would defeat the project; behaviour is matched
by observation only, from a separate dev-only project the library never imports.
Consequences that shaped the design: the oracle's version is pinned (1.28.2) and
was never changed during the run; probes use files saved to disk, because an
unsaved in-memory document behaves differently (the first optional-content probe
got this wrong, ADR 44); and where observation cannot determine a rule, as with
`/ActualText` distribution, the implementation takes the simple rule that fits
every non-pathological probe and lists the rest as divergence (ADR 41).

**Why CropBox ∩ MediaBox (ADR 45, 46).** The first sign-off clipped glyphs to the
MediaBox. That came from a probe that placed text with `insert_text` after
`set_cropbox`, whose coordinates are relative to the crop box, so it wrongly
concluded the crop box does not clip. After unit 03's generators contradicted it,
the probe was redone with text in absolute PDF coordinates: text outside a
CropBox inside the MediaBox was dropped, and text inside a CropBox that extends
beyond the MediaBox was dropped where it leaves the MediaBox. Both happen only if
the clip is the intersection. This matches the design specification's definition
of the visible box. The test is per glyph, not per line: a glyph is dropped when
its box is entirely outside, kept if partly inside, so `x590` at the page edge
yields `x`. The glyph box uses the font's ascent and descent because the oracle
keeps an 11 pt glyph whose baseline is 3 pt below the page edge and drops an 8 pt
one, which brackets Helvetica's ascent. Clip paths use their bounding box because
the oracle does the same (a diamond clip keeps text in its corner).

**Why four thresholds, and why those numbers (ADR 47).** The oracle's line
assembly is a set of comparisons against the incoming glyph's size, not a
distance. Sweeps at 11 and 22 pt, mixed sizes (a small glyph after a big one
behaves by the incoming glyph's size), superscripts and backward moves show that
the along-the-line and across-the-line tests are independent. Fitting a single
space threshold and a single line-break threshold failed on the recorded data,
twice. The space bracket under a strict comparison is [0.1477, 0.15): a 3.25 pt
gap at 22 pt joins (0.1477 em) and a 3 pt gap before a 20 pt glyph (exactly 0.15
em) gets a space, so 0.15 itself contradicts a data row; the earlier "(0.148,
0.159]" was a bracket error. A single 0.8 em line break fails because an 11 pt
glyph raised 8.8 pt joins (needs `>= 0.8`) while an 8-to-20 pt forward gap of 16
pt breaks (needs `< 0.8`). Hence three break thresholds with separate brackets
(forward gap [0.7955, 0.8), perpendicular [0.8, 0.8182), backward [0.7727,
0.8182)). Each default is placed strictly inside its bracket, so no committed
fixture lies on a boundary and the result does not depend on floating point. The
oracle itself compares with `>=` and lets its single-precision glyph placement
decide the hair-under cases (the same 3 pt gap is a space at x = 72 and a join at
x = 10); the three position sweeps that exposed this were moved out of the oracle
comparison and kept as our own tests. Anything finer (why the oracle's perpendicular and forward thresholds differ by a hair at
0.8 em) is not recoverable from the sources; the specification only notes that
the asymmetry may itself be noise in the oracle.

**Why the oracle's two code point sets and ToUnicode rules (ADR 48, 49).** Both
are black-box measurements adopted as recommendation (b) with user sign-off on
2026-10-02. The synthetic space is suppressed only by an incoming U+0020 or a
line ending in U+0020, U+1680, U+3000 or U+FEFF. Using `char::is_whitespace`
would have turned eleven probed fixtures into divergences. Control characters from
ToUnicode: a single-code-point entry in U+0008-U+000D reads as a space, other
single controls are ignored (the glyph name decides), and multi-code-point entries
are verbatim. The alternative of keeping single controls was dropped in favour of
parity; no deeper motive is recorded.

**Why Unicode text is sanitised and the AGL is Adobe-only (ADR 50, 51).** The
oracle emits raw codes, NUL included, for unmapped glyphs. The recorded reason to
replace NUL and other C0 controls is that PostgreSQL `text` rejects NUL whatever
its source. The glyph-name table dropped about 255 TeX names, a user decision,
because their source list is GPL-2 and the project bans copyleft-derived data in
the shipped graph; the oracle leaves most TeX names raw anyway.

**Why PyO3 0.29 and free-threaded from 3.14t (ADR 11).** A user decision on
2026-10-02. PyO3 0.28.3 would carry two security advisories fixed only in 0.29, and
the user would not accept advisory exceptions; 0.29 cannot build for 3.13t. The
cost is a narrowing of the design specification's free-threading target.

**Why our own code is GPL-3.0-or-later (ADR 8, 9).** The project began MIT (rationale
not recovered from project sources); on 2026-10-02 the user chose
GPL-3.0-or-later for our own code, leaving the dependency graph permissive-only.
The mechanism keeps the policy mechanical: workspace crates are `publish = false`
and exempt through cargo-deny's `private` setting, so a GPL dependency is still
rejected and a new crate needs no edit. A scoped exception was rejected because
it would need editing for every new crate. The motive for the choice of license
itself is "the user's licensing choice", nothing more.

**Why the notices have a Part 5 for the Rust standard library (ADR 94).** Every
notice-producing tool in the pipeline reads our Cargo graph: cargo-about for
Part 1, `cargo tree` for the SBOM, cargo-deny for the policy. The standard
library is not in that graph. The compiler links it from the sysroot, so it
was invisible to all three, although both shipped binaries contain it
statically, along with ten crates.io crates that std itself depends on.
v0.1.0 therefore shipped without their notices. The gap surfaced in the Phase 3
licence audit, not in any check, because no check knew the sysroot existed.

The fix keeps the pipeline's rule that notices are derived, never typed. Std's
own sources ship with the toolchain (`rust-src`) as a Cargo workspace, so the
same `cargo tree` that describes our graph can describe std's, provided it is
the pinned toolchain's (hence `rustup run`, never the `rustc` on PATH). The one
real choice is which features to resolve std with. `backtrace,panic-unwind` is
what the Rust distribution builds, and the cross-check against the rlibs that
`rust-std` actually ships is what makes that choice verifiable rather than
assumed. The Rust project's own copyright document ships unchanged, because
any text extraction from it would put a converter's output into a legal
notice. The verifier's new rule, that every SBOM crate must appear in the
notices, exists because Part 1 and the SBOM are still computed from different
graphs. That difference is harmless today, and this rule makes sure a future
divergence is caught rather than shipped.

**Why the licence checks are a reusable workflow, and why one schedule trigger
exists (ADR 95).** The release workflow used to trust that main's `deny` job had
passed on the tagged commit; nothing enforced it. Making `license.yml` a
`workflow_call` that both CI and the release invoke means there is one list of
checks, and a release cannot skip it. The weekly advisories run is the only
`schedule` trigger in a repository that otherwise refuses them on cost grounds
(ADR 87). It is allowed because it builds nothing and costs a few runner-minutes, and
because a library that stays stable between releases might otherwise go weeks
without anyone seeing a new RustSec advisory against it.

**Why OCR is built the way it is (run 0002; spec 0002 §6, D-n).** The shape
follows from keeping the text path's guarantees while adding a C++ library.

- *Vocabulary in core, engine elsewhere (D-2).* Core stays `forbid(unsafe_code)`
  and must build without Tesseract (`cargo test -p pdf-extract-core` never
  compiles C++); so `OcrError`, `TextSource::Ocr`, `PageRecognizer` and
  `extract()` live in core, while the engine, pool and Tesseract live in
  `pdf-extract-ocr`. `Page::ocr` is an extension trait, a deviation from the
  design spec, for the same reason; the Python surface is unchanged.
- *A shim, not Tesseract's C API (D-5).* `capi.h` has no `try/catch`, so a C++
  exception would unwind into Rust. The one C++ file we own catches everything
  and returns codes. A timeout is Tesseract's `-1` plus our own clock past the
  deadline (D-11), because `-1` also means cancel, uninitialised and layout
  failure.
- *Vendored and trimmed (D-4).* The legacy engine, graphics, OpenMP, codecs and
  native-CPU tuning are off. That removes every `exit()` on a model-load path
  and ScrollView's `exit(0)`, keeps one-thread-per-instance (C5) and keeps
  wheels portable, and leaves `SetImage` plus model parsing as the attack
  surface. The cost: only the LSTM engine works on the vendored build.
- *In process by default in Rust, process isolation by default in Python
  (D-7, D-6; ADR 0061, 0092).* Rust's default matches the design spec and
  PyMuPDF's exposure: an `abort()` inside Tesseract ends the process. Python
  flipped once the cost was measured (internal #53): a Python process is usually a
  long-lived server where one bad scan must not take the interpreter down,
  and the wheel always ships the helper beside the extension (ADR 0091), where
  the binding's own default engine looks for it. A Rust binary has no such
  directory, so making Rust callers install a helper beside their executable
  was not worth the same default; they opt in. `Isolation::Process` contains it with a bespoke
  length-prefixed protocol over the child's pipes and `poll(2)` deadlines: no
  new dependency, no filesystem endpoint, identical on Linux and macOS, one
  file compiled into both sides. Measured cost on clean 300 dpi pages was
  1.6 % (0.3 % re-measured), which is what let Python flip. A missing helper
  is a stored `engine_not_built`, never a silent fall back to in-process:
  that would turn a broken install into an uncontained one.
- *Why the oracle rasterises corpus D (D-9).* A scan made by the renderer under
  test would test nothing; PyMuPDF rasterises the text PDFs, and the page is
  re-wrapped as an image-only PDF with the known text as ground truth. Noise at
  sigma 12 does not compress, so those twelve files are built at test time from
  the committed clean ones with integer Irwin-Hall deviates (D-22: deflate output differs between `miniz_oxide`
  versions and a pinned hash needs byte identity) and
  pinned by SHA-256.
- *Quality is a CER comparison, not equality (D-10, D-12).* Rendering differs by
  host SIMD level and from MuPDF; the gate compares error rates (clean ours at 72 dpi
  no worse than the oracle's at 72 plus 0.01, clean ours at 300 below both
  72-dpi figures, and noise, skew and blur on the first rule only) and latency is report-only (no timing gates on shared runners). The
  model recommended and used in CI is `tessdata_fast` (D-14, D-17), decided by
  measurement. Gate (a) is reading-order sensitive, because the oracle
  interleaves the columns of two-column pages at 72 dpi (CER about 0.73
  against about 0.02 for ours), so an order-insensitive word-multiset F1
  (`bow_f1`) is reported beside it and not gated; the gates' other known
  weaknesses are recorded as residuals in spec 0002 §2.5 (internal issue #55, B24).
- *Why no `atfork` handler (run 0003 F3).* A fork-deadlock defence would be a
  `pthread_atfork` child handler that resets the locks, and none is safe. A
  `OnceLock` caught mid-initialisation has no reset: the state it guards is
  half built, and replacing the cell is not possible through its API. Releasing
  the pool mutex in the child would hand the child an instance another parent
  thread was using at that instant. And a library that installs process-wide
  `pthread_atfork` handlers into its host is itself a global side effect on
  every later fork in the process, including ones unrelated to OCR. So the
  hazard is documented with three rules (build engines first, do not fork
  while another thread is inside OCR, prefer exec) and the pool's pid check
  handles only the case that can be handled: a child that gets as far as its
  first checkout.
- *The image cap sits in hayro-syntax (D-8).* CCITT, JBIG2 and JPEG 2000 size
  their output from encoded data, not from the declared dimensions, so the cap
  has to be checked where each size first becomes known. What it cannot bound
  (JBIG2 intermediate bitmaps, JPEG 2000 tile records) was a stated residual
  until run 0003 closed it (next bullet).
- *Codec memory is a per-decode budget derived from the pixel cap (run 0003
  D-2).* One existing knob, `max_image_pixels`, already bounds image memory, and
  the per-codec multipliers (2 B/px JBIG2, 64 B/px JPEG 2000) are facts about
  what a legitimate image of each codec needs, not policy; a separate `Limits`
  field would need one default that is wrong for one codec or the other. The
  budget is cumulative and never refunded, charged before each allocation
  whose size the file declares, so that "refused" means "not allocated". The
  cost is stated, not hidden: a legitimate file whose transient allocations
  exceed the budget is refused. The refusal is a distinct
  `ImageMemoryLimit` warning so `ImagePixelLimit` keeps meaning "declared too
  big", the same reason D-8 kept `ImageDecodeFailure` separate. The measured
  margins (JPEG 2000 only 1.5x at four components) are in spec 0003 Appendix A.5.
- *The noise pre-check replicates Tesseract's Otsu rather than calling it (run
  0003 D-4).* Measured mechanism: Tesseract binarises the whole raster with one
  global Otsu threshold, and on a page with little ink the split lands inside
  the noise, producing about a million tiny components and 16-25 s of layout
  analysis on garbage. A half-resolution copy of the same page reads exactly in
  22 ms. The check is pure Rust in front of the backend, not Leptonica, because
  Leptonica in the parent would load native code under `Isolation::Process`
  (O29) and put the check inside the backend. It judges the side Tesseract
  treats as ink, never its class alone, because the first rule (class `None` is
  speckle) skipped real text on pages with large dark regions and let inverted
  noise pages stall (spec revision of Q10). The remedy is one downsample, a
  re-check, then a skip, because a downsample recovers real text where a skip
  would lose it and the re-check turns pure noise into a skip rather than a
  second garbage pass. It is on by default (every page it changes took 16-46 s
  to produce garbage) with a single opt-out boolean; the thresholds come from
  one synthetic corpus, so they are constants rather than a tuning surface.
- *Tesseract variables are refused by asking Tesseract, plus a short
  hand-kept list (run 0003 D-6; ADR 82 and its 2026-10-14 addendum).* A
  process-global parameter set by one instance changes every instance (the
  pinned case reverts `debug_file`, so output returns to stderr); the set of
  such names belongs to Tesseract and changes with versions, so the shim
  answers from `GlobalParams()` instead of a Rust list that would drift.
  Per-instance parameters cannot be found that way, and some still write
  files, read arbitrary paths, enter training, dump or box modes, or change
  which models Init loads (which would undo the language-list check). Those
  come from a hand-kept list, `REFUSED_INSTANCE_PARAMS`, whose scope is
  written down (file-opening or file-derived, training/dump/box modes, model
  selection). It grew from two names to 19 after internal issue #65 (A1-A3) and is
  re-audited at every Tesseract upgrade; other per-instance parameters pass
  through and are residuals, not defects. The addendum extends the list
  rather than switching to an allowlist; the reason for rejecting the
  allowlist is not recorded in the ADR (rationale not recovered from project
  sources).
- *H11 and H12 are targeted (D-19, D-20).* H12 fixes the observed resources
  leak with a fresh context and a property getter that walks parents, rather
  than resetting the parent chain on every lazy lookup (a change that would
  need a full parity re-run). H11 adds `PagesRoot { Ref, Inline }` because an
  inline root is a distinct valid shape, not "no `/Pages`".

**Why `/Count` is ignored and dangling trees open (ADR 22, 25).** The
requirement is an honest count that cannot be lied to. That ruled out `/Count`
as even a capacity hint. Leniency when the tree cannot be built mirrors the
oracle (a catalog without `/Pages` opens with zero pages in PyMuPDF), except the
missing-key case, which hayro rejects before core can intervene and which stays
`Corrupt`.

**Why hostile fixtures never compare against the oracle's text (ADR 16).** The
oracle's output for deeply nested input depends on where the nesting sits and
sometimes raises. Matching it is not a goal; the outcome class, the warning kinds
and our own pinned text are.

**Why the codec panic boundary is narrow (ADR 86, 35, 79).** ADR 35 said the Rust
API catches no panics, so one crafted image failed the whole page, which broke
the requirement (R1) that a crafted image must not. The internal #63 fix pass patched one
panic shape (H16) and concluded that fixing panic sites one at a time never
ends; fuzzing is what finds the reachable ones, and a boundary is what stops
them failing a page in production. So the fork wraps only the call into each
image codec crate (H18, patch 0028). It deliberately does not wrap
`Filter::apply` or the page: those would also swallow panics in xref
resolution, lazy repair and the nested Flate decode of `JBIG2Globals`, which
are parser bugs that must keep propagating, and catching at the page is the
behaviour being removed. A caught panic reuses `image: decode failed`; the
decision record says the user chose that over a new warning kind, since to a
caller it is an image that could not be decoded. `AssertUnwindSafe` is sound
only because the closures touch decode-local state, which is why a new codec
call site must follow the same rule (ARCHITECTURE.md, invariant "Codec panic
boundary"). Two further choices follow from "the library does not own
process-global state". The library never installs a panic hook for Rust
callers: a chained, thread-scoped quiet hook was rejected as seizing global
state for the library's convenience, and catching only when called from Python
was rejected because it gives one Rust API two behaviours. So a caught codec
panic still runs the application's hook, and what that prints is the
application's choice. The Python binding already owns a silent hook (T48); it
records a message for every panic, including a caught codec panic that no
`guarded` frame unwinds, so `guarded` clears the slot on entry rather than let
a later, unrelated `InternalError` carry a stale message. And the catch is
compiled out under `cfg(fuzzing)` instead of switched off at run time: a
runtime switch does nothing useful in production and is easy to leave on, while
a fuzz build must see every codec panic as a crash. The price, accepted in the
ADR, is that production Rust callers no longer see codec bugs as panics; the
warning still records the skipped image and fuzzing still finds the bug.
Tests arm the panic through a feature-gated hook rather than a committed
panicking fixture, which would go stale as soon as the underlying bug is fixed.

**Why decoder time is bounded by guards and an allowance, not by the deadline (ADR 88, 27, 79).**
`page_time_budget` is checked between operators (ADR 27), and an image decode is one
operator, so a codec loop that never ends is out of its reach, and the memory budget
(ADR 79) does not charge a loop that allocates nothing. Fuzzing found two such loops
in the JBIG2 symbol dictionary (internal #94, #116) and one way to spend unbounded CPU on a
finite file (internal #116). The fork's answer is to fix each shape where it is rather than to
add a timer to the codecs. The two loops that end only on what the arithmetic data
decodes to get a forward-progress guard: once the MQ decoder is exhausted it reads
nothing, and a loop that has stopped advancing from then on is refused (H17 for the
export flags, H22 for the height classes). The thresholds differ for a measured
reason: a legitimate two-byte stream decodes three empty height classes from its
padding before its symbol, so the export guard's "two in a row" would refuse it, and
the height-class guard waits for more than 64. The 64 is a heuristic, not a figure
derived from the standard. Rationale for 64 over some other number
was not recovered from project sources beyond "far above three and still ends a spin
at once". The finite-but-expensive shape is different: a text region may place
10,000 instances and each costs its symbol's area, so a large symbol drawn over and
over is bounded in memory and in count yet not in time. H23 charges each placement
(symbol area plus 64) to an allowance of 16 pixels per pixel of `max_image_pixels`,
beside the byte budget and refused through the same `ImageMemoryLimit`, because both
mean the decoder outgrew what the cap allows and one knob raises both. The 16 is "as
many pixels as the byte budget can allocate as one-bit bitmaps", so a decode may draw
each pixel it could hold once; the 64 stands for the placement overhead and keeps an
empty symbol from being free. Two limits are stated, not hidden: halftone pattern
rendering is not charged (no rationale for leaving it out was recorded; it is a known
gap), and every other codec loop is bounded only by its own termination or the memory
budget. The same fix pass chose to clamp an over-long standard-handler key to 16 bytes
(H19, "as pdf.js does") so such files open, rather than reject them; the project
sources record that choice but not why clamping was preferred to a typed open error.
One decision was deliberately not taken: no `catch_unwind` around open. ADR 35
stands, and the parser panics fuzzing finds (H19, H20) are fixed at their sites,
as the narrow codec boundary above already assumes.

**Why H24 refuses a document's `bcmap` stream instead of vendoring `hayro-cmap` (ADR 90).**
The last fuzzing finding (internal #121) sits in a crate that is not vendored: `hayro-cmap`
parses any CMap data that starts with `bcmap` as the binary format of its own embedded
CMaps, and that parser rebuilds ranges by adding Huffman-decoded deltas without
checking the arithmetic. A `/ToUnicode` or Type 0 `/Encoding` stream reached it
straight from the document. The fork's answer is a prefix test in the vendored
interpreter at the two places a document's CMap is parsed: such a stream is treated
like a CMap that fails to parse. The format is private to `hayro-cmap` and
legitimately comes only from its own bundle, which `usecmap` and predefined names still
reach, so what is refused is a stream no other PDF reader is known to parse. Vendoring the crate
would have fixed the cause, but at the price of a sixth crate, a third pristine tag, a
patch export generalised from two ranges to N, and changes to the notices and licence
checks, and it would only have been worth that if a hostile binary stream had to be
accepted. The costs that remain are stated in the ADR: the `cmap` fuzz target no
longer feeds hostile input to the binary parser, its embedded-CMap seeds went dead
(since replaced by hand-written text CMaps, internal #147), and the gate is a prefix test to
revisit if the upstream format or its checks change.

**Why loom, and what it found (ADR 40 addendum, design D5).** The close
protocol and the OCR pool's checkout/return are the two places where this
library's own concurrency protocol (not hayro's) could be wrong. Stress tests
sample interleavings; the spec says "every interleaving", so loom was chosen
over randomised checking (shuttle), and a test-only replica was rejected
because it checks a copy rather than the shipped code. That drove the one
structural change to production code: the close state moved out of `DocShared`
into the generic `CloseCell<T>`, so the model runs the real code with a counter
in place of a hayro document (which would drag hayro's std locks into the
model). Both crates take their lock types from a private `sync` shim. Only
the cell and the pool's lock and condvar are shimmed because loom types panic
outside a model run and loom forbids statics of them; shimming more would make
the build unusable, shimming less would check a copy. The pool takes its pid
source as a field so a test can stage a fork. The first run of the pool model
found a real lost wake-up: a fork reset performed by a returning pre-fork slot
freed every slot without notifying the condvar, so a caller already waiting on
the full pool slept forever. The reset now calls `notify_all`, and the quiet
return in `Slot::drop` is only correct because the reset wakes the waiters. The
`Pages` cursor and the lazy-repair flag stay on `std`: single atomic
operations, no protocol to check. Rationale not recovered from project sources
for the choice of `LOOM_MAX_PREEMPTIONS=3`.

**Why hardening is cost-capped and mostly local (ADR 87, 85).** Phase 3 needs
fuzzing, sanitizers and loom, with 72 clean hours of fuzzing as the original exit (waived, ADR 93). The
repository is internal, so Actions minutes are billed and the owner capped
recurring runner cost tightly. Seventy-two hours across five targets in CI
would be 360 runner-hours; a nightly sanitizer schedule would be a standing bill
for results that change only when the code does; a cache-based ledger can be
evicted silently and lose the count. So long fuzzing runs on the developer's
machine, resumably, and is counted in a committed ledger, and the workflow in
CI is dispatch-only, with a time-capped smoke run and the two sanitizer runs.
`ci.yml` depends on none of it (ADR 85 is unchanged). The consequences are
stated plainly in the ADR: fuzzing depth depends on someone running the script,
the ledger is a progress log and not an attestation, and CI catches only
regressions of committed reproducers and shallow crashes. Fixed findings
stay fixed because their minimised input is committed and replayed in the
ordinary suite on a 512 KiB stack. Rationale not recovered from project
sources for the figure of 72 hours itself (it was the design specification's
exit criterion).

**Why fuzzing depth stopped being a release gate (ADR 93).** The 72 hours
were never run. Five targets make 360 machine-hours, which is impractical on a
developer machine, and the cost cap rules out CI. When the rest of Phase 3 was
done, every target's ledger total was zero, and the owner decided GA would not
wait. The trade is availability risk, not memory-safety risk. The engine is
Rust apart from Tesseract and Leptonica, sanitizer and loom runs check the
unsafe and concurrent code on the test suite, panics are caught at the Python
boundary, and from Python, by default, Tesseract aborts are contained by
process isolation. What fuzzing would still find are inputs that
overflow the stack, exhaust memory or hang a page. These kill or stall the
process, and nothing in the library can catch them. So the containment moved to
the caller: a process it can afford to lose, a wall-clock limit per document
enforced by killing that process, and bounded retries with quarantine (design
spec §8.5). It ships in the consumer's cut-over release, whose production
traffic then serves as the real-world fuzzing (the rollout runs no production
shadow; design spec §15.3). The ledger keeps counting, now against a "depth
goal", so hours from any later campaign (internal #167) are still recorded.
A library-side `extract_isolated()` helper is possible but was not taken up
in this decision. Fuzzing lives in its own workspace on a pinned dated nightly,
because libFuzzer, `-Zsanitizer` and `-Zbuild-std` need nightly while the
product is pinned to stable; a workspace member would drag nightly-only
dependencies into stable CI and cargo-deny. For the same reason the fork's
`[patch.crates-io]` table is copied there and a CI check fails if the two
drift, so fuzzing always runs the forked code. The limits fuzzing runs under
are tightened but allow documents of up to 1,000 pages to open, with the text
target sampling the first six pages and the last: a long page tree is itself
something to fuzz, and sampling keeps the worst case (seven pages at the 2 s
budget) under libFuzzer's timeout. The sanitizers instrument the vendored
Leptonica, Tesseract and the shim as well as the Rust, because the FFI path is
the part the sanitizer requirement most wants covered; on Linux that means
clang, since gcc's sanitizer runtime is not the LLVM one rustc links. Findings
from either kind of run become issues and go through the ordinary issue workflow; our own
crates are never suppressed. The first findings are filed as internal #119–#121.

**Why every wheel comes from one script and is checked twice (ADR 91).**
Until Phase 3 no wheel had ever been built: CI used `maturin develop`, and a
wheel built without first running the helper-staging script quietly lacked
the OCR helper, which maturin can neither build nor audit. Making process
isolation the Python default (internal #53, ADR 92) turned that quiet gap into a hard
failure for every OCR user, so the build had to make the helper impossible to
forget, and CI had to prove it shipped. Hence one script, `build-wheel.sh`,
that stages the helper before maturin runs, used identically by the per-push
job, the release matrix and a developer, and a verifier that opens the wheel
and checks the helper the way auditwheel checks the extension (auditwheel
never looks at an included file, so a helper linked against a newer glibc
would otherwise ship). Linux builds run inside the manylinux image rather
than cross-compiling, because building the vendored C++ through cmake for
another architecture was the riskiest step and native arm runners remove it.
The SBOM was meant to come from maturin's built-in generator; inspecting a
real wheel showed it resolves every target (loom and other platforms' crates
appear as shipped) and embeds the build machine's paths, so a small
generator over `cargo tree` for the wheel's own target replaced it. abi3 is an
opt-in feature rather than always on because free-threaded CPython has no
stable ABI and development builds should keep the full API; the per-push job
runs the whole suite against the abi3 wheel so the two cannot drift. Only one
wheel is built per push and the full matrix waits for a tag or a dispatch,
for the same cost reason as hardening (ADR 87). Releases go to GitHub, not a
package index, because the package name is still undecided (design spec
§17.2 Q1).

Not recovered from project sources: why the 256 object-nesting constant and the
256 page-tree depth were chosen beyond "exceeds any real tree" (ADR 24); why
`max_stream_decoded_bytes` is 256 MiB and `max_file_size` 512 MiB; why the MIT
license was chosen at first (ADR 8); any alternative to the structured
page-block-line-char model (ADR 4).

## Invariants, and why they hold

The list is in ARCHITECTURE.md. The reasoning behind the ones that are easy to
break:

- **Lock scope and the close protocol.** An operation clones an `Arc<Loaded>`
  under a read lock and works lock-free; `close()` takes the write lock
  briefly to swap the state to `None` and drops the document outside the lock.
  That keeps close from waiting on running extractions and extraction from waiting on close,
  and it makes "memory freed when the last operation drops" fall out of
  reference counting rather than a protocol (design spec 8.2, ADR 40). Holding
  the lock across interpretation would reintroduce both blockings; storing a
  hayro `Page` or cache would pin lifetimes that the public API deliberately
  hides (`Page` is `(Arc<DocShared>, index)`), and the interpreter cache is not
  `Send` in any case. The protocol is model-checked by loom against the shipped
  code, not a replica (see "Why loom, and what it found"). `page_count` is copied out at open so it stays readable
  after close. Free-threaded Python adds one more rule: the test-kit builder's
  mutex is taken only while detached, so no Python code ever runs under it.
- **Panics.** The Rust API does not catch panics (ADR 35), because hiding them
  from Rust users would mask bugs; the Python layer must, because
  `pyo3.PanicException` is a `BaseException` and would escape ordinary handlers.
  Every guarded body therefore needs to be guarded, and `panic = "unwind"` is a
  correctness setting, not a tuning knob. The one exception is the image
  codecs (ADR 86): their panic sites are reachable from input and are found
  faster than they can be fixed, so the fork catches a panic around each codec
  call and skips the image, rather than fail the page. Fuzzing builds compile
  that catch out, so the bugs stay visible where they are hunted. The decision
  and its scope are told in "Why the codec panic boundary is narrow" below;
  its enumerated invariant is in ARCHITECTURE.md.
- **Lazy xref repair.** hayro repairs a broken cross-reference table the first
  time an object lookup fails, which can happen at any point on any thread. The
  original implementation could not tolerate two threads in that region
  (`try_*().unwrap()`, an `assert!` that repair had not happened, a full-file
  scan under the write lock). Forcing repair eagerly at open would not have
  helped, because core cannot prove a lookup never triggers one later; so the
  fix is in the lock discipline (ADR 30). The consequences are visible: the
  warning for a repair is a property of the document, not of the page that
  happened to trigger it (page output must not depend on a race), and a
  concurrent repair may run the scan several times.
- **Thread-local signal windows** (above) and the lexer rule that a failed
  operand ends the stream: resynchronising at every following byte would be
  quadratic and would change what H2a pins.
- **The device never runs Type 3 procedures, patterns or soft masks.** The text
  path asks "what text, where", not "what pixels"; the nesting-cap fix for Type
  3 (ADR 29) matters to the OCR run's renderer, not to this path. The
  corresponding divergences (tiling-pattern text) are accepted.
- **Deadline, not operator cap, bounds time** (ADR 27). hayro's speed varies by
  more than an order of magnitude between debug and release, so no
  operator-count claim translates to seconds. Tests therefore assert op-cap
  behaviour with small caps and bound time only through `page_time_budget`.

## Where the bodies are buried

- **The residuals are a feature.** ARCHITECTURE.md's list of parity residuals is
  the project's honest claim ("no universal claim is made over the space of all
  PDFs", spec §2.5). The largest is right-to-left text, where the oracle
  reorders runs into logical order and we emit content order (corpus-B
  similarity 0.14-0.77 on those pages). Handling it was not
  attempted in this run.
- **Anisotropic scaling cannot be tuned away.** The oracle's font size under
  unequal x/y scale is the geometric mean; ours is the em height across the
  baseline. A gap of 0.799 em by its measure is 0.818 em by ours, and no value
  inside the `gap_break` bracket reaches 0.818. The specification records that tuning cannot
  close it.
- **Brackets are narrow.** `space_gap`'s is 0.0023 em wide. A small change to
  glyph advance computation, to AFM widths or to f32 handling can move rows
  across a boundary; the committed fixtures are placed off the boundaries, so
  they will not tell you, and the position sweeps in `tests/text_layout.rs` will.
- **Several rules are only partly probed.** What the oracle does with an ignored
  ToUnicode entry in a font that has an embedded program or a known collection,
  at a KP15 position, or with multi-code-point entries holding C1 controls, is
  unknown, and the code takes the conservative path (raw CID for composite fonts,
  glyph name for simple ones).
- **Q7 and Q8 were closed by run 0002** (H11: a direct-dictionary `/Pages` now
  opens; H12: a form's `/Resources` that is the page's own object, and named
  properties through shared or inherited resources, resolve). Older text that
  calls a direct `/Pages` `Corrupt` is out of date.
- **OCR residuals** (fork deadlock, the codec memory budget's uncounted
  allocations and refused-legitimate-file case, the noise pre-check's
  approximations, late timeouts) are listed in `ARCHITECTURE.md`; they are accepted
  limits, not open bugs.
- **Unguarded module init**, TOCTOU at open, allocation-failure aborts and
  N-fold concurrent repair scans are known and accepted (spec §2.4).

## Making common changes

**Adjust a layout rule.** Probe the oracle, record the evidence in the spec's
Appendix A, derive the bracket under the strict comparison, pick a value strictly
inside it, update `LayoutParams` and its docs, add fixtures (exact rows off the
boundary, a position sweep for rows near it), regenerate expectations with
`tools/oracle/gen.py`, and re-run gates C and B. If the change is user-visible,
it is a specification revision and an ADR (supersede, do not edit).

**Change glyph→Unicode behaviour.** The chain's order is fixed by T26; a new rule
usually lands as a step inside `text/unicode.rs` plus a corpus-C recipe. If the
rule needs information hayro does not expose (where a glyph name came from, a
font's embedded program kind), it is a fork change and follows the next recipe.

**Add or change a hayro behaviour.** Edit `vendor/hayro-*` in its own commit, run
`tools/vendor/export-patches.sh`, commit the patches, and add a test in
`crates/core/tests/` that goes through hayro's API. Decide how core will see the
result: a field on `InterpretStats` if it crosses nested contexts, a thread-local
if it happens where no context exists, a `Device` hook if it is a boundary.
Never introduce a process-wide knob, a global lock or a thread pool; the
concurrency requirement forbids them and the fork-safety story depends on it.

**Add a bound.** Choose the enforcement point that sees the data, define the
outcome (skip plus warning, hard stream failure, or typed error), map it in core
and in the Python layer (a `LimitKind` is also a Python `limit_name`), write a
hostile generator in the test kit, and gate it on outcome and warning kind rather
than on the oracle's text.

**Change a concurrency protocol (close cell, pool).** Keep the code inside
`CloseCell` or `Pool` and route its lock types through the `sync` shim; run the
loom tests as in ARCHITECTURE.md before and after, and add a scenario for the new
interleaving. Anything a `static` or code outside the model touches stays on `std`.

**Fix a fuzz finding.** File an issue; fix it through the ordinary issue workflow; commit the
minimised input to `fuzz/regressions/<target>/` with the fix so the ordinary
suite replays it; reset the target's ledger entry if the fix changed its code path.

**Record a decision.** Create a file in `docs/adr/` with `adr_index.py --next`,
fill context, decision, alternatives and consequences from what was actually
decided, regenerate the index, and cite it by id. A decision that changes a prior
one supersedes it: the old record changes only its status and `superseded_by`.
