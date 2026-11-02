//! Helpers shared by the tests of the vendored hayro fork (spec §4.4–§4.7):
//! two glyph-collecting `Device`s, a page runner that reports
//! `Context::stats()`, and writers for small hand-assembled PDFs. The
//! small-stack runner is not here but in `mod.rs` (see there for why): this
//! file is also compiled into the crate's own unit tests, which cannot use it.
//!
//! [`run_device`] runs a page into any device, for the tests that bring their
//! own (H4-H8).
//!
//! These tests observe hayro directly, through its own APIs; the library's
//! `Warning`s and `Page::text` do not exist yet and are tested by later units
//! (spec D-30). Neither device is the real text device (unit 09):
//!
//! - [`Collector`] (H2 tests, via [`interpret`]) records glyph Unicode and the
//!   number of paths, and nothing else.
//! - The device behind [`run_page`] (H3/H3b tests) also breaks lines at each
//!   new baseline (so `nested_forms` levels read as separate lines) and, when
//!   asked, runs nested content the way hayro's renderer does (Type 3 glyph
//!   procedures, tiling patterns, soft masks), because those are the paths
//!   that create nested interpreter contexts; the text device never does.

// Each test crate includes this module and uses only part of it.
#![allow(dead_code)]

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use hayro_interpret::font::Glyph;
use hayro_interpret::hayro_cmap::BfString;
use hayro_interpret::pattern::Pattern;
use hayro_interpret::{
    BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpretBudget, InterpretStats,
    InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask, interpret_page,
};
use hayro_syntax::Pdf;
use hayro_syntax::page::Page;
use kurbo::{Affine, BezPath, Point, Rect};

// ---------------------------------------------------------------------------
// The H2 collector

/// A device that records what hayro draws: the text of each glyph (U+FFFD
/// when the glyph has no Unicode mapping) and the number of filled or stroked
/// paths. Everything else is ignored.
#[derive(Debug, Default)]
pub struct Collector {
    pub text: String,
    pub paths: usize,
}

impl<'a> Device<'a> for Collector {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}

    fn set_blend_mode(&mut self, _: BlendMode) {}

    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {
        self.paths += 1;
    }

    fn push_clip_path(&mut self, _: &ClipPath) {}

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        _: Affine,
        _: Affine,
        _: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
        push_unicode(&mut self.text, glyph);
    }

    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}

    fn pop_clip_path(&mut self) {}

    fn pop_transparency_group(&mut self) {}
}

/// Interprets `page` with hayro's default settings and returns what the
/// [`Collector`] saw.
pub fn interpret<'a>(pdf: &'a Pdf, page: &Page<'a>) -> Collector {
    let cache = InterpreterCache::new();
    // Transform and bbox only matter for rendering; the collector ignores
    // positions.
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(0.0, 0.0, 1.0, 1.0),
        &cache,
        pdf.xref(),
        InterpreterSettings::default(),
    );
    let mut collector = Collector::default();
    interpret_page(page, &mut context, &mut collector);
    collector
}

/// Opens `bytes` and interprets page 0 into `device` with hayro's default
/// settings (annotations included), returning the device. For the tests of
/// the device-facing fork changes (H4-H8), which each bring a device that
/// records what they check.
pub fn run_device<D>(bytes: &[u8], mut device: D) -> D
where
    D: for<'a> Device<'a>,
{
    let pdf = Pdf::new(bytes.to_vec()).expect("fixture opens");
    let page = &pdf.pages()[0];
    let cache = InterpreterCache::new();
    // A bbox covering any page, so nothing is culled before the device sees it.
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(-1e5, -1e5, 1e5, 1e5),
        &cache,
        pdf.xref(),
        InterpreterSettings::default(),
    );
    interpret_page(page, &mut context, &mut device);
    device
}

/// Appends the glyph's Unicode, or U+FFFD when it has no mapping.
pub fn push_unicode(text: &mut String, glyph: &Glyph<'_>) {
    match glyph.as_unicode() {
        Some(BfString::Char(c)) => text.push(c),
        Some(BfString::String(s)) => text.push_str(&s),
        None => text.push(char::REPLACEMENT_CHARACTER),
    }
}

// ---------------------------------------------------------------------------
// The H3 page runner

/// What one page produced.
#[derive(Debug, Default)]
pub struct Output {
    /// Glyph Unicode, with `\n` between baselines.
    pub text: String,
    /// `draw_path` calls.
    pub paths: usize,
    /// `Context::stats()` after `interpret_page` returned.
    pub stats: InterpretStats,
}

impl Output {
    /// The text split into baselines.
    pub fn lines(&self) -> Vec<&str> {
        self.text.lines().collect()
    }
}

/// Collects glyph text by baseline; see the module docs.
struct TextCollector<'a> {
    text: String,
    last_y: Option<f64>,
    paths: usize,
    run_nested: bool,
    // The mask most recently set. A renderer applies a mask when it changes;
    // interpreting it on every `set_soft_mask` call instead would branch at
    // each nesting level (hayro re-sends the current mask on every fill and
    // state restore) and turn a linear chain into an exponential one.
    mask: Option<SoftMask<'a>>,
}

impl<'a> Device<'a> for TextCollector<'a> {
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        if mask == self.mask {
            return;
        }
        self.mask.clone_from(&mask);
        if self.run_nested
            && let Some(mask) = mask
        {
            mask.interpret(self);
            self.mask = Some(mask);
        }
    }

    fn set_blend_mode(&mut self, _: BlendMode) {}

    fn draw_path(&mut self, _: &BezPath, transform: Affine, paint: &Paint<'a>, _: &PathDrawMode) {
        self.paths += 1;
        if self.run_nested
            && let Paint::Pattern(pattern) = paint
            && let Pattern::Tiling(tiling) = pattern.as_ref()
        {
            tiling.interpret(self, transform, false);
        }
    }

    fn push_clip_path(&mut self, _: &ClipPath) {}

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
        // Baselines in these fixtures are whole points apart; compare with a
        // tolerance well below that so rounding never splits a line.
        let y = (transform * glyph_transform * Point::ORIGIN).y;
        if self.last_y.is_some_and(|last| (last - y).abs() > 0.5) {
            self.text.push('\n');
        }
        self.last_y = Some(y);
        push_unicode(&mut self.text, glyph);
        if self.run_nested
            && let Glyph::Type3(t3) = glyph
        {
            t3.interpret(self, transform, glyph_transform, paint);
        }
    }

    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}

    fn pop_clip_path(&mut self) {}

    fn pop_transparency_group(&mut self) {}
}

/// Options for [`run_page`].
#[derive(Clone, Debug, Default)]
pub struct Run {
    /// Installed as `InterpreterSettings::budget` (default: no limits).
    pub budget: InterpretBudget,
    /// Run Type 3 glyph procedures, tiling patterns and soft masks (see the
    /// module docs).
    pub run_nested: bool,
}

/// Interprets page 0 of `pdf` with `run` and collects its output.
pub fn run_page(pdf: &Pdf, run: &Run) -> Output {
    run_page_timed(pdf, run, |_| {}).0
}

/// Like [`run_page`], but calls `before` on the budget immediately before
/// `interpret_page` and returns the time `interpret_page` took. The page's
/// content stream is decoded before that point (hayro caches it on the
/// `Page`), so a deadline set in `before` measures interpretation, not the
/// decode, which is one non-interruptible step (spec §2.4).
pub fn run_page_timed(
    pdf: &Pdf,
    run: &Run,
    before: impl FnOnce(&mut InterpretBudget),
) -> (Output, Duration) {
    let page = &pdf.pages()[0];
    let _ = page.page_stream();

    let mut budget = run.budget.clone();
    before(&mut budget);
    let settings = InterpreterSettings {
        budget,
        ..InterpreterSettings::default()
    };
    let cache = InterpreterCache::new();
    // The bbox only matters for clipping decisions; make it cover any page.
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(-1e5, -1e5, 1e5, 1e5),
        &cache,
        pdf.xref(),
        settings,
    );
    let mut device = TextCollector {
        text: String::new(),
        last_y: None,
        paths: 0,
        run_nested: run.run_nested,
        mask: None,
    };

    let start = Instant::now();
    interpret_page(page, &mut context, &mut device);
    let elapsed = start.elapsed();

    let out = Output {
        text: device.text,
        paths: device.paths,
        stats: context.stats(),
    };
    (out, elapsed)
}

// ---------------------------------------------------------------------------
// PDF writers, for inputs the test kit's builder cannot express (arbitrary
// stream filters, patterns, soft masks, self-references). Everything else
// comes from `pdf_extract_testkit`.

/// A classic-xref PDF whose object `i + 1` is `bodies[i]` and whose catalog
/// is object 1. The xref offsets are exact, so hayro opens it without repair.
pub fn assemble_pdf<B: AsRef<[u8]>>(bodies: &[B]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::with_capacity(bodies.len());
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body.as_ref());
        out.extend_from_slice(b"\nendobj\n");
    }
    let mut tail = String::new();
    let _ = write!(tail, "xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1);
    for offset in offsets {
        let _ = writeln!(tail, "{offset:010} 00000 n ");
    }
    let _ = write!(
        tail,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        bodies.len() + 1,
        out.len()
    );
    out.extend_from_slice(tail.as_bytes());
    out
}

/// A one-page A4 PDF whose page content is a single stream:
/// `<< /Length n {stream_dict} >>` followed by `data` as is (so it can carry
/// any filter). The page has Helvetica as `/F1`. The content stream is
/// object 4.
pub fn one_page_pdf(stream_dict: &str, data: &[u8]) -> Vec<u8> {
    let mut stream = format!("<< /Length {} {stream_dict} >>\nstream\n", data.len()).into_bytes();
    stream.extend_from_slice(data);
    stream.extend_from_slice(b"\nendstream");
    assemble_pdf(&[
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] \
           /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_vec(),
        stream,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ])
}

/// A stream object body: `<< dict /Length n >> stream … endstream`.
pub fn stream(dict: &str, data: &str) -> String {
    format!(
        "<< {dict} /Length {} >>\nstream\n{data}\nendstream",
        data.len() + 1
    )
}

/// A one-page PDF from raw object bodies, numbered from 4: objects 1–3 are
/// the catalog, the page tree and a 200 × 200 page whose `/Resources` is
/// object 4 and whose `/Contents` is object 5.
pub fn raw_pdf(objects: &[String]) -> Vec<u8> {
    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources 4 0 R /Contents 5 0 R >>"
            .to_owned(),
    ];
    bodies.extend_from_slice(objects);
    assemble_pdf(&bodies)
}
