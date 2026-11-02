//! The public result of text extraction: [`TextPage`] and its structured
//! model ([`Block`], [`Line`], [`Char`]), the serialisation to text, and the
//! `Page::text_page` / `Page::text` entry points (spec §3.3 `text/page.rs`,
//! T19, T33, T36; design spec §6.5).
//!
//! A `TextPage` owns everything it holds (no borrow of the document), so it
//! is `Send + Sync`, outlives `Document::close`, and can be serialised any
//! number of times with either [`BlockSeparator`]. The structured model is
//! `#[doc(hidden)]` in this run: it exists so that later output forms (words,
//! blocks, JSON) need no change to the extractor, but its shape is not yet a
//! promise.
//!
//! **Serialisation (T33).** Each line's characters are concatenated and
//! followed by `'\n'`, including the last line; `BlankLine` adds one `'\n'`
//! before every block but the first. A line is never empty (it holds at
//! least one record, and records are never empty), so `"\n\n"` can occur
//! under `Newline` only when a mapping step itself produced a line feed.

use std::fmt;

use super::layout;
use super::options::{BlockSeparator, TextOptions};
use super::record::CharText;
use crate::error::PageError;
use crate::page::Page;
use crate::warning::Warning;

/// Where a [`TextPage`]'s text came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextSource {
    /// The page's own text layer (content streams, forms, annotation
    /// appearances).
    Content,
    /// Optical character recognition of the rendered page (run 0002 O18);
    /// [`TextPage::dpi`] gives the resolution it ran at.
    Ocr,
}

/// The extracted text of one page (spec T36). Immutable; owns its data.
///
/// Both sources produce the same shape (blocks of lines of characters, the
/// same serialisation and the same D-10 sanitising), so a caller can treat
/// an OCR result exactly like a text-layer one.
#[derive(Clone, Debug, PartialEq)]
pub struct TextPage {
    blocks: Vec<Block>,
    warnings: Vec<Warning>,
    source: TextSource,
    /// `Some` exactly when `source` is `Ocr`; kept together by the two
    /// constructors, the only places either is set.
    dpi: Option<u32>,
}

static_assertions::assert_impl_all!(TextPage: Send, Sync);
static_assertions::assert_impl_all!(Block: Send, Sync);

impl TextPage {
    /// A page read from the content (the text path).
    pub(crate) fn new(blocks: Vec<Block>, warnings: Vec<Warning>) -> Self {
        Self {
            blocks,
            warnings,
            source: TextSource::Content,
            dpi: None,
        }
    }

    /// A page recognised by OCR at `dpi` (built by `ocr_support`).
    #[cfg(feature = "render-support")]
    pub(super) fn from_ocr(blocks: Vec<Block>, warnings: Vec<Warning>, dpi: u32) -> Self {
        Self {
            blocks,
            warnings,
            source: TextSource::Ocr,
            dpi: Some(dpi),
        }
    }

    /// Where the text came from.
    pub fn source(&self) -> TextSource {
        self.source
    }

    /// The resolution OCR ran at (after any lowering to respect the
    /// engine's pixel cap); `None` for text read from the content.
    pub fn dpi(&self) -> Option<u32> {
        self.dpi
    }

    /// The page's text: every line followed by `'\n'`; an empty page gives
    /// `""`. `sep` decides whether an empty line separates blocks.
    pub fn to_text(&self, sep: BlockSeparator) -> String {
        let mut out = String::new();
        for (i, block) in self.blocks.iter().enumerate() {
            if i > 0 && sep == BlockSeparator::BlankLine {
                out.push('\n');
            }
            for line in &block.lines {
                line.write_to(&mut out);
                out.push('\n');
            }
        }
        out
    }

    /// Problems worked around while extracting this page, one per kind with
    /// a count.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// The structured model: blocks of lines of characters, in output
    /// order. Not yet a stable interface.
    #[doc(hidden)]
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }
}

/// An axis-aligned box in page space: `[x0, y0, x1, y1]` with `x0 <= x1` and
/// `y0 <= y1` (PDF orientation, y up, `/Rotate` applied).
pub(crate) type BBox = [f32; 4];

/// Smallest box holding both.
pub(super) fn union(a: BBox, b: BBox) -> BBox {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// A group of consecutive lines (spec T32). Blocks matter only to
/// [`BlockSeparator::BlankLine`].
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub(super) lines: Vec<Line>,
    pub(super) bbox: BBox,
}

impl Block {
    /// The lines, in output order (never empty).
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Union of the lines' boxes.
    pub fn bbox(&self) -> BBox {
        self.bbox
    }
}

/// One output line: characters along one baseline direction (spec T29).
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub(super) chars: Vec<Char>,
    pub(super) bbox: BBox,
    pub(super) dir: (f32, f32),
}

impl Line {
    /// The characters, including synthetic spaces (never empty).
    pub fn chars(&self) -> &[Char] {
        &self.chars
    }

    /// The line's box: for text from the content, the union of the
    /// characters' approximate boxes; for OCR, the engine's line box widened
    /// to hold every word (the engine may report a line wider than its
    /// words).
    pub fn bbox(&self) -> BBox {
        self.bbox
    }

    /// Unit baseline direction of the line's first character.
    pub fn dir(&self) -> (f32, f32) {
        self.dir
    }

    /// The line's text, without its terminating `'\n'`.
    pub fn text(&self) -> String {
        let mut s = String::new();
        self.write_to(&mut s);
        s
    }

    fn write_to(&self, out: &mut String) {
        for c in &self.chars {
            out.push_str(c.text());
        }
    }
}

/// One character of a line: an emitted glyph, or a space the assembler
/// inserted for a gap (spec T30).
#[doc(hidden)]
#[derive(Clone, PartialEq)]
pub struct Char {
    pub(super) text: CharText,
    pub(super) origin: (f32, f32),
    pub(super) dir: (f32, f32),
    pub(super) size: f32,
    pub(super) advance: f32,
}

impl Char {
    /// The text: one or more code points (never empty).
    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    /// Baseline start, page space. For a synthetic space, the end of the
    /// previous character.
    pub fn origin(&self) -> (f32, f32) {
        self.origin
    }

    /// Unit baseline direction, page space.
    pub fn dir(&self) -> (f32, f32) {
        self.dir
    }

    /// Effective font size (points). For a synthetic space, the size of the
    /// character after it.
    pub fn size(&self) -> f32 {
        self.size
    }

    /// How far the character moves the pen along `dir`. For a synthetic
    /// space, the width of the gap it stands for.
    pub fn advance(&self) -> f32 {
        self.advance
    }

    /// Approximate box: from the origin to the end of the advance along the
    /// baseline, and from −0.2 to +0.8 of the size across it (design spec
    /// §6.3's approximation; the record's font metrics are not kept).
    pub(super) fn bbox(&self) -> BBox {
        let (ox, oy) = self.origin;
        let (dx, dy) = self.dir;
        let (nx, ny) = (-dy, dx);
        let end = (ox + dx * self.advance, oy + dy * self.advance);
        let (up, down) = (0.8 * self.size, -0.2 * self.size);
        let corners = [
            (ox + nx * down, oy + ny * down),
            (ox + nx * up, oy + ny * up),
            (end.0 + nx * down, end.1 + ny * down),
            (end.0 + nx * up, end.1 + ny * up),
        ];
        corners.iter().fold(
            [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            |b, &(x, y)| union(b, [x, y, x, y]),
        )
    }
}

impl fmt::Debug for Char {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Char")
            .field("text", &self.text)
            .field("origin", &self.origin)
            .field("dir", &self.dir)
            .field("size", &self.size)
            .field("advance", &self.advance)
            .finish()
    }
}

impl Page {
    /// Extracts the page's text layer (spec T19): its content streams,
    /// Form XObjects and the appearance streams of visible annotations,
    /// assembled into lines and blocks.
    ///
    /// Malformed content is worked around and reported in
    /// [`TextPage::warnings`]; only the documented limits fail.
    ///
    /// # Errors
    /// [`PageError::Closed`] if the document was closed before the call;
    /// [`PageError::Limit`] with `OpsPerPage` or `PageTimeBudget` when the
    /// page needed more interpretation than the document's limits allow.
    pub fn text_page(&self, opts: &TextOptions) -> Result<TextPage, PageError> {
        let page = self.extract_chars(opts.unmapped)?;
        let blocks = layout::assemble(page.chars, &page.markers, &opts.layout);
        Ok(TextPage::new(blocks, page.warnings))
    }

    /// The page's text, serialised with `opts.block_separator`: the same as
    /// `self.text_page(opts)?.to_text(opts.block_separator)`.
    ///
    /// ```
    /// use pdf_extract_core::{Document, OpenOptions, TextOptions};
    /// # let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"),
    /// #     "/../../tests/fixtures/corpus-c/enc_aes_128_owner.pdf")).unwrap();
    /// let doc = Document::from_bytes(bytes, &OpenOptions::default())?;
    /// let text = doc.page(0)?.text(&TextOptions::default())?;
    /// assert!(text.is_empty() || text.ends_with('\n'));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// As [`Self::text_page`].
    pub fn text(&self, opts: &TextOptions) -> Result<String, PageError> {
        Ok(self.text_page(opts)?.to_text(opts.block_separator))
    }
}
