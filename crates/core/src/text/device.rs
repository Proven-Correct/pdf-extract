//! `TextDevice`: the `hayro_interpret::Device` that turns glyph draws into
//! [`CharRecord`]s (design spec §6.1; spec T19-T28, T34).
//!
//! hayro interprets the page (content, Form XObjects, annotation appearance
//! streams; hidden optional content never reaches a device) and calls
//! `draw_glyph` once per glyph per paint operation. For each call the device:
//!
//! 1. drops the `Stroke` half of a fill-and-stroke glyph (T20, KP14);
//! 2. stops once `max_chars_per_page` characters are recorded (T34), and
//!    raises the interpreter's cancel flag (H10g) so hayro stops too;
//! 3. computes the glyph's page-space geometry and box (T23);
//! 4. drops the glyph when its box misses the visible box or the bounding
//!    box of the active clip paths (T22, T24);
//! 5. lets an enclosing `/ActualText` span replace it (T28);
//! 6. maps it to Unicode, or applies the unmapped policy (T26, T27).
//!
//! Paths, images, soft masks, blend modes and transparency groups are
//! ignored. Type 3 glyph procedures are never run (D-33, §6.1): the Type 3
//! glyph itself is the character, and its procedure could only draw shapes or
//! nest text the reference does not extract.
//!
//! **Fill-then-stroke (T20).** For render modes 2 and 6 hayro calls
//! `draw_glyph` twice in a row, `Fill` then `Stroke`, with the same glyph and
//! transforms. The device remembers the last `Fill` and skips a `Stroke` that
//! matches it. Any other device call clears the memory, and hayro sets the
//! soft mask and blend mode before every glyph it shows, so a glyph shown
//! again by a later operator (fake bold, KP4) is always recorded again.
//!
//! **Clip regions (T24).** The device keeps a stack of regions: the visible
//! box (CropBox ∩ MediaBox in page space) at the bottom, and for each pushed
//! clip path the intersection of the region below it with the path's bounding
//! box. Non-rectangular clips are thus approximated by their bounding box, as
//! the reference does. A region can be empty, which drops everything until
//! the clip is popped.
//!
//! **Marked content.** Plain marked content only needs a depth count; only
//! spans carrying `/ActualText` are kept on a stack, so a stream of a million
//! unclosed `BMC`s costs no memory. At most [`MAX_ACTUAL_TEXT_NESTING`] spans
//! are tracked at once; deeper ones are treated as plain marked content.
//!
//! **Boundaries (T32).** hayro announces each drawn Form XObject and
//! annotation appearance (fork change H10d); the device records them as
//! [`Marker`]s beside the records, at the index of the next record, for the
//! line assembler's blocks. A form or annotation that produced no record
//! leaves no marker (its start is taken back at its end): it separates no
//! text, and a page that draws a decorative form a million times would
//! otherwise hold two million markers. What remains is bounded by
//! [`max_markers`].
//!
//! **ActualText budget.** Every `BDC` carrying `/ActualText` reads and
//! decodes its string, and one named property list can be referenced by any
//! number of `BDC`s, so a 1 MB string costs 1 MB of work per operator: two
//! thousand operators take seconds, far below the 4,096 operators between
//! deadline checks. A page therefore reads at most four bytes of
//! `/ActualText` per character of `max_chars_per_page` in total; after that
//! spans are plain marked content. This bounds both the time and the memory
//! (nested spans each hold a decoded copy) that strings can cost.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, CacheKey, ClipPath, Device, GlyphDrawMode, Image, Paint, PathDrawMode, SoftMask,
};
use hayro_syntax::object::dict::keys::ACTUAL_TEXT;
use hayro_syntax::object::{Dict, ObjectIdentifier};
use kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};

use super::options::Unmapped;
use super::record::{Boundary, CharRecord, CharText, Marker};
use super::unicode::{self, Lookup};

/// Ascent and descent (fraction of the font size) for fonts without usable
/// metrics (T23).
const DEFAULT_ASCENT_DESCENT: (f64, f64) = (0.72, -0.22);

/// Glyph-space units per em: hayro's `glyph_transform` maps text space
/// scaled by 1000 (the units of `/Widths`), for every font kind.
const UNITS_PER_EM: f64 = 1000.0;

/// Most `/ActualText` spans tracked at once. Real documents nest a handful;
/// the bound keeps a hostile stream of nested spans from holding one decoded
/// string per level.
pub(super) const MAX_ACTUAL_TEXT_NESTING: usize = 64;

/// `/ActualText` bytes a page may read per character of its limit: four, so
/// a page of UTF-8-sized text strings always fits.
const ACTUAL_TEXT_BYTES_PER_CHAR: usize = 4;

/// Most boundary markers a page keeps: two per character of its limit (a
/// start and an end around every record), plus room for a few nested forms
/// around them. Only markers around text survive (module docs), so only
/// forms nested many levels deep around single characters can reach this;
/// later markers are dropped, which can only merge blocks.
fn max_markers(max_chars: usize) -> usize {
    max_chars.saturating_mul(2).saturating_add(256)
}

/// Collects the [`CharRecord`]s of one page. Created per page operation and
/// never shared (spec §8.3).
pub(super) struct TextDevice {
    unmapped: Unmapped,
    /// Clip regions; the bottom entry is the visible box (module docs).
    regions: Vec<Option<Rect>>,
    /// Current marked-content nesting depth (`BMC`/`BDC` minus `EMC`).
    marked_depth: usize,
    /// Open `/ActualText` spans, innermost last.
    spans: Vec<ActualTextSpan>,
    /// The last `draw_glyph(…, Fill)`, if it was the last device call.
    last_fill: Option<DrawKey>,
    chars: Vec<CharRecord>,
    /// Code points recorded so far, against `max_chars`.
    char_count: usize,
    max_chars: usize,
    /// Glyphs dropped because the character limit was reached.
    over_limit: u64,
    /// `/ActualText` string bytes this page may still read (module docs).
    actual_text_bytes_left: usize,
    /// Form and annotation boundaries (module docs).
    markers: Vec<Marker>,
    /// Raised when the character limit is reached, so that hayro stops
    /// interpreting (fork change H10g); `None` in unit tests that drive the
    /// device by hand.
    cancel: Option<Arc<AtomicBool>>,
}

/// One open `/ActualText` span.
struct ActualTextSpan {
    /// `marked_depth` just inside the span; its `EMC` brings the depth below.
    depth: usize,
    text: String,
    state: SpanState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpanState {
    /// No glyph of the span has been seen yet.
    Pending,
    /// The span's text was emitted as `chars[i]`, or nothing was emitted
    /// (`None`: an empty string, a nested span that emitted instead, or the
    /// character limit).
    Done(Option<usize>),
}

/// Identifies one paint call, to recognise the `Stroke` that repeats a
/// `Fill` (module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
struct DrawKey {
    font: u128,
    glyph: u32,
    code: u32,
    transform: [f64; 6],
    glyph_transform: [f64; 6],
}

impl DrawKey {
    fn new(glyph: &Glyph<'_>, transform: Affine, glyph_transform: Affine) -> Self {
        let (font, id) = match glyph {
            Glyph::Outline(o) => (o.font_cache_key(), o.glyph_id().to_u32()),
            // The cache key covers both the font and the glyph procedure.
            Glyph::Type3(t) => (t.cache_key(), 0),
        };
        Self {
            font,
            glyph: id,
            code: glyph.char_code(),
            transform: transform.as_coeffs(),
            glyph_transform: glyph_transform.as_coeffs(),
        }
    }
}

/// Where a glyph sits on the page.
struct Geometry {
    origin: Point,
    dir: Vec2,
    size: f64,
    advance: f64,
    bbox: Rect,
}

impl Geometry {
    /// Page-space geometry of `glyph` drawn with `transform` (the CTM) and
    /// `glyph_transform` (text matrix, font size, rise and scaling, in units
    /// of 1/1000 text space).
    fn new(glyph: &Glyph<'_>, transform: Affine, glyph_transform: Affine) -> Self {
        let m = transform * glyph_transform;
        let [a, b, c, d, _, _] = m.as_coeffs();
        let x_axis = Vec2::new(a, b);
        let y_axis = Vec2::new(c, d);
        let origin = m * Point::ORIGIN;

        let x_len = x_axis.hypot();
        let y_len = y_axis.hypot();
        // T29: the direction is the text matrix's x axis after the CTM. A
        // degenerate x axis (zero font size or `Tz 0`) borrows the y axis
        // turned clockwise, and a fully degenerate matrix reads as
        // left-to-right.
        let (dir, size) = if x_len > 0.0 && x_len.is_finite() {
            let dir = x_axis / x_len;
            (dir, x_axis.cross(y_axis).abs() / x_len * UNITS_PER_EM)
        } else if y_len > 0.0 && y_len.is_finite() {
            (Vec2::new(y_axis.y, -y_axis.x) / y_len, y_len * UNITS_PER_EM)
        } else {
            (Vec2::new(1.0, 0.0), 0.0)
        };

        let glyph_advance = glyph.advance();
        let advance = (x_axis * glyph_advance.x + y_axis * glyph_advance.y).dot(dir);

        // T23: the box spans the advance along the baseline (one em for
        // vertical writing, whose advance runs down the column) and the
        // font's descent to ascent across it.
        let (ascent, descent) = ascent_descent(glyph);
        let width = if glyph.is_vertical() {
            UNITS_PER_EM
        } else {
            glyph_advance.x
        };
        let corners = [
            Point::new(0.0, descent * UNITS_PER_EM),
            Point::new(width, descent * UNITS_PER_EM),
            Point::new(width, ascent * UNITS_PER_EM),
            Point::new(0.0, ascent * UNITS_PER_EM),
        ]
        .map(|p| m * p);
        let bbox = corners
            .iter()
            .skip(1)
            .fold(Rect::from_points(corners[0], corners[0]), |r, &p| {
                r.union_pt(p)
            });

        Self {
            origin,
            dir,
            size,
            advance,
            bbox,
        }
    }

    /// Whether every stored value is finite as `f32`.
    fn fits_f32(&self) -> bool {
        [
            self.origin.x,
            self.origin.y,
            self.dir.x,
            self.dir.y,
            self.size,
            self.advance,
        ]
        .iter()
        .all(|&v| (v as f32).is_finite())
    }

    fn record(&self, text: CharText, invisible: bool) -> CharRecord {
        CharRecord {
            text,
            origin: (self.origin.x as f32, self.origin.y as f32),
            dir: (self.dir.x as f32, self.dir.y as f32),
            size: self.size as f32,
            advance: self.advance as f32,
            invisible,
        }
    }
}

/// The font's ascent and descent as fractions of the font size, or the
/// defaults. hayro (H5) already refuses pairs with ascent not above descent;
/// T23 also refuses a positive descent or a non-positive ascent, either of
/// which would float the box off the baseline.
fn ascent_descent(glyph: &Glyph<'_>) -> (f64, f64) {
    match glyph.ascent_descent() {
        Some((ascent, descent)) if ascent > 0.0 && descent <= 0.0 => {
            (f64::from(ascent), f64::from(descent))
        }
        _ => DEFAULT_ASCENT_DESCENT,
    }
}

/// Whether two rectangles share at least a point (edges count, so a
/// zero-size glyph on the boundary is kept).
fn touches(a: &Rect, b: &Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// The intersection of two rectangles, `None` when they are disjoint.
fn intersection(a: &Rect, b: &Rect) -> Option<Rect> {
    touches(a, b).then(|| {
        Rect::new(
            a.x0.max(b.x0),
            a.y0.max(b.y0),
            a.x1.min(b.x1),
            a.y1.min(b.y1),
        )
    })
}

impl TextDevice {
    /// A device for a page whose visible box is `visible` (page space).
    pub(super) fn new(visible: Rect, unmapped: Unmapped, max_chars: usize) -> Self {
        Self {
            unmapped,
            regions: vec![Some(visible)],
            marked_depth: 0,
            spans: Vec::new(),
            last_fill: None,
            chars: Vec::new(),
            char_count: 0,
            max_chars,
            over_limit: 0,
            actual_text_bytes_left: max_chars.saturating_mul(ACTUAL_TEXT_BYTES_PER_CHAR),
            markers: Vec::new(),
            cancel: None,
        }
    }

    /// Raises `flag` when the character limit is reached; the same flag is
    /// the interpreter budget's `cancel` (T34).
    pub(super) fn with_cancel(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancel = Some(flag);
        self
    }

    /// Closes every marked-content span still open (T28): called when
    /// `interpret_page` returns, so nothing interpreted later with this
    /// device can be swallowed by a span the content never closed. (Spans
    /// left open inside one form or content stream are closed there by fork
    /// change H10b.)
    pub(super) fn close_open_spans(&mut self) {
        self.spans.clear();
        self.marked_depth = 0;
    }

    /// The records, the boundary markers, and how many glyphs the character
    /// limit dropped.
    pub(super) fn finish(self) -> (Vec<CharRecord>, Vec<Marker>, u64) {
        (self.chars, self.markers, self.over_limit)
    }

    /// Records the start of a form or annotation at the current position.
    fn begin_boundary(&mut self, start: Boundary) {
        self.last_fill = None;
        if self.markers.len() < max_markers(self.max_chars) {
            self.markers.push(Marker {
                at: self.chars.len(),
                boundary: start,
            });
        }
    }

    /// Records the end of a form or annotation, or takes back its start if
    /// nothing was recorded in between (module docs).
    fn end_boundary(&mut self, start: Boundary, end: Boundary) {
        self.last_fill = None;
        let at = self.chars.len();
        if self
            .markers
            .last()
            .is_some_and(|m| m.at == at && m.boundary == start)
        {
            self.markers.pop();
        } else if self.markers.len() < max_markers(self.max_chars) {
            self.markers.push(Marker { at, boundary: end });
        }
    }

    /// Notes one glyph dropped by the character limit and, the first time,
    /// asks hayro to stop (T34).
    fn over_the_limit(&mut self) {
        if self.over_limit == 0
            && let Some(flag) = &self.cancel
        {
            flag.store(true, Ordering::Relaxed);
        }
        self.over_limit += 1;
    }

    fn region(&self) -> Option<&Rect> {
        self.regions.last().and_then(Option::as_ref)
    }

    /// Appends a record unless that would pass the character limit; once
    /// the limit is reached nothing more is recorded (T34). Returns the
    /// record's index.
    fn push(&mut self, record: CharRecord) -> Option<usize> {
        let n = record.text.char_count();
        if self.over_limit > 0 || self.char_count + n > self.max_chars {
            self.over_the_limit();
            return None;
        }
        self.char_count += n;
        self.chars.push(record);
        Some(self.chars.len() - 1)
    }

    /// T28: lets the innermost `/ActualText` span take the glyph. Returns
    /// `false` when no span is open.
    fn actual_text(&mut self, geometry: &Geometry, invisible: bool) -> bool {
        let Some(innermost) = self.spans.len().checked_sub(1) else {
            return false;
        };
        match self.spans[innermost].state {
            SpanState::Pending => {
                // The whole string goes out at the span's first glyph, with
                // that glyph's geometry; an empty string suppresses the span.
                let text = std::mem::take(&mut self.spans[innermost].text);
                let emitted = if text.is_empty() {
                    None
                } else {
                    self.push(geometry.record(CharText::new(&text), invisible))
                };
                self.spans[innermost].state = SpanState::Done(emitted);
                // An enclosing span that has not emitted yet never will: its
                // text would duplicate what the inner span stands for.
                for outer in &mut self.spans[..innermost] {
                    if outer.state == SpanState::Pending {
                        outer.state = SpanState::Done(None);
                    }
                }
            }
            // Later glyphs emit nothing but still move the pen: the span's
            // record's advance grows by each of their advances (T28), as long
            // as it is still the last record (otherwise the pen has moved on
            // to other text).
            SpanState::Done(Some(i)) if i + 1 == self.chars.len() => {
                self.chars[i].advance += geometry.advance as f32;
            }
            SpanState::Done(_) => {}
        }
        true
    }
}

impl<'a> Device<'a> for TextDevice {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {
        self.last_fill = None;
    }

    fn set_blend_mode(&mut self, _: BlendMode) {
        self.last_fill = None;
    }

    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {
        self.last_fill = None;
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.last_fill = None;
        let bbox = clip_path.path.bounding_box();
        let region = self.region().and_then(|r| intersection(r, &bbox));
        self.regions.push(region);
    }

    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {
        self.last_fill = None;
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        _: &Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        let key = DrawKey::new(glyph, transform, glyph_transform);
        let repeat = matches!(draw_mode, GlyphDrawMode::Stroke(_)) && self.last_fill == Some(key);
        self.last_fill = matches!(draw_mode, GlyphDrawMode::Fill).then_some(key);
        if repeat {
            return;
        }
        if self.over_limit > 0 {
            // Stopped (T34); count, but do no more work.
            self.over_the_limit();
            return;
        }

        let geometry = Geometry::new(glyph, transform, glyph_transform);
        // A matrix that overflows `f32` (or is NaN) gives no usable
        // position; the line assembler must never see one.
        if !geometry.fits_f32() {
            return;
        }
        if !self.region().is_some_and(|r| touches(&geometry.bbox, r)) {
            return;
        }
        let invisible = matches!(draw_mode, GlyphDrawMode::Invisible);
        if self.actual_text(&geometry, invisible) {
            return;
        }
        let text = match unicode::lookup(glyph) {
            Lookup::Mapped(text) => text,
            Lookup::Unmapped(code) => match unicode::unmapped_text(code, self.unmapped) {
                Some(c) => CharText::from_char(c),
                None => return,
            },
        };
        self.push(geometry.record(text, invisible));
    }

    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {
        self.last_fill = None;
    }

    fn pop_clip_path(&mut self) {
        self.last_fill = None;
        // The visible box at the bottom is never popped, even if a damaged
        // stream pops more than it pushed.
        if self.regions.len() > 1 {
            self.regions.pop();
        }
    }

    fn pop_transparency_group(&mut self) {
        self.last_fill = None;
    }

    fn begin_marked_content(&mut self, _: &[u8], _: Option<i32>, properties: Option<&Dict<'_>>) {
        self.last_fill = None;
        self.marked_depth += 1;
        if self.spans.len() >= MAX_ACTUAL_TEXT_NESTING {
            return;
        }
        // Once the page's ActualText budget is spent, spans are plain marked
        // content and the string is not even read (module docs).
        if self.actual_text_bytes_left == 0 {
            return;
        }
        let Some(bytes) =
            properties.and_then(|p| p.get::<hayro_syntax::object::String<'_>>(ACTUAL_TEXT))
        else {
            return;
        };
        self.actual_text_bytes_left = self
            .actual_text_bytes_left
            .saturating_sub(bytes.as_bytes().len());
        // No record can hold more than the remaining budget, so decoding
        // more would be wasted.
        let budget = self.max_chars.saturating_sub(self.char_count);
        self.spans.push(ActualTextSpan {
            depth: self.marked_depth,
            text: unicode::sanitise(&unicode::decode_text_string(
                bytes.as_bytes(),
                budget.saturating_add(1),
            ))
            .into_owned(),
            state: SpanState::Pending,
        });
    }

    fn end_marked_content(&mut self) {
        self.last_fill = None;
        if self
            .spans
            .last()
            .is_some_and(|s| s.depth == self.marked_depth)
        {
            self.spans.pop();
        }
        self.marked_depth = self.marked_depth.saturating_sub(1);
    }

    fn begin_form(&mut self, _: Option<ObjectIdentifier>) {
        self.begin_boundary(Boundary::FormStart);
    }

    fn end_form(&mut self) {
        self.end_boundary(Boundary::FormStart, Boundary::FormEnd);
    }

    fn begin_annotation(&mut self, _: &[u8]) {
        self.begin_boundary(Boundary::AnnotationStart);
    }

    fn end_annotation(&mut self) {
        self.end_boundary(Boundary::AnnotationStart, Boundary::AnnotationEnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_touch_inclusively() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(touches(&a, &Rect::new(10.0, 10.0, 20.0, 20.0)));
        assert!(touches(&a, &Rect::new(5.0, 5.0, 5.0, 5.0)));
        assert!(!touches(&a, &Rect::new(10.1, 0.0, 20.0, 10.0)));
        assert!(!touches(&a, &Rect::new(0.0, -5.0, 10.0, -0.1)));
        assert_eq!(
            intersection(&a, &Rect::new(5.0, -5.0, 15.0, 5.0)),
            Some(Rect::new(5.0, 0.0, 10.0, 5.0))
        );
        assert_eq!(intersection(&a, &Rect::new(11.0, 0.0, 12.0, 1.0)), None);
    }

    #[test]
    fn clip_regions_nest_and_the_visible_box_is_never_popped() {
        let mut d = TextDevice::new(Rect::new(0.0, 0.0, 100.0, 100.0), Unmapped::Cid, 10);
        let clip = |r: Rect| ClipPath {
            path: r.to_path(0.1),
            fill: hayro_interpret::FillRule::NonZero,
        };
        d.push_clip_path(&clip(Rect::new(50.0, 50.0, 200.0, 200.0)));
        assert_eq!(d.region(), Some(&Rect::new(50.0, 50.0, 100.0, 100.0)));
        d.push_clip_path(&clip(Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(d.region(), None, "disjoint clips leave nothing visible");
        d.push_clip_path(&clip(Rect::new(0.0, 0.0, 100.0, 100.0)));
        assert_eq!(d.region(), None, "an empty region stays empty");
        for _ in 0..5 {
            d.pop_clip_path();
        }
        assert_eq!(d.region(), Some(&Rect::new(0.0, 0.0, 100.0, 100.0)));
    }

    #[test]
    fn marked_content_depth_tolerates_stray_emc() {
        let mut d = TextDevice::new(Rect::ZERO, Unmapped::Cid, 10);
        d.end_marked_content();
        assert_eq!(d.marked_depth, 0);
        d.begin_marked_content(b"P", None, None);
        d.begin_marked_content(b"P", None, None);
        assert_eq!(d.marked_depth, 2);
        assert!(d.spans.is_empty(), "plain marked content holds no span");
    }
}
