# Run 0001: the text path (design-spec Phases 0 + 1)

**Status:** signed off 2026-10-01 (Q1: MIT; Q2–Q6: defaults); revised 2026-10-01 after unit 03 and re-signed off the same day (see Revision log); 2026-10-02 (8): user decision — PyO3 0.29, CPython 3.13t dropped from this run; 2026-10-02 (9): user decision — our own code relicensed GPL-3.0-or-later (unit 12b), dependencies stay permissive-only · **Date:** 2026-10-01 · **Lands on:** `main`
**Inputs:** `docs/design-spec.md` (cited as §n), the internal survey of the first consumer's PyMuPDF usage, kept in the internal archive (cited as internal survey §n)

This document is the specification for one spec-first implementation run. It
fixes the observable behaviour, the interfaces, the verification oracle and
the work-unit stack for everything the design spec calls Phase 0 and Phase 1:
opening a PDF, counting and iterating pages, and extracting each page's text
layer in PyMuPDF's `'text'` shape, from Rust and from Python, safely from many
threads, on hostile input. OCR, rendering, fuzzing, sanitizers, `loom`, wheels
and the SBOM are later runs.

Everything here was checked against the real crate sources (hayro-syntax
0.7.2, hayro-interpret 0.7.0, hayro-postscript 0.1.0, hayro 0.7.1) and against
PyMuPDF 1.28.2 run as a black box. Where the design spec was wrong or silent,
this document says so and decides. Appendix A records the probe evidence;
Appendix B records the hayro facts that differ from the design spec.

---

## Revision log

**2026-10-01 (after unit 03).** Unit 03's generators were run through the
oracle and contradicted several claims; each was re-probed as a black box
(Appendix A, items marked **Revised 2026-10-01**). Changes:

- **KP11 flipped:** hidden optional content is **excluded** by the oracle for
  documents read from disk (the earlier probe used an unsaved in-memory
  document, which the oracle treats differently). H8a and the
  `optional_content` option are removed; hayro's default already matches.
  *User-visible change; needs re-sign-off.*
- **KP7 flipped back to the design spec:** the visible box is
  CropBox ∩ MediaBox (the earlier probe placed text in PyMuPDF's crop-relative
  coordinates). Per-glyph intersection test unchanged. *User-visible change;
  needs re-sign-off.*
- `max_page_tree_depth` default 64 → **256**; the oracle has no depth cap (a
  20,000-deep tree extracts), so this is a pure robustness limit with a
  documented divergence beyond it (KP23). *Default change; needs re-sign-off.*
- Hostile fixtures are gated on outcome class and warning kind only; the
  oracle's text for deeply nested input depends on where the nesting sits
  (and it raises for some placements). §4.4 and Appendix A.5 corrected.
- Nested Form XObjects: the oracle extracts 60 levels; hayro stops at 50
  (KP21 residual). Fan-out op count corrected to ≈25.2M for unit 03's
  generator.
- Non-embedded CID fonts without a `/FontDescriptor` make the oracle emit
  raw bytes (KP22 residual); corpus-C CID fixtures must carry one.
- §4.3 AFM check reformulated (`get_text_length` returns 278 units for the
  WinAnsi 0x80–0x9F block, which is not what the oracle renders).
- Encrypted fixtures are not byte-reproducible (random `/ID`); provenance
  records the committed hash. Contract-input generators are assigned to unit
  08. `PageBuilder::set_form_ocg` added to the test-kit API.

**2026-10-01 (2) (units 04 and 05 started).** Layering fix and two signal
designs; items marked **Revised 2026-10-01 (2)**. No user-visible change.

- The hayro-level units (04–07) now assert only what hayro itself exposes;
  every `WarningKind`, `page_count` and extracted-text assertion moved to the
  unit 08–10 oracles. Unit 04 depends on 03 only, as the unit table says.
- H2a signal: an over-deep read returning `None` is indistinguishable from any
  parse failure, and content operands are read through a bare `Reader` with
  no context, so the cap reports through a thread-local hit counter in
  hayro-syntax (`object_depth_limit_hits()`), sampled by core around each
  synchronous operation (D-12 amended). The Type 4 procedure parser that
  recurses is hayro-interpret's own (`function/type4.rs`), not
  hayro-postscript's; the H2a row is corrected, and hayro-postscript (used by
  hayro-cmap for CMap parsing) gets a cap with `Error::LimitCheck`.
- H3 signal: `Context::stats()` returns `InterpretStats` (ops, exhausted,
  nesting-cap hits, form-cycle hits) from the shared budget state, which is
  always allocated; core maps the hit counters to warnings (D-29).
- A deep content operand ends the content stream (`UntypedIter::next` returns
  `None`), so `After` is lost; pinned.
- H1 measured: `Pdf::new` on a 100k-page tree retains ~33 B and peaks at
  ~49 B per xref entry (hayro's `XrefMap`); the memory bullet is restated per
  xref entry. H1 makes a dangling `/Pages` reference open with no pages
  instead of failing; core keeps the T5 outcome by resolving `pages_root()`
  itself (D-31). `OnceLock<CachedPages>`, infallible `Page::new`, a fresh
  `ReaderContext` in `Page::from_dict` and `pub(crate) is_repaired` confirmed.

**2026-10-01 (3) (unit 06 findings).** Items marked **Revised 2026-10-01 (3)**.

- The "fan-out fails within 5 s on CI" claim is withdrawn: hayro interprets
  about 0.9M operators/s in release and 60k/s in debug, so 10M operators
  take ~11 s / ~165 s. The time guarantee is the deadline (`page_time_budget`,
  checked every 4,096 operators), not the operator cap; §2.3 item 3 is
  restated accordingly (a change to a stated claim, not to any default or
  API). Op-count tests use small caps; the vendored and font crates get
  `opt-level = 2` in the dev profile. Defaults (10M ops, 30 s) unchanged.
- R1 gap closed: Type 3 glyphs, tiling patterns and soft masks carried a
  nesting depth but never checked the 50-level cap, so a self-referential
  Type 3 font overflowed a 512 KiB stack when a device runs glyph procedures
  (as the renderer does). H3 adds the check to all three. The text device of
  unit 09 never runs Type 3 procedures (§6.1), so the text path was not
  exposed; the fix matters for the OCR run.
- `InterpretStats.exhausted` is `Option<BudgetExhaustion { Ops, Deadline }>`
  so core can distinguish `OpsPerPage` from `PageTimeBudget`.
- `nested_forms` labels are `L1`…`L60`; the cap yields `L1`…`L50`.

**2026-10-01 (4) (clarifications adjudicated by the unit 04/05/06 reviews).**
Items marked **Revised 2026-10-01 (4)**; factual only, no behaviour change:
the Type 4 fixture still fills (DeviceGray fallback); H2b's scope, defaults
and three new residuals; debug-build stack needs for the 256-deep cases until
unit 06's dev-profile `opt-level` lands (and `pdf-extract-core` at
`opt-level = 1`); the nesting cap refuses at `> 50`, `ops` counts the refused
operator, the reason lives in an `AtomicU8`; further H1 cases that now open
with 0 pages; `xref_was_repaired()` is re-sampled after the index walk.

**2026-10-01 (5) (unit 08 in review; owed fixes from units 07/08).** Items
marked **Revised 2026-10-01 (5)**.

- **Blocking finding for C2 (shared `Document`):** hayro's lazy xref repair
  is not thread-safe — `XRef::get_with` does `try_read().unwrap()`, `repair()`
  does `try_write().unwrap()` + `assert!(!repaired)`, and the whole-file scan
  runs under the write lock. New fork change **H9** (new unit 08b, before 09):
  blocking locks, scan outside the lock, idempotent re-check, no panics; plus
  `Pdf::is_encrypted()` so unit 08 drops its second `Pdf::new` with a bogus
  password. Core-level concurrency tests that would have panicked land in
  units 08b (hayro level) and 10 (shared document).
- Unit 08's interpretations accepted and written into T2, T3, T7, T9, T13,
  T15 and the `Warning` type (coalescing, `count`, `"<category>: <detail>"`).
  Non-regular paths (FIFO, directory, device) are rejected at open.
- Owed text fixes: H4 takes `Option<&Dict<'_>>`; H6b applies to embedded
  programs only; the §4.7 H6 asset is fontations' `tinos_subset.ttf`.
- Unit 09 notes folded in: positive `/Descent` is invalid → defaults; KP15
  normalisation is core's job (`glyph_name()` is raw); hayro calls
  `draw_glyph` twice for Tr 2 and 6 (Fill then Stroke) → one record; H6b's
  reverse-cmap build is unbudgeted (§2.4); skrifa ignores (3,0) symbolic and
  Macintosh cmaps (§2.5).
- *User-visible (minor, consistent with the spec's intent):* `from_bytes` /
  `open_bytes` enforce `max_file_size`; the `%PDF-` header must lie wholly
  within the first 1024 bytes; non-regular files raise `OpenError`; repeated
  warnings are coalesced with a count in their text.

**2026-10-01 (6) (unit 09 implemented; findings adjudicated).** Items marked
**Revised 2026-10-01 (6)**.

- User decision: the ~255 TeX glyph names added to `crates/core/src/text/agl.rs`
  are dropped (their source list is GPL-2 lcdf-typetools); the table is Adobe
  AGL single-code-point entries only (BSD-3 notice kept). Core's chain already
  bypasses hayro's own glyph-name table for simple fonts without ToUnicode;
  with ToUnicode present, hayro's `as_unicode()` may still fall back to its
  table, so H10 adds a ToUnicode-only lookup. TeX names are a §2.5 residual.
- New fork unit **09b / H10** (hayro-interpret + hayro-syntax, ~400 lines):
  content-lexer resync with a damage counter (T35), auto-close of unbalanced
  marked content at the end of each content stream (T28), per-context cache of
  named property lists, `Device` form/annotation hooks (T32), font-load-failure
  and stream-decode-limit signals routed through `InterpretStats` and a
  thread-local, a cooperative cancel flag (`BudgetExhaustion::Cancelled`) so the
  char cap stops interpretation, and `Glyph::to_unicode()`.
- Sanitising applies to **all** emitted text, not only unmapped codes (D-10
  amended; T33 reworded: LF/CR/TAB from any source are kept, as the oracle
  emits them).
- Char cap vs budget: the cap cancels interpretation and the partial page is
  returned; a budget that ran out before the cap stays `Err(Limit)`.
- KP15 and KP22 reworded to the re-probed oracle behaviour; ActualText
  decoding details recorded (T28); `CharRecord` has no `font`/`seq` fields,
  ActualText grows the span record's advance, `Skip` emits no record (T29/T30
  note).
- *User-visible:* sanitising of mapped text (NUL/C0 → U+FFFD) — a tightening
  of an already documented deliberate difference; nothing else.
- **(6b)** T37 corrected to hayro's and the oracle's behaviour (failed fonts are substituted, text kept, counted); KP6 fixture uses `/whitesquare`, `/square` recorded as a residual (**Revised 2026-10-01 (6b)**).

**2026-10-02 (7) (unit 10 implemented; rulings).** Items marked
**Revised 2026-10-02 (7)**.

- `SPACE_GAP` default 0.15 → **0.149**. Appendix A.1's own data, under the
  strict `along > SPACE_GAP · em` rule, brackets the threshold at
  **[0.1477, 0.15)**: the 22 pt join at 3.25 pt (0.1477 em) is the floor and
  the 8→20 pt space at exactly 3 pt (0.1500 em) excludes 0.15 itself. The
  earlier "(0.148, 0.159]" was wrong on both ends. 0.149 sits strictly inside,
  so no corpus-C row lies on a boundary. (Equivalently the oracle may use
  `>=` with 0.15; strict `>` with 0.149 reproduces every data point without
  float fragility.) Rust `LayoutParams` default change; not Python-visible.
- `BlockSeparator::BlankLine` differs from `Newline` by exactly one `'\n'`
  per interior block boundary; the shape invariant is restated.
- `Document::warnings()` also reports a lazy xref repair discovered during a
  later operation (one `XrefRepaired`, appended once via a `DocShared` flag);
  `TextPage` warnings never carry it. T8 and §3.3 updated.
- The pre-H9 abort regression lives only in hayro-level `h9_pre_fix.rs`; the
  core-level C2 test asserts correctness and absence of panics.
- Unicode whitespace in the space-insertion rule (`char::is_whitespace`,
  incl. U+00A0, U+0085, U+2028) is unprobed → unit-13 probe item KP24.

**2026-10-02 (7b) (unit 10: second threshold boundary).** Items marked
**Revised 2026-10-02 (7b)**. The single `LINE_BREAK = 0.8 em` cannot
reproduce Appendix A.1: the perpendicular rows need `≥ 0.8` (11 pt raised
8.8 pt = 0.800 em joins) while the along rows need `< 0.8` (8→20 pt forward
gap 16 pt = 0.800 em breaks; 11 pt 8.8 pt breaks). Recomputed from every A.1
row under the strict `>` rule: **PERP_BREAK ∈ [0.8000, 0.8182)**,
**GAP_BREAK ∈ [0.7955, 0.8000)**, **BACK_BREAK ∈ [0.7727, 0.8182)**.
`LayoutParams.line_break` is replaced by `perp_break = 0.809`,
`gap_break = 0.798`, `back_break = 0.795`, each strictly inside its bracket;
every data point is then reproduced at every page position. Public Rust
struct change (user-visible); not Python-visible. T29, KP2/KP3, D-9, §3.3,
§4.10 (position sweeps for every row within 0.01 em of a threshold), §2.5
and Appendix C updated.

**2026-10-02 (8) (user decision: PyO3 0.29, no CPython 3.13t).** Items
marked **Revised 2026-10-02 (8)**. `pyo3-ffi 0.29`'s `build.rs` sets
`MIN_FREE_THREADED_VERSION = 3.14` (verified), so 0.29 cannot build for
3.13t; staying on 0.28.3 would carry RUSTSEC-2026-0176 (list/tuple iterator
`nth`/`nth_back` out-of-bounds read) and RUSTSEC-2026-0177
(`PyCFunction::new_closure` missing `Sync`), both fixed only in 0.29, and the
user chose no advisory exceptions. Free-threaded support therefore starts at
**3.14t**; GIL builds 3.11–3.14 are unaffected. This narrows design-spec C3
("3.13t and later") — recorded as a deliberate deviation (D-24, Q3). Follow-up:
revisit 3.13t only if a consumer needs it.

**2026-10-02 (9) (user decision: relicense our own code GPL-3.0-or-later).**
Items marked **Revised 2026-10-02 (9)**. The workspace crates, the Python
package and `LICENSE` move from MIT (Q1) to **GPL-3.0-or-later**; the shipped
**dependency** graph stays permissive-only (L1 unchanged: GPL/LGPL/AGPL
remain banned for third-party code, and cargo-deny's dependency allow-list is
unchanged). Delivered by new unit **12b** `feat/12b-relicense-gpl3` after 12
and before 13; internal PRs #1–#13 stay as merged, so history carries MIT until 12b
lands. Everything we ship alongside our code is GPLv3-compatible: hayro
(Apache-2.0 OR MIT), PyO3 (Apache-2.0 OR MIT), the Adobe AFM/AGL data
(BSD-3-style notice), Tinos (Apache-2.0). cargo-deny: our crates are
`publish = false` and `[licenses.private] ignore = true` keeps them out of the
license check, so a GPL **dependency** is still rejected (proved by the 12b
oracle). D-40.

**2026-10-02 (10) (unit 13: corpus C assembled, 202 fixtures).** Items marked
**Revised 2026-10-02 (10)**.

- The oracle compares with `>=` at exactly 0.15 em and 0.8 em and its own f32
  glyph placement decides hair-under cases (fresh probe: gap 3.0 pt → space,
  2.9999962 pt → join; 16.0 pt → break, 15.99999 pt → space). Our strict `>`
  with in-bracket defaults reproduces every exact-boundary row and is
  position-independent; only noise-level gaps differ. The three position
  sweeps leave corpus C (they test the oracle's floating point, not a rule)
  and stay as our own §4.10 tests; §2.5 residual restated.
- KP24 probed: the oracle suppresses the synthetic space only for an incoming
  U+0020, or a line ending in U+0020, U+1680, U+3000 or U+FEFF; nothing else
  (TAB, LF, U+0085, U+00A0, U+2000–U+200A, U+2028/9, U+202F, U+205F) counts.
  **Recommendation (b): adopt the oracle's two sets in T30** (small change,
  user-visible parity improvement; signed off 2026-10-02).
- The oracle ignores a ToUnicode entry mapping to U+0085 (glyph-name fallback)
  but honours U+00A0 and U+2028: residual now, C1-range probe in unit 14
  (done in 10b, see below).
- `kp15_type3_nbspace` is exact (oracle matches T26); `kp09_gap` recipe note;
  hostile rows keep text-exact + warning-kind gating (pinned text, D-27
  clarified); lazy repair adds `XrefRepaired` to `deep_obj_20k` and
  `deep_resources_20k`; Appendix C gains `kp05_trailing_space_then_gap` and
  the KP24 fixtures; no parked CI jobs remain (`.github/actionlint.yaml`
  removed); the oracle sniffs content (a valid PDF named `.txt` opens).

**2026-10-02 (10b) (unit 13: ToUnicode entries that map to control
characters).** Items marked **Revised 2026-10-02 (10b)**. Probed black-box
over every C0 code point, U+007F and every C1 code point, for a Type 1
Helvetica, a non-embedded TrueType and a Type 0 Identity-H font, plus
multi-code-point entries: a **single-code-point** ToUnicode entry whose target
is U+0008–U+000D (BS, TAB, LF, VT, FF, CR) is emitted as **U+0020**; one whose
target is any other control (U+0000–U+0007, U+000E–U+001F, U+007F,
U+0080–U+009F) is **ignored** and the glyph name decides; U+00A0 and U+2028
are honoured; **multi-code-point** entries are emitted verbatim, controls
included (`<0041 000A 0042>` → `A\nB`). Identical for all three font types.
**Recommendation (b), adopted:** T26 step (1) applies these rules; D-10's
sanitising still applies afterwards (so a multi-code-point NUL becomes U+FFFD,
a multi-code-point LF stays). A single-code-point mapping can therefore no
longer emit LF, TAB or any control. User-visible parity improvement; the 12
`kp24_w_{0009,000A,000B,000C,000D,0085}_*` rows become exact. Unit 14's C1
probe is folded in here and dropped from its scope.

**2026-10-02 (11) (consolidation before the docs/ADR unit; no new
decisions).** Items marked **Revised 2026-10-02 (11)**, each checked against
the code in unit 14's tree: stale text corrected (cargo-deny ≥ 0.18 has no
`copyleft`/`unlicensed` keys; H10 is patches 0011–0019; the manifest's
`hostile` gate and `warnings` key; the §2.5 failed-font bullet; the widget
bullet — the oracle synthesises check-box and signature appearances); shipped
API extras recorded (`TextPage: Clone + Debug + PartialEq`, `Line::text()`,
`WarningKind::category()`, `LimitKind::name()`, `PdfBuilder::page_mut`,
`__repr__`, `__length_hint__`, `_panic_for_test(detached=)`, `@final` stubs,
`Limits` range validation, `limit_name` values, time budget in ms,
`Document::warnings()` dedup); unit-13/14 findings and residuals folded into
§2.5 (controls-in-ToUnicode update incl. Type 3 probed and composite fonts →
raw CID, right-to-left scripts, tiling-pattern text, anisotropic scaling,
combining marks with kerns, ToUnicode→U+FFFD, missing-glyph advance), §2.4
(H9 concurrent scans, cross-kind cycles, unguarded module init) and T28/T32/T35
(ActualText byte budget and nesting cap, UTF-8 BOM, marker cap, resync
limits); Appendix C lists every committed corpus-C family; Gate B results and
headroom recorded; two real open questions added (Q7, Q8).

---

## Contents

1. [Behavioural requirements](#1-behavioural-requirements)
2. [Non-goals and bounded claims](#2-non-goals-and-bounded-claims)
3. [Interface changes](#3-interface-changes)
4. [Verification oracle](#4-verification-oracle)
5. [Work units](#5-work-units)
6. [Decisions record](#6-decisions-record)
7. [Open questions for the user](#7-open-questions-for-the-user)
- [Appendix A: Oracle probe evidence](#appendix-a-oracle-probe-evidence)
- [Appendix B: hayro facts verified against source](#appendix-b-hayro-facts-verified-against-source)
- [Appendix C: Fixture catalogue](#appendix-c-fixture-catalogue)

---

## 1. Behavioural requirements

Requirement IDs from the design spec are cited in brackets. New requirements
introduced by this run carry the prefix **T** (text run). Compat points are
**KP**; this run adds KP14–KP20.

### 1.1 Opening documents [F1, F2, F3, F4, R3, L1]

| ID | Requirement |
|---|---|
| T1 | `Document::open(path, &opts)` reads the whole file into memory and closes the descriptor before returning. The size is checked with metadata (`fstat`) before reading; a file over `max_file_size` fails with `OpenError::TooLarge` without being read. [§5.1] |
| T2 | `Document::from_bytes(Vec<u8>, &opts)` opens from memory. The Python `open_bytes` copies the buffer once; no Python object is referenced after return. **Revised 2026-10-01 (5)**: `from_bytes`/`from_shared` enforce `max_file_size` too (`TooLarge`), so a buffer is never parsed if the same bytes on disk would have been refused. [§5.1] |
| T3 | File-type detection is content-only: zero bytes → `Empty`; no `%PDF-` lying **wholly** within the first 1024 bytes (**Revised 2026-10-01 (5)**: a header straddling byte 1024 does not count) → `NotPdf`; otherwise hand to the parser. The name and extension are never consulted. [F2, §5.2] |
| T4 | Parser outcomes map as follows (verified against hayro, Appendix B): `LoadPdfError::Invalid` → `Corrupt`; `Decryption(PasswordProtected)` → `PasswordRequired`; `Decryption(MissingIDEntry \| InvalidEncryption \| UnsupportedAlgorithm)` → `UnsupportedEncryption`. hayro tolerates a missing `/ID` by assuming an empty one, so such a file opens if it decrypts; §5.5's "missing `/ID` → UnsupportedEncryptionError" holds only when hayro reports it. |
| T5 | A `/Pages` tree that yields no leaves, followed by a brute-force scan that finds no `/Type /Page` dictionaries, opens as a document with `page_count == 0` (oracle: empty `/Kids` → 0 pages). A catalog **without** `/Pages` is `Corrupt`: hayro's `XRef::new` requires the `/Pages` reference and rejects the file (Appendix A.7), while the oracle opens it with 0 pages — a documented divergence pinned by the `catalog_no_pages` fixture. **Revised 2026-10-01 (2)**: a `/Pages` reference that is present but does not resolve opens (H1 no longer fails `Pdf::new` for it); core's walk falls back to the brute-force scan and, if that finds nothing, yields `page_count == 0` with warnings `PageTreeNodeUnreadable` and `PageTreeBruteForce` (D-31). `%PDF-` + garbage is `Corrupt` for both. |
| T6 | Owner-password-only documents (RC4 40/128, AES-128, AES-256 R6) open and extract transparently; `is_encrypted` is true. User-password documents raise `PasswordRequired` **at open**. Verified against hayro for all eight fixtures (Appendix A.7). [F4] |
| T7 | After open, `xref.len() > max_objects` → `OpenError::Limit(Objects)`. The xref was already parsed at that point; this limit bounds downstream work, not the xref parse. **Revised 2026-10-01 (5)**: the check is repeated after the index walk, because a lazy xref repair during the walk can replace the table with a larger one. [§10] |
| T8 | When hayro repaired the xref (its `is_repaired`), the document carries the warning `xref: repaired`. (H1 exposes `Pdf::xref_was_repaired()`.) **Revised 2026-10-02 (7)**: repair is lazy, so a later operation may be the first to trigger it; `Document::warnings()` then reports one `XrefRepaired` from that point on (set once through a flag on `DocShared` by any operation that observes `xref_was_repaired()` turning true), while the `TextPage` of the operation that triggered it carries nothing — the repair belongs to the document, not the page. [§5.3] |

### 1.2 Page index and page handles [F5, F6, D7, P1, R2]

| ID | Requirement |
|---|---|
| T9 | Open builds the page index with the iterative walk of §5.4 over `catalog./Pages`, resolving one dictionary per node via `XRef::get::<Dict>`, never building hayro `Page` objects and never touching `/Contents` or `/Resources` values (only their references are recorded). `/Count` is ignored entirely (**Revised 2026-10-01 (5)**: not even as a capacity hint — a lying `/Count` must not size an allocation). |
| T10 | A node (by object identifier) reached a second time is skipped with the warning `page tree: node revisited`, whether through a cycle or a duplicate `/Kids` entry. A kid that is a direct dictionary (not a reference) is treated as a leaf and cannot be deduplicated. **KP10 (deliberate difference):** the oracle counts a duplicated leaf twice and raises on a cycle when the page is loaded (Appendix A.6); we yield each leaf once and never raise. |
| T11 | A node deeper than `max_page_tree_depth` (default **256**) is skipped with a warning (kind `PageTreeDepth`). **Revised 2026-10-01**: the oracle has no depth limit (a 20,000-deep chain extracts normally, Appendix A.5), so pages deeper than the limit are a documented divergence (KP23); 256 covers every real tree while keeping the index walk bounded. |
| T12 | A node without `/Kids` is a leaf regardless of `/Type` (hayro and the oracle both accept typeless leaves). A node with `/Kids` is interior regardless of `/Type`. **Revised 2026-10-02 (11)**: consequently a `/Kids` entry that references the catalog (which has no `/Kids`) is indexed as a typeless leaf and yields an empty page — reported by the unit-08 review; not re-verified against the code here. |
| T13 | Inherited attributes (`MediaBox`, `CropBox`, `Rotate`, `Resources`) are recorded per interior node in a table; each leaf stores a 16-byte `PageRef { obj: ObjectIdentifier, inherit: u32, kid: u16 }` (**Revised 2026-10-01 (5)**): a kid that is a direct dictionary is re-found at operation time by its position `kid` in the parent's `/Kids`; positions beyond 4096 are not indexed (warning `page tree: direct kid beyond position limit`). |
| T14 | `max_pages = Some(n)`: the walk stops with `TooManyPages { limit: n, at_least: n + 1 }` when leaf `n + 1` is found. Cost is O(n + depth) node resolutions. Checked by test, not by timing. |
| T15 | Fallback: when the walk yields zero leaves **and no subtree was cut at `max_page_tree_depth`** (**Revised 2026-10-01 (5)**: a depth-limited tree is a known shape, not a broken one; otherwise `deep_page_tree(3000)` would be rescued by the scan and contradict §4.8), scan all objects (`Pdf::objects()`) in object-number order for dictionaries with `/Type /Page`, bounded by `max_pages`; warning `page tree: brute-force scan`. Unreadable nodes (D-31) do trigger the scan. |
| T16 | `page_count` is O(1) after open and remains readable after `close()`. `pages()` is an `ExactSizeIterator`. `Page` is `(Arc<DocShared>, index)`. [F6] |
| T17 | A hayro `Page` is built from `(PageRef, inherited)` via the H1 constructor only inside an operation, on the operation's stack, and is never cached or stored in `Loaded`. |
| T18 | P1 (open + `page_count` of a 100k-page file ≤ 100 ms; 1M pages ≤ 1.5 s and ≤ 150 MB over the file) is a **report-only benchmark** on CI. The gating correctness tests are: H1's `Pdf::new` builds no pages (asserted via `Pdf::pages_built()`), its retained memory is bounded per xref entry (§4.5; **Revised 2026-10-01 (2)**: hayro's `XrefMap` costs ~33 B retained / ~49 B peak per entry, so a 1M-page file with ~2M entries retains ~65 MB at open before the 16 MB index — P1's 150 MB bound stays report-only and plausible), and the walk resolves at most `leaves + interior nodes` dictionaries (asserted via a counting `XRef` wrapper in tests). |

### 1.3 Text extraction [F7, F9, D3, K1, K2]

| ID | Requirement |
|---|---|
| T19 | `Page::text_page(&TextOptions)` interprets the page's content streams, Form XObjects, and the `/AP /N` appearance streams of annotations that are not hidden, not NoView, and not of subtype `Popup` or `Link` (Appendix A.5; H8c). Annotation text follows page content in output order. |
| T20 | Text in render modes 0–3 and 7 is captured once. Render mode 3 (invisible) is kept [KP13]. Mode 7 (clip) is kept once (oracle: once). **KP14 (deliberate difference):** the oracle emits text in modes 4–6 twice; we emit it once. **Revised 2026-10-01 (5)**: hayro itself calls `draw_glyph` twice for modes 2 and 6 (`Fill` then `Stroke(..)` with identical glyph and transforms); the device records a glyph once per show by skipping a `Stroke` call that immediately follows a `Fill` call with the same glyph, `transform` and `glyph_transform`. A later identical glyph from a separate show (fake bold, KP4) is a new call sequence and is kept. |
| T21 | **KP11 resolved (**Revised 2026-10-01**): hidden optional content is excluded.** For documents read from disk the oracle drops text in OCGs that are OFF in `/OCProperties /D` (via `BDC`, a form's `/OC`, an OCMD, or `/BaseState /OFF`); an OCG referenced without any `/OCProperties` is treated as visible (Appendix A.3). hayro's `OcgState` already implements exactly this, so no fork change and no option exist; there is no `ShowAll` mode in this run. |
| T22 | **KP7 resolved (**Revised 2026-10-01**): the clip box is the visible box, CropBox ∩ MediaBox** (as §1.3 of the design spec; hayro `Page::intersected_crop_box()`). Text inside the MediaBox but outside the CropBox is dropped, and text inside a CropBox that extends beyond the MediaBox is dropped where it leaves the MediaBox (Appendix A.2). A glyph is dropped when its box lies entirely outside the visible box; a glyph partly inside is kept. The test is per glyph, not per line (`x590` → `x`). |
| T23 | The glyph box is `[origin, origin + advance]` along the baseline and `[descent, ascent]` across it, in page space, where `ascent`/`descent` come from the font (H5: FontDescriptor `/Ascent` `/Descent`, Type 3 `FontBBox × FontMatrix`, standard-14 AFM values; otherwise 0.72 / −0.22 em; **Revised 2026-10-01 (5)**: a descriptor whose `/Descent` is positive or whose `/Ascent` is not positive passes H5's validity check but is rejected here as a whole and the defaults apply, since it would give a box that floats above or below the baseline). The oracle keeps an 11 pt glyph whose baseline is 3 pt below the page edge and drops one 8 pt below (Appendix A.2), which brackets Helvetica's ascent at (0.27, 0.73] em; AFM says 0.718. |
| T24 | **Clip paths filter text (new; T22's rule applied to the clip).** A glyph whose box does not intersect the *bounding box* of the intersection of the active clip paths is dropped. Non-rectangular clips are approximated by their bounding box; the oracle does the same (Appendix A.2: text inside the bbox of a diamond clip but outside the path is kept). Form XObject `/BBox` clips and text-clip modes (Tr 4–7, applied at `ET`) take part because hayro pushes them as clip paths. |
| T25 | Glyphs with zero size are kept if they have a Unicode value. [§6.3] |
| T26 | Unicode chain per glyph (§6.2, refined): (1) ToUnicode, with the oracle's control-character rules (**Revised 2026-10-02 (10b)**, probed over U+0000–U+001F, U+007F, U+0080–U+009F for Type 1, TrueType and Type 0 fonts): an entry whose value is a **single** code point in U+0008–U+000D yields **U+0020**; an entry whose single code point is any other control (U+0000–U+0007, U+000E–U+001F, U+007F, U+0080–U+009F) is **ignored** and the chain continues at (2)–(5) as if the font had no entry for that code; U+00A0, U+2028 and every non-control value are taken as is; an entry with **several** code points is taken verbatim, controls included, and D-10's sanitising then applies (a NUL or C0 control other than TAB/LF/CR inside it becomes U+FFFD; TAB/LF/CR and C1 stay, as the oracle emits them); (2) simple fonts: encoding/Differences glyph name → AGL / `uniXXXX` / `uXXXX[XX]`, where the AGL table is Adobe's single-code-point list only (**Revised 2026-10-01 (6)**: no TeX names; `parenleftbig` etc. stay unmapped, as the oracle leaves them) and, for Type 1/TrueType fonts without ToUnicode, the three **base-encoding** pairs WinAnsi 0xA0/`nbspace` → U+0020, WinAnsi 0xAD/`sfthyphen` → U+002D and MacRoman 0xCA/`nbspace` → U+0020 are normalised by core from the raw `glyph_name()` (**KP15**, **Revised 2026-10-01 (6)**, re-probed: the oracle normalises only those; `/Differences` names, Type 3 names and ToUnicode keep U+00A0/U+00AD; StandardEncoding 0xA0 is unmapped and yields the raw code); ToUnicode is consulted through `Glyph::to_unicode()` (H10) so hayro's own name table never decides; (3) CID fonts with a known ordering (GB1/CNS1/Japan1/Korea1): the ordering's `*-UCS2` CMap (H6a, embedded or not); (4) Identity encodings with an embedded TrueType/OpenType program and no ToUnicode: reverse of the font's `cmap` (H6b); (4b) **non-embedded** CIDFontType0 or CIDFontType2 (**Revised 2026-10-01 (6)**: both, as the oracle does) with Identity encoding and no ToUnicode, with or without a `/FontDescriptor`: CID as glyph index in the standard Macintosh glyph order (258 names from the TrueType `post` format 1 table) → AGL; CIDs ≥ 258 fall through to (5) (**KP16**, Appendix A.4: oracle gives `ABC a` for CIDs 36,37,38,3,68); (5) unmapped → `Unmapped` policy. |
| T27 | **KP6 resolved: unmapped glyphs emit the raw character code as a code point** (oracle: Type 3 code 65 with name `/g65` → `A`; code 200 → `È`; code 0 → NUL; codes 1, 9, 10 → the control characters; Type 1 Differences `/g128` → U+0080). Compat default `Unmapped::Cid` does the same **except** NUL, C0 controls other than TAB/LF/CR, surrogates, U+FFFE/FFFF and values above U+10FFFF become U+FFFD (deliberate difference, §15.2). C1 controls (U+0080–U+009F) are kept as the oracle does. `Replacement` and `Skip` as §6.2. |
| T28 | **KP9 resolved: `/ActualText` is honoured.** For a `BDC` whose property dictionary (inline or named via `/Properties`) carries `/ActualText` (PDFDocEncoding or UTF-16BE with BOM), the whole string is emitted at the first glyph of the span with that glyph's geometry; later glyphs of the span emit nothing but still advance the pen; an empty string suppresses the span's glyphs; a nested span replaces its own glyphs and the outer span emits nothing further after the inner span ends. Spans that cross line breaks and spans whose glyphs are separated by gaps are **KP17 expected divergences** (the oracle's distribution rule is not reproducible black-box; Appendix A.4). **Revised 2026-10-01 (6)**: string decoding follows the re-probed oracle — in PDFDocEncoding a NUL, 0x9F or 0xAD byte terminates the string, 0x7F is kept, UTF-16 with either BOM (BE or LE) is accepted; unpaired surrogates become U+FFFD where the oracle emits garbage. A span left open at the end of a content stream or form is closed there (H10b), so it cannot swallow later text; core also closes any open span when `interpret_page` returns. The span record's `advance` grows with each swallowed glyph, which is how "advances the pen" is realised. **Revised 2026-10-02 (11)** (as built): a UTF-8 byte-order mark (`EF BB BF`, PDF 2.0) is also accepted; spans nest at most 64 deep (`MAX_ACTUAL_TEXT_NESTING`), deeper `BDC`s are plain marked content; a page may decode at most 4 × `max_chars_per_page` bytes of `/ActualText` in total (`ACTUAL_TEXT_BYTES_PER_CHAR = 4`), after which further spans are plain marked content — a hostile stream of huge or deeply nested spans is bounded by both. |
| T29 | Line assembly works in content-stream order and compares each glyph only with the current line (§6.4) with these corrections from the probes (Appendix A.1): all thresholds are in units of **the incoming glyph's size** (`em = c.size`, not `max(c.size, L.size)`); `dir` is the text-matrix x-axis direction after the CTM regardless of writing mode (**KP12 resolved:** vertical text therefore produces one line per glyph, as the oracle does); the three break tests have **separate** thresholds (**Revised 2026-10-02 (7b)**; one shared 0.8 em cannot satisfy the data): `PERP_BREAK` **0.809 em** on `perp` (bracket [0.8000, 0.8182): 11 pt raised 8.8 pt joins, 9 pt breaks; the 7 pt rows give the looser [0.714, 0.857)), `GAP_BREAK` **0.798 em** on `along` forwards (bracket [0.7955, 0.8000): 22 pt gap 17.5 pt joins with a space, 11 pt gap 8.8 pt and 8→20 pt gap 16 pt break; 11 pt 8.7 pt gives the floor 0.7909 and 22 pt 18 pt the ceiling 0.818, both looser), and `BACK_BREAK` **0.795 em** on `along` backwards (bracket [0.7727, 0.8182): 11 pt back 8.5 pt joins, back 9 pt breaks) (**KP2, KP3 resolved**: backward movement up to the back threshold joins the line without a space; beyond it starts a new line); all three in units of the incoming glyph's size, all strict `>`; `SPACE_GAP` initial **0.149 em** (**Revised 2026-10-02 (7)**; **KP1**, bracket **[0.1477, 0.15)** under the strict `>` rule: the 22 pt join at 3.25 pt is the floor, the 8→20 pt space at exactly 3 pt = 0.15 em is the open upper end; 0.15 itself fails that row); the direction-change test stays at 2°. **KP4 resolved:** duplicated fake-bold glyphs are **not** removed; the oracle keeps them, and they land on the same line or a new one purely by the backward-movement rule. |
| T30 | A synthetic U+0020 is appended before a glyph when `along > SPACE_GAP · em` and the gap is not already a space: **Revised 2026-10-02 (10)** (**KP24, option (b), signed off 2026-10-02**) the incoming glyph suppresses it only when it is **U+0020**; the line's last character suppresses it when it is **U+0020, U+1680, U+3000 or U+FEFF**; every other code point — including TAB, LF, VT, FF, CR, U+0085, U+00A0, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F — does not (Appendix A.1, probed over the whole Unicode whitespace set, each before and after a 0.3 em gap). Character spacing (`Tc`), word spacing (`Tw`) and `TJ` adjustments participate through the pen position (Appendix A.1: `Tc 2` → `H e l l o`; `Tc 9` → one glyph per line). |
| T31 | **KP5 resolved:** leading and trailing spaces, tabs and other whitespace glyphs are kept verbatim. |
| T32 | Blocks: a new block starts on direction change, on `perp > BLOCK_GAP` (1.5 × the incoming size), when the line starts more than 2 em left of the block, or at a Form XObject / annotation boundary (**Revised 2026-10-01 (6)**: seen through the H10 `Device` hooks `begin_form`/`end_form` and `begin_annotation`/`end_annotation`, which the device turns into boundary markers in the record stream). **Revised 2026-10-02 (11)** (as built): markers are kept in a separate list capped at 2 × `max_chars_per_page` + 256; beyond the cap later markers are dropped, which can only merge blocks, and the assembler tolerates an unbalanced begin/end list; empty forms emit no marker; a boundary that falls inside a line starts a new block at the *next* line. Blocks affect only `BlockSeparator::BlankLine`. [§6.4] |
| T33 | Serialisation per §6.5: `'\n'` after every line including the last; compat separator between blocks is nothing extra; empty page → `""`. A line's text may contain `'\n'` only when a mapping step yielded LF — an unmapped LF code (T27), a **multi-code-point** ToUnicode entry (**Revised 2026-10-02 (10b)**: a single-code-point entry mapping to LF becomes a space and can no longer produce `'\n'`), a glyph name (`uni000A`) or an ActualText string — which the oracle also emits in each case (**Revised 2026-10-01 (6)**). |
| T34 | `max_chars_per_page` (default 2,000,000) stops the device with the warning `text: character limit reached`; the partial page is returned. **Revised 2026-10-01 (6)**: the device raises the H10 cancel flag, so interpretation ends within `check_interval` operators (`BudgetExhaustion::Cancelled`), and no further operator or time budget is consumed; a budget that ran out *before* the cap stays `PageError::Limit`. |
| T35 | Damaged content streams yield partial text plus the warning `content: damaged` when the lexer skipped bytes to resynchronise (**Revised 2026-10-01 (6)**: upstream hayro abandons the whole stream at a stray `)`, `]` or `>>` and gives no signal; H10a makes `UntypedIter::next` skip the offending byte and continue, counting each skip in a thread-local sampled by core — **Revised 2026-10-02 (11)**: only for bytes that can start no token and for malformed `#` name runs; a failed *operand* still ends the stream, see §2.5 "Damaged content"); the operation does not fail. (Oracle: `GoodOp\nPartial\nAfterGarbage\n`.) [R3] |
| T36 | `TextPage` is immutable, owns its data, is `Send + Sync`, and stays readable after `close()`. `TextPage::source()` is `TextSource::Content` in this run; `dpi()` is `None`. [F9] |
| T37 | **Revised 2026-10-01 (6b)**: fonts that fail to load **keep their text**, as hayro and the oracle both do — a Type 1/TrueType/CID font whose embedded program is corrupt is replaced by a standard substitute inside `Font::new`, and a `/Font` entry that resolves to nothing (dangling reference, non-dictionary, unknown or missing `/Subtype`, Type 0 without `/DescendantFonts`) is replaced by Helvetica in the `Tf`/`Tj` handlers; the oracle extracts the text in every one of these cases (Appendix A.4). Each substitution adds to the warning `font: substituted` with a count (H10e thread-local `font_substitutions()`, bumped where the program fails to parse and where `Font::new` returns `None`; hayro keeps no name for the failed font). The *missing resource* case (`/Fx` not in `/Font` at all) is hayro's deliberate Helvetica fallback and is not counted. Text shown with no font resource at all is rendered through hayro's Helvetica fallback and extracted; the oracle extracts it when the resource is missing and drops it when no `Tf` was ever executed (**KP18 residual**, Appendix A.4). |

### 1.4 Limits and budgets [R1, R2, R4, §10]

The `Limits` struct for this run. Fields marked *deferred* exist in the design
spec but not in this run's API.

| Limit | Default | Enforced where | Effect | Tested by |
|---|---|---|---|---|
| `max_file_size` | 512 MiB | before reading (T1) | `FileTooLargeError` | unit 08 |
| `max_pages` (on `open`) | none | page-tree walk (T14) | `TooManyPagesError` | unit 08 |
| `max_page_tree_depth` | 256 (**Revised 2026-10-01**; was 64) | page-tree walk (T11) | warning; subtree skipped | unit 08 |
| `max_objects` | 10,000,000 | after xref parse (T7) | `CorruptDocumentError` (`LimitExceeded` detail) | unit 08 |
| object nesting depth | 256 (`hayro_syntax::MAX_OBJECT_DEPTH`, fixed) | hayro-syntax object parser and content lexer; hayro-postscript parser (**H2a**) | the over-deep object fails to parse; the containing object is treated as missing; warning `object: nesting limit` | units 04, 08 |
| `max_stream_decoded_bytes` | 256 MiB | hayro-syntax filters flate/LZW/RunLength/ASCII (**H2b**) | the stream fails to decode (`DecodeFailure::LimitExceeded`), hayro treats it as absent; warning `stream: decoded size limit` | units 04, 10 |
| `max_ops_per_page` | 10,000,000 (≈11 s of interpretation in release; **Revised 2026-10-01 (3)**) | hayro-interpret budget (**H3**) | `LimitExceededError(ops_per_page)` | units 06, 10 |
| `page_time_budget` | 30 s | hayro-interpret budget, checked every 4,096 operators (**H3**) | `LimitExceededError(page_time_budget)` | units 06, 10 |
| `max_chars_per_page` | 2,000,000 | text device (T34) | warning; extraction stops | units 09, 10 |
| nested interpretation depth | 50 (hayro constant; H3 extends the check to Type 3 glyphs, patterns and soft masks, **Revised 2026-10-01 (3)**) | hayro-interpret | warning; nested content skipped | unit 06 |
| Form XObject cycles | n/a | hayro-interpret (**H3b**) | a form already on the nesting stack is skipped once | unit 06 |
| `max_object_depth` (as a knob) | *deferred* | — | fixed at 256 in this run (D-12) | — |
| `max_document_decoded_bytes` | *deferred* | — | — | — |
| `max_image_pixels`, `max_render_pixels`, `page_timeout` | *deferred (OCR run)* | — | — | — |

`max_document_decoded_bytes` is dropped from this run (decision D-11): hayro
does not cache decoded streams, so a cumulative counter measures work, not
peak memory, and would reject large legitimate documents; a live-bytes
budget needs decode caching in hayro first.

### 1.5 Concurrency [C1–C4, C6, C7, D6]

| ID | Requirement |
|---|---|
| T38 | `Document`, `Page`, `TextPage` are `Send + Sync` (compile-time assertions). Nothing on the text path takes a process-wide lock; hayro's per-document locks (the xref-map `RwLock`, the object-stream cache `Mutex`) are the only shared locks and are held only for the duration of one lookup or one map swap. **Revised 2026-10-01 (5)**: upstream hayro violates this — lazy xref repair used `try_read`/`try_write` + `unwrap` and scanned the whole file under the write lock, so concurrent lookups during a repair panicked; H9 (unit 08b) fixes it before any multi-threaded extraction exists. |
| T39 | One `Document` used from several threads gives results identical to sequential use. Each operation clones the `Arc<Loaded>` under a read lock and releases the lock before interpreting. |
| T40 | `close()` sets the state to `None`; in-flight operations finish normally; later operations return `PageError::Closed`; memory is freed when the last clone drops. Calling `close()` twice is harmless. |
| T41 | Output depends only on bytes, options and library version (C6). A hayro `InterpreterCache` is created per operation and never shared across threads (it is `!Send`). |
| T42 | Importing `pdf_extract` does no work and starts no threads. The library creates no threads in this run. [C7, §8.5] |
| T43 | Every native Python method whose work is not constant runs inside `py.detach(...)`: `open`, `open_bytes`, `Page.text`, `Page.text_page`, `TextPage.text`, `PdfBuilder.to_bytes/save`. The module is declared `gil_used = false` and is tested on **3.14t** with `PYTHON_GIL=0` (**Revised 2026-10-02 (8)**: 3.13t is dropped — PyO3 0.29 requires 3.14 for free-threaded builds; a deliberate narrowing of C3's "3.13t and later", D-24). [C3] |

### 1.6 Errors and the Python boundary [R1, §9]

| ID | Requirement |
|---|---|
| T44 | The Python hierarchy of §9.2 is defined in full in `pdf_extract/errors.py`, including the OCR classes, so `__all__` and the `RuntimeError` rule are stable now. Only `OcrUnavailableError` derives from `RuntimeError`; `DocumentNotFoundError` also derives from `FileNotFoundError`. |
| T45 | Every exported native function body runs inside `catch_unwind`; a panic becomes `InternalError(message)`. `pyo3.PanicException` must never reach callers. Profiles set `panic = "unwind"` explicitly. A test-only native hook `_native._panic_for_test()` panics on purpose. |
| T46 | `doc[i]` raises `IndexError` out of range and accepts negative indices; non-integer indices raise `TypeError`. |
| T47 | Any `OSError` other than not-found raised while opening becomes `OpenError` with `__cause__` set. |
| T47b | Rust → Python mapping: `OpenError::{NotFound, Io, Empty, TooLarge, NotPdf, Corrupt, PasswordRequired, UnsupportedEncryption, TooManyPages}` → `DocumentNotFoundError`, `OpenError`, `EmptyFileError`, `FileTooLargeError`, `NotAPdfError`, `CorruptDocumentError`, `PasswordRequiredError`, `UnsupportedEncryptionError`, `TooManyPagesError`; `OpenError::Limit{Objects}` → `CorruptDocumentError` (message names the limit, per §10); `PageError::Closed` → `DocumentClosedError`; `PageError::IndexOutOfRange` → `IndexError`; `PageError::Limit` → `LimitExceededError(limit_name, value)`. |
| T48 | Nothing is written to file descriptors 1 or 2 by any library call, including hayro's `log` macros (the `logging` feature is off; without it hayro's `warn!` expands to nothing). |

### 1.7 Test kit [F10, §12.3]

| ID | Requirement |
|---|---|
| T49 | `pdf-extract-testkit` (Rust) and `pdf_extract.testing` (Python) provide `PdfBuilder`/`PageBuilder` per §12.3: pages of a given size; `insert_text(point, text, fontsize, render_mode)` with top-left origin and y growing downwards; standard-14 Helvetica with WinAnsiEncoding, not embedded; characters outside WinAnsi raise `ValueError`/`BuildError`; `to_bytes()` and `save(path)`. `text_width(text, fontsize)` returns the AFM width so fixtures can place runs exactly. |
| T50 | The Rust test kit additionally offers raw content streams, CropBox/Rotate, FreeText and generic annotations with appearance streams, optional-content groups (content `BDC` and form-level `/OC` via `set_form_ocg`), Form XObjects, Type 3 and Type 0 font dictionaries (non-embedded), encryption-free only, and a `hostile` module generating the fixtures of Appendix C (deep nesting, page-tree shapes, lying `/Count`, flate bombs, operator floods, Form XObject fan-out and self-recursion, deep page trees, 100k/1M-page trees). |
| T51 | Fixtures that the builder cannot make (encrypted files) are produced by `tools/oracle/mkfixtures.py` with PyMuPDF as a black-box tool, committed as binaries, and listed in `tests/fixtures/PROVENANCE.md` with the exact command. **Revised 2026-10-01**: these files are not byte-reproducible (the oracle randomises `/ID` and the file key on every save); the manifest hash is that of the committed file, and `mkfixtures.py enc` rewrites the manifest entries when it regenerates them. |

### 1.8 Compat-point register

| ID | Resolution in this run | Pinned by |
|---|---|---|
| KP1 | `SPACE_GAP` ∈ [0.1477, 0.15) em of the incoming glyph (strict `>`); default 0.149 (**Revised 2026-10-02 (7)**) | C: `kp01_*` sweeps at 11 and 22 pt and mixed sizes |
| KP24 | space suppression: incoming glyph only U+0020; line end U+0020/U+1680/U+3000/U+FEFF; nothing else (**Revised 2026-10-02 (10)**, probed) — adopted under option (b), else residual. **Revised 2026-10-02 (10b)**: the `kp24_w_*` rows reach their characters through ToUnicode, so T26's control rules apply first (TAB/LF/VT/FF/CR → U+0020, U+0085 ignored → glyph name) | C: `kp24_*` (all exact with (b) and the 10b ToUnicode rules) |
| KP2 | `GAP_BREAK` ∈ [0.7955, 0.8000) em of the incoming glyph (along, forwards), default 0.798; `PERP_BREAK` ∈ [0.8000, 0.8182), default 0.809 (**Revised 2026-10-02 (7b)**) | C: `kp02_*` sweeps, `perp_*` |
| KP3 | `BACK_BREAK` ∈ [0.7727, 0.8182) em, default 0.795: backward movement up to it joins (no space); more → new line (**Revised 2026-10-02 (7b)**) | C: `kp03_*` |
| KP4 | fake-bold duplicates are kept | C: `kp04_*` |
| KP5 | leading/trailing whitespace kept; tab kept; a space glyph before a gap is not doubled (**Revised 2026-10-02 (10)**) | C: `kp05_*` incl. `kp05_trailing_space_then_gap` |
| KP6 | raw code point for unmapped; FFFD for NUL/C0/surrogates (deliberate) | C: `kp06_*` (divergence-gated for codes 0,1) |
| KP7 | visible box = CropBox ∩ MediaBox (**Revised 2026-10-01**), per-glyph intersection test | C: `kp07_*` |
| KP8 | annotation `/AP /N` text included after page content | C: `kp08_*` |
| KP9 | ActualText honoured (simple rule, T28) | C: `kp09_*` (3 exact, 3 divergence; **Revised 2026-10-02 (10)**: `kp09_gap` places the glyph starts 40 pt apart and the oracle gives `a\n b`, not A.4's `a \nb` — a divergence under either recipe) |
| KP10 | honest count; duplicates and cycles collapsed (deliberate) | C: `kp10_*` (divergence) |
| KP11 | hidden OCG text excluded; OCG without `/OCProperties` visible (**Revised 2026-10-01**) | C: `kp11_*` |
| KP12 | vertical text one line per glyph (by construction of `dir`) | C: `kp12_*` |
| KP13 | invisible text included | C: `kp13_*` |
| KP14 | Tr 4–6 not duplicated (deliberate) | C: `kp14_*` (divergence) |
| KP15 | base-encoding WinAnsi 0xA0/0xAD and MacRoman 0xCA → space/hyphen; `/Differences`, Type 3 and ToUnicode keep U+00A0/U+00AD (**Revised 2026-10-01 (6)**) | C: `kp15_*` (`kp15_differences_nbspace` divergence; `kp15_type3_nbspace` **exact** — **Revised 2026-10-02 (10)**, the oracle matches T26) |
| KP16 | non-embedded Identity CIDFontType2 → standard Mac glyph order | C: `kp16_*` |
| KP17 | ActualText edge cases | C: `kp17_*` (divergence) |
| KP18 | no-`Tf` text extracted by us, dropped by the oracle | C: `kp18_*` (divergence) |
| KP19 | clip paths filter glyphs by bbox intersection | C: `kp19_*` |
| KP20 | Form XObject self-recursion emitted once; nested forms in order | C: `kp20_*` |
| KP21 | nested Form XObjects deeper than 50: oracle extracts every level, we stop at hayro's cap (**Revised 2026-10-01**) | C: `kp21_nested_forms_60` (divergence) |
| KP22 | non-embedded CID font without `/FontDescriptor`: the oracle emits the raw string bytes; we apply the same chain as with a descriptor (Mac order for CIDs < 258, raw CID beyond) (**Revised 2026-10-01 (6)**) | C: `kp22_cid_no_descriptor` (divergence) |
| KP23 | page-tree nodes deeper than `max_page_tree_depth`: oracle keeps them, we skip with a warning (**Revised 2026-10-01**) | hostile: `deep_tree_3000` (outcome gate) |

---

## 2. Non-goals and bounded claims

### 2.1 Out of this run

- Everything OCR: `pdf-extract-render`, `pdf-extract-ocr`, `pdf-extract-tesseract-sys`, `OcrEngine`, the pool, `Page.ocr`, `default_ocr_engine`, `TextSource::Ocr`, `max_render_pixels`, `page_timeout`. No `render` crate stub is created (decision D-18): an empty crate adds a workspace member, a CI matrix cell and a `[patch]` entry for `hayro` with no behaviour to verify.
- `extract()`, `Extraction`, `PageResult` (§12.4): deferred to the OCR run (decision D-19). The helper exists to run the OCR policy; a text-only variant would be a second API to deprecate.
- `Document::extract` batch workers and the sharded `FontCache` of §8.3 (H7 territory; P2/P3 are measured against per-operation caching).
- Password authentication, `mmap` source (`Source::Mmap`, `open(mmap=True)`), structured output (`blocks()` is `#[doc(hidden)]`), non-PDF formats, writing PDFs beyond the test kit.
- Fuzzing, sanitizers, `loom`, wheels, SBOM, `THIRD_PARTY_NOTICES` (Phase 3; **Revised 2026-10-02 (11)**: the wheel ships a stopgap `THIRD_PARTY_NOTICES` with the Adobe Glyph List notice and `README-AFM.txt`; the full generated file must also cover hayro-interpret's bundled Foxit `.pfb` fonts (BSD-style) and CC0 ICC profile, which cargo-deny cannot see, and the Tinos test asset). CI exists from unit 01; `cargo-deny` license enforcement lands in unit 01 (decision D-2). **Revised 2026-10-02 (9)**: L1 governs **third-party** code only — the shipped dependency graph (Rust and, later, C/C++) stays permissive and GPL/LGPL/AGPL remain banned there; the project's **own** code is GPL-3.0-or-later from unit 12b (D-40), which is compatible with every license in that graph.
- Corpus A (private uploads) and corpus D (OCR). The corpus-A gate is a documented script the user runs against private storage (unit 14).
- Opening upstream hayro pull requests. Patches are exported, not sent.

### 2.2 Trust boundary

PDF bytes are hostile input. Three boundaries catch malformed input:

| Boundary | What is caught | Converted to |
|---|---|---|
| **Open** (`Document::open`, `from_bytes`) | I/O errors; size limit; detection failures; `LoadPdfError` (T4); `max_objects`; page-tree limits (`TooManyPages`); parser nesting failures while reading page-tree dictionaries (an unreadable node is skipped with a warning) | `OpenError` → `pdf_extract.OpenError` subclasses |
| **Per-page extraction** (`Page::text_page`, `text`) | Interpreter budget exhaustion (ops, time); character cap; stream decode failures and nesting failures inside content (page or form content becomes empty, warning); fonts that fail to load (warning); hayro interpreter warnings | `PageError::Limit` → `LimitExceededError`; everything else is a `Warning` on the `TextPage`, not an error |
| **Python boundary** (`_native`) | Rust panics from any crate, including hayro and skrifa | `InternalError`; never `PanicException` |

Rust callers of `pdf-extract-core` receive panics as panics (the Rust API does
not `catch_unwind`; decision D-16). The design spec's `PageError::Internal` is
therefore not part of the Rust enum.

### 2.3 What R1 means in this run

R1 ("no input may crash the process") is bounded to:

1. **Panics** anywhere in the native code are caught at the Python boundary. Tested with an injected panic (T45) and by running the hostile fixture set through Python.
2. **Stack depth** is bounded by: the iterative page-tree walk (H1), the fixed object-nesting cap of 256 in hayro-syntax, hayro-interpret's Type 4 parser and hayro-postscript (H2a), hayro's nested-interpretation cap of 50 — which H3 extends to Type 3 glyph procedures, tiling patterns and soft masks, where upstream carried the depth but never checked it (**Revised 2026-10-01 (3)**) — and hayro-cmap's `usecmap` depth cap of 16. Every hostile fixture in Appendix C is run on a 512 KiB thread stack in Rust and in Python (`threading.stack_size`).
3. **Time** (**Revised 2026-10-01 (3)**) is bounded per page by `page_time_budget`: the deadline is checked every `check_interval` (4,096) operators, so extraction stops within `page_time_budget` + the time of 4,096 operators + one non-interruptible operator (§2.4). `max_ops_per_page` bounds work, not wall-clock time: hayro interprets ≈0.9M operators/s in release and ≈60k/s in debug on an M3 Max, so the 10M default is ≈11 s in release (under the 30 s budget) and is never the binding limit in debug. At open, time is bounded by `max_pages` plus the xref parse. Tested with the operator-flood fixture (op cap) and the Form XObject fan-out fixture under a short deadline (§4.6, §4.10); no test asserts a wall-clock bound on the operator cap.
4. **Memory** is bounded by `max_file_size`, the per-stream decoded cap, the page index's 16 bytes per leaf, and `max_chars_per_page`.

### 2.4 Robustness residuals (not defended in this run)

- **Peak memory per extraction** is bounded only by `max_stream_decoded_bytes × live decoded streams`; hayro holds a Form XObject's decoded content for the duration of its nested interpretation, so the theoretical bound is 50 × 256 MiB. In practice the operator budget ends such pages first. No document-level decode budget exists (D-11).
- **Streams without document context** (**Revised 2026-10-01 (4)**): inline-image data and `FromBytes` parsing bypass `DecodeLimits`; the text device never requests pixels, so they are not decoded on this path.
- **Predictor allocation** (**Revised 2026-10-01 (4)**): `apply_predictor` allocates `/Columns × bytes-per-pixel` rows from the stream dictionary before checking them against the data length; this allocation is bounded by dictionary values, not by H2b.
- **PostScript scanner error class** (**Revised 2026-10-01 (4)**): runs of more than 256 `<<`, `>>` or `[` in arbitrary binary data now yield `Error::LimitCheck` where upstream yielded `SyntaxError`; both end the CMap parse the same way.
- **Nesting at the cap.** Fifty nested interpretation levels (forms, Type 3 glyphs, patterns, soft masks in any mix) must fit the 512 KiB stack; the self-recursion fixtures of §4.6 exercise exactly 50 levels of each kind on a 512 KiB stack, mixed chains are not tested (**Revised 2026-10-01 (3)**).
- **H6b reverse-cmap build** (**Revised 2026-10-01 (5)**): reversing an embedded font's `cmap` (up to ~1.1M entries for a full Unicode font) happens once per font per context inside one operator and is not budgeted or interruptible; bounded by the font program's size, i.e. by `max_stream_decoded_bytes`.
- **TOCTOU at open** (**Revised 2026-10-01 (5)**): the regular-file check and size check precede the read; a path swapped for a FIFO or a growing file in between can block or be refused by the size bound during the read. Not defended.
- **Signal attribution** (**Revised 2026-10-01 (6)**): `ObjectDepth`, `StreamDecodeLimit` and `ContentDamaged` warnings come from thread-local counters sampled around each synchronous operation (H2a, H10a, H10f), so concurrent operations on one document never charge each other; `Pdf::decode_stats()` remains a document-wide total for diagnostics only.
- **Concurrent lazy repairs** (**Revised 2026-10-02 (11)**, H9): when several threads hit a broken xref entry at once, each may run its own full-file scan before the first swap lands (13–16 scans per round at 16 threads in the unit-08b test), so peak memory is N× one scan and `decode_stats` counts each scan's object-stream decodes; a scan-once guard is a follow-on.
- **Cross-kind interpretation cycles** (**Revised 2026-10-02 (11)**, H3b): the form-cycle stack lives in the `Context`, and Type 3 glyphs, patterns and soft masks start fresh contexts, so a form that draws a Type 3 glyph (or pattern) that draws the form again is stopped only by the 50-level nesting cap, repeating up to 50 times.
- **Module initialisation is unguarded** (**Revised 2026-10-02 (11)**): a panic inside `_native`'s `#[pymodule]` init would surface as `pyo3.PanicException` at import time; every method body after init is guarded (T45).
- **Non-interruptible single operations.** The time budget is checked between operators. One operator can decode a 256 MiB stream, parse a large font program with skrifa, or parse a CMap; each is bounded by the stream cap, not by the clock.
- **Allocation failure** aborts the process (Rust default). Not defended.
- **hayro internals not covered by H2/H3**: the xref fallback scan (`fallback_xref_map_inner`) is linear in the file but may re-run once for encrypted files; object-stream decoding is cached per document without a cap other than the per-stream cap; inline-image `EI` searching is linear per image; the Type 1 font parser, CFF/TrueType parsing (skrifa), the PostScript calculator evaluator (bounded by its own instruction loop, not by our budget), JBIG2/JPEG 2000/CCITT decoders (never reached on the text path because the device never requests pixels). Panics in any of these are caught at the Python boundary; infinite loops are not.
- **`unsafe` in the dependency graph**: hayro-syntax's `CachedPages` transmute (unreachable from our code after H1 unless `pages()` is called), `memchr`, `flate2`/`miniz_oxide`, `smallvec`, PyO3. Not audited in this run.
- **Fork safety** (C7) is untested in this run because nothing lazy exists yet.
- The free-threaded tests prove the absence of crashes and non-determinism under the test loads, not the absence of data races in general (ThreadSanitizer is Phase 3).

### 2.5 Parity residuals (differences from the oracle that are not findings)

The line assembler and Unicode chain approximate an external oracle. These
inputs may differ and are documented, not fixed, in this run:

- **Reverse cmap coverage** (**Revised 2026-10-01 (5)**): skrifa's `charmap()` ignores (3,0) symbolic and Macintosh (1,x) cmap subtables, so H6b yields nothing for symbol-encoded or Mac-only embedded TrueType CID fonts where the oracle may still map them.
- **Fonts that fail to load** (**Revised 2026-10-02 (11)**, replaces the stale "dropped" wording; T37 as revised in 6b): hayro substitutes a standard font and the text is kept, as the oracle does; the substitute's glyph order and widths may differ from the oracle's substitute, so advances and clip classification near edges can differ. One `font: substituted` warning carries the count.
- **CID fonts**: embedded CIDFontType0 with Identity encoding, no ToUnicode and a non-standard charset; non-embedded Identity fonts beyond the 258 standard Macintosh names; custom orderings; CJK text where the UCS2 CMap and MuPDF's mapping disagree.
- **Text without `Tf`** (KP18); **Tr 4–6 duplication** (KP14); **ActualText edge cases** (KP17); **duplicate leaves and cycles** (KP10); **Form XObjects nested deeper than 50** (KP21: hayro's constant; the oracle extracts at least 60 levels); **CID fonts without a `/FontDescriptor`** (KP22); **page trees deeper than 256** (KP23). **Revised 2026-10-01**.
- **Revised 2026-10-01 (6)** — **TeX glyph names** (`parenleftbig`, `summationtext`, …): our AGL table has no TeX names, so they are unmapped (raw code) for fonts without ToUnicode; the oracle leaves most of them raw too (`parenleftbig` → raw), but maps some that also appear in its own list — `square` → U+25A1, where we emit the raw code (**Revised 2026-10-01 (6b)**, residual pinned by `kp06_type3_square`); when a font has a ToUnicode CMap that lacks the code, hayro's `as_unicode()` fallback is bypassed by `Glyph::to_unicode()` (H10), so the same rule applies. **Restated `nbspace`:** a `/Differences` entry naming `nbspace` at 0xA0 under WinAnsi is normalised by us and kept as U+00A0 by the oracle (hayro cannot say where a name came from). **Named-property cost:** a 4 MB property list referenced thousands of times is parsed once per context with H10c; without it the oracle itself needs 25 s.
- **Hostile inputs**: the oracle's text for over-deep objects depends on where the nesting sits (`''`, partial text, or a raised error, Appendix A.5); our outcome is defined by the warning kind, not by matching that text.
- **FreeText annotations without an appearance stream**: the oracle synthesises an appearance from `/Contents` and `/DA`; we extract nothing. **Revised 2026-10-02 (11)**: **the oracle synthesises widget appearances** — for a checked check box whose `/N` is a state dictionary with `/AS` it emits its own glyph (U+0014), and for an unsigned signature field with a blank `/AP` it emits `SIGN` (corpus B `acroform`); we use the `/AP` stream as written and skip a state-dictionary `/N`, so such widgets are a divergence, not "skipped by both".
- **Glyph boxes at a clip or MediaBox edge**: our ascent/descent come from the FontDescriptor or AFM; MuPDF's come from its substitute fonts. Glyphs within about 0.1 em of an edge may be classified differently.
- **Threshold boundaries**: gaps within ±1% of `SPACE_GAP · em`, `GAP_BREAK · em`, `PERP_BREAK · em` or `BACK_BREAK · em` may be classified differently (**Revised 2026-10-02 (7b)**: the along bracket [0.7955, 0.8) and the perpendicular bracket [0.8, 0.8182) are each under 0.02 em wide and abut at 0.8 em, so a forward gap and a rise of the same size around 0.8 em are classified oppositely by design). The compat defaults are tuned against the KP fixtures; natural text rarely sits on the boundary. **Revised 2026-10-02 (7)**: `SPACE_GAP`'s bracket is only 0.0023 em wide, so a gap between 0.1477 and 0.15 em of the incoming size is the one region where a ±0.001 em difference flips the result; corpus-C rows are kept off that region. **Revised 2026-10-02 (10)**: the oracle itself compares with `>=` at exactly 0.15 em and 0.8 em and lets its f32 glyph placement decide — the same 3 pt gap at 20 pt is a space at x = 72, 100, 250 and a join at x = 10, 33.3, 499.9 (measured gap 2.9999962 pt), and likewise 16 pt and 8.8 pt at 0.8 em. Our strict `>` with 0.149 / 0.798 / 0.809 / 0.795 reproduces every exactly-on-boundary row and is independent of page position; the residual is gaps within ≈1e-6 em of a threshold, where the oracle's answer depends on its own rounding and ours does not. The perp/along asymmetry at 0.8 em (a rise of exactly 0.8 em joins, a forward gap of exactly 0.8 em breaks) may itself be such noise in the oracle; the split thresholds encode the observed outcomes either way.
- **Unicode whitespace** (**Revised 2026-10-02 (10)**, KP24 probed): the oracle suppresses the synthetic space only when the incoming glyph is U+0020, or when the line's last character is U+0020, U+1680, U+3000 or U+FEFF; TAB, LF, VT, FF, CR, U+0085, U+00A0, U+2000–U+200A, U+2028, U+2029, U+202F and U+205F suppress nothing on either side. Under option (b) T30 adopts these sets and this is no longer a residual. Under option (a) (`char::is_whitespace` on both sides) we emit no space next to those characters where the oracle inserts one — 11 KP24 fixtures become divergences.
- **Controls in ToUnicode** (**Revised 2026-10-02 (10)**, resolved **Revised 2026-10-02 (10b)**): the oracle's rules (single-code-point U+0008–U+000D → space; other single-code-point controls ignored; multi-code-point entries verbatim) are adopted in T26, so this is no longer a residual for Type 1, TrueType and Type 0 fonts. **Revised 2026-10-02 (11)**: Type 3 is now probed and matches (`tounicode_ctl_t3_*`). In a **composite** font an ignored entry leaves the glyph **unmapped (raw CID)**, as the oracle does (`tounicode_ctl_t0_*`: CID 91 reads `[`, not the Macintosh-order `x`); steps 3–4 (UCS2 ordering, reversed `cmap`) and 4b are not tried, because hayro offers no way to run them without the ToUnicode map — unprobed whether the oracle would use them for a font with an embedded program or a known collection. In a **simple** font an ignored entry falls to the glyph name without the KP15 normalisation (`has_to_unicode()` is true), so `/F 0xA0 nbspace` with an ignored entry yields U+00A0 — unprobed. Still unprobed: multi-code-point entries holding C1 or U+007F beyond the five probed shapes.
- **ToUnicode entries mapping to U+FFFD** (**Revised 2026-10-02 (11)**; corpus B `GlyphLayoutDIN91379`, one observation): the oracle ignores such an entry and falls back to the raw code (CID 0x0C93 reads `ಓ`); we take U+FFFD as written.
- **Right-to-left scripts** (**Revised 2026-10-02 (11)**; corpus B `BidiSample`, `hello3`, `FC60_Times`): the oracle reorders Arabic and Hebrew runs into logical order; we emit glyphs in content order (visual), so RTL words read reversed and mixed-direction lines differ. The largest single gap on corpus B (similarity 0.14–0.77). Pages without RTL text are unaffected.
- **Text inside tiling-pattern cells** (**Revised 2026-10-02 (11)**; corpus B `pattern_tiling_with_text`): the oracle extracts glyphs drawn by a tiling pattern's own content; the text device never runs pattern content (as it never runs Type 3 procedures), so we emit nothing for it.
- **Anisotropic scaling** (**Revised 2026-10-02 (11)**; corpus B `CombTest`): when x and y scale differently (a widget appearance fitted to its `/Rect`, 0.945 × 0.902), the oracle's font size is the geometric mean of the two scales (11.078 pt for 12 pt) while ours is the em height across the baseline (10.82 pt), so a gap of 0.799 em by its measure is 0.818 em by ours and the line breaks here but not there. No in-bracket `gap_break` reaches 0.818; tuning cannot close it.
- **Combining marks placed with kerns** (**Revised 2026-10-02 (11)**; corpus B `GlyphLayoutDIN91379`): a mark shown with a backward `TJ` kern and the base glyph with the matching forward kern (`[180.66 <mark>] TJ [-180.66 (H)] TJ`, plus `Ts`) leaves a 0.18 em forward gap after the mark's pen, so we insert a space (`K͟ H`) and the oracle does not.
- **Codes without a glyph in a non-embedded standard-14 font** (**Revised 2026-10-02 (11)**; corpus B `embedded_zip`): a literal LF byte in a WinAnsi Helvetica string advances the oracle's pen by 0.278 em (its space width) and ours by a different amount, so a line that runs off the page is cut at a different character (`pdf_hays` there, `pd` here). Belongs with "Standard-14 glyph widths".
- **Type 3 fonts**: glyph size and advance come from `FontMatrix` and `/Widths`; MuPDF may size them from the glyph procedures.
- **Damaged content**: both parsers are lenient but resynchronise at different points. **Revised 2026-10-02 (11)** (H10a as built): we resynchronise only at a byte that can start neither an operand nor an operator (stray `)`, `]`, `>`, `}`) or a malformed `#`-escaped name run; a byte that starts an operand but fails to parse — a stray `[`, `<` or `(`, an over-deep array — still ends the stream (retrying at each following byte would be quadratic), as does an instruction with more than 10 operands. The oracle recovers from some of these.
- **Encryption** hayro rejects (unusual `/V 4` crypt filters, `/Identity` filters mixed with others): `UnsupportedEncryptionError` where the oracle opens. Counted against the open-rate gate.
- **xref repair** outcomes on heavily damaged files.
- **Standard-14 glyph widths**: hayro uses Foxit substitutes with AFM-compatible advances; mismatches would move pen positions and change spacing.

No universal claim is made over the space of all PDFs. The gates in §4.7 are
the claim.

---

## 3. Interface changes

### 3.1 Repository layout

```
pdf-extract/
├── Cargo.toml                  workspace: members = crates/*, resolver = "3"
├── Cargo.lock                  committed
├── rust-toolchain.toml         channel pinned (1.98.0)
├── deny.toml                   cargo-deny: licenses allow-list, advisories, bans, sources
├── pyproject.toml              maturin project; module pdf_extract._native; python-source = python
├── uv.lock                     dev tooling lock (pytest, pytest-cov, mypy, ruff)
├── LICENSE                         GPL-3.0-or-later full text (Revised 2026-10-02 (9); was MIT, Q1)
├── README.md · ARCHITECTURE.md · THEORY.md (unit 15)
├── crates/
│   ├── core/       pdf-extract-core      forbid(unsafe_code); benches/ (criterion, report-only) + benches/baseline.json
│   ├── testkit/    pdf-extract-testkit   forbid(unsafe_code); dev-only + Python binding
│   └── python/     pdf-extract-python    cdylib `_native`; PyO3 0.29
├── python/pdf_extract/   __init__.py, errors.py, testing.py, *.pyi, py.typed
├── vendor/
│   ├── PROVENANCE.md         crate name, version, crates.io tarball sha256, import date
│   ├── hayro-syntax/ hayro-interpret/ hayro-postscript/    (pristine then patched)
│   └── patches/hayro/0001-h2-….patch …                      upstream-ready, repo-relative
├── tests/fixtures/
│   ├── manifest.toml · PROVENANCE.md
│   ├── corpus-c/  *.pdf   (generated; also regenerable)
│   ├── corpus-b/  *.pdf   (imported; license per file)
│   ├── hostile/   *.pdf   (generated at test time by testkit; large ones never committed)
│   ├── expectations/<sha256>.json         oracle outputs (checked in)
│   └── snapshots/<sha256>.txt             our pinned output for divergence-gated fixtures
├── tools/
│   ├── oracle/    uv project: pymupdf==1.28.2; mkfixtures.py, gen.py, score.py, run_private_corpus.sh
│   └── vendor/    import.sh (pristine import + checksums), export-patches.sh, check-patches.sh
└── docs/ design-spec.md, reference/, specs/0001-text-path.md, adr/ (unit 15)
```

**Revised 2026-10-02 (9)**: every workspace crate and the Python package carry `license = "GPL-3.0-or-later"` (SPDX); vendored hayro crates keep their upstream `Apache-2.0 OR MIT`, and `vendor/PROVENANCE.md` and the patch series are untouched by the relicensing.

Crate names stay `pdf-extract-*` with `publish = false` (the crates.io name
`pdf-extract` is taken; §17.2). Python distribution `pdf-extract`, import
`pdf_extract`.

### 3.2 Vendored hayro fork

**What is vendored:** `hayro-syntax 0.7.2`, `hayro-interpret 0.7.0`,
`hayro-postscript 0.1.0` — the three crates this run patches. The others are
pinned exactly from crates.io and locked: `hayro-cmap =0.1.0`,
`hayro-jbig2 =0.3.0`, `hayro-jpeg2000 =0.3.5` (hayro-syntax 0.7.2 requires
`0.3.5`, not the newer 0.4.0), `hayro-ccitt =0.3.0`. `hayro 0.7.1` (the
rasteriser) is not a dependency in this run and is vendored when the render
crate arrives.

**Import procedure** (`tools/vendor/import.sh`): `cargo vendor` into a scratch
directory; copy the three crate directories to `vendor/<name>/`; delete
`.cargo-checksum.json` (cargo would verify per-file hashes and refuse patched
files); record the `package` checksum from that file in `vendor/PROVENANCE.md`.
Verified checksums: hayro-syntax `0edeafd70aa2db743de8ede8637d07ec87db05efe69cde371d03f1b185fcef27`,
hayro-interpret `f2613d0406898995042d0794c4245b2f2fba1246490c8ac593769bba6551129d`,
hayro-postscript `885c5ef0654933139a9b9546fc2c69e18d37f38aa2520f079092b1be18f1fcaa`.
The pristine import is one commit, tagged `vendor/hayro-pristine-1`.

**Wiring:** workspace `[patch.crates-io]` entries `hayro-syntax = { path = "vendor/hayro-syntax" }` etc. The vendored crates are not workspace members: `cargo fmt`/`clippy` exclude them, their upstream tests (which read `../hayro-tests/…`) are not run, and our tests of fork behaviour live in `crates/core`. The vendored `Cargo.toml` files keep their names and versions.

**Patch export** (`tools/vendor/export-patches.sh`): for each H commit,
`git format-patch -1 <sha> --relative=vendor -o vendor/patches/hayro/` so paths
read `hayro-syntax/src/...` as in the upstream monorepo. `check-patches.sh`
(CI job) re-applies the patch series onto a fresh pristine import in a temp
dir and diffs against `vendor/`; any drift fails CI.

**Changes, with the files and functions each touches** (verified against the
0.7 sources):

| ID | Change | Where |
|---|---|---|
| **H2a** | Object nesting cap (**Revised 2026-10-01 (2)**): `Reader` gets a `depth: u32` field (private; `Reader::new` keeps its signature) incremented in `Array::read/skip`, `Dict::read/skip` (`parse_dict_with`), decremented on exit; beyond `pub const MAX_OBJECT_DEPTH: u32 = 256` the read returns `None` **and bumps a thread-local hit counter** (`std` feature only; `thread_local! { static OBJECT_DEPTH_LIMIT_HITS: Cell<u64> }`): `pub fn object_depth_limit_hits() -> u64` and `pub fn note_object_depth_limit_hit()`. Content operands go through the same `Object::read` (via `UntypedIter`'s bare `Reader` and `read_without_context`), so `TJ` arrays are covered without any context plumbing; when an operand fails to parse `UntypedIter::next` returns `None` and the rest of that content stream is lost (pinned, §4.4). The recursive Type 4 procedure parser is `hayro-interpret/src/function/type4.rs::parse_procedure_inner`: it gets a depth parameter capped at 256 and calls `note_object_depth_limit_hit()` on refusal. hayro-postscript (`no_std`, used by hayro-cmap for CMap parsing): `object.rs::read` caps `<<`/`[` nesting at 256 and returns `Err(Error::LimitCheck)` (existing variant); these hits are not counted (a CMap that fails to parse surfaces as a font with no ToUnicode, D-12). | `hayro-syntax/src/byte_reader.rs`, `object/{array,dict,mod}.rs`, `lib.rs`; `hayro-interpret/src/function/type4.rs`; `hayro-postscript/src/{object,array}.rs` |
| **H2b** | Decoded-size cap: `pub struct DecodeLimits { max_stream_decoded_bytes: u64 }` stored in `XRef`'s `SomeRepr`; `Pdf::new_with_options(data, password, DecodeLimits)`; `flate::decode` uses `Read::take(limit + 1)` and fails when `limit` is exceeded; the pdf.js-port fallback decoder, `lzw::decode`, `run_length::decode`, `ascii_85`/`ascii_hex` check output length; `DecodeFailure::LimitExceeded` added; `Stream::decoded_image` passes the limit from `self.dict.ctx().xref()`. `Pdf::decode_stats() -> DecodeStats { streams_over_limit: u64 }` (atomic) so the caller can attach a warning. **Revised 2026-10-01 (4)**: the limit also covers xref streams and the object streams decoded by the repair scan before an `XRef` exists (the budget is passed explicitly there); `DecodeLimits::default()` is unlimited (core always passes its `Limits`); stats count decode *events*, and since hayro caches nothing a stream decoded twice counts twice; streams decoded without document context (inline images, `FromBytes`) are not limited — not reachable on the text path unless the device asks for pixels (§2.4). | `hayro-syntax/src/filter/{mod,lzw_flate,run_length,ascii_85,ascii_hex}.rs`, `object/stream.rs`, `xref.rs`, `pdf.rs` |
| **H1** | Lazy pages: `Pdf.pages: OnceLock<Option<CachedPages>>`, built on first `pages()`; `Pdf::new` no longer walks the tree (it still requires `/Root` and `/Pages` refs via `TrailerData`); `pub fn pages_root(&self) -> ObjectIdentifier`; `pub fn pages_built(&self) -> bool`; `pub fn xref_was_repaired(&self) -> bool`. `pub struct InheritedAttributes<'a> { media_box, crop_box, rotate: Option<_>, resources: Vec<Dict<'a>> /* root→parent */ }` and `pub fn Page::from_dict(dict: &Dict<'a>, inherited: &InheritedAttributes<'a>) -> Page<'a>` (uses the dict's `ReaderContext`; builds the `Resources` chain from the ancestor list then the page's own `/Resources`). `Pages::new` keeps its recursive implementation for `pages()` callers (the renderer); our index never calls it. **Revised 2026-10-01 (2)**: the field is a private `OnceLock<CachedPages>` (a failed walk and failed brute force store an empty `Pages`, since `pages()` returns `&Pages`); `Page::new` is infallible and the brute-force `/Contents` check lives in its only caller; `Page::from_dict` builds a fresh `ReaderContext::new(xref, false)` as the eager walk does; `XRef::is_repaired` becomes `pub(crate)`. Consequence: a `/Pages` reference that does not resolve no longer fails `Pdf::new` (it used to be `Invalid`); **Revised 2026-10-01 (4)**: the same holds for a `/Pages` root that resolves to a dictionary without `/Kids`, or to a non-dictionary, in a file with no `/Type /Page` objects — all open with an empty `pages()`. A catalog without a `/Pages` reference still fails in `XRef::new`. `xref_was_repaired()` can flip to `true` after open because repair is lazy (first failed object lookup). Core decides the outcome (T5, D-31). | `hayro-syntax/src/pdf.rs`, `page.rs`, `xref.rs` |
| **H3** | Budget (**Revised 2026-10-01 (2)**): `pub struct InterpretBudget { pub max_ops: Option<u64>, pub deadline: Option<std::time::Instant>, pub check_interval: u32 }` in `InterpreterSettings` (always present; `Default` has no limits) with shared `Arc<BudgetState { ops: AtomicU64, exhausted: AtomicBool, nesting_cap_hits: AtomicU32, form_cycle_hits: AtomicU32 }>`, so nested contexts created by Type 3 glyphs, patterns and soft masks (`Context::new_with` from `glyph.settings.clone()`) share one state. `interpret()` calls `context.budget_tick()` per operator: increments; every `check_interval` ops checks the deadline; on exhaustion sets the flag and `break`s. `draw_form_xobject`, `Type3::render_glyph`, `TilingPattern::interpret`, `SoftMask::interpret` return early when exhausted. `begin_nested_interpretation` bumps `nesting_cap_hits` when it refuses; **Revised 2026-10-01 (3)**: `Type3::render_glyph`, `TilingPattern::interpret` and `SoftMask::interpret` apply the same cap to the `nesting_depth` they carry (upstream stores `ctx.nesting_depth() + 1` in the glyph/pattern/mask but never checks it) — they return without interpreting and bump `nesting_cap_hits` when `nesting_depth > MAX_NESTED_INTERPRETATION_DEPTH` (**Revised 2026-10-01 (4)**: strict, giving exactly 50 levels as for forms). `InterpretStats.ops` counts the operator that was refused (so it reads `max_ops + 1` on exhaustion), and the exhaustion reason is stored in an `AtomicU8` in `BudgetState`. `pub enum BudgetExhaustion { Ops, Deadline }`; `pub fn Context::stats(&self) -> InterpretStats { ops: u64, exhausted: Option<BudgetExhaustion>, nesting_cap_hits: u32, form_cycle_hits: u32, }` (**Revised 2026-10-01 (6b)**: no `font_load_failures` field; H10e is a thread-local)` (`BudgetExhaustion { Ops, Deadline, Cancelled }`, `Cancelled` from the H10g flag; **Revised 2026-10-01 (6)**)``; `pub fn Context::budget_exhausted(&self) -> bool` is `stats().exhausted.is_some()`. Core maps `Ops` → `LimitKind::OpsPerPage` and `Deadline` → `LimitKind::PageTimeBudget`. **H3b**: `Context.form_stack: Vec<ObjectIdentifier>`; `draw_form_xobject` skips a form whose stream `obj_id()` is already on the stack and bumps `form_cycle_hits`. | `hayro-interpret/src/interpret/mod.rs`, `context.rs`, `x_object.rs`, `font/type3.rs`, `pattern.rs`, `soft_mask.rs` |
| **H4** | `Device::begin_marked_content(&mut self, tag: &[u8], mcid: Option<i32>, properties: Option<&Dict<'_>>)` (**Revised 2026-10-01 (5)**: not `'a` — an inline `BDC` dictionary borrows the form's decoded buffer, which outlives only the call); the `BeginMarkedContentWithProperties` arm resolves the named or inline dictionary once and passes it. Default impl ignores it; hayro's renderer unaffected. | `hayro-interpret/src/device.rs`, `interpret/mod.rs` |
| **H5** | Glyph info: on `Glyph`/`OutlineGlyph`/`Type3Glyph`: `char_code() -> u32`; `cid() -> Option<u32>` (Type0 `code_to_cid`); `font_kind() -> FontKind { Type1, TrueType, Type0, Type3 }`; `is_vertical() -> bool`; `glyph_name() -> Option<&str>` (simple fonts: Differences, base encoding, or built-in); `has_to_unicode() -> bool`; `is_embedded() -> bool` (not `fallback`); `character_collection() -> Option<&CharacterCollection>`; `advance() -> Vec2` in text-space units/1000 for all kinds (Type 3 included); `ascent_descent() -> Option<(f32, f32)>` from FontDescriptor `/Ascent` `/Descent` read at `Font::new` (Type 3: `FontBBox` × `FontMatrix`), stored as two `f32` on each font struct. | `hayro-interpret/src/font/{mod,cid,true_type,type1,type3,outline,standard_font}.rs` |
| **H6** | Unicode fallbacks for CID fonts: (a) in `Type0Font::new`, use the ordering's UCS2 CMap when `to_unicode` is `None` regardless of `fallback`; (b) `Type0Font::char_code_to_unicode`: when there is still no map and the **embedded** program is OpenType/TrueType (**Revised 2026-10-01 (5)**: never for a substituted fallback font, whose glyph order is unrelated to the CIDs), reverse the `cmap` once into a `OnceLock<HashMap<GlyphId, char>>` and look up `cid_to_gid_map.map(cid)`; the build is unbudgeted (§2.4). | `hayro-interpret/src/font/cid.rs` |
| **H10** | Device and lexer signals (**Revised 2026-10-01 (6)**; unit 09b): **(a)** content-lexer resync — in `UntypedIter::next`, when neither an object nor an operator parses at the current byte, skip one byte, clear the operand stack and continue instead of returning `None`; count each skip in a thread-local `content_damage_events()` (hayro-syntax, `std` only, same pattern as H2a); **(b)** `interpret()` records the marked-content nesting level on entry and, before returning, pops every level opened in that invocation (`ocg_state.end_marked_content()` + `device.end_marked_content()`), so an unbalanced `BDC` never leaks past its content stream or form; **(c)** `Context.property_cache: HashMap<(usize /* properties dict data ptr */, Name), Option<Dict<'a>>>` so a named property list is resolved once per context; **(d)** `Device` hooks with default no-op bodies: `begin_form(&mut self, id: Option<ObjectIdentifier>)`, `end_form(&mut self)`, `begin_annotation(&mut self, subtype: &[u8])`, `end_annotation(&mut self)`, called around `draw_form_xobject` and each annotation appearance in `interpret_page`; **(e)** a thread-local `font_substitutions()` in hayro-interpret (`std` pattern as H2a), bumped where a font constructor falls back to a substitute after its embedded program fails to parse (`Type1Font::new`, `TrueTypeFont::new`, `Type0Font::new`/`FontType::new`) and in `Context::resolve_font` when `Font::new` returns `None` (**Revised 2026-10-01 (6b)**: glyphs are **not** suppressed; text is kept as the oracle keeps it); **(f)** `DecodeFailure::LimitExceeded` also bumps a thread-local `stream_decode_limit_hits()` in hayro-syntax; **(g)** `InterpretBudget.cancel: Option<Arc<AtomicBool>>`, checked in `budget_tick()`; when set, `exhausted = Some(BudgetExhaustion::Cancelled)` and interpretation stops like any exhaustion; **(h)** `Glyph::to_unicode(&self) -> Option<BfString>` — the ToUnicode lookup alone, with no glyph-name fallback. **Revised 2026-10-02 (11)**: exported as patches **0011–0019** (one per sub-change; 0019 keeps the inline-image `EI` end probe strict so image data is never resynchronised as content), not a single 0011. | `hayro-syntax/src/content/mod.rs`, `lib.rs`, `object/stream.rs`; `hayro-interpret/src/interpret/mod.rs`, `context.rs`, `device.rs`, `x_object.rs`, `font/{mod,outline,type1,true_type,cid,type3}.rs` |
| **H9** | Thread-safe lazy xref repair (**Revised 2026-10-01 (5)**; unit 08b): `XRef::get_with` takes the map lock with a blocking `read()`; `repair()` runs `fallback_xref_map` on the immutable file bytes **outside** any lock, then takes a blocking `write()`, re-checks `repaired` (returns if another thread got there first), swaps the map and sets the flag — idempotent and panic-free, no `try_*().unwrap()`, no `assert!`; `is_repaired()` keeps a blocking read. Concurrent lookups block for the duration of the swap only. **Revised 2026-10-02 (11)** (as built): `get_with` reads the `repaired` flag under the same read guard as the entry and, on an entry that proves stale after a concurrent swap, drops the guard and retries through `repair()` (holding the read guard across `repair()` would deadlock on the blocking write lock); without this a lookup racing a swap could return `None`. `pub fn Pdf::is_encrypted(&self) -> bool` (`!matches!(*xref.decryptor, Decryptor::None)`). | `hayro-syntax/src/xref.rs`, `pdf.rs` |
| **H8** | Compat switches (**Revised 2026-10-01**: the former (a) `OptionalContentMode::ShowAll` is dropped; hayro's OCG handling already matches the oracle, KP11): (b) `show_glyph` emits `draw_glyph(…, GlyphDrawMode::Invisible)` for `TextRenderingMode::Clip` after collecting the clip (the renderer already ignores `Invisible`); (c) `interpret_page` skips annotations with flag bit 6 (NoView, value 32) and subtypes `Popup` and `Link`. | `hayro-interpret/src/interpret/{mod,text}.rs` |
| H7 | **Not in this run.** Shared font cache is a performance change; P2/P3 are report-only here. | — |

Each H-change is its own commit with a test in `crates/core` and an exported
patch. The `warn!` macro in hayro expands to nothing without the `logging`
feature, so none of these changes writes to stderr (T48).

### 3.3 Rust API (`pdf-extract-core`)

Exactly what lands. Types not listed do not exist in this run.

```rust
// limits.rs
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    pub max_file_size: u64,                 // 512 MiB
    pub max_page_tree_depth: u32,           // 256 (Revised 2026-10-01)
    pub max_objects: usize,                 // 10_000_000
    pub max_stream_decoded_bytes: u64,      // 256 MiB
    pub max_ops_per_page: u64,              // 10_000_000
    pub page_time_budget: Duration,         // 30 s
    pub max_chars_per_page: usize,          // 2_000_000
}
impl Default for Limits { … }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Revised 2026-10-02 (11): LimitKind::name() -> &'static str gives the Python `limit_name`:
//   file_size | pages | page_tree_depth | objects | stream_decoded_bytes | ops_per_page | page_time_budget | chars_per_page
// (page_time_budget's `value` is in milliseconds). Derives Clone, Copy, Debug, PartialEq, Eq, Hash.
pub enum LimitKind { FileSize, Pages, PageTreeDepth, Objects,
                     StreamDecodedBytes, OpsPerPage, PageTimeBudget, CharsPerPage }

// error.rs
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    NotFound(PathBuf), Io(#[from] std::io::Error), Empty,
    TooLarge { size: u64, limit: u64 }, NotPdf, Corrupt(String),
    PasswordRequired, UnsupportedEncryption(String),
    TooManyPages { limit: usize, at_least: usize },
    Limit { kind: LimitKind, value: u64 },
}
#[derive(Debug, thiserror::Error)]
pub enum PageError {
    Closed, IndexOutOfRange { index: usize, count: usize },
    Limit { kind: LimitKind, value: u64 },
}

// warning.rs
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning { pub page: Option<usize>, pub kind: WarningKind, pub detail: String, pub count: u32 }   // count ≥ 1 (Revised 2026-10-01 (5): repeats of one kind are coalesced by the sink)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarningKind { XrefRepaired, PageTreeRevisited, PageTreeDepth, PageTreeBruteForce,
                       PageTreeNodeUnreadable, ObjectDepth, StreamDecodeLimit, StreamDecodeFailed,
                       ContentDamaged, FontLoadFailed, CharLimit, InterpreterNesting,
                       FormXObjectCycle, UnsupportedFont, ImageDecodeFailure }
impl Display for Warning { … }          // "<category>: <detail>" (+ " (×N)" when count > 1) — this is what Python sees; category is the §10 prefix ("xref", "page tree", "object", "stream", "content", "font", "text")
// Revised 2026-10-02 (11): WarningKind::category() -> &'static str is public; coalescing keeps the FIRST detail of a kind, so a later
// `page tree: direct kid beyond position limit` is absorbed into an earlier PageTreeNodeUnreadable's count (its text is lost).

// document.rs
pub struct OpenOptions { pub max_pages: Option<usize>, pub limits: Limits }
impl Default for OpenOptions { … }

#[derive(Clone)] pub struct Document(Arc<DocShared>);
impl Document {
    pub fn open(path: impl AsRef<Path>, opts: &OpenOptions) -> Result<Self, OpenError>;
    pub fn from_bytes(bytes: Vec<u8>, opts: &OpenOptions) -> Result<Self, OpenError>;
    pub fn from_shared(bytes: Arc<Vec<u8>>, opts: &OpenOptions) -> Result<Self, OpenError>;
    pub fn page_count(&self) -> usize;          // O(1); valid after close
    pub fn is_encrypted(&self) -> bool;         // valid after close
    pub fn warnings(&self) -> Vec<Warning>;     // open-time warnings, plus one XrefRepaired if a later operation triggered the lazy repair (Revised 2026-10-02 (7)); never two XrefRepaired (deduplicated when open already reported one; Revised 2026-10-02 (11)); valid after close
    pub fn page(&self, index: usize) -> Result<Page, PageError>;   // Closed or IndexOutOfRange
    pub fn pages(&self) -> Pages;               // ExactSizeIterator<Item = Page>; yields handles even after close
    pub fn close(&self);
    pub fn is_closed(&self) -> bool;
}

// page.rs
#[derive(Clone)] pub struct Page { /* Arc<DocShared>, index */ }
impl Page {
    pub fn index(&self) -> usize;
    pub fn text_page(&self, opts: &TextOptions) -> Result<TextPage, PageError>;
    pub fn text(&self, opts: &TextOptions) -> Result<String, PageError>;
}

// text/options.rs
#[derive(Clone, Debug, PartialEq)]
pub struct TextOptions { pub unmapped: Unmapped, pub block_separator: BlockSeparator,
                         pub layout: LayoutParams }          // no optional-content switch (Revised 2026-10-01, KP11)
pub enum Unmapped { Cid, Replacement, Skip }               // default Cid
pub enum BlockSeparator { Newline, BlankLine }             // default Newline
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutParams { pub space_gap: f32 /*0.149, Revised 2026-10-02 (7)*/,
                          pub perp_break: f32 /*0.809*/, pub gap_break: f32 /*0.798*/, pub back_break: f32 /*0.795*/,   // Revised 2026-10-02 (7b): replaces line_break; each strictly inside its Appendix A.1 bracket
                          pub block_gap: f32 /*1.5*/, pub block_indent: f32 /*2.0*/,
                          pub angle_tolerance_deg: f32 /*2.0*/ }

// text/page.rs
pub struct TextPage { /* blocks, warnings, source */ }
impl TextPage {
    pub fn source(&self) -> TextSource;                    // Content
    pub fn dpi(&self) -> Option<u32>;                      // None
    pub fn to_text(&self, sep: BlockSeparator) -> String;
    pub fn warnings(&self) -> &[Warning];
    #[doc(hidden)] pub fn blocks(&self) -> &[Block];       // model: Block { lines, bbox }, Line { chars, bbox, dir }, Char { text, origin, dir, size, advance }
}
// Revised 2026-10-02 (11), as shipped: TextPage, Block and Line derive Clone + Debug + PartialEq (Char: Clone + PartialEq);
// Line::text() -> String joins its chars; TextSource derives Clone, Copy, Debug, PartialEq, Eq, Hash.
pub enum TextSource { Content }
```

`Document`, `Page`, `TextPage` carry `static_assertions::assert_impl_all!(…: Send, Sync)`.
`Pages` holds a clone of the `Arc` and an `AtomicUsize` cursor (T43's Python iterator wraps it).

Internal structure (not public, binding for the implementer):

```rust
struct DocShared { state: RwLock<Option<Arc<Loaded>>>, page_count: usize,
                   is_encrypted: bool, open_warnings: Vec<Warning>, limits: Limits,
                   repaired_later: AtomicBool /* Revised 2026-10-02 (7): set by any operation that sees xref_was_repaired() flip; warnings() appends one XrefRepaired when set */ }
struct Loaded  { source: Arc<Vec<u8>>, pdf: hayro_syntax::Pdf, index: PageIndex }
struct PageIndex { leaves: Vec<PageRef>, inherited: Vec<InheritedNode> }   // InheritedNode holds ObjectIdentifiers of MediaBox/CropBox/Rotate/Resources-bearing dicts, resolved on use
```

`Loaded.pdf` is constructed with `Pdf::new_with_options(source.clone(), "", DecodeLimits { … })` after H2. The object-nesting cap is the fixed constant `hayro_syntax::MAX_OBJECT_DEPTH = 256` (D-12). **Revised 2026-10-01 (2)**: core turns hayro's signals into `Warning`s by sampling `hayro_syntax::object_depth_limit_hits()` and `pdf.decode_stats()` before and after `Pdf::new`, the index walk and each page extraction (all synchronous on the calling thread; the thread-local counter is only valid under that invariant, which ARCHITECTURE.md records), by re-reading `pdf.xref_was_repaired()` after the index walk as well as after `Pdf::new` (**Revised 2026-10-01 (4)**: repair is lazy and can happen on the first failed lookup during the walk), and by reading `Context::stats()` after each extraction: hits → `ObjectDepth`, `StreamDecodeLimit`, `InterpreterNesting`, `FormXObjectCycle`; `exhausted` → `PageError::Limit`. The index walk resolves `pdf.pages_root()` itself; an unreadable root falls back to the brute-force scan (T15) and, if that finds nothing, the document opens with `page_count == 0` and the warnings `PageTreeNodeUnreadable` + `PageTreeBruteForce` (D-31).

### 3.4 Test-kit API (`pdf-extract-testkit`)

```rust
pub struct PdfBuilder { … }
impl PdfBuilder {
    pub fn new() -> Self;
    pub fn add_page(&mut self, width: f32, height: f32) -> &mut PageBuilder;   // default 595 × 842 via add_page_a4()
    pub fn to_bytes(&self) -> Vec<u8>;
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()>;
    pub fn page_mut(&mut self, index: usize) -> Option<&mut PageBuilder>;   // Revised 2026-10-02 (11): shipped; the Python PageBuilder reaches its page through it
}
pub struct PageBuilder { … }
impl PageBuilder {
    pub fn insert_text(&mut self, point: (f32, f32), text: &str, style: TextStyle) -> Result<(), BuildError>; // top-left origin, y down
    pub fn raw_content(&mut self, ops: &str) -> &mut Self;                      // appended verbatim (PDF coordinates)
    pub fn set_crop_box(&mut self, rect: [f32; 4]) -> &mut Self;
    pub fn set_rotation(&mut self, degrees: i32) -> &mut Self;
    pub fn add_font(&mut self, name: &str, dict: FontDict) -> &mut Self;        // Type1 std-14 w/ encoding, Type3, Type0 non-embedded
    pub fn add_form_xobject(&mut self, name: &str, bbox: [f32; 4], content: &str) -> &mut Self;
    pub fn add_annotation(&mut self, annot: Annotation) -> &mut Self;           // subtype, rect, flags, appearance content, optional /Contents+/DA
    pub fn add_ocg(&mut self, name: &str, on: bool) -> OcgRef;                   // for `/OC /name BDC`
    pub fn set_form_ocg(&mut self, form: &str, ocg: &OcgRef) -> &mut Self;        // form-level /OC (Revised 2026-10-01: landed in unit 03)
}
pub struct TextStyle { pub fontsize: f32 /*11*/, pub render_mode: u8 /*0*/, pub font: &'static str /*"Helv"*/ }
pub fn text_width(text: &str, fontsize: f32) -> Result<f32, BuildError>;       // Helvetica AFM, WinAnsi
pub mod hostile {
    pub fn deep_nesting_in_page_dict(depth: usize) -> Vec<u8>;
    pub fn deep_nesting_in_content(depth: usize) -> Vec<u8>;
    pub fn deep_nesting_in_resources(depth: usize) -> Vec<u8>;
    pub fn deep_nesting_in_type4_function(depth: usize) -> Vec<u8>;
    pub fn deep_page_tree(depth: usize) -> Vec<u8>;
    pub fn page_tree(shape: TreeShape /* Flat(n) | Fanout{leaves_per_node, levels} | Cycle | DuplicateLeaf | LyingCount{claimed} | TypelessLeaf | DirectDictKid */) -> Vec<u8>;
    pub fn flate_bomb(decoded_len: u64) -> Vec<u8>;
    pub fn operator_flood(ops: u64) -> Vec<u8>;
    pub fn form_fanout(depth: u32) -> Vec<u8>;           // 2^depth leaf draws
    pub fn form_self_recursion() -> Vec<u8>;
    pub fn nested_forms(depth: u32) -> Vec<u8>;          // linear chain, for the 512 KiB stack test at hayro's cap of 50
    pub fn big_tree_to_path(leaves: usize, path: &Path) -> std::io::Result<()>;   // 100k / 1M pages for benches
}
pub mod contract {   // Revised 2026-10-01: added in unit 08 for the open-contract tests
    pub fn garbage_after_header() -> Vec<u8>;            // `%PDF-1.4` + non-PDF bytes
    pub fn header_at_offset(offset: usize) -> Vec<u8>;   // junk prefix before a valid file
    pub fn broken_startxref() -> Vec<u8>;                // bad offset; xref repair path
    pub fn truncated_tail(cut: usize) -> Vec<u8>;        // valid file minus its last `cut` bytes
    pub fn plain_text() -> Vec<u8>;                      // not a PDF at all
    pub fn catalog_without_pages() -> Vec<u8>;
    pub fn misoffset_object(pages: usize, page: usize, delta: i64) -> Vec<u8>;   // one wrong xref offset (Revised 2026-10-01 (5): drives the lazy-repair tests)
}
```

### 3.5 Python API (normative stub for this run)

```python
from __future__ import annotations
import os
from collections.abc import Iterator
from typing import Literal

__version__: str

BlockSeparator = Literal["newline", "blank_line"]
Unmapped = Literal["cid", "replacement", "skip"]
PathLike = str | os.PathLike[str]

class Limits:
    def __init__(self, *, max_file_size: int = 512 * 2**20, max_page_tree_depth: int = 256,
                 max_objects: int = 10_000_000,
                 max_stream_decoded_bytes: int = 256 * 2**20, max_ops_per_page: int = 10_000_000,
                 page_time_budget: float = 30.0, max_chars_per_page: int = 2_000_000) -> None: ...
    # read-only properties with the same names

def open(path: PathLike, *, max_pages: int | None = None, limits: Limits | None = None) -> Document: ...
def open_bytes(data: bytes | bytearray | memoryview, *, max_pages: int | None = None,
               limits: Limits | None = None) -> Document: ...

class Document:
    @property
    def page_count(self) -> int: ...
    @property
    def closed(self) -> bool: ...
    @property
    def is_encrypted(self) -> bool: ...
    @property
    def warnings(self) -> tuple[str, ...]: ...
    def __len__(self) -> int: ...
    def __getitem__(self, index: int) -> Page: ...        # negative ok; IndexError; TypeError
    def __iter__(self) -> Iterator[Page]: ...
    def close(self) -> None: ...
    def __enter__(self) -> Document: ...
    def __exit__(self, *exc_info: object) -> None: ...    # always closes; never suppresses

class Page:
    @property
    def index(self) -> int: ...
    def text(self, *, block_separator: BlockSeparator = "newline", unmapped: Unmapped = "cid") -> str: ...
    def text_page(self, *, unmapped: Unmapped = "cid") -> TextPage: ...

class TextPage:
    @property
    def source(self) -> Literal["content"]: ...
    @property
    def dpi(self) -> int | None: ...
    @property
    def warnings(self) -> tuple[str, ...]: ...
    def text(self, *, block_separator: BlockSeparator = "newline") -> str: ...

from .errors import *      # full §9.2 hierarchy; __all__ lists every class
```

`pdf_extract.testing`:

```python
class PdfBuilder:
    def __init__(self) -> None: ...
    def new_page(self, width: float = 595, height: float = 842) -> PageBuilder: ...
    def to_bytes(self) -> bytes: ...
    def save(self, path: PathLike) -> None: ...
class PageBuilder:
    def insert_text(self, point: tuple[float, float], text: str, *, fontsize: float = 11,
                    render_mode: int = 0) -> None: ...    # ValueError outside WinAnsi
def text_width(text: str, fontsize: float = 11) -> float: ...
```

`LimitExceededError` has `.limit_name: str` (one of `file_size`, `pages`, `page_tree_depth`, `objects`, `stream_decoded_bytes`, `ops_per_page`, `page_time_budget`, `chars_per_page` — `LimitKind::name()`, deliberately not the `Limits` kwarg spelling) and `.value: int` (milliseconds for `page_time_budget`; **Revised 2026-10-02 (11)**); `TooManyPagesError` has `.limit` and `.at_least`. **Revised 2026-10-02 (11)**, shipped extras beyond the stub: `Document.__repr__`, `Document.__length_hint__` (the page iterator's remaining count), `Limits` range-validates its arguments (negative or oversized → `ValueError` naming the argument; values beyond `i128` → `OverflowError` from the conversion), every class in `_native.pyi` is `@final`, and `_native._panic_for_test(*, detached: bool = False)` panics either attached or inside `py.detach`. Classes are not `immutable_type`, so `mock.patch.object(pdf_extract.Page, "text", …)` works. `LayoutParams` is not exposed to Python in this run (tuning is a Rust-test concern).

### 3.6 On-disk formats

**Oracle expectation** — `tests/fixtures/expectations/<sha256>.json`, one per input file, written by `tools/oracle/gen.py`, read by `tools/oracle/score.py` and by `crates/core/tests/parity.rs` (serde):

```json
{
  "schema": 1,
  "sha256": "<hex of the PDF bytes>",
  "size": 1234,
  "source": "corpus-c/kp01_space_gap_11pt_1.50.pdf",
  "oracle": { "pymupdf": "1.28.2", "mupdf": "1.28.2", "python": "3.13.15", "platform": "macOS-14-arm64", "generated": "2026-10-02" },
  "open": { "ok": true, "exception": null, "needs_pass": false, "is_encrypted": false },
  "page_count": 1,
  "pages": [ { "index": 0, "text": "Alpha Beta\n", "sha256": "<hex of UTF-8 text>", "error": null } ]
}
```

`exception` is the oracle's exception class name (`FileDataError`, …) when
`ok` is false. `error` is set when `get_text` raised for that page (user
password). Texts are stored inline; a page text over 1 MiB is stored as its
hash only (`"text": null`), which no fixture in this run reaches.

**Fixture manifest** — `tests/fixtures/manifest.toml`:

```toml
[[fixture]]
path = "corpus-c/kp01_space_gap_11pt_1.50.pdf"
sha256 = "…"
generator = "tools/oracle/mkfixtures.py::kp01"      # or "testkit::hostile::…" or "import"
compat_points = ["KP1"]
gate = "exact"                 # exact | similarity | divergence | hostile | skip   (Revised 2026-10-02 (11): `hostile` added by unit 13)
# gate = "hostile" (D-27) compares the open outcome class, requires
# warnings = ["ObjectDepth", "XrefRepaired", ...]   # the exact expected warning-kind set
# and pins our own text in snapshots/<sha256>.txt; the oracle's text is recorded but not compared.
# gate = "divergence" requires:
# divergence = "KP14: oracle duplicates Tr 4–6 text"; snapshot = "snapshots/<sha256>.txt"
# corpus-b entries add:
# origin = "https://…"; license = "Apache-2.0"; license_note = "…"
```

**Provenance** — `tests/fixtures/PROVENANCE.md` (one row per non-generated file: origin URL, commit, license, SHA-256, why it is in the corpus) and `vendor/PROVENANCE.md` (crate, version, tarball SHA-256, import date, pristine tag, patch list).

### 3.7 CI workflow (`.github/workflows/ci.yml`)

| Job | Runs on | Steps | Gating |
|---|---|---|---|
| `rust` | ubuntu-24.04, macos-14 | `cargo fmt --check` (workspace members only), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo doc --no-deps -D warnings` | yes |
| `deny` | ubuntu-24.04 | `cargo deny check licenses advisories bans sources` | yes |
| `vendor-patches` | ubuntu-24.04 | `tools/vendor/check-patches.sh` | yes (from unit 02) |
| `python` | ubuntu-24.04 × {3.11, 3.13, 3.14}; macos-14 × {3.13} | `uv sync`, `uv run maturin develop --release`, `uv run pytest --cov=pdf_extract --cov-fail-under=95` | yes (from unit 11) |
| `python-freethreaded` | ubuntu-24.04 × {3.14t} (**Revised 2026-10-02 (8)**: 3.13t removed) | same with `PYTHON_GIL=0`; asserts `sys._is_gil_enabled() is False` after import | yes (from unit 11) |
| `coverage` | ubuntu-24.04 | `cargo llvm-cov --workspace --lcov`, thresholds per crate (§4.8) | yes (from unit 08) |
| `parity` | ubuntu-24.04 | `cargo test -p pdf-extract-core --test parity` against checked-in expectations; no PyMuPDF | yes (from unit 13) |
| `bench` | ubuntu-24.04 | `cargo bench -p pdf-extract-core -- --output-format bencher`; peak-RSS report; uploads `bench.json` artefact; compares to `crates/core/benches/baseline.json` and **annotates** (**Revised 2026-10-02 (11)**: the committed baseline was recorded on a local arm64 Mac — 100k pages open 32.5 ms / 22 MB, 1M pages 443 ms / ~127 MB over the file, `max_pages=10` on 1M 47.5 ms — and is to be replaced by a CI `bench.json` artefact; CI runs on both `push` and `pull_request`, so branch pushes run twice; since unit 12b every job carries a `timeout-minutes`) | **report-only** (`continue-on-error: true`) |

The Rust jobs build with `--locked`. Python interpreters come from
`astral-sh/setup-uv` + `uv python install`. CI never installs PyMuPDF.

---

## 4. Verification oracle

"Done" for each unit means its listed items pass on CI. Items are stated
before implementation; a unit may add tests, never remove these.

### 4.1 Unit 01 — workspace and CI

- `cargo build --locked`, `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`, `cargo doc -D warnings` pass on an empty-but-real `pdf-extract-core` (exports `pub const VERSION`).
- `cargo deny check` passes with `deny.toml` allow-list: MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, Unlicense, 0BSD, BSL-1.0, OFL-1.1 (fonts embedded by hayro-interpret, if present); `multiple-versions = "warn"`; `unknown-registry = "deny"`, `unknown-git = "deny"` (**Revised 2026-10-02 (11)**: cargo-deny ≥ 0.18 removed the `copyleft` and `unlicensed` keys; with an allow-list, copyleft and unlicensed crates are rejected by construction, which `tools/license/check-deny-rejects-copyleft.sh` proves). **Revised 2026-10-02 (9)**: from unit 12b, `[licenses.private] ignore = true` so the workspace's own `publish = false` GPL-3.0-or-later crates are exempt from the dependency allow-list, which itself is unchanged.
- The workflow file lints (`actionlint`) and every job in §3.7 exists, with jobs that have nothing to run yet marked `if: false` and noted in ARCHITECTURE.md. **Revised 2026-10-02 (10)**: as of unit 13 every job is active and no `if: false` or `actionlint.yaml` suppression remains.
- **Revised 2026-10-01 (3)** (lands with unit 06): `[profile.dev.package.{hayro-interpret,hayro-syntax,hayro-cmap,hayro-postscript,skrifa,read-fonts}] opt-level = 2` so `cargo test` interprets at a usable speed while our own crates stay debuggable; **Revised 2026-10-01 (4)**: also `[profile.dev.package.pdf-extract-core] opt-level = 1`, because hayro's generic `interpret` is monomorphised in our crate and costs ≈14 KiB of stack per form level unoptimised.
- `rust-toolchain.toml` pins `1.98.0`; `edition = "2024"`; release profile `panic = "unwind"`, `lto = "thin"`, `codegen-units = 1`, `debug = "line-tables-only"`, `split-debuginfo` per platform default.

### 4.2 Unit 02 — vendored hayro, pristine

- Commit 1 (pristine): `vendor/{hayro-syntax,hayro-interpret,hayro-postscript}` byte-identical to the crates.io tarballs minus `.cargo-checksum.json`; `vendor/PROVENANCE.md` lists the three package checksums above; tag `vendor/hayro-pristine-1`.
- Commit 2 (wiring): `[patch.crates-io]` applied; `cargo tree -i hayro-syntax` shows the path source; `Cargo.lock` pins `hayro-cmap 0.1.0`, `hayro-jbig2 0.3.0`, `hayro-jpeg2000 0.3.5`, `hayro-ccitt 0.3.0`; `cargo deny` still clean (no new license kinds); smoke test in core opens a one-page builder-free PDF literal through `hayro_syntax::Pdf::new` and counts one page; `tools/vendor/check-patches.sh` passes with an empty patch series.
- `tools/vendor/import.sh` re-run into a temp dir reproduces the pristine tree (diff empty).

### 4.3 Unit 03 — test kit and fixtures

- `PdfBuilder` output opens in hayro and in the oracle (`tools/oracle/mkfixtures.py --verify` runs the oracle over every generated fixture and must raise on none).
- Coordinate convention: `insert_text((100, 100), "A")` on an 842 pt page places the baseline at PDF y = 742 (checked by parsing the content stream).
- **Revised 2026-10-01**: `text_width` is checked two ways against a vector generated once by `mkfixtures.py --afm-check`: (a) it equals the oracle's `get_text_length(fontname='helv')` to within 0.01 pt for every Latin-1 WinAnsi character (0x20–0x7E, 0xA0–0xFF) and for the multi-character rows; (b) it equals the oracle's **rendered** advance (the `rawdict` origin of a sentinel glyph drawn after the run from a WinAnsi Helvetica font, minus the run origin) to within 0.05 pt for **every** row, including the 27 WinAnsi characters in 0x80–0x9F, for which `get_text_length` returns the 278-unit fallback rather than the AFM width; (c) that 278-unit fallback is pinned in the vector so a future oracle change is noticed.
- Non-WinAnsi input (`"日本"`) → `BuildError::NotWinAnsi` / `ValueError`.
- Every Appendix C fixture generator produces a file that hayro's `Pdf::new` accepts or rejects as stated there; `manifest.toml` lists every committed fixture with a matching SHA-256 (test walks the directory).
- Encrypted fixtures: eight files from `mkfixtures.py enc`, listed in `PROVENANCE.md` with the note that regeneration yields different bytes (T51, **Revised 2026-10-01**).

### 4.4 Unit 04 — H2 parser and decode limits

**Revised 2026-10-01 (2)**: hayro-level observables only (hayro-syntax, hayro-interpret, hayro-postscript APIs plus a test `Device` that collects `as_unicode()` strings); the `WarningKind`, `page_count` and `Page::text` assertions for these fixtures live in §4.8 and §4.10. Fixtures from `testkit::hostile` (unit 03). Every bullet runs on a **512 KiB** thread stack.

- `deep_nesting_in_page_dict(200_000)` → `Pdf::new` is `Ok`, `pdf.pages().len() == 0` (the leaf's dictionary cannot be read, so the eager walk skips it) and `object_depth_limit_hits()` increased by at least 1 across the call.
- `deep_nesting_in_content(200_000)` (`(Before) Tj <deep> TJ (After) Tj`) → `interpret_page` with the collecting device yields exactly `Before`: the over-deep operand fails to parse, `UntypedIter::next` returns `None`, and the rest of the stream — including `After` — is lost (pinned; the oracle also loses it). Hits increased by at least 1.
- `deep_nesting_in_resources(200_000)` → the indirect resources dictionary is unreadable, `Page::resources()` is empty, the font falls back to Helvetica, and the device collects `Res`. Hits increased by at least 1.
- `deep_nesting_in_type4_function(200_000)` → `Function::new` returns `None` for the tint transform; **Revised 2026-10-01 (4)**: the `cs` handler falls back to DeviceGray and the fill still happens, so the device collects `Fn` and exactly one path (`paths == 1`); hits increased by at least 1 (bumped by `type4.rs` via `note_object_depth_limit_hit()`).
- Boundary: a dictionary nested 255 deep parses; 257 fails; 256 is the boundary test. **Revised 2026-10-01 (4)**: two accepted-depth cases need more than 512 KiB of stack in an unoptimised build until unit 06's dev-profile `opt-level` lands — evaluating a 256-deep Type 4 procedure (≈1.5 MiB) and 256 nested `<<` in key position (≈768 KiB); release needs ≤ 192 KiB. Unit 04 runs those two tests on 8 MiB and 2 MiB stacks respectively; unit 06 moves them back to 512 KiB (§4.6). All other bullets stay on 512 KiB. The thread-local is per thread: hits produced on a spawned thread are not visible on the test thread, and the counter starts at 0 on a fresh thread.
- `flate_bomb(50 MiB)` with `max_stream_decoded_bytes = 16 MiB` → `Stream::decoded()` is `Err(DecodeFailure::LimitExceeded)`, `interpret_page` collects no glyphs and `decode_stats().streams_over_limit == 1`; with the default limit the stream decodes, no glyphs (the content is whitespace) and `streams_over_limit == 0`.
- A valid 1 MiB flate stream decodes identically with and without the cap (no truncation side effect). The same for LZW, RunLength, ASCIIHex, ASCII85 (small synthetic streams).
- Type 4 parser (hayro-interpret): a procedure with 300 nested `{` → `Type4::new` is `None`; 200 nested parses and evaluates as before on hayro's own examples. hayro-postscript `Scanner::parse_object` on 300 nested `<<` → `Err(Error::LimitCheck)`; 200 nested parses.
- Regression: before H2 the four nesting fixtures abort with a stack overflow; the test runs each in a child process and asserts exit status 0 (as written).
- Patches `0001-h2a-…`, `0002-h2b-…` exported; `check-patches.sh` green.

### 4.5 Unit 05 — H1 lazy pages

**Revised 2026-10-01 (2)**: hayro-level observables only; the benches and the `testkit::contract` generators arrive with unit 08.

- `Pdf::new` on `page_tree(Flat(100_000))` returns with `pages_built() == false`; `pdf.pages()` later builds them (`len() == 100_000`) and `pages_built() == true`.
- `Page::from_dict` on a leaf with inherited `MediaBox`/`CropBox`/`Rotate`/`Resources` yields the same `media_box()`, `crop_box()`, `rotation()` and resolves the same font via `resources().get_font()` as the eager `pages()[i]`, for every page of every unit-03 `TreeShape` fixture (property test).
- `pages_root()` equals the trailer's `/Pages` reference; `/Pages` resolving to a dictionary without `/Kids` or to a non-dictionary, with no page objects in the file → `Pdf::new` Ok and `pages().len() == 0` (**Revised 2026-10-01 (4)**).
- **Revised 2026-10-01 (4)**: `xref_was_repaired()` is false right after `Pdf::new` on a file whose table is intact except for one wrong object offset, and true after that object is first looked up (lazy repair). `xref_was_repaired()` is true for a file whose `startxref` offset the test corrupts in place (a builder PDF with the offset digits overwritten; unit 08's `testkit::contract::broken_startxref` replaces this later), false for the intact file.
- Memory (option a, gating): with a counting global allocator, `Pdf::new` on `page_tree(Flat(100_000))` retains fewer than **64 bytes per xref entry** (`pdf.len()`) and peaks below **96 bytes per entry** beyond the input buffer (measured: ~33 B / ~49 B, all of it hayro's `XrefMap`); the first `pages()` call then adds at least 10 × what `Pdf::new` retained. The P1 wall-clock numbers are report-only in the unit-08 bench.
- Dangling `/Pages`: a catalog whose `/Pages` reference points at a missing object and whose file contains no `/Type /Page` dictionaries → `Pdf::new` is `Ok`, `pages().len() == 0`. A catalog with no `/Pages` key → `Pdf::new` is `Err(Invalid)` (unchanged).
- Patch `0003-h1-…` exported.

### 4.6 Unit 06 — H3 budget and cycles

**Revised 2026-10-01 (2)**: hayro-level observables only — `Context::stats()` and a collecting test `Device`; the `LimitExceededError` and the `InterpreterNesting`/`FormXObjectCycle` warnings are asserted in §4.10.

- **Revised 2026-10-01 (3)**: `operator_flood(400_000)` with `max_ops = 200_000` → `stats().exhausted == Some(BudgetExhaustion::Ops)` after `interpret_page` and `stats().ops` is within `check_interval` of 200,000 (exactly 200,001 when the cap is the only limit — the refused operator is counted; **Revised 2026-10-01 (4)**); glyphs emitted before the cut are collected; with no limits it completes with `exhausted == None` and `ops ≈ 400,000`. No wall-clock assertion on the operator cap anywhere (hayro: ≈0.9M ops/s release, ≈60k/s debug).
- **Revised 2026-10-01 (3)**: `form_fanout(22)` (≈25.2M operators with unit 03's generator: 4 per leaf, 2 per interior form) with `max_ops = 200_000` → `Some(Ops)` (functional check: the budget cuts a fan-out, not just a flat flood); with `deadline = now + 500 ms` and no op cap → `Some(Deadline)` within 500 ms + 2 s (the slack covers 4,096 debug-speed operators and CI noise; this is the gating wall-clock test); without a budget it is not run (>20 s in the oracle, ~28 s in release hayro).
- `deadline = now + 50 ms` on `operator_flood(50_000_000)` → `Some(Deadline)` within 300 ms (measured 50.2 ms; loose bound; report the actual).
- Budget sharing: a Type 3 font whose glyph procedure contains 10,000 operators, drawn 200 times with `max_ops = 100_000` → `Some(Ops)` (proves nested contexts share the state; **Revised 2026-10-01 (3)**: sizes reduced).
- `form_self_recursion()` → the device collects `Rec` exactly once (oracle: once) and `stats().form_cycle_hits == 1`; `nested_forms(49)` collects 49 levels with `nesting_cap_hits == 0`; `nested_forms(60)` collects 50 levels (`L1`…`L50`; **Revised 2026-10-01 (3)**: unit 03 labels levels from 1) with `nesting_cap_hits == 1` (the oracle collects all 60 — KP21); all three on a 512 KiB stack.
- **Revised 2026-10-01 (3)** (R1, Type 3 / pattern / soft-mask recursion), with a test device that **does** run Type 3 glyph procedures (`Type3Glyph::interpret`) as the renderer does, on a 512 KiB stack: a Type 3 font whose glyph procedure shows text in the same font → terminates, the device sees at most 50 nested levels, `nesting_cap_hits >= 1`, `form_cycle_hits == 0`; a tiling pattern whose content fills with itself (`TilingPattern::interpret`) → terminates, `nesting_cap_hits >= 1`; a soft mask whose group references itself through an `ExtGState` → terminates, `nesting_cap_hits >= 1`. Before H3 the Type 3 case aborts with a stack overflow (child-process regression test as in §4.4).
- `InterpreterSettings::default()` has no limits and `Context::stats()` works without any budget configured (state always allocated).
- **Revised 2026-10-01 (4)**: with the dev-profile `opt-level` settings of §4.1 in place, unit 04's `type4_procedure_nesting_is_capped` (evaluation) and `dictionaries_in_key_position_are_capped` run on 512 KiB stacks again (unit 04 ran them on 8 MiB / 2 MiB).
- Patches `0004-h3-…`, `0005-h3b-…` exported.

### 4.7 Unit 07 — H4, H5, H6, H8

- **Revised 2026-10-01 (2)**: hayro-level observables only, through a collecting test `Device`; fixtures are built in the tests with the unit-03 builder (`FontDict::{standard14, Type3, Type0}`), not taken from corpus C (unit 13).
- H5: for Type 3 / Type 1 / Type 0 fixtures equivalent to the KP6 cases, `char_code()`, `cid()`, `font_kind()`, `glyph_name()`, `is_embedded()`, `has_to_unicode()` take the expected values (table in the test); `advance()` equals `/Widths` or `/W` or `DW`; for Type 3 it equals `Widths × FontMatrix`; `ascent_descent()` is `(0.718, −0.207)` for standard Helvetica (AFM), `(0.9, −0.2)` for the KP16 descriptor, `None` where absent.
- H6a: the Japan1 Identity-H fixture (Appendix A.4; its CIDFont carries a `/FontDescriptor`, which the oracle requires — **Revised 2026-10-01**, KP22) yields `ABC` via `as_unicode()` both with and without an embedded font program (**Revised 2026-10-01 (5)**: the embedded program is fontations' `tinos_subset.ttf`, Apache-2.0, imported into `crates/core/tests/assets/` with its own `PROVENANCE.md`; the test kit has no CFF generator).
- H6b: an embedded TrueType CID font with Identity-H and no ToUnicode (imported with provenance into `crates/core/tests/assets/`) yields its Latin text via `as_unicode()` through the reversed `cmap`.
- H4: a device recording `begin_marked_content` sees the `/ActualText` dictionary for inline and named properties and `None` for `BMC`.
- KP11 (**Revised 2026-10-01**, unpatched hayro behaviour, asserted so a future hayro change is noticed): the `kp11_hidden_ocg` fixture yields no `HiddenOCG` glyph for `BDC`, form `/OC`, OCMD and `/BaseState /OFF` variants; `kp11_ocg_no_ocprops` yields its glyph. H8b: the Tr 7 fixture yields one `Invisible` `draw_glyph` per glyph; H8c: Popup, Link and NoView annotations produce no glyphs, Hidden likewise, FreeText with `/AP` does.
- Patches `0006-h4…` `0007-h5…` `0008-h6…` `0009-h8…` exported.

### 4.8 Unit 08 — core: open, index, handles, close

Contract rows (internal survey §2.2, §13.5), all as Rust tests here and repeated in Python in unit 11:

| Input | Expected |
|---|---|
| missing path | `OpenError::NotFound` |
| directory, FIFO, socket or device path (**Revised 2026-10-01 (5)**) | `OpenError::Io` (kind `InvalidInput`, "not a regular file") without reading it; a symlink to a regular file opens |
| `from_bytes` with `len > max_file_size` (**Revised 2026-10-01 (5)**) | `TooLarge` |
| header starting at byte 1020 (straddling 1024) (**Revised 2026-10-01 (5)**) | `NotPdf` |
| `/Kids` with a direct dictionary at position 4097 (**Revised 2026-10-01 (5)**) | that kid is not indexed; warning `page tree: direct kid beyond position limit` |
| 1,000 duplicate leaves (**Revised 2026-10-01 (5)**) | one `PageTreeRevisited` warning with `count == 999`; Display ends in ` (×999)` |
| zero-byte file | `Empty` |
| `%PDF-` + garbage | `Corrupt` |
| plain text named `.pdf` | `NotPdf` |
| valid PDF named `.txt` or without extension | opens (**Revised 2026-10-02 (10)**: the oracle also opens it — it sniffs content, as internal survey §2.2 says) |
| header at offset 500 | opens |
| header at offset 1500 | `NotPdf` (oracle opens it; documented difference, F2 says 1024; Q6) |
| 8 encryption fixtures | owner: opens, `is_encrypted`, text extracted; user: `PasswordRequired` |
| broken `startxref`, truncated tail | opens, warning `XrefRepaired` |
| one wrong xref offset for a page-tree node (**Revised 2026-10-01 (4)**) | opens; repair happens during the index walk; warning `XrefRepaired` is still attached (sampled after the walk) |
| catalog without `/Pages` | `Corrupt` (oracle: 0 pages; divergence) |
| empty `/Kids` | `page_count == 0` |
| `TreeShape::LyingCount{claimed: 5}` on 100 leaves | `page_count == 100` |
| `DuplicateLeaf` | 1 page, warning `PageTreeRevisited`; `Cycle` | 1 page, same warning |
| `TypelessLeaf`, `DirectDictKid` | counted as pages |
| `deep_page_tree(200)` | 1 page (as the oracle) |
| `deep_page_tree(3000)` | 0 pages, warning `PageTreeDepth`, runs on 512 KiB stack (oracle: 1 page; KP23, **Revised 2026-10-01**) |
| `Fanout{100,2}` (10k leaves) with `max_pages = 10` | `TooManyPages{10, 11}`; the counting xref wrapper saw ≤ 10 + 3 resolutions |
| `max_objects = 3` on any fixture | `Limit{Objects}` |
| `max_objects` exceeded only after the walk's lazy repair enlarged the table (**Revised 2026-10-01 (5)**) | `Limit{Objects}` |
| `deep_nesting_in_page_dict(200_000)` (**Revised 2026-10-01 (2)**, moved from §4.4) | opens; `page_count == 0`; warnings `PageTreeNodeUnreadable`, `ObjectDepth` (counter delta across the walk) and `XrefRepaired` (**Revised 2026-10-02 (10)**: hayro's lazy repair runs when the over-deep object cannot be read at its offset, and the repaired table still cannot read it); 512 KiB stack |
| dangling `/Pages` reference, no page objects (**Revised 2026-10-01 (2)**, D-31) | opens; `page_count == 0`; warnings `PageTreeNodeUnreadable`, `PageTreeBruteForce` |
| `page(i)` out of range | `IndexOutOfRange` |

Close protocol: `close()` then `page(0)` → `Closed`; `page_count`, `is_encrypted`, `warnings` still answer; double close ok; `Drop` of the last clone frees `Loaded` (weak-pointer test); an extraction started before `close()` completes (test holds the `Arc<Loaded>` via a test hook that pauses inside the operation).

Concurrency (Rust): 16 threads × 200 `text()` calls on a 40-page builder document equal the sequential output byte for byte (needs unit 10; the unit-08 version compares `page_count`/`page()` only); 8 threads opening separate documents in parallel; close-during-use loop (1,000 iterations): every call returns `Ok` or `Err(Closed)`, never panics.

Benchmarks (report-only): `crates/core/benches/open.rs` — 100k- and 1M-page files generated into a temp dir, wall-clock of `Document::open` + `page_count`, and `max_pages = 10` on the 1M file; peak RSS via `getrusage`. Output compared to `crates/core/benches/baseline.json` with annotations only.

Coverage gate from this unit: `pdf-extract-core` line coverage ≥ 95%.

### 4.8b Unit 08b — H9 thread-safe xref repair (**Revised 2026-10-01 (5)**)

hayro-level observables only.

- A builder PDF whose xref offset for one object is wrong by 10 bytes (`testkit::contract::misoffset_object`): `XRef::get::<Dict>(id)` from 16 threads at once, each 100 times, all return `Some` with identical contents; `xref_was_repaired()` is true afterwards; no panic. Before H9 the same test aborts (`try_read().unwrap()` / `assert!(!repaired)`), pinned as a child-process regression test.
- Two threads that both observe `is_repaired() == false` and call into the repair path concurrently: both complete; the map is replaced once (a test hook counts `fallback_xref_map` runs ≤ 2 and the final map is identical to a single-threaded repair).
- Lock hold time: with a 50 MB file, a thread looping on `XRef::get` of a valid object during a concurrent repair is never blocked for longer than the repair's map swap (asserted loosely: no single `get` exceeds 50 ms while the scan itself takes > 200 ms; report-only numbers, gating on no panic and correctness).
- `Pdf::is_encrypted()` is true for the four owner-password-only fixtures after open and false for every unencrypted fixture (**Revised 2026-10-02 (11)**: the four user-password fixtures never reach the accessor — `Pdf::new` fails with `PasswordProtected`).
- Patch `0010-h9-…` exported; `check-patches.sh` green.

### 4.9 Unit 09 — text device

Tests drive `TextDevice` through hayro directly (internal `extract_chars()`), comparing `Vec<CharRecord>` fields:

- Geometry: `insert_text((100, 100), "AB", 11)` → origins `(100, 742)` and `(100 + 0.667·11, 742)`, `size 11`, `dir (1, 0)`, `advance` from AFM; with `set_rotation(90)` the page-space values follow `initial_transform(false)`; with a 90° text matrix `dir (0, 1)`.
- Unicode chain, one fixture per step of T26 (ToUnicode incl. the control rules of **Revised 2026-10-02 (10b)**; Differences AGL; `uni2603`; `nbspace`/`sfthyphen` WinAnsi and MacRoman — normalised by core from the raw `glyph_name()`, **Revised 2026-10-01 (5)**; Symbol built-in → `αβγ`; Japan1 UCS2; reversed cmap; standard-Mac-order; unmapped).
- **Revised 2026-10-01 (5)**: a FontDescriptor with `/Descent 200` (positive) or `/Ascent 0` → the glyph box uses the 0.72 / −0.22 defaults (clip classification matches the AFM case for Helvetica fixtures).
- `Unmapped::{Cid, Replacement, Skip}` on the KP6 Type 3 fixture (code 66 named `/whitesquare`, the AGL name for U+25A1; **Revised 2026-10-01 (6b)**): `Cid` → `A □ U+FFFD U+FFFD \t \n ' ' È`; `Skip` leaves only `□` (every other code is unmapped) while pen positions are unchanged; `Replacement` gives seven U+FFFD around `□`.
- Visible-box clip (**Revised 2026-10-01**): on a page without a CropBox, the eight KP7 glyph placements of Appendix A.2 classify as the oracle did (`y840` kept, `y845` kept, `y850` dropped, `x590` → `x` only, `xneg5` kept, `y2` kept, `yneg3` dropped) using Helvetica AFM ascent/descent; with CropBox `[100 100 400 700]`, `BelowCrop` (150,50), `RightOfCrop` (450,300) and `AboveCrop` (150,750) are dropped, `Straddle95`/`Straddle105` and `InsideCrop` kept; with CropBox `[100 -100 400 1000]`, a glyph at y=850 (inside the crop, outside the media box) is dropped.
- Clip paths: rect clip `x < 150` with `Straddle` at 140 → `S`,`t` kept; diamond clip → `Corner` kept; Tr 7 clip then `inside`/`outside` → `outside` dropped; Form `/BBox` clip drops `OutsideBBox`.
- ActualText: `Hello world`, `fine`, `ABC` (named), empty-string suppression, nested spans → `OUTERIN` (ours; snapshot), span across lines → ours snapshot. **Revised 2026-10-01 (6)**: decoding — `(a\x00b)` → `a`; `(a\x9fb)` and `(a\xadb)` → `a`; `(a\x7fb)` keeps 0x7F; `<FFFE6100>` (UTF-16LE BOM) → `a`; `<FEFFD800>` → U+FFFD; a `BDC` with no `EMC` before the end of a form → text after the form is present (needs H10b; before H10 the fixture loses it, pinned as a regression); a 4 MB named property referenced 2,000 times extracts in under 2 s on CI (needs H10c).
- Render modes 0–7: one record each for 0–3 and 7; one each for 4–6 (not two); `invisible` flag true for 3 and 7. **Revised 2026-10-01 (5)**: for modes 2 and 6 the device receives two `draw_glyph` calls (Fill, Stroke) and keeps one; a Tr 1 (Stroke-only) glyph is kept; `Bold` shown twice at the same origin (KP4) yields two records.
- Annotations: order `Body`, `AnnotVisible`; Hidden/NoView/Popup/Link excluded; `/Matrix`+`/BBox` scaling places `Scaled` inside the `/Rect`.
- OCG (**Revised 2026-10-01**): `HiddenOCG` absent for every OFF variant; present when the OCG is referenced but the catalog has no `/OCProperties`.
- `max_chars_per_page = 5` → 5 records + `CharLimit` warning, and `stats().exhausted == Some(Cancelled)` with `ops` far below the page's operator count (**Revised 2026-10-01 (6)**, needs H10g); a page that exhausts `max_ops_per_page` before reaching the cap → `PageError::Limit{OpsPerPage}`; the budget: `operator_flood` with `max_ops_per_page = 1000` → `PageError::Limit{OpsPerPage}` (from `BudgetExhaustion::Ops`); `page_time_budget = 50 ms` on the flood → `Limit{PageTimeBudget}` (from `Deadline`); **Revised 2026-10-01 (3)**: the text device records the Type 3 glyph itself and never calls `Type3Glyph::interpret` (§6.1), asserted with a self-referential Type 3 font on a 512 KiB stack (one record per shown glyph, no nested records, no `InterpreterNesting` warning).
- Damaged content fixture (`) ) ] >> garbage` between text objects) → records for `GoodOp`, `Partial` and `AfterGarbage` and one `ContentDamaged` warning whose count equals the number of skipped bytes (**Revised 2026-10-01 (6)**, needs H10a; a plain unknown operator yields the three words with no warning). Before H10 the fixture gives `GoodOp`, `Partial` only — the bullet is gated on 09b, not on unit 09.
- **Revised 2026-10-01 (6)**: sanitising — a `uni0000` glyph name and an ActualText `<FEFF0001>` each yield U+FFFD; a multi-code-point ToUnicode entry `<0041 0000>` yields `A` + U+FFFD and `<0041 000A 0042>` yields `A\nB` (LF kept). **Revised 2026-10-02 (10b)**: single-code-point ToUnicode entries — `<0009>`, `<000A>`, `<000D>` (and BS, VT, FF) → U+0020; `<0000>`, `<0001>`, `<001F>`, `<007F>`, `<0080>`, `<0085>`, `<009F>` → the entry is ignored and the glyph name (`x`) is used; `<00A0>` and `<2028>` verbatim — for a Type 1, a non-embedded TrueType and a Type 0 Identity-H font. A font dictionary with a corrupt `FontFile`/`FontFile2`/`FontFile3`, a dangling `/Font` reference, a non-dictionary entry, an unknown or missing `/Subtype`, and a Type 0 without `/DescendantFonts` → the text is extracted (`BrokenTT`, `BrokenT1`, `BrokenCFF`, `ABC`, `Dangling`, `NotADict`, `BadSubtype`, `NoSubtype`; the Type 0 case gives the raw bytes `\x00$\x00%`, sanitised to `U+FFFD$U+FFFD%`) and one `FontLoadFailed` warning (`font: substituted`) with the count of substitutions (needs H10e; **Revised 2026-10-01 (6b)**, oracle-matched). A missing resource name is extracted with no warning. Form and annotation boundaries appear as markers in the record stream (needs H10d). Two threads extracting different pages of one document, one page over the decode limit → only that page's `TextPage` carries `StreamDecodeLimit` (needs H10f; with H9 for the shared document).

### 4.9b Unit 09b — H10 device and lexer signals (**Revised 2026-10-01 (6)**)

hayro-level observables (collecting device, `Context::stats()`, thread-local counters), plus un-ignoring the gated §4.9 bullets.

- H10a: `UntypedIter` over `(A) Tj ) ) ] >> junk (B) Tj` yields both `Tj` instructions and `content_damage_events()` grows by the number of skipped bytes; a well-formed stream grows it by 0; the `operator-in-TJ-array` leniency of upstream is unchanged (its test case still parses).
- H10b: a form whose content ends inside `BDC` → the device sees one `end_marked_content` per unclosed `begin_marked_content` before `end_form`; OCG visibility is restored after the form (text after a form that leaked a hidden `/OC` span is drawn).
- H10c: a `/Properties` name referenced 2,000 times is resolved once per context (test hook counts `Dict` reads of the property object); the 4 MB fixture interprets in under 2 s on CI.
- H10d: `begin_form`/`end_form` are called once around every form draw including nested and annotation appearance forms; `begin_annotation`/`end_annotation` once per drawn annotation with its subtype; default implementations compile for the existing test devices unchanged.
- H10e (**Revised 2026-10-01 (6b)**): `font_substitutions()` on the test thread grows by 1 for each of: corrupt `FontFile2` (TrueType), corrupt `FontFile` (Type 1), corrupt `FontFile3` (CFF), corrupt CID `FontFile2`, dangling `/Font` reference, non-dictionary entry, unknown `/Subtype`, missing `/Subtype`, Type 0 without `/DescendantFonts`; the collecting device still receives every glyph of the shown text; it grows by 0 for the standard-14 fixtures and for a missing resource name (hayro's deliberate Helvetica fallback).
- H10f: `stream_decode_limit_hits()` on the current thread equals the number of `LimitExceeded` decodes performed on it; a decode on another thread is not visible.
- H10g: setting the cancel flag from the device after the 5th glyph → `stats().exhausted == Some(Cancelled)` and interpretation stops within `check_interval` operators; the flag is shared with nested contexts (a Type 3 glyph procedure is cut too).
- H10h: `Glyph::to_unicode()` returns `Some` only from the ToUnicode CMap (a simple font with ToUnicode lacking the code and a glyph name in hayro's table → `None`, while `as_unicode()` returns `Some`).
- Patches `0011`–`0019` exported (one per H10 sub-change, **Revised 2026-10-02 (11)**); `check-patches.sh` green; the §4.9 bullets marked "needs H10x" pass and are un-ignored.

### 4.10 Unit 10 — line assembler, serialisation, `Page::text`

Every row below is a `PdfBuilder` fixture whose expected string is the oracle's from Appendix A (also regenerated as corpus C in unit 13):

- KP1 sweeps: 11 pt gaps {0.5, 1, 1.25, 1.5 → join; 1.75, 2, 2.5, 3, 4 → space}; 22 pt {≤ 3.25 join; 3.5, 3.75, 4 space}; mixed sizes 20→8 {0 join; 2, 4 space; 8 newline} and 8→20 {1, 1.5, 2 join; 3–12 space; 16, 18 newline}. **Revised 2026-10-02 (7)**: the 8→20 pt gap-3 row is exactly 0.15 em and must give `small Big` at every page position (tested at x = 10, 33.3, 72, 100, 250, 499.9), which holds with `space_gap = 0.149` and fails with 0.15; the 22 pt gap-3.25 row (0.1477 em) must give `AlphaBeta` at the same positions. **Revised 2026-10-02 (10)**: these position sweeps are **our own** tests, not corpus-C fixtures — the oracle flips at x = 10, 33.3 and 499.9 because its f32 placement puts the gap at 2.9999962 pt and it compares with `>=` (Appendix A.1).
- KP2 sweeps: 11 pt {6, 8, 8.5, 8.7 space; 8.8–28 newline}; 22 pt {≤ 17.5 space; 18–28 newline}. **Revised 2026-10-02 (7b)**: rows within 0.01 em of `gap_break = 0.798` — 11 pt gap 8.7 (0.7909, space), 11 pt gap 8.8 (0.8000, newline), 22 pt gap 17.5 (0.7955, space) and 8→20 pt gap 16 (0.8000, newline) — run at x ∈ {10, 33.3, 72, 100, 250, 499.9} and must give the same result at every position (**Revised 2026-10-02 (10)**: our own tests only; the oracle gives `Alpha Beta` / `small Big` at x = 10, 33.3, 499.9 for the two 0.8 em rows).
- KP3 back sweep at 11 pt: {1–8.5 join without space; 9–26 newline}; `Right`/`Left` → two lines. **Revised 2026-10-02 (7b)**: no back row lies within 0.01 em of `back_break = 0.795` (8.5 pt = 0.7727, 9 pt = 0.8182), so no position sweep is needed; the `TJ +500` (back 0.5 em, join) and fake-bold overprint (back ≈ 2 em, newline) rows cover the far sides.
- Perp sweeps: 7 pt superscript raised {2–5 join; 6–12 newline}; 11 pt raised {2–8.8 join; 9–12 newline}; lowered 7 pt {2, 4 join; 6–10 newline}; diagonals of Appendix A.1. **Revised 2026-10-02 (7b)**: rows within 0.01 em of `perp_break = 0.809` — 11 pt raised 8.8 (0.8000, join) and raised 9 (0.8182, newline) — run at the six page positions above with identical results (**Revised 2026-10-02 (10)**: our own tests only, not corpus-C fixtures); the diagonal (1, 8.5) must join and (2, 8.5) must join with a space (perp 0.7727 < 0.809; along 1 pt ≤ 0.149 em, 2 pt > 0.149 em).
- KP4: `Bold` twice at +0.3 pt → `Bold\nBold\n`; exact overprint → same.
- KP5: `Hello   \nWorld\n`, `   Lead\n`, `A\tB\n`; WinAnsi 0xA0 → `A B\n`.
- `Tc` 1.5 → `Hello`; 2 → `H e l l o`; 9 → five lines; `Tw 20` on `a b c` → `a \nb \nc\n`; `TJ [(A) -300 (B) -2000 (C) 500 (D) -9000 (E)]` → `A B\nCD\nE\n`; `Tz 50` case → `Alpha\nBeta\n`.
- KP12: Identity-V Japan1 → `A\nB\nC\n` then `Horizontal\n`.
- Multi-line and two blocks → `Line one.\nline two.\nBlock two.\n`; `BlankLine` separator → `Line one.\nline two.\n\nBlock two.\n`.
- Shape invariants (property test over random builder pages): output is empty or ends with `'\n'`; contains no `"\n\n"` under `Newline` unless an unmapped LF was emitted; `text()` equals `text_page().to_text()`; output is identical across 8 threads and 50 repetitions; `to_text(BlankLine)` equals `to_text(Newline)` with exactly one extra `'\n'` inserted at each interior block boundary (**Revised 2026-10-02 (7)**), so `len(text.strip())` differs by the number of boundaries and the single-block case is identical.
- Rotated page (`/Rotate 90`) output equals the unrotated output.
- Empty page → `""`; zero-size font → `Zero\n`.
- **Revised 2026-10-01 (6)** (record model from unit 09): `CharRecord` has no `font` or `seq` field — ordering is the `Vec` order and glyph identity is not used by the assembler (KP4 keeps duplicates); spacing uses `origin + advance`, where an ActualText span record's advance is the sum of its glyphs' advances; under `Unmapped::Skip` the skipped glyph leaves no record, so its advance appears as a gap at the next glyph (a synthetic space or break may result — §6.2's "the advance still counts for spacing" is realised this way and is tested with `Skip` on the KP6 Type 3 fixture).
- **Revised 2026-10-01 (2)** (moved from §4.4/§4.6, now core-level): `deep_nesting_in_content(200_000)` → `Before\n` with warning `ObjectDepth` (`After` is lost, as in hayro and the oracle); `deep_nesting_in_resources` → `Res\n` + `ObjectDepth` + `XrefRepaired` (**Revised 2026-10-02 (10)**, lazy repair after the unreadable resources object); `deep_nesting_in_type4_function` → `Fn\n` + `ObjectDepth`; `flate_bomb(50 MiB)` with `max_stream_decoded_bytes = 16 MiB` → `""` + `StreamDecodeLimit`; `form_self_recursion` → `Rec\n` + `FormXObjectCycle`; `nested_forms(60)` → 50 lines + `InterpreterNesting` (KP21 snapshot); `nested_forms(49)` → 49 lines, no warning; `operator_flood(400_000)` with `max_ops_per_page = 200_000` → `LimitExceededError(ops_per_page)`; `form_fanout(22)` with `page_time_budget = 500 ms` → `LimitExceededError(page_time_budget)` within 500 ms + 2 s (**Revised 2026-10-01 (3)**: the only wall-clock gate; the operator cap has none). All on a 512 KiB stack.
- **Revised 2026-10-01 (5)** (C2 under lazy repair, needs H9): a 40-page builder document whose xref offset for page 7's content stream is wrong by 10 bytes (`testkit::contract::misoffset_object`) → 16 threads each extracting all pages concurrently, 50 rounds: no panic, every thread's output equals the sequential output, the document carries exactly one `XrefRepaired` warning after the first round (`Document::warnings()`, T8), and no page's `TextPage` warnings mention it. **Revised 2026-10-02 (7)**: the "before H9 this aborts" regression is covered only at hayro level (`crates/core/tests/h9_pre_fix.rs`, unit 08b), since core cannot be built against pre-H9 hayro; this bullet asserts correctness and absence of panics.

Coverage gate: core ≥ 95% holds with the assembler included.

### 4.11 Unit 11 — Python binding

- All contract rows of §4.8 in pytest with the exception classes of §9.2; `isinstance(err, PdfExtractError)` for all; `DocumentNotFoundError` is a `FileNotFoundError`; `PasswordRequiredError` at `open`, not at page access.
- `errors.__all__` walk: exactly one class is a `RuntimeError` subclass and it is `OcrUnavailableError`; all classes are `Exception` subclasses; `InternalError` is not a `BaseException`-only type.
- `_native._panic_for_test()` raises `InternalError`; `except Exception` catches it; `pyo3.PanicException` never appears (`sys.exc_info` type check).
- `doc[-1]` is the last page; `doc[len(doc)]` → `IndexError`; `doc["a"]` → `TypeError`; `list(doc)` has `page_count` elements; `len(doc) == doc.page_count`.
- Context manager closes on exception and re-raises; `doc.closed` after; `page.text()` after close → `DocumentClosedError`; a `TextPage` obtained before close still answers.
- Output-shape rows of §13.5 (every line ends `'\n'`; no `'\n\n'` in compat; empty page `''`).
- No stdout/stderr: a subprocess test redirects fds 1 and 2 to files, imports the module, runs open/extract over the hostile and KP fixtures plus every error row, and asserts both files are empty.
- GIL release: a thread calling `page.text()` on a 300-page operator-heavy page does not block a second thread's Python-only counter for more than 50 ms (coarse but detects a missing `detach`); `sys._is_gil_enabled()` is `False` under 3.14t after import (**Revised 2026-10-02 (8)**).
- Import does no work: `python -X importtime -c "import pdf_extract"` shows the module under 50 ms and no threads (`threading.active_count() == 1`).
- `mypy --strict` passes on the stubs and on a sample consumer; `ruff` clean.
- Coverage: `pdf_extract` package ≥ 95% (the Rust side of the binding is covered by the `coverage` job via `cargo llvm-cov` with the Python tests when feasible; otherwise report-only for `crates/python`).

### 4.12 Unit 12 — Python test kit and concurrency

- `pdf_extract.testing.PdfBuilder` reproduces the Rust test-kit bytes for the same calls (golden test); `text_width` matches.
- Stress: 16 threads × 200 `text()` on a 40-page document and on 16 separate documents, byte-identical to sequential (§13.3); runs under `PYTHON_GIL=0` on 3.14t (**Revised 2026-10-02 (8)**).
- Shared document: N threads extracting the same page and different pages concurrently; results identical.
- Close during use: a closer thread closes at a random moment while 8 workers extract; every call returns text or raises `DocumentClosedError`; the process survives 500 rounds.
- Shared iterator: two threads consuming one `iter(doc)` together see each index exactly once overall.
- 512 KiB stacks: `threading.stack_size(512 * 1024)`; run every hostile fixture and every KP fixture in such a thread; no crash.
- Free-threaded job green with the whole suite, including stress.

### 4.12b Unit 12b — relicense our own code to GPL-3.0-or-later (**Revised 2026-10-02 (9)**)

- `LICENSE` is byte-identical to the canonical GPL-3.0 text (SHA-256 of the FSF `gpl-3.0.txt` recorded in the test); `cargo metadata` shows `license = "GPL-3.0-or-later"` for every workspace member and no `license` change for any crate under `vendor/`; `python -m build --sdist` metadata (or `pyproject.toml` parsed in the test) shows `license = "GPL-3.0-or-later"` and the GPLv3+ classifier (**Revised 2026-10-02 (11)**: PEP 639 deprecates license classifiers; maturin accepts the combination, stricter backends may not — revisit when packaging for PyPI); `grep -rl MIT` over `LICENSE`, `README.md`, `pyproject.toml` and the workspace `Cargo.toml` files returns nothing (vendored notices excluded).
- `cargo deny check licenses` passes with the dependency allow-list **unchanged** (diff of `[licenses].allow` against unit 01 is empty) and `[licenses.private] ignore = true`.
- Negative proof: a scratch crate in the test (`cargo deny` run on a temp workspace copy) that adds a dev-only dependency declaring `license = "GPL-3.0-only"` fails `cargo deny check licenses`; the same with `LGPL-2.1` and `AGPL-3.0` also fails — L1 still bites for dependencies.
- Compatibility note (recorded in ARCHITECTURE): every license shipped with our code is GPLv3-compatible — hayro and PyO3 `Apache-2.0 OR MIT`, Adobe AFM/AGL BSD-3-style, Tinos `Apache-2.0`, `cargo tree` otherwise MIT/Apache-2.0/BSD/ISC/Zlib/Unicode-3.0 — so the combined wheel can be distributed under GPL-3.0-or-later.
- Consumer note (factual): a GPL-3.0-or-later `pdf_extract` wheel used by a consumer places that consumer's distribution, if any, under the GPL's source-availability obligations for the combined work; use within one organisation without distribution carries no such obligation. No further legal analysis is part of this run.
- `vendor/PROVENANCE.md`, `vendor/patches/` and the vendored crates' `Cargo.toml`/license files are unchanged (diff empty).

### 4.13 Unit 13 — oracle harness and corpus C parity

- `tools/oracle/gen.py <dir>` writes expectation JSON for every PDF under `tests/fixtures/corpus-c/` with the schema of §3.6, always opening files from disk (**Revised 2026-10-01**: the oracle's optional-content handling differs for unsaved in-memory documents, Appendix A.3). Corpus-C CID-font fixtures carry a `/FontDescriptor` (KP22). **Revised 2026-10-02 (7)**: unit 13 adds the KP24 probe over the whole Unicode whitespace set (Type 3 glyph names, each before and after a 0.3 em gap). **Revised 2026-10-02 (10)**: result in T30/Appendix A.1; under option (b) all KP24 fixtures are `exact`, under (a) the 11 rows next to TAB/LF/VT/FF/CR/U+0085/U+00A0/U+2000–U+200A/U+2028/U+2029/U+202F/U+205F are `divergence`. The three position-sweep fixtures (`kp01_mixed_8to20_3_positions`, `kp02_break_11pt_8.8_positions`, `kp02_break_8to20_16_positions`) are removed from corpus C and kept as §4.10 unit tests, since the oracle's answer there depends on its own f32 rounding. Hostile rows: the committed hostile fixtures are gated on open/extract outcome, the expected warning-kind set **and** our pinned text (snapshot), never on the oracle's text (D-27 clarified); lazy xref repair after an unreadable object adds `XrefRepaired` to the expected sets of `deep_obj_20k` and `deep_resources_20k`. The gate is `crates/core/tests/parity.rs`, which reads the expectation files directly (serde) and compares them with `pdf-extract-core` output; it also writes `target/parity-report.json` (per-page metrics, using `strsim` as a dev-dependency for Levenshtein). `tools/oracle/score.py` only pretty-prints that report; the oracle venv never imports our package and our tests never import PyMuPDF.
- `check_expectations` test: every expectation file's `sha256` matches a fixture in `manifest.toml` and vice versa; the `oracle.pymupdf` field is `1.28.2`.
- **Gate C:** for every manifest entry with `gate = "exact"`: our `text()` equals the oracle text exactly, and our open outcome class matches (`ok`/not). For `gate = "divergence"`: our output equals `snapshots/<sha>.txt` and differs from the oracle (a divergence that disappears fails the test so the manifest is cleaned). `gate = "similarity"` is not used in corpus C.
- Corpus C contents: one or more fixtures per KP1–KP24 (Appendix C), the ToUnicode control sweeps, the contract-row inputs, and the hostile fixtures that are small enough to commit (deep nesting at 20,000 instead of 200,000 to keep files small; the 200,000 variant is generated at test time). **Revised 2026-10-02 (11)**, as committed: 284 manifest entries for corpus C — 256 `exact`, 19 `divergence`, 9 `hostile` — plus 60 `similarity` entries for corpus B; Gate C passes 256/256, 19/19, 9/9.
- `tools/oracle/README.md` states L2 (black box only), how to regenerate, and that the venv is never used by the library or its tests.

### 4.14 Unit 14 — corpus B subset and tuning

- Import ≤ 60 files: hayro's `hayro-tests/pdfs/custom/*.pdf` that contain text (Apache-2.0 OR MIT, repo commit recorded), and PDFBox test resources with text (Apache-2.0), each with `origin`, `license`, `sha256` in `PROVENANCE.md` and `manifest.toml` (`gate = "similarity"`). No pdf.js files unless a per-file license is established (see Q2).
- **Gate B** (per page, over all B fixtures): (1) opens: oracle opened ⇒ we open, allowing a documented exception list ≤ 2 files; (2) routing agreement on `len(text.strip()) >= 50`: ≥ 99% of pages; (3) similarity `1 − normalised Levenshtein(ours, oracle)` (after `.strip()`): median ≥ 0.98, 10th percentile ≥ 0.90; (4) exact-match rate and line-count delta: reported in the test output, no gate. These initial thresholds are tightened in a follow-on once the residual list is characterised; a change to any gate is a spec change. **Revised 2026-10-02 (11)**, results on the committed corpus B (60 files: 20 from hayro-tests at `febaff5`, 40 from PDFBox at `c4d556a`; 91 pages): opens 60/60 with no exceptions, routing agreement 1.000, similarity median 1.000, p10 0.968 (min 0.000, mean 0.960), exact after strip 0.879, line-count delta mean |Δ| 0.13, max 3. No `LayoutParams` tuning was done — the gate passes at the revision-7b defaults and the one layout residual (anisotropic scaling) lies outside every bracket. The gate has headroom (median 1.000 vs 0.98, p10 0.968 vs 0.90) and may be tightened in a follow-on.
- Tuning: `LayoutParams` compat defaults adjusted only within the KP brackets; after any change, Gate C and Gate B re-run (the parity test is the regression harness).
- `tools/oracle/run_private_corpus.sh <dir>`: generates expectations into a private directory and runs the same metrics with the §13.2 corpus-A gates (100% open, ≥ 99.9% routing, median ≥ 0.999, p1 ≥ 0.98), printing a report; documented in `tools/oracle/README.md`; never run on CI.
- The residual list in §2.5 is updated from what B shows and copied into ARCHITECTURE.md. **Revised 2026-10-02 (10b)**: the C1-range ToUnicode probe planned for this unit was done in 10b and is dropped from its scope; the only ToUnicode item left for unit 14 is a Type 3 font with a control-valued ToUnicode entry (expected identical).

### 4.15 Unit 15 — docs and ADRs

- `docs/adr/0001…` created with the ADR workflow (`adr_index.py --next`, then the index regenerated) for every decision in §6; `adr_index.py` exits 0.
- README (install/build/test/usage), ARCHITECTURE (map, invariants, landmines, CI jobs, residuals), THEORY (why these thresholds, why MediaBox, why the fork); no assistant or tool-vendor references anywhere (a wordlist check in `tools/` is a test).

### 4.16 Cross-cutting checks (every unit)

- `#![forbid(unsafe_code)]` in `core` and `testkit`; `crates/python` allows it only where PyO3 requires. (*Note 2026-10-21*: core's own unit-test build is `deny(unsafe_code)`, not `forbid`, so its tests may include the measuring small-stack runner; ADR 0089.)
- Every source file starts with a header comment stating its role; comments explain decisions and invariants (reviewers check).
- No file over 2,000 lines outside `vendor/`.
- `cargo deny` green; patches in sync; no stdout/stderr test green from unit 11 on.
- Adversarial review before merge: each unit's PR is reviewed by a reviewer who did not implement it, using the unit's oracle list as the checklist and attempting to construct a failing input.

---

## 5. Work units

Stack order; each PR is independently reviewable and CI-green. Branch names
`feat/NN-slug`. Tier: **workhorse** = code; **mid** = docs/CI config. Sizes
exclude vendored imports and generated fixtures.

| NN | Branch | Contents | Oracle | Tier | Depends on | Size |
|---|---|---|---|---|---|---|
| 01 | `feat/01-workspace-ci` | Cargo workspace with an empty `pdf-extract-core`; `rust-toolchain.toml`; `deny.toml`; `.github/workflows/ci.yml` with all §3.7 jobs (inactive ones `if: false` until their unit; **Revised 2026-10-02 (10)**: none remain parked after unit 13 turned on `parity`, and `.github/actionlint.yaml` was removed); `pyproject.toml` skeleton (maturin, no module yet); `uv.lock` for dev tools; README, ARCHITECTURE (pickup notes incl. the unit stack), LICENSE files; `.gitignore` | §4.1 | mid | — | ~300 |
| 02 | `feat/02-vendor-hayro` | Commit A: pristine import of hayro-syntax 0.7.2, hayro-interpret 0.7.0, hayro-postscript 0.1.0 + `vendor/PROVENANCE.md` + tag. Commit B: `[patch.crates-io]`, exact pins, `tools/vendor/{import,export-patches,check-patches}.sh`, smoke test, `vendor-patches` CI job on | §4.2 | workhorse | 01 | ~250 (+ vendored) |
| 03 | `feat/03-testkit-fixtures` | `pdf-extract-testkit`: `PdfBuilder`/`PageBuilder`, Helvetica AFM table, fonts/annotations/OCG/forms helpers, `hostile` module; `tests/fixtures/` layout, `manifest.toml`, `PROVENANCE.md`; `tools/oracle/` uv project with `mkfixtures.py` (encrypted fixtures, `--verify`, `--afm-check`); the 8 encrypted fixtures committed | §4.3 | workhorse | 02 | ~1,200 |
| 04 | `feat/04-hayro-h2-limits` | H2a object nesting cap with thread-local hit counter (hayro-syntax), Type 4 procedure depth cap (hayro-interpret), PostScript scanner cap (hayro-postscript); H2b per-stream decode cap, `DecodeLimits`, `DecodeFailure::LimitExceeded`, `decode_stats()`; hayro-level tests on 512 KiB stacks using `testkit::hostile`; patches 0001–0002 | §4.4 | workhorse | 03 | ~550 |
| 05 | `feat/05-hayro-h1-lazy-pages` | H1 lazy pages, `Page::from_dict`, `InheritedAttributes`, `pages_root`, `pages_built`, `xref_was_repaired`; tests; patch 0003 | §4.5 | workhorse | 03 | ~350 |
| 06 | `feat/06-hayro-h3-budget` | H3 `InterpretBudget` + always-present shared state + `Context::stats()`/`budget_exhausted()`; nesting-cap hit counter and the cap applied to Type 3 / pattern / soft-mask contexts; `BudgetExhaustion`; H3b form cycle stack with hit counter; dev-profile `opt-level` for vendored and font crates and `opt-level = 1` for `pdf-extract-core`, moving the two unit-04 deep tests back to 512 KiB (**Revised 2026-10-01 (4)**); hayro-level tests; patches 0004–0005 | §4.6 | workhorse | 03 | ~550 |
| 07 | `feat/07-hayro-glyph-info` | H4 properties on `begin_marked_content`; H5 glyph info incl. ascent/descent and `advance()`; H6 CID Unicode fallbacks; H8 Tr 7 emission and annotation subtype/NoView skip (no OCG switch, **Revised 2026-10-01**); KP11 assertion tests; patches 0006–0009 | §4.7 | workhorse | 04, 05, 06 | ~650 |
| 08 | `feat/08-core-open-index` | `pdf-extract-core`: limits, errors, warnings, source, detection, encryption mapping, page index, `Document`/`Page`/`Pages`, close protocol; `testkit::contract` generators (**Revised 2026-10-01**); contract tests; concurrency tests (open/close/page_count); `crates/core/benches/open.rs`; coverage job gating on. **Revised 2026-10-01 (5)** follow-up commits on the same branch: reject non-regular paths; `from_bytes` size check; header-wholly-within-1024; `max_objects` re-check after the walk; `PageRef.kid` + 4096 cap; `Warning.count` coalescing and `"<category>: <detail>"` Display; `testkit::contract::misoffset_object`; after 08b merges, replace the bogus-password second parse with `pdf.is_encrypted()` | §4.8 | workhorse | 07 | ~1,700 |
| 08b | `feat/08b-hayro-h9-xref-concurrency` | H9: panic-free, idempotent lazy xref repair with the scan outside the lock; `Pdf::is_encrypted()`; hayro-level tests incl. the N-thread mis-offset lookup; patch 0010 | §4.8b | workhorse | 08 | ~250 |
| 09 | `feat/09-core-text-device` (depends on 08b, **Revised 2026-10-01 (5)**) | `TextDevice`, `CharRecord`, Unicode chain (incl. standard-Mac-order table, nbspace/sfthyphen), visible-box and clip-bbox filtering, ActualText spans, render-mode handling, char cap, budget → `PageError`, warnings; internal `extract_chars()`; tests. **Revised 2026-10-01 (6)** follow-ups on the branch: drop the TeX names from `agl.rs` (keep the BSD-3 notice); sanitise all emitted text (D-10); KP15 base-encoding-only normalisation; KP16/KP22 Mac order for CIDFontType0 too, CIDs ≥ 258 raw; ActualText decoding per T28; close open spans when `interpret_page` returns; `kp15_differences_nbspace` divergence fixture; mark the damaged-content, leaked-`BDC`, property-cache, cancel, `FontLoadFailed`, boundary and attribution tests `#[ignore]` until 09b | §4.9 | workhorse | 08b | ~1,300 |
| 09b | `feat/09b-hayro-h10-device-signals` | H10 (a–h): lexer resync + damage counter, auto-closed marked content, property cache, form/annotation hooks, font-load and decode-limit signals, cancel flag, `Glyph::to_unicode()`; core wiring (thread-local sampling, `Cancelled` → partial page, boundary markers, `FontLoadFailed`); un-ignores the unit-09 tests; patches 0011–0019 (**Revised 2026-10-02 (11)**) | §4.9 (gated bullets) + §4.9b | workhorse | 09 | ~450 |
| 10 | `feat/10-core-line-assembler` (depends on 09b, **Revised 2026-10-01 (6)**) | `LayoutParams`, `LineAssembler`, blocks (boundary markers from H10d), `TextPage`, serialisation, `TextOptions`, `Page::text_page/text`; KP sweep tests; shape property tests; full Rust stress test | §4.10 | workhorse | 09b | ~900 |
| 11 | `feat/11-python-binding` | `crates/python` (`_native`): `Limits`, `open`, `open_bytes`, `Document`, `Page`, `TextPage`, `catch_unwind` wrapper, `py.detach`, `gil_used = false`, `_panic_for_test`; `python/pdf_extract/{__init__,errors}.py`, stubs, `py.typed`; pytest contract/shape/no-stdout/GIL tests; `python` and `python-freethreaded` CI jobs on; mypy/ruff | §4.11 | workhorse | 10 | ~1,100 |
| 12 | `feat/12-python-testkit-concurrency` | `pdf_extract.testing` over the testkit crate; concurrency and 512 KiB-stack pytest suites; shared-iterator test | §4.12 | workhorse | 11 | ~500 |
| 12b | `feat/12b-relicense-gpl3` (**Revised 2026-10-02 (9)**) | Relicense our own code to GPL-3.0-or-later: `LICENSE` → full GPLv3 text; `license = "GPL-3.0-or-later"` in every workspace `Cargo.toml` (not the vendored crates) and `pyproject.toml` (plus the `License :: OSI Approved :: GNU General Public License v3 or later (GPLv3+)` classifier); README license section; ARCHITECTURE note (own code GPL, dependencies permissive, L1 unchanged); `deny.toml` `[licenses.private] ignore = true`; `tools/` license-wordlist check updated; Q1 resolution and D-40 recorded | §4.12b | mid | 12 | ~150 (+ GPL text) |
| 13 | `feat/13-oracle-parity-c` | `tools/oracle/gen.py`, `score.py`, README; corpus-C fixtures (KP1–KP20, contract inputs, small hostile) generated and committed with expectations and snapshots; `crates/core/tests/parity.rs` (+ `strsim` dev-dependency); `parity` CI job on | §4.13 | workhorse | 12 | ~700 (+ fixtures/JSON) |
| 14 | `feat/14-corpus-b-tuning` | corpus-B subset import with provenance; expectations; Gate B in `parity.rs`; `LayoutParams` tuning within brackets (**Revised 2026-10-02 (11)**: none was needed); `run_private_corpus.sh`; residual list update; core fix: an ignored ToUnicode control entry in a composite font leaves the glyph unmapped (raw CID) instead of taking step 4b, oracle-matched | §4.14 | workhorse | 13 | ~400 (+ fixtures/JSON) |
| 15 | `feat/15-docs-adrs` | ADRs for §6 in `docs/adr/`; README/ARCHITECTURE/THEORY refresh; wordlist check | §4.15 | mid | 14 | ~600 |

Unit 08b is a fork change inserted after 08 (**Revised 2026-10-01 (5)**); units 09–15 keep their numbers and 09 depends on 08b. Unit 09b is a fork change inserted after 09 (**Revised 2026-10-01 (6)**); 10 depends on 09b. Unit 12b (relicensing, mid tier) is inserted after 12 (**Revised 2026-10-02 (9)**); 13 depends on 12b so corpus-B provenance is written against the final license.
Units 04, 05, 06 are independent of each other and may be developed in
parallel on top of 03; 07 rebases on all three. Everything from 08 on is
sequential.

---

## 6. Decisions record

To be recorded as ADRs in unit 15. One line of rationale each.

| ID | Decision | Rationale |
|---|---|---|
| D-1 | Vendor only the three patched hayro crates (`-syntax`, `-interpret`, `-postscript`) under `vendor/` with a pristine commit, `[patch.crates-io]`, exported patches and a CI drift check | Reviewable diff; the unpatched leaf crates are reproducible from `Cargo.lock`. |
| D-2 | `cargo-deny` license enforcement from unit 01 (**Revised 2026-10-02 (9)**: from 12b with `[licenses.private] ignore = true`; the dependency allow-list is unchanged) | Cheap; guards every later dependency (L1). |
| D-3 | H2 includes an object-nesting cap in hayro-syntax and hayro-postscript | Verified stack overflow at 200k nesting on 8 MiB stacks in three places (Appendix B); R1 is unachievable without it. |
| D-4 | H8 added: Tr 7 glyph emission, annotation subtype/NoView filtering (**Revised 2026-10-01**: the `ShowAll` switch was dropped) | Oracle includes Tr 7 text and excludes Popup/Link/NoView; hidden-OCG text is excluded by both the oracle and unpatched hayro (Appendix A). |
| D-5 | H4 is in (ActualText honoured); simple first-glyph rule | Oracle honours ActualText; its distribution rule is not reproducible black-box; the simple rule matches all non-pathological probes. |
| D-6 | H6 is in (UCS2 for embedded fonts; reversed cmap) | Small, localised in `cid.rs`; shrinks the CJK residual before corpus B. |
| D-7 | H7 out | Performance only; P2/P3 are report-only in this run. |
| D-8 | Clip box is the visible box CropBox ∩ MediaBox (**Revised 2026-10-01**; an earlier probe in crop-relative coordinates had said MediaBox); per-glyph intersection test; clip paths filter by bbox | Measured oracle behaviour with absolute coordinates (Appendix A.2), confirming §1.3 of the design spec. |
| D-9 | Thresholds in units of the incoming glyph's size, all strict `>`: `SPACE_GAP` 0.149 ∈ [0.1477, 0.15); `GAP_BREAK` 0.798 ∈ [0.7955, 0.8000); `PERP_BREAK` 0.809 ∈ [0.8000, 0.8182); `BACK_BREAK` 0.795 ∈ [0.7727, 0.8182) (**Revised 2026-10-02 (7)**, **Revised 2026-10-02 (7b)**) | Sweeps at 11/22 pt, mixed sizes, superscripts and backward moves (Appendix A.1) fit this and not `max(c, L)`; a single 0.8 em sits on two data points with opposite outcomes (11 pt raised 8.8 pt joins, 8→20 pt gap 16 pt breaks) and flips with page position. |
| D-10 | Unmapped glyphs: raw code; **all** emitted text (unmapped codes, ToUnicode, glyph names, ActualText) has NUL, C0 controls other than TAB/LF/CR, U+FFFE/U+FFFF and unpaired surrogates replaced by U+FFFD; C1 kept (**Revised 2026-10-01 (6)**: extended from the unmapped path to every source). **Revised 2026-10-02 (10b)**: sanitising runs **after** T26's ToUnicode rules, which already turn a single-code-point C0-whitespace target into U+0020 and ignore other single-code-point control targets, so sanitising affects ToUnicode only through multi-code-point entries. | Oracle emits raw codes incl. NUL from any source; PostgreSQL `text` rejects NUL regardless of where it came from; LF/CR/TAB are kept because the oracle emits them and T33 allows them. |
| D-11 | `max_document_decoded_bytes` dropped from this run | Cumulative counting measures work not memory; needs decode caching upstream. |
| D-12 | The object-nesting cap is a fixed constant (256) in the fork, not a `Limits` field in this run; it reports through a thread-local hit counter in hayro-syntax (`object_depth_limit_hits()`), sampled by core around each synchronous operation (**Revised 2026-10-01 (2)**) | `Reader` has no access to per-document settings and content operands are read with no context at all, so neither a per-document counter nor a `Limits` field can see them; a process-wide knob would let documents interfere; the thread-local costs nothing on the hot path and core never crosses threads inside an operation. hayro-postscript cap hits are not counted (`no_std`; a failed CMap surfaces as a font without ToUnicode). |
| D-13 | Per-stream decode over-limit is a hard decode failure, not truncation | Truncated fonts/CMaps are garbage; absence plus a warning is the honest outcome. |
| D-14 | Page index deduplicates leaves and skips cycles (KP10); oracle differs | Bounded memory and no exceptions during iteration; a duplicated leaf is a malformed file. |
| D-15 | Source buffer is `Arc<Vec<u8>>` | hayro's `PdfData: From<Arc<T>>` requires `T: Sized`; `Arc<[u8]>` does not compile (verified). |
| D-16 | The Rust API does not catch panics; the Python boundary does | Idiomatic Rust; `catch_unwind` at every Rust call would hide bugs from Rust users and cost nothing to add later. |
| D-17 | The full §9.2 exception hierarchy including OCR classes is defined now | Stable `__all__`; the `RuntimeError` invariant test lands once. |
| D-18 | No `render` crate stub | Nothing to verify; one more matrix cell. |
| D-19 | `extract()` deferred to the OCR run | It encodes the OCR policy; a text-only version would be a second API. |
| D-20 | `mmap` source deferred | Opt-in feature with a SIGBUS caveat; no consumer in this run. |
| D-21 | Oracle expectations are checked-in JSON keyed by SHA-256; corpus-C gate is exact match (or pinned snapshot for documented divergences); corpus-B gate is open-rate / routing / similarity thresholds | Library CI never needs PyMuPDF (L2); exactness on synthetic fixtures catches regressions; B thresholds reflect its font mix. |
| D-22 | Perf numbers on shared CI are report-only; correctness assertions (no pages built at open, bounded node resolutions, allocation bound) gate P1 | Shared runners are too noisy to gate on. |
| D-23 | Coverage gates: core ≥ 95% lines (cargo-llvm-cov), `pdf_extract` ≥ 95% (pytest-cov); vendored crates excluded | As close to 100% as practical; vendored upstream code is tested by its own suite. |
| D-24 | PyO3 pinned `pyo3 = "=0.29.x"` (the current 0.29 patch release, bumped only by a deliberate commit), `gil_used = false`, `py.detach` for all non-constant work, `panic = "unwind"` in all profiles; free-threaded CPython supported from **3.14t**, 3.13t not supported in this run (**Revised 2026-10-02 (8)**, user decision) | C3, R1. PyO3 0.29 fixes RUSTSEC-2026-0176/0177 but requires CPython ≥ 3.14 for free-threaded builds (`pyo3-ffi` `MIN_FREE_THREADED_VERSION = 3.14`); 0.28.3 would need two advisory exceptions in `deny.toml`, which the user declined. Deliberate deviation from design-spec C3 ("3.13t and later"); revisit 3.13t only if a consumer needs it. |
| D-25 | Corpus B is hayro-tests custom files + PDFBox test resources; pdf.js excluded by default | Clear redistribution licenses; pdf.js test files carry no per-file license. |
| D-26 | `max_page_tree_depth` defaults to 256 (**Revised 2026-10-01**) | The oracle has no cap; 256 exceeds any real tree and bounds the walk; divergence beyond it is documented (KP23). |
| D-27 | Hostile fixtures are gated on outcome class, the expected warning-kind set and our own pinned text (snapshot), never on the oracle's text (**Revised 2026-10-01**; **Revised 2026-10-02 (10)**: "where it matters" became "always" in unit 13 — stricter than the residual wording and kept) | The oracle's output for over-deep input depends on placement and sometimes raises; matching it is not a goal. |
| D-28 | No optional-content switch in `TextOptions` (**Revised 2026-10-01**) | Both the oracle (from disk) and hayro respect `/OCProperties`; a `ShowAll` mode would be an untested behaviour addition. |
| D-29 | Nesting-cap and form-cycle events are counted in the shared budget state and read through `Context::stats()` (**Revised 2026-10-01 (2)**) | hayro's `warn!` is a no-op without `logging`; the budget state already crosses nested contexts and is always allocated, so it is the one place every interpreter event can report to. |
| D-30 | Units 04–07 assert hayro-level observables only; `Warning`, `page_count` and `Page::text` assertions belong to units 08–10 (**Revised 2026-10-01 (2)**) | A unit's oracle must be checkable with the code that exists when it lands; the unit table already said 04–07 depend only on 03. |
| D-31 | A dangling `/Pages` reference — and, **Revised 2026-10-01 (4)**, a `/Pages` root without `/Kids` or that is not a dictionary — opens with `page_count == 0` plus warnings when the brute-force scan finds nothing; a missing `/Pages` key stays `Corrupt` (**Revised 2026-10-01 (2)**) | H1 removed hayro's failure for an unbuildable tree; core resolves `pages_root()` itself, and the oracle is lenient about missing page trees (it opens a catalog without `/Pages` with 0 pages). |
| D-32 | The wall-clock guarantee is the deadline, not the operator cap; tests assert op-cap exhaustion functionally and bound time only through `page_time_budget` (**Revised 2026-10-01 (3)**) | hayro's interpretation speed (≈0.9M ops/s release, ≈60k/s debug) makes any seconds-per-op-count claim platform- and profile-dependent; the deadline check every 4,096 operators is what actually bounds time. Defaults unchanged. |
| D-34 | H9 makes hayro's lazy xref repair panic-free and idempotent, with the scan outside the lock, rather than forcing an eager repair in core (**Revised 2026-10-01 (5)**) | Any later object lookup can trigger a repair, so core cannot prove one never happens after open; the fix belongs where the lock is. |
| D-35 | `Pdf::is_encrypted()` accessor instead of a second parse with a bogus password (**Revised 2026-10-01 (5)**) | One line in the fork saves 1–17 ms per open and a code path that only exists to observe an error. |
| D-37 | One bundled fork unit 09b (H10) for the signals unit 09 found missing, instead of residuals or several units (**Revised 2026-10-01 (6)**) | Each gap (lexer abandonment, leaked `BDC`, per-`BDC` re-parse, invisible form/annotation boundaries, uncounted font failures, misattributed decode events, no cancel) is a few lines in hayro and unobservable from core; one patch keeps the fork diff reviewable. |
| D-38 | The char cap cancels interpretation and returns the partial page; an earlier budget exhaustion stays an error (**Revised 2026-10-01 (6)**) | Returning the 2M characters already collected is what T34 promised; cancelling stops the CPU spend (R4) instead of letting the page run on to a later error. |
| D-40 | Our own code is GPL-3.0-or-later; dependencies stay permissive-only under L1; delivered as unit 12b with `publish = false` + `[licenses.private] ignore = true` rather than an allow-list exception (**Revised 2026-10-02 (9)**, user decision) | The user's licensing choice for the project; keeping the dependency allow-list untouched preserves L1's guarantee and the negative test proves it; a scoped exception would have to be edited for every new crate, whereas `private` covers all unpublished workspace members by construction. |
| D-39 | AGL table = Adobe single-code-point entries only; no TeX names; core's chain owns every mapping step via raw `glyph_name()` and `Glyph::to_unicode()` (**Revised 2026-10-01 (6)**, user decision) | The TeX list's provenance is GPL-2 (L1); the oracle leaves TeX names raw anyway. |
| D-36 | Non-regular paths are rejected before reading; header must be wholly within 1024 bytes; `from_bytes` enforces `max_file_size`; `/Count` ignored entirely; brute-force scan skipped after a depth cut; repeated warnings coalesced with a count (**Revised 2026-10-01 (5)**, unit 08's interpretations accepted) | Each closes a blocking or unbounded path (FIFO read, lying `/Count` as allocation hint, warning floods) or removes an inconsistency between T-rows and §4.8. |
| D-33 | H3 applies the 50-level nesting cap to Type 3 glyphs, tiling patterns and soft masks; the text device never runs Type 3 procedures (**Revised 2026-10-01 (3)**) | Upstream stores the depth in each but checks it only for forms; a self-referential Type 3 font overflows a 512 KiB stack under a glyph-running device. The text path is unaffected by construction; the renderer (OCR run) needs the fix. |

---

## 7. Open questions for the user

Only questions that change the work. Each has a default the units follow
unless told otherwise.

| # | Question | Proposed default |
|---|---|---|
| Q1 | Project license for our own code (needed for `Cargo.toml`, `pyproject.toml`, LICENSE files)? | **Resolved: MIT** on 2026-10-01; **superseded 2026-10-02 (9): GPL-3.0-or-later** for our own code (unit 12b, D-40). Dependencies stay permissive-only (L1). Vendored hayro keeps its own Apache-2.0 OR MIT notices. |
| Q2 | Corpus B sources: hayro-tests custom + PDFBox only, or also pdf.js files after per-file license checks? | hayro-tests custom + PDFBox only (D-25). |
| Q3 | Python matrix for this run's CI? | 3.11, 3.13, 3.14, 3.14t on Linux; 3.13 on macOS. Wheels/abi3 are Phase 3. **Revised 2026-10-02 (8)**: resolved — 3.13t dropped with the PyO3 0.29 decision (D-24); follow-up for a later run: add 3.13t only if a consumer needs it (would require a PyO3 that supports it without the two advisories). |
| Q4 | KP14: match the oracle's duplication of Tr 4–6 text, or emit once? | Emit once (deliberate difference; a real document with clip-mode headings would otherwise get doubled text). |
| Q5 | `max_document_decoded_bytes`: accept dropping it from this run (D-11)? | Drop; revisit when image decoding (OCR run) needs a memory budget. |
| Q6 | Header search window: F2 says 1024 bytes; the oracle accepts a header at offset 1500. Keep 1024? | Keep 1024 (F2 as written); document the difference. |
| Q7 (**Revised 2026-10-02 (11)**) | A catalog whose `/Pages` is a **direct dictionary** rather than a reference is `Corrupt` for us (hayro's `XRef::new` requires the reference); the oracle probably opens it (unprobed). Accept, or add an H-change that tolerates a direct `/Pages`? | Accept for this run; probe the oracle and, if it opens such files, make it a small fork change in the next run. |
| Q8 (**Revised 2026-10-02 (11)**) | A Form XObject whose `/Resources` is an indirect reference to the *page's* own resources object reads as empty in hayro (named `/Properties` resolve to `None`; probably the resolution cycle guard) — seen during unit 09b, not investigated. Fix in the fork now, or carry as a residual? | Carry as a residual in this run (no corpus-B file hits it); investigate in the next run. |

---

## Appendix A: Oracle probe evidence

PyMuPDF 1.28.2 (MuPDF 1.28.2), CPython 3.13.15, macOS 14 arm64, 2026-10-01,
used strictly as a black box through `open`, `new_page`, `insert_text`,
`get_text('text')`, raw PDF bytes, and `save(encryption=…)`. Helvetica 11 pt
unless stated; gaps measured from `get_text_length`.

### A.1 Line assembly

- Space gap, 11 pt: 0.5, 1, 1.25, 1.5 → `AlphaBeta`; 1.75–4 → `Alpha Beta`. 22 pt: ≤ 3.25 → joined; 3.5, 3.75, 4 → space.
- Same-baseline break, 11 pt: 6, 8, 8.5, 8.7 → space; 8.8–28 → new line. 22 pt: ≤ 17.5 → space; 18–28 → new line.
- Backward movement, 11 pt (`Beta` placed at pen − b): b = 1–8.5 → `AlphaBeta`; 9–26 → `Alpha\nBeta`. `Right` at x=300 then `Left` at x=100 → two lines.
- Superscript (perpendicular), base 11 pt: 7 pt raised 2–5 → `Basesup`; 6–12 → new line. 11 pt raised 2–8.8 → joined; 9–12 → new line. 7 pt lowered 2, 4 → joined; 6–10 → new line.
- Diagonal offsets (along, up) at 11 pt: (5,5) (6,5) (6,6) (7,5) (5,7) (7,7) (8,3) (3,8) (8.5,1) (8.5,2) (2,8.5) → `Base sup`; (1,8.5) → `Basesup`. Independent along/perp thresholds, not Euclidean.
- Mixed sizes: 20 pt `Big` then 8 pt `small`: gap 0 → `Bigsmall`; 2, 4 → `Big small`; 8 → two lines. 8 pt `small` then 20 pt `Big`: 1, 1.5, 2 → `smallBig`; 3–12 → `small Big`; 16, 18 → two lines. Thresholds scale with the **incoming** glyph.
- `Tc 1.5` → `Hello`; `Tc 2` → `H e l l o`; `Tc 9` → `H\ne\nl\nl\no`. `Tw 20` on `a b c` → `a \nb \nc`. `[(A) -300 (B) -2000 (C) 500 (D) -9000 (E)] TJ` → `A B\nCD\nE`. `Tz 50` then `1 0 Td` → `Alpha\nBeta`.
- Fake bold: `Bold` twice at +0.3 pt, at +0.3/+0.3, and exactly overprinted → `Bold\nBold\n` each time.
- Whitespace: `Hello   ` then `World` → `Hello   \nWorld\n`; `   Lead\n`; `A\tB\n`; `A B` (WinAnsi 0xA0) → `A B\n`.
- `Alpha ` (trailing space) then gap 3 then `Beta` → `Alpha Beta\n` (no second space).
- Two lines in one block and a block 200 pt lower → `Line one.\nline two.\nBlock two.\n`. Empty page → `''`. Font size 0.001 → `Zero\n`. `/Rotate 90` → same text as unrotated. 90° text matrix → own line.
- Vertical (`Identity-V`, Japan1, non-embedded): CIDs 34,35,36 → `A\nB\nC\n`.
- **Revised 2026-10-02 (10)** — **Comparison operator and float noise:** the same 8 pt `small` + gap 3 pt + 20 pt `Big` (3/20 = 0.15 em) gives `small Big` at x = 72, 100, 250 (rawdict gap exactly 3.0) and `smallBig` at x = 10, 33.3, 499.9 (gap 2.9999962); 8→20 pt gap 16 (0.8 em) breaks at exactly 16.0 and joins with a space at 15.99999; 11 pt gap 8.8 likewise. The oracle compares with `>=` at 0.15 and 0.8 em and its f32 placement decides hair-under cases.
- **Revised 2026-10-02 (10)** — **KP24, space suppression** (Type 3 glyph names over the Unicode whitespace set, each before and after a 0.3 em gap): as the incoming glyph only U+0020 suppresses the synthetic space; as the line's last character U+0020, U+1680, U+3000 and U+FEFF suppress it; TAB, LF, VT, FF, CR, U+0085, U+00A0, U+2000–U+200A, U+2028, U+2029, U+202F and U+205F suppress nothing on either side.
- **Revised 2026-10-02 (10)** — a ToUnicode entry mapping to U+0085 is ignored (glyph-name fallback); entries mapping to U+00A0 and U+2028 are honoured. **Revised 2026-10-02 (10b)** — full sweep (`A`, code 0x78 → target, `B`; Type 1 Helvetica, non-embedded TrueType Arial, Type 0 Identity-H Arial with descriptor): single-code-point targets U+0000–U+0007 → `AxB` (ignored, glyph name); U+0008–U+000D → `A B` (space glyph in rawdict); U+000E–U+001F, U+007F, U+0080–U+009F → `AxB`; U+00A0 → `A\xa0B`, U+2028 → `A\u2028B`. Multi-code-point: `<0041 0009>` → `AA\tB`, `<0009 0041>` → `A\tAB`, `<0041 0085>` → `AA\x85B`, `<0041 0000>` → `AA\x00B`, `<0041 000A 0042>` → `AA\nBB`, `<0041 00A0>` → `AA\xa0B` (verbatim). With a 0.3 em gap: TAB/LF targets → `A B` as incoming and as last; U+0085 → `A xB` / `Ax B`; U+00A0 → `A \xa0B` / `A\xa0 B`; U+2028 likewise.

### A.2 Clipping

- **Revised 2026-10-01** — CropBox `[100 100 400 700]` on 595×842, text placed in **absolute** PDF coordinates: `BelowCrop` (150,50), `RightOfCrop` (450,300), `AboveCrop` (150,750) **dropped**; `InsideCrop` (150,300), `Straddle95`, `Straddle105` kept. CropBox `[100 -100 400 1000]` (beyond the MediaBox): y=50 kept, y=850 dropped. So the clip is CropBox ∩ MediaBox, per glyph. (The 2026-10-01 first-pass probe used `page.insert_text` after `set_cropbox`, whose coordinates are relative to the CropBox, and wrongly concluded the CropBox does not clip; unit 03's `builder_crop_rotate` with text outside the crop gives `''`.)
- MediaBox edges: baseline y=840 → kept; y=845 → kept; y=850 → dropped; `x590` at x=590 → `x` only; `xneg5` at x=−5 → kept; `y2` → kept; `yneg3` → dropped.
- `0 0 150 842 re W n`, `Straddle` at x=140 → `St`; `ab` at 140 + `cd` at 160 → `ab`.
- Diamond clip (bbox 100–200): `Corner` at (102,102) (outside the path, inside the bbox) → kept; `Mid` kept.
- `7 Tr` 40 pt `CLIP` at (100,700); then `0 Tr` `inside` at (100,705) and `outside` at (300,300) → `CLIP\ninside\n`.
- Form XObject `/BBox [0 0 50 50]`: `OutsideBBox` at (100,600) dropped; `InBBox` kept.

### A.3 Content and structure

- Render modes 0–3 → text once; 4, 5, 6 → text **twice** (`Mode4\nMode4\n`); 7 → once.
- **Revised 2026-10-01** — Hidden OCG: an **unsaved in-memory** document built with `add_ocg(on=False)` + `insert_text(oc=…)` gives `VisibleOCG\nHiddenOCG\nPlain\n` (hidden text included); the same document saved and reopened gives `VisibleOCG\nPlain\n`. Files from disk: OFF via `/OC /name BDC` → excluded; form XObject `/OC` → excluded; OCMD over an OFF group → excluded; `/BaseState /OFF` → excluded; OCG referenced in `/Properties` but no `/OCProperties` in the catalog → **included**. Unit 03's `builder_ocg_actualtext` gives `VisibleOCG\n…` without `HiddenOCG`, and its `builder_forms` drops `FormOCG`.
- Annotations: FreeText with appearance → included after body; `PDF_ANNOT_IS_HIDDEN` → excluded; `PDF_ANNOT_IS_NO_VIEW` → excluded; Widget with `/N` state dictionary and `/AS` → excluded; Popup with `/AP` → excluded; Link with `/AP` → excluded; FreeText **without** `/AP` (only `/Contents` + `/DA`) → `NoAppearance` **included** (synthesised); Stamp with `/Matrix`/`/BBox` scaling → `Scaled` included.
- Form XObject order: `Before\nInForm\nAfter\n`. Self-recursive form → `Rec` once. Fan-out 2^22 → 22.6 s, 16.8M characters (no budget). **Revised 2026-10-01**: a linear chain of 60 nested forms (unit 03 `nested_forms_60`, each form's bbox far larger than the page) → all 60 levels `L0…L59` extracted.
- Damaged content (`) ) ] >> garbage` between text objects) → `GoodOp\nPartial\nAfterGarbage\n`. `/Contents` array → `A\nB\n`.
- Missing font resource → `NoFontRes\n`; no `Tf` at all → `''`.

### A.4 Unicode

- ActualText inline `(Hello)` over `H e l l o` → `Hello world`; `<FEFF00660069>` over one glyph → `fine`; named property → `ABC`; empty `()` → span dropped (`kept`); `(abcdef)` over 3 glyphs → `abcdef`; `(ab)` over 5 → `ab`; `(a)` over 3 → `a`; `(a b)` over 3 glyphs → `a b`; `(a b)` over 2 glyphs 40 pt apart → `a \nb` (**Revised 2026-10-02 (10)**: unit 13's `kp09_gap` recipe spaces the glyph *starts* 40 pt apart and gets `a\n b`; divergence from our `a b` either way); `(joined)` over `ab` / newline / `cd` → `jo\nine d`; nested `(OUTER)[a (IN)[b] c]` → `OUTER INUTER`; span with no glyphs → nothing. **Revised 2026-10-01**: `(Hello)` over a single `(H e l l o) Tj` (9 glyphs incl. spaces; unit 03 `builder_ocg_actualtext`) → `Hel\nl o` (KP17).
- Type 3 without ToUnicode, codes 65 (`/g65`), 66 (`/whitesquare`; **Revised 2026-10-01 (6b)**: the fixture was renamed from `/square`, which the oracle also maps to U+25A1 but which is not in our AGL table), 0, 1, 9, 10, 32, 200 → U+0041, U+25A1, U+0000, U+0001, U+0009, U+000A, U+0020, U+00C8.
- Type 1 Helvetica `/Differences [65 /zzunknown 66 /uni2603 128 /g128]` → U+0041, U+2603, U+0080.
- Type 0 Identity-H, non-embedded `Arial`, no ToUnicode, CIDs 0x24 0x25 0x26 0x03 0x44 → `ABC a` **when the CIDFont has a `/FontDescriptor`**. **Revised 2026-10-01**: without one the oracle emits the raw bytes (`\x00$\x00%\x00&\x00\x03\x00D`), and the Japan1 `90ms-RKSJ-H` case likewise degrades to raw bytes without a descriptor (KP22).
- Type 0 `90ms-RKSJ-H`, Japan1, non-embedded, no ToUnicode → `日本語AB`; Identity-H with Japan1 ordering CIDs 34–36 → `ABC`.
- Encodings: WinAnsi `a\xa0b\xadc` → `a b-c`; MacRoman `a\xcab` → `a b`; no `/Encoding` (Standard) → `a b›c`; Symbol `abg` → `αβγ`.

### A.5 Page tree and opening

- `/Count 1` with two kids (second typeless) → `page_count 1`, one page. Duplicate leaf → 2 pages, both `PageOne`. Kids `[3 0 R 2 0 R]` (self-cycle) → `page_count 2`; loading page 2 raises `FzErrorFormat: cycle in page tree`. Deep single-chain page trees (**Revised 2026-10-01**; the first-pass generator had mis-numbered the content object): depth 64, 200, 256, 500, 1000, 3000, 10,000 and 20,000 all give one page with `DeepLeaf` — the oracle has no depth cap. Inherited `MediaBox [0 0 200 100]` and `Resources` honoured; text 200 pt below dropped.
- `%PDF-1.4\ngarbage` → `FileDataError`; catalog without `/Pages` → opens, 0 pages; empty `/Kids` → 0 pages; header at offset 500 and 1500 → opens; broken `startxref` → repaired; truncated tail → repaired.
- Deep nesting (**Revised 2026-10-01**, placement matters): 200,000-deep array as a `TJ` operand inside `BT … ET` → `''`; the same array as a bare operand before or after a text object → the oracle **raises** `RuntimeError: code=5: exception stack overflow`; unit 03's `(Before) Tj <20,000 deep> TJ (After) Tj` → `Before`; 200,000 deep in the page dictionary → `''`; 200,000 deep inside an unused `/ExtGState` entry of the resources → `''`; 20,000 deep beside `/Font` in an indirect resources dictionary (unit 03) → `Res`; 200,000 deep inside the font dictionary itself → `Res`; unit 03's 2,000-deep Type 4 function → `Fn`.

### A.6 Encryption (fixtures generated with `save(encryption=…)`)

RC4-40, RC4-128, AES-128, AES-256: owner-only → `needs_pass 0`, text extracted; user password → `needs_pass 1`, `is_encrypted True`, `page_count 1`, page access raises `ValueError: document closed or encrypted`.

### A.7 Same fixtures through hayro 0.7 (release build, 512 KiB thread stack)

- Owner-only × 4 → opens, 22–23 glyphs with correct Unicode and AFM advances; user × 4 → `Decryption(PasswordProtected)`.
- `notf.pdf` → 4 glyphs emitted (Helvetica fallback). `garbage.pdf`, `nopages.pdf` → `Invalid`. `emptykids.pdf` → 0 pages. `tree_cycle.pdf` → 1 page. `tree_dup_leaf.pdf` → 2 pages. `form_self_recursion.pdf` → 150 glyphs (50 nesting levels × 3). `flate_bomb_content.pdf` (50 MiB decoded) → 30 ms, 0 glyphs. `many_ops.pdf` (2M operators) → 84 ms.
- **Aborts with stack overflow:** `deep_tree.pdf` (depth 3000) at 512 KiB (opens at 8 MiB in 512 ms; the recursion is in `resolve_pages`); `deep_obj.pdf`, `deep_content.pdf`, `deep_resources.pdf` (200,000 nesting) at **both** 512 KiB and 8 MiB.

## Appendix B: hayro facts verified against source

Facts that differ from or sharpen `design-spec.md`:

1. `hayro-syntax 0.7.2` depends on `hayro-jpeg2000 =0.3.5`, not 0.4.0; `hayro-interpret 0.7.0` additionally depends on `hayro-postscript 0.1.0` (Type 4 functions). Vendor and pin accordingly.
2. The object parser has **no nesting-depth cap** (`Array::read/skip`, `Dict::read/skip`, `Object::read` recurse; `hayro-postscript/src/object.rs` carries a TODO about `<<<<` overflow). §8.6's "nested PDF objects while parsing, capped by `max_object_depth`" describes something that does not exist upstream; H2a adds it.
3. `Pages::new` / `resolve_pages` is **recursive**; a 3,000-deep tree overflows a 512 KiB stack inside `Pdf::new`. H1 (lazy) plus our iterative walk removes this from the open path.
4. `PdfData` implements `From<Arc<T>>` only for `T: Sized`; `Arc<[u8]>` does not compile (§5.1 says `Arc<[u8]>`). Use `Arc<Vec<u8>>`.
5. `LoadPdfError` is `{ Decryption(DecryptionError), Invalid }`; `DecryptionError` is `{ MissingIDEntry, PasswordProtected, InvalidEncryption, UnsupportedAlgorithm }`. A missing `/ID` is tolerated (empty ID assumed).
6. `Device::draw_glyph(glyph, transform /* CTM */, glyph_transform /* Tm·[Tfs·Th,0,0,Tfs,0,Ts]·(1/1000)·origin_displacement */, paint, mode)`; `GlyphDrawMode` is `{ Fill, Stroke(StrokeProps), Invisible }`. Text render mode 7 never reaches the device (clip only); modes 4–6 reach it once. `begin_marked_content(tag, mcid)` has no properties parameter (confirms H4).
7. `OutlineGlyph.char_code` is `pub(crate)`; `advance_width()`, `as_unicode()`, `font_cache_key()`, `font_data()` are public. Fonts do not retain their dictionary (only `cache_key`), so H5 exposes derived facts rather than a dictionary reference (deviation from §3.3 H5 wording).
8. `Type0Font::new` already loads the ordering's UCS2 CMap when `to_unicode` is absent — but only for non-embedded (`fallback`) fonts. H6a removes that condition.
9. `show_glyph` returns early when `ocg_state.is_visible()` is false, so hidden-OCG text never reaches a device. **Revised 2026-10-01**: the oracle excludes it too for files read from disk, so this is the wanted behaviour and no fork change is needed.
10. `interpret_page` skips annotations with flag bit 2 (Hidden) only; it needs `/AP /N` to be a stream (state dictionaries are skipped). It does not check NoView or subtype (H8c).
11. Nested interpretation (`Type3::render_glyph`, `TilingPattern::interpret`, `SoftMask::interpret`) creates a fresh `Context` via `Context::new_with(..., glyph.settings.clone(), ...)`; an op counter must live in shared state inside `InterpreterSettings` to survive this (H3 design).
12. `InterpreterCache.font_cache` is `Rc<RefCell<HashMap<u128, Option<Font>>>>` (confirms §3.2 item 1); `Cache` (object cache) is `Arc<Mutex<…>>`.
13. `MAX_XREF_CHAIN_DEPTH = 256` (confirms §5.3); `hayro-cmap` `MAX_NESTING_DEPTH = 16` for `usecmap`; `MAX_NESTED_INTERPRETATION_DEPTH = 50` (confirms §3.2 item 6).
14. `FormXObject::new` decodes the whole content stream on every draw (no cache), so repeated forms re-decode; bounded by the op budget, relevant to §2.4.
15. hayro's `warn!`/`error!` expand to nothing unless the `logging` feature is on; with it they go through the `log` crate, not stderr. T48 holds without changes.
16. `Resources::new` and `ReaderContext::new` are `pub(crate)`; `Dict::get_ref`, `Dict::obj_id`, `Array::raw_iter`, `ObjRef`, `XRef::get`, `XRef::root_id` are public — enough for the index walk; `Page::from_dict` (H1) is needed only to build a page for interpretation.
17. The `unsafe` feature of hayro-syntax selects `flate2` (pure-Rust `miniz_oxide` backend by default), `memchr`, and SIMD in image decoders; `hayro-interpret` is `#![forbid(unsafe_code)]`.

Facts that confirm the design spec: `Pdf`, `Page` are `Send + Sync` (compile-time check passed); `Pdf::new` builds every page eagerly through `CachedPages`; owner-only encrypted files open and user-password files fail at `Pdf::new`; standard-14 fonts are embedded (Foxit) with AFM-compatible advances.

## Appendix C: Fixture catalogue

Generated fixtures (`tests/fixtures/corpus-c/`, by `mkfixtures.py` or the test kit; small enough to commit unless marked *runtime*):

| Group | Files | Notes |
|---|---|---|
| KP1 (**Revised 2026-10-02 (7)**: every row at least 0.001 em from 0.149; **Revised 2026-10-02 (10)**: the x-position sweep is a §4.10 unit test, not a fixture) | `kp01_space_11pt_{0.5,1,1.25,1.5,1.75,2,2.5,3,4}`, `kp01_space_22pt_{…}`, `kp01_mixed_20to8_{0,2,4,8}`, `kp01_mixed_8to20_{1,1.5,2,3,4,6,8,12,16,18}` | exact |
| KP2 (**Revised 2026-10-02 (7b)**; **Revised 2026-10-02 (10)**: the six-position sweeps are §4.10 unit tests, not fixtures) | `kp02_break_11pt_{6,8,8.5,8.7,8.8,9,12,28}`, `kp02_break_22pt_{16,17.5,18,28}` | exact |
| KP3 | `kp03_back_11pt_{1,4,8,8.5,9,12,26}`, `kp03_right_left` | exact |
| KP4 | `kp04_fakebold_{0.3,0.3x0.3,overprint}` | exact |
| KP5 | `kp05_trailing`, `kp05_leading`, `kp05_tab`, `kp05_nbsp_winansi`, `kp05_trailing_space_then_gap` (**Revised 2026-10-02 (11)**) | exact |
| KP6 | `kp06_type3_unmapped` (divergence: NUL/C0 → FFFD; code 66 is `/whitesquare`), `kp06_type3_square` (divergence: oracle U+25A1, ours raw; **Revised 2026-10-01 (6b)**), `kp06_type1_differences`, `kp06_type0_identity_nonembedded` | mixed |
| KP7 | `kp07_cropbox_clips`, `kp07_crop_beyond_media`, `kp07_mediabox_edges` (**Revised 2026-10-01**) | exact |
| KP8 | `kp08_freetext_ap`, `kp08_hidden`, `kp08_noview`, `kp08_popup`, `kp08_link`, `kp08_widget_states`, `kp08_stamp_matrix`, `kp08_freetext_no_ap` (divergence) | mixed |
| KP9 | `kp09_inline`, `kp09_ligature`, `kp09_named`, `kp09_empty` (exact); `kp09_cross_line`, `kp09_gap`, `kp09_nested` (divergence, KP17) | mixed |
| KP10 | `kp10_lying_count`, `kp10_dup_leaf`, `kp10_cycle` (divergence), `kp10_typeless_leaf`, `kp10_direct_kid`, `kp10_inherited` | mixed |
| KP11 | `kp11_hidden_ocg` (BDC, form `/OC`, OCMD, `/BaseState /OFF` variants), `kp11_ocg_no_ocprops` (**Revised 2026-10-01**) | exact |
| KP12 | `kp12_identity_v`, `kp12_vertical_then_horizontal` | exact |
| KP13/14 | `kp13_tr{0..7}` (4–6 divergence) | mixed |
| KP15 | `kp15_winansi`, `kp15_macroman`, `kp15_standard`, `kp15_symbol` (exact); `kp15_differences_nbspace`, `kp15_type3_nbspace` (divergence, **Revised 2026-10-01 (6)**) | mixed |
| KP16 | `kp16_identity_nonembedded_macorder` | exact |
| KP18 | `kp18_no_tf` (divergence), `kp18_missing_font_resource` | mixed |
| KP19 | `kp19_rect_clip`, `kp19_diamond_clip`, `kp19_tr7_clip`, `kp19_form_bbox` | exact |
| KP20 | `kp20_form_order`, `kp20_form_self_recursion`, `kp20_contents_array`, `kp20_damaged_content` | exact |
| KP21 | `kp21_nested_forms_60` (**Revised 2026-10-01**) | divergence |
| KP22 | `kp22_cid_no_descriptor` (**Revised 2026-10-01**), `kp22_cidfonttype0_macorder`, `kp22_cid_ge_258_raw` (**Revised 2026-10-01 (6)**) | divergence / exact / exact |
| perp (**Revised 2026-10-02 (7b)**; **Revised 2026-10-02 (10)**: position sweeps are §4.10 unit tests) | `perp_7pt_{2,5,6,12}`, `perp_11pt_{8.8,9}`, `perp_low_{4,6}`, `diag_{…}` | exact |
| KP24 (**Revised 2026-10-02 (10)**) | `kp24_w_{0020,0009,000A,000B,000C,000D,0085,00A0,1680,2000,2002,2003,2007,2009,200A,200B,2028,2029,202F,205F,3000,FEFF}_{before,after}`, `kp24_t3_{space,nbspace,uni0009,uni0085,uni00A0,uni2003,uni2028,uni3000}_{before,after}` | exact (**Revised 2026-10-02 (10b)**: the `kp24_w_{0009,000A,000B,000C,000D}_*` rows give `A B` because a single-code-point ToUnicode target in U+0008–U+000D is a space, and `kp24_w_0085_*` give `Ax B`/`A xB` because the entry is ignored and the glyph name `x` decides); under option (a) for T30 the U+00A0/U+2000–U+200A/U+2028/U+2029/U+202F/U+205F rows would be divergence |
| ToUnicode controls (**Revised 2026-10-02 (11)**; T26 10b rules) | `tounicode_ctl_{0000,0001,0007,0008,0009,000A,000B,000C,000D,000E,001F,007F,0080,0085,009F,00A0,2028,…}` (Type 1 Helvetica, 18 rows), `tounicode_ctl_tt_*` (non-embedded TrueType, 6), `tounicode_ctl_t0_*` (Type 0 Identity-H, 6; ignored entry → raw CID), `tounicode_ctl_t3_*` (Type 3, 6), `tounicode_multi_*` (multi-code-point entries incl. `<0041 0009>`, `<0041 000A 0042>`, `<0041 0000>`) | exact, except `tounicode_multi_*` with NUL (divergence, D-10) |
| spacing ops | `tc_{1.5,2,9}`, `tw_20`, `tj_adjust`, `tz_50` | exact |
| contract | `empty_page`, `zero_size_font`, `rotate_90`, `two_blocks`, `header_offset_500`, `header_offset_1500` (divergence), `broken_startxref`, `truncated_tail`, `catalog_no_pages`, `empty_kids`, `garbage_after_header`, `plain_text.pdf`, `valid.txt`, `valid_noext` (generated by `testkit::contract`, unit 08; **Revised 2026-10-01**) | mixed |
| encryption | `enc_{rc4_40,rc4_128,aes_128,aes_256}_{owner,user}` | exact (user: open outcome only) |
| hostile (committed) | `deep_obj_20k`, `deep_content_20k`, `deep_resources_20k`, `deep_type4_2k`, `deep_tree_200`, `flate_bomb_8mib`, `many_ops_200k`, `form_fanout_12`, `form_self_recursion`, `nested_forms_60` | open/extract outcome + expected warning-kind set + **our** pinned text (snapshot), never the oracle's text (D-27; **Revised 2026-10-02 (10)**); expected sets include `XrefRepaired` for `deep_obj_20k` and `deep_resources_20k` (lazy repair after the unreadable object) |
| hostile (*runtime*) | 200,000-deep variants, `deep_tree_3000`, `flate_bomb_50mib`, `many_ops_2m`, `form_fanout_22`, 100k/1M-page trees | generated by tests/benches; never committed |

Corpus B (`tests/fixtures/corpus-b/`): ≤ 60 files, chosen in unit 14 from
hayro-tests custom files and PDFBox test resources containing text, with
preference for embedded TrueType/CFF simple fonts, CID fonts (embedded and
not), Type 3 fonts, rotated pages, annotations, damaged xrefs, and encrypted
files; each listed in `PROVENANCE.md` with origin, commit, license and SHA-256.
