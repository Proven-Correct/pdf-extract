# text-extraction Specification

## Purpose

Extract a page's text layer into an immutable `TextPage` and serialise it in the compat shape of PyMuPDF 1.28.2's `get_text('text')`, with every deliberate difference from that oracle stated as behaviour.

## ADDED Requirements

### Requirement: Text entry points
The system SHALL expose text extraction as `Page::text_page(&TextOptions) -> Result<TextPage, PageError>` and `Page::text(&TextOptions) -> Result<String, PageError>` in Rust, and as `Page.text(*, block_separator="newline", unmapped="cid") -> str` and `Page.text_page(*, unmapped="cid") -> TextPage` in Python. `Page.text` MUST equal serialising `Page.text_page` with the same options.

#### Scenario: text equals the serialised text page
- **WHEN** a caller calls `page.text()` and `page.text_page().text()` on the same page with default options
- **THEN** both return the same string [F7, F9; 0001 T19, T33]

#### Scenario: default options are the compat profile
- **WHEN** `TextOptions::default()` is constructed
- **THEN** `unmapped` is `Cid`, `block_separator` is `Newline`, and the layout thresholds are `space_gap` 0.149, `perp_break` 0.809, `gap_break` 0.798, `back_break` 0.795, `block_gap` 1.5, `block_indent` 2.0, `angle_tolerance_deg` 2.0 [§6.4; 0001 §3.3]

#### Scenario: invalid Python option strings are rejected
- **WHEN** a Python caller passes `block_separator="paragraph"` or an `unmapped` value other than `"cid"`, `"replacement"` or `"skip"`
- **THEN** a `ValueError` naming the accepted values is raised [§12.2; 0001 §3.5]

#### Scenario: layout thresholds are not exposed to Python
- **WHEN** a Python caller inspects the `Page.text` and `Page.text_page` signatures
- **THEN** neither accepts layout thresholds; tuning `LayoutParams` is available only through the Rust `TextOptions` [0001 §3.5]

### Requirement: Compat serialisation shape
The system SHALL serialise a `TextPage` by writing each line's characters followed by `'\n'`, including the last line. Under the default `Newline` separator nothing extra is written between blocks. A page with no extracted characters MUST serialise to the empty string.

#### Scenario: every line ends in a newline
- **WHEN** a page holds two lines `Hello` and `World` in one block
- **THEN** `text()` returns `"Hello\nWorld\n"` [F7; 0001 T33]

#### Scenario: empty page
- **WHEN** a page has no text (or all of it is filtered out)
- **THEN** `text()` returns `""` under either block separator [F7, §6.5; 0001 T33]

#### Scenario: no blank line between blocks in compat mode
- **WHEN** a page holds two blocks and `block_separator` is `newline`
- **THEN** the output contains no `"\n\n"` produced by the block boundary [F7, §6.5; 0001 T33]

#### Scenario: line feeds inside a line come only from mapping
- **WHEN** a line's text contains `'\n'`
- **THEN** it was produced by a mapping step that yielded LF (an unmapped LF code, a multi-code-point ToUnicode entry, a `uni000A` glyph name, or an `/ActualText` string), as the oracle also emits it [0001 T33]

### Requirement: Blank-line block separator
The system SHALL, when `block_separator` is `blank_line` (`BlockSeparator::BlankLine`), write one additional `'\n'` before every block except the first, leaving line termination unchanged.

#### Scenario: blank line between blocks
- **WHEN** a page with blocks `["A"]` and `["B"]` is serialised with `blank_line`
- **THEN** the result is `"A\n\nB\n"` [§6.5; 0001 T32, T33]

#### Scenario: single block unaffected
- **WHEN** a page with one block is serialised with `blank_line`
- **THEN** the result equals the `newline` serialisation [§6.5; 0001 T33]

### Requirement: Interpreted content and output order
The system SHALL extract glyphs from the page's content streams, the Form XObjects they draw (recursively), and the `/AP /N` appearance streams of annotations that are not Hidden, not NoView, and not of subtype `Popup` or `Link`. Glyphs MUST be taken in content-stream order with annotation text following page content.

#### Scenario: annotation appearance text follows page content
- **WHEN** a page has body text and a visible FreeText annotation with an appearance stream
- **THEN** the annotation's text appears after the body text [§6.1, KP8; 0001 T19, §1.8 KP8]

#### Scenario: hidden, NoView, Popup and Link annotations are skipped
- **WHEN** an annotation is flagged Hidden or NoView, or has subtype `Popup` or `Link`
- **THEN** its appearance text is not extracted [0001 T19; ADR-0043]

#### Scenario: annotation without an appearance stream
- **WHEN** a FreeText annotation has `/Contents` and `/DA` but no `/AP`
- **THEN** nothing is extracted for it (the oracle synthesises an appearance; a documented divergence) [0001 §2.5]

#### Scenario: widget with a state-dictionary appearance
- **WHEN** a widget's `/AP /N` is a state dictionary rather than a stream
- **THEN** it contributes no text [0001 §2.5]

#### Scenario: no bidirectional reordering
- **WHEN** a page holds Arabic or Hebrew text drawn in visual order
- **THEN** glyphs are emitted in content-stream order without reordering into logical order (a documented divergence) [0001 §2.5]

### Requirement: Type 3 glyphs and pattern content
The system SHALL treat each Type 3 glyph as one character and SHALL NOT extract text from Type 3 glyph procedures or from the content of tiling-pattern cells.

#### Scenario: Type 3 glyph is the character
- **WHEN** a Type 3 font shows a glyph whose procedure itself shows text
- **THEN** only the Type 3 glyph's own mapping is emitted, not the procedure's inner text [§6.1; ADR-0029]

#### Scenario: text inside a tiling pattern
- **WHEN** a tiling pattern's cell content shows text
- **THEN** that text is not extracted (a documented divergence) [0001 §2.5]

### Requirement: Render modes
The system SHALL capture text in render modes 0 through 7 exactly once per show, including invisible text (mode 3) and clip-only text (mode 7). Text in modes 4 through 6 MUST be emitted once even though the oracle emits it twice (KP14, a deliberate difference).

#### Scenario: invisible text is kept
- **WHEN** a page shows `Hidden` in render mode 3
- **THEN** `text()` contains `Hidden` [F7, §6.1, KP13; 0001 T20]

#### Scenario: fill-and-stroke text is not duplicated
- **WHEN** a page shows `Bold` once in render mode 2 or 6
- **THEN** `text()` contains `Bold` once [0001 T20]

#### Scenario: modes 4 to 6 emitted once
- **WHEN** a page shows `Clip` once in render mode 4, 5 or 6
- **THEN** `text()` contains `Clip` once, where the oracle gives it twice [0001 T20, §1.8 KP14; ADR-0053]

### Requirement: Hidden optional content excluded
The system SHALL drop glyphs in optional-content groups that are OFF in the document's default configuration (`/OCProperties /D`), whether reached through `BDC`, a form's `/OC`, an OCMD, or `/BaseState /OFF`. An OCG referenced without any `/OCProperties` MUST be treated as visible. No option to show hidden content exists.

#### Scenario: OFF group dropped
- **WHEN** text is marked with an OCG that is OFF in `/OCProperties /D`
- **THEN** that text is absent from `text()` [§6.3, KP11; 0001 T21]

#### Scenario: OCG without OCProperties is visible
- **WHEN** content references an OCG but the catalog has no `/OCProperties`
- **THEN** the text is extracted [0001 T21, §1.8 KP11; ADR-0044]

#### Scenario: no show-all switch
- **WHEN** a caller builds `TextOptions`
- **THEN** there is no field that re-enables hidden optional content [0001 T21, §3.3]

### Requirement: Visible-box filtering per glyph
The system SHALL drop a glyph whose box lies entirely outside the visible box, CropBox intersected with MediaBox, in page space. A glyph whose box touches or partly overlaps the visible box MUST be kept. The test is per glyph, not per line.

#### Scenario: text outside the CropBox but inside the MediaBox
- **WHEN** a glyph lies inside the MediaBox but entirely outside the CropBox
- **THEN** it is dropped [F7, §1.3, KP7; 0001 T22]

#### Scenario: CropBox larger than MediaBox
- **WHEN** the CropBox extends beyond the MediaBox and a glyph lies in the part outside the MediaBox
- **THEN** it is dropped [0001 T22; ADR-0046]

#### Scenario: per-glyph test
- **WHEN** the run `x590` starts inside the visible box and its last three glyphs lie entirely outside it
- **THEN** `text()` gives `x` [0001 T22, §1.8 KP7]

#### Scenario: glyph partly inside
- **WHEN** an 11 pt Helvetica glyph's baseline sits 3 pt below the bottom edge of the page
- **THEN** it is kept, and one whose baseline sits 8 pt below is dropped [0001 T23, Appendix A.2]

### Requirement: Glyph box geometry
The system SHALL use as a glyph's box the span from its origin to origin plus advance along the baseline, and from its descent to its ascent across it, taking ascent and descent from the font's metrics (FontDescriptor, Type 3 `FontBBox × FontMatrix`, or standard-14 AFM values), and otherwise 0.72 em and -0.22 em.

#### Scenario: unusable descriptor metrics
- **WHEN** a FontDescriptor has a positive `/Descent` or a non-positive `/Ascent`
- **THEN** both values are discarded and the defaults 0.72 em / -0.22 em are used [0001 T23]

#### Scenario: zero-size glyphs kept
- **WHEN** a glyph with a Unicode value is drawn at font size 0
- **THEN** it is kept and joins the current line at the pen [§6.3; 0001 T25]

#### Scenario: geometry that does not fit f32
- **WHEN** a glyph's transform overflows `f32` or is NaN
- **THEN** the glyph is dropped rather than passed to line assembly [0001 T23]

### Requirement: Clip paths filter glyphs
The system SHALL drop a glyph whose box does not intersect the bounding box of the intersection of the active clip paths, approximating each non-rectangular clip by its bounding box. Form XObject `/BBox` clips and text-clip render modes MUST take part.

#### Scenario: rectangular clip
- **WHEN** a glyph lies entirely outside an active rectangular clip
- **THEN** it is dropped [0001 T24, §1.8 KP19]

#### Scenario: non-rectangular clip uses its bounding box
- **WHEN** a glyph lies inside the bounding box of a diamond clip but outside the diamond
- **THEN** it is kept, as the oracle does [0001 T24, Appendix A.2]

#### Scenario: form BBox clips
- **WHEN** a Form XObject shows text outside its `/BBox`
- **THEN** that text is dropped [0001 T24, §1.8 KP19]

### Requirement: ToUnicode mapping with control-character rules
The system SHALL map a glyph first through its font's ToUnicode CMap. A single-code-point entry in U+0008-U+000D MUST yield U+0020; a single-code-point entry that is any other control (U+0000-U+0007, U+000E-U+001F, U+007F-U+009F) MUST be ignored and mapping continues as if absent; any other entry, and every multi-code-point entry, is taken verbatim.

#### Scenario: multi-code-point entry kept whole
- **WHEN** a ToUnicode entry maps one code to `ffi` as three code points
- **THEN** all three are emitted for that glyph [§6.2; 0001 T26]

#### Scenario: single control maps to space
- **WHEN** a ToUnicode entry maps a code to U+0009 or U+000A alone
- **THEN** the glyph is emitted as U+0020 [0001 T26; ADR-0049]

#### Scenario: other single control is ignored
- **WHEN** a simple font's ToUnicode entry maps a code to U+0001 alone
- **THEN** the entry is ignored and the glyph is mapped by the later steps (glyph name, or unmapped policy) [0001 T26; ADR-0049]

#### Scenario: ignored entry in a composite font
- **WHEN** a Type 0 font's ToUnicode entry for a CID maps to a single ignored control
- **THEN** the glyph is treated as unmapped (raw CID under `cid`), not mapped through the UCS2 or cmap steps [0001 §2.5 "Controls in ToUnicode"]

#### Scenario: ToUnicode entry mapping to U+FFFD
- **WHEN** a ToUnicode entry maps a code to U+FFFD
- **THEN** U+FFFD is emitted as written (the oracle falls back to the raw code; a documented divergence) [0001 §2.5]

#### Scenario: hayro's own name table never decides
- **WHEN** a font has a ToUnicode CMap that lacks a code
- **THEN** that code is mapped by the glyph-name, CID and unmapped steps of this specification, not by any other fallback [0001 T26]

### Requirement: Simple-font glyph names
The system SHALL map glyphs of simple fonts (Type 1, TrueType, CFF, Type 3) without a usable ToUnicode entry through the glyph name from `/Encoding` (base encoding plus `/Differences`), looked up in Adobe's single-code-point Glyph List or read as `uniXXXX` / `uXXXX[XX]`. TeX glyph names MUST NOT be mapped.

#### Scenario: Differences name mapped
- **WHEN** a Type 1 font's `/Differences` names code 65 `/Aacute`
- **THEN** the glyph is emitted as U+00C1 [§6.2; 0001 T26]

#### Scenario: uniXXXX name
- **WHEN** a Type 3 font's `/Differences` names a glyph `/uni0041`
- **THEN** it is emitted as `A` [§6.2; 0001 T26]

#### Scenario: TeX names stay unmapped
- **WHEN** a font without ToUnicode names a glyph `/parenleftbig` or `/square`
- **THEN** the glyph is unmapped and goes to the unmapped policy (for `/square` the oracle gives U+25A1; a documented residual) [0001 T26, §2.5; ADR-0051]

### Requirement: Base-encoding no-break normalisation
The system SHALL, for Type 1 and TrueType fonts without ToUnicode, emit WinAnsi 0xA0 (`nbspace`) and MacRoman 0xCA (`nbspace`) as U+0020 and WinAnsi 0xAD (`sfthyphen`) as U+002D. No other position, encoding or font kind is normalised (KP15).

#### Scenario: WinAnsi no-break space
- **WHEN** a WinAnsi Type 1 font without ToUnicode shows byte 0xA0
- **THEN** U+0020 is emitted [0001 T26, §1.8 KP15; ADR-0052]

#### Scenario: Type 3 and ToUnicode keep U+00A0
- **WHEN** a Type 3 font names 0xA0 `nbspace`, or a ToUnicode entry maps a code to U+00A0
- **THEN** U+00A0 is emitted unchanged [0001 T26, §1.8 KP15]

#### Scenario: Differences restating nbspace
- **WHEN** a WinAnsi font's `/Differences` names 0xA0 `nbspace`
- **THEN** U+0020 is emitted, where the oracle keeps U+00A0 (a documented divergence) [0001 §2.5, §1.8 KP15]

#### Scenario: StandardEncoding 0xA0
- **WHEN** a StandardEncoding font without ToUnicode shows byte 0xA0
- **THEN** the glyph is unmapped and goes to the unmapped policy [0001 T26]

### Requirement: CID font fallbacks
The system SHALL map CID-font glyphs without a ToUnicode entry through, in order: the `*-UCS2` CMap of a known ordering (Adobe-GB1, CNS1, Japan1, Korea1); for Identity encodings with an embedded TrueType/OpenType program, the reversed font `cmap`; for non-embedded Identity CIDFontType0/2, CIDs below 258 as standard Macintosh glyph-order names through the Glyph List.

#### Scenario: known ordering
- **WHEN** an Adobe-Japan1 CID font has no ToUnicode
- **THEN** glyphs map through the Japan1 UCS2 CMap, embedded or not [§6.2; 0001 T26; ADR-0042]

#### Scenario: embedded Identity font
- **WHEN** an Identity-H font with an embedded TrueType program has no ToUnicode
- **THEN** glyphs map through the reverse of the font's `cmap` [§6.2; 0001 T26; ADR-0042]

#### Scenario: non-embedded Identity font in Macintosh order
- **WHEN** a non-embedded Identity CIDFontType2 shows CIDs 36, 37, 38, 3, 68
- **THEN** `ABC a` is emitted [0001 T26, §1.8 KP16]

#### Scenario: CID beyond the Macintosh order
- **WHEN** such a font shows a CID of 258 or more
- **THEN** the glyph is unmapped and goes to the unmapped policy [0001 T26, §1.8 KP22]

#### Scenario: no FontDescriptor
- **WHEN** a non-embedded Identity CID font has no `/FontDescriptor`
- **THEN** the same Macintosh-order chain applies (the oracle emits raw string bytes; a documented divergence) [0001 §1.8 KP22]

### Requirement: Unmapped-glyph policy
The system SHALL apply the `unmapped` option to glyphs no mapping step resolves: `cid` (default) emits the raw character code (the CID for composite fonts) as a code point; `replacement` emits U+FFFD; `skip` emits nothing, leaving the glyph's space as a gap before the next glyph.

#### Scenario: raw code emitted
- **WHEN** a Type 3 glyph `/g65` with no mapping is shown under `cid`
- **THEN** `A` is emitted, and code 200 is emitted as `È` [§6.2, KP6; 0001 T27]

#### Scenario: unstorable code under cid
- **WHEN** an unmapped glyph's code is 0 or 1 under `cid`
- **THEN** U+FFFD is emitted (the oracle emits NUL or U+0001; a deliberate divergence) [0001 T27, §1.8 KP6; ADR-0050]

#### Scenario: C1 and permitted controls kept
- **WHEN** an unmapped code is 9, 10, 13 or in U+0080-U+009F under `cid`
- **THEN** that code point is emitted as is [0001 T27]

#### Scenario: replacement policy
- **WHEN** any glyph is unmapped under `replacement`
- **THEN** U+FFFD is emitted [§6.2; 0001 T27]

#### Scenario: skip policy
- **WHEN** a glyph is unmapped under `skip`
- **THEN** nothing is emitted for it, and the next glyph's gap to the pen includes the skipped glyph's advance (which may produce a space or a line break) [§6.2; 0001 T27]

### Requirement: Sanitised output
The system SHALL replace NUL, C0 controls other than TAB, LF and CR, U+FFFE, U+FFFF, unpaired surrogates and values above U+10FFFF with U+FFFD in every emitted string, whatever step produced it. TAB, LF, CR and C1 controls MUST be kept.

#### Scenario: NUL inside a multi-code-point entry
- **WHEN** a multi-code-point ToUnicode entry contains U+0000
- **THEN** that code point is emitted as U+FFFD and the rest verbatim (a documented divergence) [0001 T26; ADR-0050]

#### Scenario: ActualText is sanitised
- **WHEN** an `/ActualText` string decodes to text containing U+0001
- **THEN** U+FFFD is emitted in its place [0001 T28; ADR-0050]

### Requirement: No Unicode normalisation
The system SHALL emit mapped code points without normalisation: ligature code points (for example U+FB01), tabs, no-break spaces and other whitespace are kept, and no NFC/NFKD folding is applied, except for the base-encoding normalisation and ToUnicode control rules above.

#### Scenario: ligature preserved
- **WHEN** a font maps a glyph to U+FB01
- **THEN** `text()` contains U+FB01, not `fi` [F7, §6.2]

#### Scenario: leading and trailing whitespace kept
- **WHEN** a line begins with a tab and ends with two space glyphs
- **THEN** the tab and both spaces appear in that line's text [§6.5, KP5; 0001 T31]

### Requirement: ActualText replacement
The system SHALL honour `/ActualText` on a `BDC` whose property list (inline or named via `/Properties`) carries it, emitting the whole string at the span's first visible glyph with that glyph's geometry; later glyphs of the span emit nothing but advance the pen. An empty string MUST suppress the span's glyphs.

#### Scenario: span emits its text once
- **WHEN** a span with `/ActualText (fi)` covers two glyphs
- **THEN** `fi` is emitted once at the first glyph's position [KP9; 0001 T28, §1.8 KP9; ADR-0041]

#### Scenario: empty ActualText
- **WHEN** a span carries `/ActualText ()`
- **THEN** none of its glyphs are emitted [0001 T28]

#### Scenario: nested spans
- **WHEN** a span nests inside another span that has not yet emitted
- **THEN** the inner span's text replaces its glyphs and the outer span emits nothing further [0001 T28]

#### Scenario: span left open
- **WHEN** a content stream or form ends with an ActualText span still open
- **THEN** the span is closed there and later text is extracted normally [0001 T28]

#### Scenario: spans across lines or gaps
- **WHEN** a span's glyphs cross a line break or are separated by a gap
- **THEN** the whole string is still emitted at the first glyph (KP17, a documented divergence) [0001 T28, §1.8 KP17]

### Requirement: ActualText string decoding
The system SHALL decode `/ActualText` as UTF-16 with either byte-order mark (BE or LE), as UTF-8 with a byte-order mark, or otherwise as PDFDocEncoding in which a NUL, 0x9F or 0xAD byte terminates the string and 0x7F is kept. Unpaired surrogates MUST become U+FFFD.

#### Scenario: UTF-16LE accepted
- **WHEN** `/ActualText` starts with `FF FE`
- **THEN** it is decoded as UTF-16LE [0001 T28]

#### Scenario: PDFDocEncoding terminator
- **WHEN** a PDFDocEncoding `/ActualText` contains byte 0xAD after `ab`
- **THEN** the emitted text is `ab` [0001 T28]

### Requirement: ActualText bounds
The system SHALL track at most 64 nested ActualText spans and SHALL read at most 4 × `max_chars_per_page` bytes of `/ActualText` per page; deeper spans, and spans after the byte budget is spent, MUST be treated as plain marked content whose glyphs are extracted normally.

#### Scenario: nesting beyond 64
- **WHEN** 100 ActualText spans are nested
- **THEN** spans beyond the 64th are plain marked content and extraction succeeds [0001 T28]

#### Scenario: byte budget spent
- **WHEN** a 4 MB named property with `/ActualText` is referenced 2,000 times on one page
- **THEN** extraction completes promptly and later spans emit their glyphs' own text [0001 T28, §2.5]

### Requirement: Line assembly in content-stream order
The system SHALL assemble lines in content-stream order, comparing each glyph only with the current line and never with earlier lines. A glyph whose baseline direction differs from the line's by more than `angle_tolerance_deg` (2°) MUST start a new line. Direction is the text matrix's x axis after the CTM, whatever the writing mode.

#### Scenario: earlier lines are never revisited
- **WHEN** content draws line 1, then line 2, then more text back at line 1's position
- **THEN** the late text forms its own line after line 2 [§6.4; 0001 T29]

#### Scenario: rotated text
- **WHEN** text at 0° is followed by text at 90°
- **THEN** the 90° text starts a new line and a new block [§6.4; 0001 T29, T32]

#### Scenario: vertical writing
- **WHEN** a `WMode 1` font shows three glyphs
- **THEN** each glyph is its own line, as the oracle gives (KP12) [0001 T29, §1.8 KP12]

### Requirement: Line break thresholds
The system SHALL start a new line, with `em` the incoming glyph's size, `along` and `perp` its offset from the line's pen, when `perp > perp_break·em` (0.809), `along > gap_break·em` (0.798), or `-along > back_break·em` (0.795). Comparisons MUST be strict; backward movement within the threshold joins without a space.

#### Scenario: superscript joins
- **WHEN** an 11 pt glyph is raised 8.8 pt from the pen
- **THEN** it joins the line, and raised 9 pt it starts a new line [0001 T29, §1.8 KP2]

#### Scenario: forward gap
- **WHEN** a 22 pt glyph follows a 17.5 pt gap
- **THEN** it joins the line with a synthetic space, and an 11 pt glyph after an 8.8 pt gap starts a new line [0001 T29, §1.8 KP2; ADR-0047]

#### Scenario: backward movement
- **WHEN** an 11 pt glyph starts 8.5 pt behind the pen
- **THEN** it joins the line without a space, and 9 pt behind it starts a new line (KP3) [0001 T29, §1.8 KP3]

#### Scenario: exactly on the boundary
- **WHEN** a forward gap is exactly 0.8 em and, separately, a rise is exactly 0.8 em
- **THEN** the gap starts a new line and the rise joins, independent of page position [0001 §2.5 "Threshold boundaries"]

#### Scenario: fake bold duplicates kept
- **WHEN** a glyph is drawn twice a fraction of a point apart to fake bold
- **THEN** both copies are emitted, on the same line or a new one purely by the backward rule (KP4) [0001 T29, §1.8 KP4]

#### Scenario: size of the incoming glyph decides
- **WHEN** an 8 pt glyph is followed by a 20 pt glyph after a 16 pt gap
- **THEN** a new line starts, the threshold being measured in the 20 pt glyph's em [0001 T29]

### Requirement: Synthetic spaces
The system SHALL insert one U+0020 before a glyph that joins the line when `along > space_gap·em` (0.149, strict), unless the incoming glyph is U+0020 or the line's last character is U+0020, U+1680, U+3000 or U+FEFF. No other whitespace suppresses the space.

#### Scenario: space threshold bracket
- **WHEN** a 20 pt glyph follows a 3 pt gap (0.15 em)
- **THEN** a space is inserted, and a 22 pt glyph after a 3.25 pt gap (0.1477 em) joins without one (KP1) [0001 T29, §1.8 KP1; ADR-0047]

#### Scenario: a space glyph is not doubled
- **WHEN** a line ends in a space glyph and the next glyph follows a 0.3 em gap
- **THEN** no synthetic space is added [0001 T30, §1.8 KP5]

#### Scenario: other whitespace does not suppress
- **WHEN** a line ends in U+00A0 or U+2003 and the next glyph follows a 0.3 em gap
- **THEN** a synthetic space is inserted (KP24) [0001 T30, §1.8 KP24; ADR-0048]

#### Scenario: spacing operators participate
- **WHEN** `Hello` is shown with `Tc 2` at 11 pt
- **THEN** `H e l l o` is emitted, and with `Tc 9` each glyph is its own line [0001 T30]

#### Scenario: zero gap never spaces
- **WHEN** a zero-size glyph follows at the pen
- **THEN** no synthetic space is inserted [0001 T29]

### Requirement: Block assembly
The system SHALL start a new block when a new line changes direction, lies more than `block_gap` (1.5) em of the incoming glyph across the previous line, starts more than `block_indent` (2.0) em left of the block's leftmost line start, or follows a Form XObject or annotation boundary. Boundaries MUST NOT break a line.

#### Scenario: paragraph gap
- **WHEN** two 11 pt lines are separated by 20 pt across the baseline
- **THEN** they are in different blocks, visible only under `blank_line` [§6.4; 0001 T32]

#### Scenario: boundary inside a line
- **WHEN** a Form XObject begins in the middle of a line
- **THEN** the line continues and the new block starts at the next line [0001 T32]

#### Scenario: empty forms leave no boundary
- **WHEN** a form that draws no text is drawn between two lines
- **THEN** it does not start a new block [0001 T32]

#### Scenario: boundary list bounded
- **WHEN** a page carries more than 2 × `max_chars_per_page` + 256 boundaries
- **THEN** later boundaries are dropped, which can only merge blocks, and extraction succeeds [0001 T32]

### Requirement: Form XObject recursion in output
The system SHALL emit the text of nested Form XObjects in drawing order. The cycle and 50-level nesting bounds are owned by resource-limits; their effect on output is a documented divergence (KP20, KP21: the oracle extracts every level).

#### Scenario: self-recursive form
- **WHEN** a form draws itself
- **THEN** its text appears once and the page carries a `form:` warning [0001 §1.8 KP20, §1.4]

#### Scenario: forms nested 60 deep
- **WHEN** each of 60 nested forms shows one word
- **THEN** words beyond level 50 are absent and the page carries an `interpreter: nesting limit` warning [0001 §1.8 KP21]

### Requirement: Character limit returns a partial page
The system SHALL stop extraction once a page has recorded `max_chars_per_page` characters (default 2,000,000, counted in code points), return the page assembled so far, and attach the warning `text: character limit reached`. A page budget that ran out before the limit was reached MUST remain a `LimitExceededError`.

#### Scenario: limit reached
- **WHEN** a page with 5,000 characters is extracted with `max_chars_per_page=1000`
- **THEN** `text_page()` succeeds with at most 1,000 characters and one warning beginning `text: character limit reached` [0001 T34; ADR-0033]

#### Scenario: limit stops interpretation
- **WHEN** the character limit is reached early on a page followed by an operator flood
- **THEN** interpretation stops and no `ops_per_page` or `page_time_budget` error is raised [0001 T34]

#### Scenario: budget exhausted first
- **WHEN** the operator budget runs out before the character limit is reached
- **THEN** `LimitExceededError` with `limit_name == "ops_per_page"` is raised [0001 T34]

### Requirement: Damaged content yields partial text
The system SHALL, when a content stream contains bytes that start no operand or operator (for example a stray `)`, `]`, `>` or a malformed `#` name), skip them, continue extraction and attach the warning `content: damaged`. The operation MUST NOT fail.

#### Scenario: garbage between operators
- **WHEN** a content stream shows `GoodOp`, `Partial`, then a stray `)`, then `AfterGarbage`
- **THEN** `text()` is `"GoodOp\nPartial\nAfterGarbage\n"` and the page warnings include `content: damaged` [R3; 0001 T35]

#### Scenario: failed operand ends the stream
- **WHEN** a byte starts an operand that fails to parse (a stray `[`, `<` or `(`)
- **THEN** text before it is returned and the rest of that stream is not extracted, without an error [0001 §2.5 "Damaged content"]

### Requirement: Font substitution keeps text
The system SHALL extract the text of fonts that fail to load (corrupt embedded program, dangling or non-dictionary `/Font` entry, unknown or missing `/Subtype`, Type 0 without `/DescendantFonts`) by drawing them with a standard substitute, attaching one `font: substituted` warning whose count is the number of substitutions.

#### Scenario: corrupt embedded program
- **WHEN** a TrueType font's embedded program is corrupt
- **THEN** its text is extracted and the page warnings include `font: substituted` [0001 T37]

#### Scenario: missing font resource is not counted
- **WHEN** `Tf` names a font absent from `/Font`
- **THEN** the text is extracted with Helvetica and no `font: substituted` warning is added [0001 T37]

#### Scenario: text with no Tf
- **WHEN** text is shown before any `Tf`
- **THEN** it is extracted with Helvetica (the oracle drops it; KP18, a documented divergence) [0001 T37, §1.8 KP18]

### Requirement: Page warnings, not errors
The system SHALL report non-fatal per-page problems (decode and nesting failures in content, damaged content, font substitution, character limit, interpreter nesting, form cycles) only as warnings on the `TextPage`, one per kind with a count. `text_page()` MUST fail only with `PageError::Closed` or `PageError::Limit` for the operator or time budget.

#### Scenario: decode failure in content
- **WHEN** a page's content stream exceeds `max_stream_decoded_bytes`
- **THEN** the page extracts as if that stream were empty and carries a `stream: decoded size limit` warning [0001 §2.2, §1.4]

#### Scenario: repeated kinds are coalesced
- **WHEN** three fonts on one page fail to load
- **THEN** `warnings` holds one `font: substituted` entry displayed with ` (×3)` [0001 T37, §3.3]

### Requirement: TextPage immutability and lifetime
The system SHALL make `TextPage` immutable, self-owned and `Send + Sync`. It MUST remain readable after its document is closed and MUST serialise identically on every call, with either block separator. A text-layer `TextPage` reports `source` `Content` (Python `"content"`) and `dpi` `None`.

#### Scenario: read many times
- **WHEN** `tp.text()` is called twice and `tp.text(block_separator="blank_line")` once
- **THEN** the first two results are identical and the third differs only by block separators [F9; 0001 T36]

#### Scenario: readable after close
- **WHEN** a `TextPage` is obtained, the document closed, and `tp.text()` called
- **THEN** it returns the same text as before the close, without error [F9; 0001 T36]

#### Scenario: source and dpi
- **WHEN** `page.text_page()` returns
- **THEN** `tp.source == "content"` and `tp.dpi is None` [F9; 0001 T36, §3.5]

#### Scenario: same shape as OCR results
- **WHEN** a caller serialises a text-layer `TextPage` and an OCR `TextPage`
- **THEN** both follow this specification's serialisation and sanitising rules [F8, F9; 0002 O18]
