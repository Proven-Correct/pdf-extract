use crate::FillRule;
use crate::color::ColorSpace;
use crate::context::{Context, MAX_NESTED_INTERPRETATION_DEPTH};
use crate::convert::{convert_line_cap, convert_line_join};
use crate::device::Device;
use crate::font::{Font, FontData, FontQuery, StandardFont};
use crate::interpret::path::{
    close_path, fill_path, fill_path_impl, fill_stroke_path, stroke_path,
};
use crate::interpret::state::{TextStateFont, handle_gs};
use crate::interpret::text::TextRenderingMode;
use crate::pattern::{Pattern, ShadingPattern};
use crate::shading::Shading;
use crate::util::{OptionLog, RectExt};
use crate::x_object::{
    FormXObject, ImageXObject, XObject, draw_form_xobject, draw_image_xobject, draw_xobject,
};
use hayro_syntax::content::TypedIter;
use hayro_syntax::content::ops::TypedInstruction;
use hayro_syntax::object::dict::keys::{ANNOTS, AP, F, MCID, N, OC, POPUP, RECT, SUBTYPE};
use hayro_syntax::object::{Array, Dict, Name, Object, Rect, Stream, dict_or_stream};
use hayro_syntax::page::{Page, Resources};
use kurbo::{Affine, Point, Shape};
use smallvec::smallvec;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

pub(crate) mod path;
pub(crate) mod state;
pub(crate) mod text;

pub use state::ActiveTransferFunction;

/// A callback function for resolving font queries.
///
/// The first argument is the raw data, the second argument is the index in case the font
/// is a TTC, otherwise it should be 0.
pub type FontResolverFn = Arc<dyn Fn(&FontQuery) -> Option<(FontData, u32)> + Send + Sync>;
/// A callback function for resolving cmap names to their files.
pub type CMapResolverFn =
    Arc<dyn Fn(hayro_cmap::CMapName<'_>) -> Option<&'static [u8]> + Send + Sync>;
/// A callback function for resolving warnings during interpretation.
pub type WarningSinkFn = Arc<dyn Fn(InterpreterWarning) + Send + Sync>;

#[derive(Clone)]
/// Settings that should be applied during the interpretation process.
pub struct InterpreterSettings {
    /// Nearly every PDF contains text. In most cases, PDF files embed the fonts they use, and
    /// hayro can therefore read the font files and do all the processing needed. However, there
    /// are two problems:
    /// - Fonts don't _have_ to be embedded, it's possible that the PDF file only defines the basic
    ///   metadata of the font, like its name, but relies on the PDF processor to find that font
    ///   in its environment.
    /// - The PDF specification requires a list of 14 fonts that should always be available to a
    ///   PDF processor. These include:
    ///   - Times New Roman (Normal, Bold, Italic, `BoldItalic`)
    ///   - Courier (Normal, Bold, Italic, `BoldItalic`)
    ///   - Helvetica (Normal, Bold, Italic, `BoldItalic`)
    ///   - `ZapfDingBats`
    ///   - Symbol
    ///
    /// Because of this, if any of the above situations occurs, this callback will be called, which
    /// expects the data of an appropriate font to be returned, if available. If no such font is
    /// provided, the text will most likely fail to render.
    ///
    /// For the font data, there are two different formats that are accepted:
    /// - Any valid TTF/OTF font.
    /// - A valid CFF font program.
    ///
    /// The following recommendations are given for the implementation of this callback function.
    ///
    /// For the standard fonts, in case the original fonts are available on the system, you should
    /// just return those. Otherwise, for Helvetica, Courier and Times New Roman, the best alternative
    /// are the corresponding fonts of the [Liberation font family](https://github.com/liberationfonts/liberation-fonts).
    /// If you prefer smaller fonts, you can use the [Foxit CFF fonts](https://github.com/LaurenzV/hayro/tree/master/assets/standard_fonts),
    /// which are much smaller but are missing glyphs for certain scripts.
    ///
    /// For the `Symbol` and `ZapfDingBats` fonts, you should also prefer the system fonts, and if
    /// not available to you, you can, similarly to above, use the corresponding fonts from Foxit.
    ///
    /// If you don't want having to deal with this, you can just enable the `embed-fonts` feature
    /// and use the default implementation of the callback.
    pub font_resolver: FontResolverFn,
    /// A callback for resolving cmaps that aren't embedded.
    ///
    /// When the PDF requires using a cmap that is not directly embedded in the PDF,
    /// this callback will be called to attempt fetching the data of the file.
    ///
    /// When the `embed-cmaps` feature is enabled, this uses `load_embedded`
    /// method from `hayro-cmap` by default, which embeds the cmap files for
    /// all 61 predefined cmaps
    /// that the PDF specification requires to be readily available on a system.
    /// Otherwise, you can implement your custom logic for lazily fetching the
    /// data. If you are fine not supporting such PDFs, you can simply pass a closure
    /// that always returns `None`.
    pub cmap_resolver: CMapResolverFn,
    /// In certain cases, `hayro` will emit a warning in case an issue was encountered while interpreting
    /// the PDF file. Providing a callback allows you to catch those warnings and handle them, if desired.
    pub warning_sink: WarningSinkFn,
    /// Whether annotations should be rendered as well.
    ///
    /// Note that this feature is currently not fully implemented yet, so some
    /// annotations might be missing.
    pub render_annotations: bool,
    /// Limits on the work done while interpreting, and the counters behind them.
    ///
    /// Every content-stream operator counts against the budget, including the operators of
    /// Form XObjects, Type3 glyph procedures, tiling patterns and soft masks. Once the budget
    /// is exhausted, interpretation stops at the next operator and nothing further is drawn;
    /// use [`Context::stats`] to find out afterwards whether the output is complete. The
    /// default budget has no limits and only counts.
    pub budget: InterpretBudget,
}

impl Default for InterpreterSettings {
    fn default() -> Self {
        Self {
            #[cfg(not(feature = "embed-fonts"))]
            font_resolver: Arc::new(|_| None),
            #[cfg(feature = "embed-fonts")]
            font_resolver: Arc::new(|query| match query {
                FontQuery::Standard(s) => Some(s.get_font_data()),
                FontQuery::Fallback(f) => Some(f.pick_standard_font().get_font_data()),
            }),
            #[cfg(feature = "embed-cmaps")]
            cmap_resolver: Arc::new(hayro_cmap::load_embedded),
            #[cfg(not(feature = "embed-cmaps"))]
            cmap_resolver: Arc::new(|_| None),
            warning_sink: Arc::new(|_| {}),
            render_annotations: true,
            budget: InterpretBudget::default(),
        }
    }
}

#[derive(Copy, Clone, Debug)]
/// Warnings that can occur while interpreting a PDF file.
pub enum InterpreterWarning {
    /// An unsupported font kind was encountered.
    ///
    /// Currently, only CID fonts with non-identity encoding are unsupported.
    UnsupportedFont,
    /// An image failed to decode.
    ImageDecodeFailure,
}

/// Limits on the work an interpreter may do, for processing untrusted input.
///
/// The budget counts operators: a content stream that is small on disk can still be very
/// expensive to interpret, for example through Form XObjects that draw each other many times,
/// so neither file size nor nesting depth bounds the time spent on a page. The operator limit
/// bounds work; only the deadline bounds wall-clock time.
///
/// Clones share one set of counters. Nested interpretations (Type3 glyphs, patterns, soft
/// masks) run in their own [`Context`] built from a clone of the [`InterpreterSettings`], and
/// sharing is what makes their operators count against the same limit. As a consequence, a
/// budget should be created afresh for each unit of work it is meant to limit (typically one
/// page).
#[derive(Clone, Debug)]
pub struct InterpretBudget {
    /// The maximum number of operators to interpret. Interpretation stops before the
    /// operator that would exceed it.
    pub max_ops: Option<u64>,
    /// A point in time after which interpretation stops.
    pub deadline: Option<Instant>,
    /// How many operators to interpret between two checks of the deadline, since reading the
    /// clock for every operator would be needlessly expensive. A value of 0 is treated as 1.
    pub check_interval: u32,
    state: Arc<BudgetState>,
}

impl Default for InterpretBudget {
    /// A budget without limits, which only counts.
    fn default() -> Self {
        Self::new(None, None)
    }
}

/// The reason why an [`InterpretBudget`] was exhausted.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BudgetExhaustion {
    /// The operator limit was reached.
    Ops,
    /// The deadline passed.
    Deadline,
}

/// What happened during interpretation, as recorded by an [`InterpretBudget`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct InterpretStats {
    /// The number of operators counted. When the operator limit was hit, this includes the
    /// operator that was refused.
    pub ops: u64,
    /// Why interpretation was cut short, if it was.
    pub exhausted: Option<BudgetExhaustion>,
    /// How often nested content (a Form XObject, Type3 glyph, tiling pattern or soft mask)
    /// was skipped because it exceeded the maximum nesting depth.
    pub nesting_cap_hits: u32,
    /// How often a Form XObject was skipped because it was already being drawn, i.e. it
    /// (directly or indirectly) draws itself.
    pub form_cycle_hits: u32,
}

// Atomics rather than `Cell`s so that `InterpreterSettings` stays `Send + Sync`. A budget is
// only ever used by one interpretation at a time, so relaxed ordering is sufficient.
#[derive(Debug, Default)]
struct BudgetState {
    ops: AtomicU64,
    // 0 while the budget is available, otherwise one of the `EXHAUSTED_*` codes below.
    exhausted: AtomicU8,
    nesting_cap_hits: AtomicU32,
    form_cycle_hits: AtomicU32,
}

const EXHAUSTED_OPS: u8 = 1;
const EXHAUSTED_DEADLINE: u8 = 2;

impl InterpretBudget {
    /// The default number of operators between two deadline checks.
    pub const DEFAULT_CHECK_INTERVAL: u32 = 4096;

    /// Create a new budget with its own counters.
    pub fn new(max_ops: Option<u64>, deadline: Option<Instant>) -> Self {
        Self {
            max_ops,
            deadline,
            check_interval: Self::DEFAULT_CHECK_INTERVAL,
            state: Arc::new(BudgetState::default()),
        }
    }

    /// The counters recorded so far.
    pub fn stats(&self) -> InterpretStats {
        InterpretStats {
            ops: self.state.ops.load(Ordering::Relaxed),
            exhausted: self.exhaustion(),
            nesting_cap_hits: self.state.nesting_cap_hits.load(Ordering::Relaxed),
            form_cycle_hits: self.state.form_cycle_hits.load(Ordering::Relaxed),
        }
    }

    fn exhaustion(&self) -> Option<BudgetExhaustion> {
        match self.state.exhausted.load(Ordering::Relaxed) {
            EXHAUSTED_OPS => Some(BudgetExhaustion::Ops),
            EXHAUSTED_DEADLINE => Some(BudgetExhaustion::Deadline),
            _ => None,
        }
    }

    /// Whether the budget has been exhausted.
    pub(crate) fn is_exhausted(&self) -> bool {
        self.exhaustion().is_some()
    }

    /// Account for one operator. Returns `false` if the operator must not be interpreted.
    ///
    /// Once exhausted, the budget stays exhausted and the counter no longer moves, so the
    /// recorded reason is the one that stopped interpretation.
    pub(crate) fn tick(&self) -> bool {
        if self.is_exhausted() {
            return false;
        }

        let ops = self.state.ops.fetch_add(1, Ordering::Relaxed) + 1;

        if self.max_ops.is_some_and(|max| ops > max) {
            self.exhaust(EXHAUSTED_OPS);
            return false;
        }

        if let Some(deadline) = self.deadline
            && ops % u64::from(self.check_interval.max(1)) == 0
            && Instant::now() >= deadline
        {
            self.exhaust(EXHAUSTED_DEADLINE);
            return false;
        }

        true
    }

    fn exhaust(&self, reason: u8) {
        warn!("interpreter budget exhausted");
        self.state.exhausted.store(reason, Ordering::Relaxed);
    }

    /// Record that nested content was skipped at the nesting cap.
    pub(crate) fn note_nesting_cap_hit(&self) {
        warn!("interpreter nesting depth exceeded");
        self.state.nesting_cap_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record that a Form XObject was skipped because it is already being drawn.
    pub(crate) fn note_form_cycle_hit(&self) {
        warn!("skipping cyclic form xobject");
        self.state.form_cycle_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Check whether nested content at `nesting_depth` (the depth of the context it would be
    /// interpreted in) may run, and count a hit if not.
    ///
    /// Type3 glyphs, tiling patterns and soft masks record the depth of the context that
    /// created them plus one, but interpret in a fresh context, so they cannot go through
    /// `Context::begin_nested_interpretation`. This applies the same cap: content at depth
    /// `MAX_NESTED_INTERPRETATION_DEPTH` still runs, as a form at that depth does.
    pub(crate) fn may_nest(&self, nesting_depth: u32) -> bool {
        if nesting_depth > MAX_NESTED_INTERPRETATION_DEPTH {
            self.note_nesting_cap_hit();
            return false;
        }

        true
    }
}

/// interpret the contents of the page and render them into the device.
pub fn interpret_page<'a>(
    page: &Page<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    let resources = page.resources();
    interpret(page.typed_operations(), resources, context, device);

    if context.settings.render_annotations
        && let Some(annot_arr) = page.raw().get::<Array<'_>>(ANNOTS)
    {
        for annot in annot_arr.iter::<Dict<'_>>() {
            let flags = annot.get::<u32>(F).unwrap_or(0);

            // Annotation should be hidden (bit 2, Hidden) or not shown on screen (bit 6,
            // NoView).
            if flags & (2 | 32) != 0 {
                continue;
            }

            // A pop-up only displays its parent's text in a window, and a link's appearance
            // is at most a border; viewers do not draw either as page content.
            if annot
                .get::<Name<'_>>(SUBTYPE)
                .is_some_and(|s| matches!(s.as_ref(), POPUP | b"Link"))
            {
                continue;
            }

            if let Some(apx) = annot
                .get::<Dict<'_>>(AP)
                .and_then(|ap| ap.get::<Stream<'_>>(N))
                .and_then(|o| FormXObject::new(&o))
            {
                let Some(rect) = annot.get::<Rect>(RECT) else {
                    continue;
                };

                let annot_rect = rect.to_kurbo();
                // 12.5.5. Appearance streams
                // "The algorithm outlined in this subclause shall be used
                // to map from the coordinate system of the appearance XObject."

                // 1) The appearance’s bounding box (specified by its BBox entry)
                // shall be transformed, using Matrix, to produce a
                // quadrilateral with arbitrary orientation. The transformed
                // appearance box is the smallest upright rectangle that
                // encompasses this quadrilateral.
                let transformed_rect = (apx.matrix
                    * kurbo::Rect::new(
                        apx.bbox[0] as f64,
                        apx.bbox[1] as f64,
                        apx.bbox[2] as f64,
                        apx.bbox[3] as f64,
                    )
                    .to_path(0.1))
                .bounding_box();

                // 2) A matrix A shall be computed that scales and translates
                // the transformed appearance box to align with the edges
                // of the annotation’s rectangle (specified by the Rect entry).
                // A maps the lower-left corner (the corner with the smallest
                // x and y coordinates) and the upper-right corner (the
                // corner with the greatest x and y coordinates) of the
                // transformed appearance box to the corresponding corners
                // of the annotation’s rectangle.
                let affine = Affine::new([
                    annot_rect.width() / transformed_rect.width(),
                    0.0,
                    0.0,
                    annot_rect.height() / transformed_rect.height(),
                    annot_rect.x0 - transformed_rect.x0,
                    annot_rect.y0 - transformed_rect.y0,
                ]);

                // 3) Matrix shall be concatenated with A to form a matrix
                // AA that maps from the appearance’s coordinate system to
                // the annotation’s rectangle in default user space.
                let subtype = annot.get::<Name<'_>>(SUBTYPE);
                device.begin_annotation(subtype.as_ref().map_or(&[][..], |s| s.as_ref()));

                context.save_state();
                context.pre_concat_affine(affine);
                context.push_root_transform();

                draw_form_xobject(resources, &apx, context, device);
                context.pop_root_transform();
                context.restore_state(device);

                device.end_annotation();
            }
        }
    }
}

/// Interpret the instructions from `ops` and render them into the device.
pub fn interpret<'a>(
    mut ops: TypedIter<'_>,
    resources: &Resources<'a>,
    context: &mut Context<'a>,
    device: &mut impl Device<'a>,
) {
    let num_states = context.num_states();
    // Marked-content sequences (`BMC`/`BDC`) opened by this content stream and not yet
    // closed by an `EMC` of the same stream; see the cleanup after the loop.
    let mut open_marked_content = 0_usize;

    context.save_state();

    while let Some(op) = ops.next() {
        // Stop as soon as the budget runs out. The flag is shared with every enclosing
        // interpretation, so each of them stops at its next operator as well, while the
        // cleanup below still restores the graphics states pushed so far.
        if !context.budget_tick() {
            break;
        }

        match op {
            TypedInstruction::SaveState(_) => context.save_state(),
            TypedInstruction::StrokeColorDeviceRgb(s) => {
                context.get_mut().graphics_state.stroke_cs = ColorSpace::device_rgb();
                context.get_mut().graphics_state.stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32()];
                context.get_mut().graphics_state.stroke_pattern = None;
            }
            TypedInstruction::StrokeColorDeviceGray(s) => {
                context.get_mut().graphics_state.stroke_cs = ColorSpace::device_gray();
                context.get_mut().graphics_state.stroke_color = smallvec![s.0.as_f32()];
                context.get_mut().graphics_state.stroke_pattern = None;
            }
            TypedInstruction::StrokeColorCmyk(s) => {
                context.get_mut().graphics_state.stroke_cs = ColorSpace::device_cmyk();
                context.get_mut().graphics_state.stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32(), s.3.as_f32()];
                context.get_mut().graphics_state.stroke_pattern = None;
            }
            TypedInstruction::LineWidth(w) => {
                context.get_mut().graphics_state.stroke_props.line_width = w.0.as_f32();
            }
            TypedInstruction::LineCap(c) => {
                context.get_mut().graphics_state.stroke_props.line_cap = convert_line_cap(c);
            }
            TypedInstruction::LineJoin(j) => {
                context.get_mut().graphics_state.stroke_props.line_join = convert_line_join(j);
            }
            TypedInstruction::MiterLimit(l) => {
                context.get_mut().graphics_state.stroke_props.miter_limit = l.0.as_f32();
            }
            TypedInstruction::Transform(t) => {
                context.pre_concat_transform(t);
            }
            TypedInstruction::RectPath(r) => {
                let rect = kurbo::Rect::new(
                    r.0.as_f64(),
                    r.1.as_f64(),
                    r.0.as_f64() + r.2.as_f64(),
                    r.1.as_f64() + r.3.as_f64(),
                )
                .to_path(0.1);
                context.path_mut().extend(rect);
            }
            TypedInstruction::MoveTo(m) => {
                let p = Point::new(m.0.as_f64(), m.1.as_f64());
                *(context.last_point_mut()) = p;
                *(context.sub_path_start_mut()) = p;
                context.path_mut().move_to(p);
            }
            TypedInstruction::FillPathEvenOdd(_) => {
                fill_path(context, device, FillRule::EvenOdd);
            }
            TypedInstruction::FillPathNonZero(_) => {
                fill_path(context, device, FillRule::NonZero);
            }
            TypedInstruction::FillPathNonZeroCompatibility(_) => {
                fill_path(context, device, FillRule::NonZero);
            }
            TypedInstruction::FillAndStrokeEvenOdd(_) => {
                fill_stroke_path(context, device, FillRule::EvenOdd);
            }
            TypedInstruction::FillAndStrokeNonZero(_) => {
                fill_stroke_path(context, device, FillRule::NonZero);
            }
            TypedInstruction::CloseAndStrokePath(_) => {
                close_path(context);
                stroke_path(context, device);
            }
            TypedInstruction::CloseFillAndStrokeEvenOdd(_) => {
                close_path(context);
                fill_stroke_path(context, device, FillRule::EvenOdd);
            }
            TypedInstruction::CloseFillAndStrokeNonZero(_) => {
                close_path(context);
                fill_stroke_path(context, device, FillRule::NonZero);
            }
            TypedInstruction::NonStrokeColorDeviceGray(s) => {
                context.get_mut().graphics_state.none_stroke_cs = ColorSpace::device_gray();
                context.get_mut().graphics_state.non_stroke_color = smallvec![s.0.as_f32()];
                context.get_mut().graphics_state.non_stroke_pattern = None;
            }
            TypedInstruction::NonStrokeColorDeviceRgb(s) => {
                context.get_mut().graphics_state.none_stroke_cs = ColorSpace::device_rgb();
                context.get_mut().graphics_state.non_stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32()];
                context.get_mut().graphics_state.non_stroke_pattern = None;
            }
            TypedInstruction::NonStrokeColorCmyk(s) => {
                context.get_mut().graphics_state.none_stroke_cs = ColorSpace::device_cmyk();
                context.get_mut().graphics_state.non_stroke_color =
                    smallvec![s.0.as_f32(), s.1.as_f32(), s.2.as_f32(), s.3.as_f32()];
                context.get_mut().graphics_state.non_stroke_pattern = None;
            }
            TypedInstruction::LineTo(m) => {
                if !context.path().elements().is_empty() {
                    let last_point = *context.last_point();
                    let mut p = Point::new(m.0.as_f64(), m.1.as_f64());
                    *(context.last_point_mut()) = p;
                    if last_point == p {
                        // Add a small delta so that zero width lines can still have a round stroke.
                        p.x += 0.0001;
                    }

                    context.path_mut().line_to(p);
                }
            }
            TypedInstruction::CubicTo(c) => {
                if !context.path().elements().is_empty() {
                    let p1 = Point::new(c.0.as_f64(), c.1.as_f64());
                    let p2 = Point::new(c.2.as_f64(), c.3.as_f64());
                    let p3 = Point::new(c.4.as_f64(), c.5.as_f64());

                    *(context.last_point_mut()) = p3;

                    context.path_mut().curve_to(p1, p2, p3);
                }
            }
            TypedInstruction::CubicStartTo(c) => {
                if !context.path().elements().is_empty() {
                    let p1 = *context.last_point();
                    let p2 = Point::new(c.0.as_f64(), c.1.as_f64());
                    let p3 = Point::new(c.2.as_f64(), c.3.as_f64());

                    *(context.last_point_mut()) = p3;

                    context.path_mut().curve_to(p1, p2, p3);
                }
            }
            TypedInstruction::CubicEndTo(c) => {
                if !context.path().elements().is_empty() {
                    let p2 = Point::new(c.0.as_f64(), c.1.as_f64());
                    let p3 = Point::new(c.2.as_f64(), c.3.as_f64());

                    *(context.last_point_mut()) = p3;

                    context.path_mut().curve_to(p2, p3, p3);
                }
            }
            TypedInstruction::ClosePath(_) => {
                close_path(context);
            }
            TypedInstruction::SetGraphicsState(gs) => {
                if let Some(gs) = resources
                    .get_ext_g_state(gs.0)
                    .warn_none(&format!("failed to get extgstate {}", gs.0.as_str()))
                {
                    handle_gs(&gs, context, resources);
                }
            }
            TypedInstruction::StrokePath(_) => {
                stroke_path(context, device);
            }
            TypedInstruction::EndPath(_) => {
                if let Some(clip) = *context.clip()
                    && !context.path().elements().is_empty()
                {
                    let clip_path = context.get().ctm * context.path().clone();
                    context.push_clip_path(clip_path, clip, device);

                    *(context.clip_mut()) = None;
                }

                context.path_mut().truncate(0);
            }
            TypedInstruction::NonStrokeColor(c) => {
                let gs = &mut context.get_mut().graphics_state;
                gs.non_stroke_color = c.0.into_iter().map(|n| n.as_f32()).collect();
                gs.non_stroke_pattern = None;
            }
            TypedInstruction::StrokeColor(c) => {
                let gs = &mut context.get_mut().graphics_state;
                gs.stroke_color = c.0.into_iter().map(|n| n.as_f32()).collect();
                gs.stroke_pattern = None;
            }
            TypedInstruction::ClipNonZero(_) => {
                *(context.clip_mut()) = Some(FillRule::NonZero);
            }
            TypedInstruction::ClipEvenOdd(_) => {
                *(context.clip_mut()) = Some(FillRule::EvenOdd);
            }
            TypedInstruction::RestoreState(_) => context.restore_state(device),
            TypedInstruction::FlatnessTolerance(_) => {
                // Ignore for now.
            }
            TypedInstruction::ColorSpaceStroke(c) => {
                let cs = if let Some(named) = ColorSpace::new_from_name(c.0) {
                    named
                } else {
                    context
                        .get_color_space(resources, c.0)
                        .unwrap_or(ColorSpace::device_gray())
                };

                if !cs.is_pattern() {
                    context.get_mut().graphics_state.stroke_pattern = None;
                }
                context.get_mut().graphics_state.stroke_color = cs.initial_color();
                context.get_mut().graphics_state.stroke_cs = cs;
            }
            TypedInstruction::ColorSpaceNonStroke(c) => {
                let cs = if let Some(named) = ColorSpace::new_from_name(c.0) {
                    named
                } else {
                    context
                        .get_color_space(resources, c.0)
                        .unwrap_or(ColorSpace::device_gray())
                };

                if !cs.is_pattern() {
                    context.get_mut().graphics_state.non_stroke_pattern = None;
                }
                context.get_mut().graphics_state.non_stroke_color = cs.initial_color();
                context.get_mut().graphics_state.none_stroke_cs = cs;
            }
            TypedInstruction::DashPattern(p) => {
                context.get_mut().graphics_state.stroke_props.dash_offset = p.1.as_f32();
                // kurbo apparently cannot properly deal with offsets that are exactly 0.
                context.get_mut().graphics_state.stroke_props.dash_array =
                    p.0.iter::<f32>()
                        .map(|n| if n == 0.0 { 0.01 } else { n })
                        .collect();
            }
            TypedInstruction::RenderingIntent(_) => {
                // Ignore for now.
            }
            TypedInstruction::NonStrokeColorNamed(n) => {
                context.get_mut().graphics_state.non_stroke_color =
                    n.0.into_iter().map(|n| n.as_f32()).collect();
                context.get_mut().graphics_state.non_stroke_pattern = n.1.and_then(|name| {
                    resources
                        .get_pattern(name)
                        .and_then(|d| Pattern::new(d, context, resources))
                });
            }
            TypedInstruction::StrokeColorNamed(n) => {
                context.get_mut().graphics_state.stroke_color =
                    n.0.into_iter().map(|n| n.as_f32()).collect();
                context.get_mut().graphics_state.stroke_pattern = n.1.and_then(|name| {
                    resources
                        .get_pattern(name)
                        .and_then(|d| Pattern::new(d, context, resources))
                });
            }
            TypedInstruction::BeginMarkedContentWithProperties(bdc) => {
                // Properties can be either:
                // 1. A Name that references an entry in the Resources/Properties dictionary
                // 2. An inline dictionary with an OC key

                let mcid = dict_or_stream(bdc.1).and_then(|(props, _)| props.get::<i32>(MCID));

                // A name refers to an entry of the resources' `/Properties` dictionary,
                // resolved through the context's cache; anything else is used inline.
                let named = bdc.1.clone().into_name().map(|name| {
                    (
                        context.named_property(&resources.properties, &name),
                        resources.properties.get_ref(name.as_ref()),
                    )
                });

                let oc = match &named {
                    Some((dict, r)) => r.map(|r| (dict.clone().unwrap_or_default(), r)),
                    None => dict_or_stream(bdc.1).and_then(|(props, _)| {
                        let r = props.get_ref(OC)?;
                        let d = props.get::<Dict<'_>>(OC).unwrap_or_default();
                        Some((d, r))
                    }),
                };

                if let Some((dict, oc_ref)) = oc {
                    context.ocg_state.begin_ocg(&dict, oc_ref.into());
                } else {
                    context.ocg_state.begin_marked_content();
                }

                // The property list itself, for the device.
                let properties = match named {
                    Some((dict, _)) => dict,
                    None => dict_or_stream(bdc.1).map(|(props, _)| props.clone()),
                };

                device.begin_marked_content(bdc.0, mcid, properties.as_ref());
                open_marked_content += 1;
            }
            TypedInstruction::MarkedContentPointWithProperties(_) => {}
            TypedInstruction::EndMarkedContent(_) => {
                context.ocg_state.end_marked_content();
                device.end_marked_content();
                // An `EMC` without a matching `BMC`/`BDC` in this stream closes a sequence
                // of an enclosing stream, as before; it is not this stream's to account for.
                open_marked_content = open_marked_content.saturating_sub(1);
            }
            TypedInstruction::MarkedContentPoint(_) => {}
            TypedInstruction::BeginMarkedContent(bmc) => {
                context.ocg_state.begin_marked_content();
                device.begin_marked_content(bmc.0, None, None);
                open_marked_content += 1;
            }
            TypedInstruction::BeginText(_) => {
                context.get_mut().text_state.text_matrix = Affine::IDENTITY;
                context.get_mut().text_state.text_line_matrix = Affine::IDENTITY;
            }
            TypedInstruction::SetTextMatrix(m) => {
                let m = Affine::new([
                    m.0.as_f64(),
                    m.1.as_f64(),
                    m.2.as_f64(),
                    m.3.as_f64(),
                    m.4.as_f64(),
                    m.5.as_f64(),
                ]);
                context.get_mut().text_state.text_line_matrix = m;
                context.get_mut().text_state.text_matrix = m;
            }
            TypedInstruction::EndText(_) => {
                let has_outline = context
                    .get()
                    .text_state
                    .clip_paths
                    .segments()
                    .next()
                    .is_some();

                if has_outline {
                    let clip_path = context.get().ctm * context.get().text_state.clip_paths.clone();

                    context.push_clip_path(clip_path, FillRule::NonZero, device);
                }

                context.get_mut().text_state.clip_paths.truncate(0);
            }
            TypedInstruction::TextFont(t) => {
                let name = t.0;

                // In case we are unable to resolve the font, two scenarios:
                // 1) If the font doesn't exist in the first place in the resource dictionary,
                // assume Helvetica (this seems to be what other PDF viewers do).
                // 2) In case it's `None` because we were unable to resolve the font
                // (for whatever reason), leave it as `None`. Better showing no
                // text at all than garbage text.
                let font = if let Some(font_dict) = resources.get_font(name) {
                    context.resolve_font(&font_dict)
                } else {
                    Font::new_standard(StandardFont::Helvetica, &context.settings.font_resolver)
                        .map(TextStateFont::Fallback)
                };

                context.get_mut().text_state.font_size = t.1.as_f32();
                context.get_mut().text_state.font = font;
            }
            TypedInstruction::ShowText(s) => {
                if context.get().text_state.font.is_none() {
                    // Even if no explicit font was set, we try to assume Helvetica. Acrobat
                    // seems to do the same.
                    context.get_mut().text_state.font = Font::new_standard(
                        StandardFont::Helvetica,
                        &context.settings.font_resolver,
                    )
                    .map(TextStateFont::Fallback);
                }

                text::show_text_string(context, device, resources, s.0);
            }
            TypedInstruction::ShowTexts(s) => {
                if context.get().text_state.font.is_none() {
                    // Even if no explicit font was set, we try to assume Helvetica. Acrobat
                    // seems to do the same.
                    context.get_mut().text_state.font = Font::new_standard(
                        StandardFont::Helvetica,
                        &context.settings.font_resolver,
                    )
                    .map(TextStateFont::Fallback);
                }

                for obj in s.0.iter::<Object<'_>>() {
                    match obj {
                        Object::Number(num) => {
                            context.get_mut().text_state.apply_adjustment(num.as_f32());
                        }
                        Object::String(text) => {
                            text::show_text_string(context, device, resources, &text);
                        }
                        _ => {}
                    }
                }
            }
            TypedInstruction::HorizontalScaling(h) => {
                context.get_mut().text_state.horizontal_scaling = h.0.as_f32();
            }
            TypedInstruction::TextLeading(tl) => {
                context.get_mut().text_state.leading = tl.0.as_f32();
            }
            TypedInstruction::CharacterSpacing(c) => {
                context.get_mut().text_state.char_space = c.0.as_f32();
            }
            TypedInstruction::WordSpacing(w) => {
                context.get_mut().text_state.word_space = w.0.as_f32();
            }
            TypedInstruction::NextLine(n) => {
                let (tx, ty) = (n.0.as_f64(), n.1.as_f64());
                text::next_line(context, tx, ty);
            }
            TypedInstruction::NextLineUsingLeading(_) => {
                text::next_line(context, 0.0, -context.get().text_state.leading as f64);
            }
            TypedInstruction::NextLineAndShowText(n) => {
                text::next_line(context, 0.0, -context.get().text_state.leading as f64);
                text::show_text_string(context, device, resources, n.0);
            }
            TypedInstruction::TextRenderingMode(r) => {
                let mode = match r.0.as_i64() {
                    0 => TextRenderingMode::Fill,
                    1 => TextRenderingMode::Stroke,
                    2 => TextRenderingMode::FillStroke,
                    3 => TextRenderingMode::Invisible,
                    4 => TextRenderingMode::FillAndClip,
                    5 => TextRenderingMode::StrokeAndClip,
                    6 => TextRenderingMode::FillAndStrokeAndClip,
                    7 => TextRenderingMode::Clip,
                    _ => {
                        warn!("unknown text rendering mode {}", r.0.as_i64());

                        TextRenderingMode::Fill
                    }
                };

                context.get_mut().text_state.render_mode = mode;
            }
            TypedInstruction::NextLineAndSetLeading(n) => {
                let (tx, ty) = (n.0.as_f64(), n.1.as_f64());
                context.get_mut().text_state.leading = -ty as f32;
                text::next_line(context, tx, ty);
            }
            TypedInstruction::ShapeGlyph(_) => {}
            TypedInstruction::XObject(x) => {
                let cache = context.interpreter_cache.object_cache.clone();
                let transfer_function = context.get().graphics_state.transfer_function.clone();
                if let Some(x_object) = resources.get_x_object(x.0).and_then(|s| {
                    XObject::new(
                        &s,
                        &context.settings.warning_sink,
                        &cache,
                        transfer_function.clone(),
                    )
                }) {
                    draw_xobject(&x_object, resources, context, device);
                }
            }
            TypedInstruction::InlineImage(i) => {
                let warning_sink = context.settings.warning_sink.clone();
                let transfer_function = context.get().graphics_state.transfer_function.clone();
                let cache = context.interpreter_cache.object_cache.clone();
                if let Some(x_object) = ImageXObject::new(
                    i.0,
                    |name| context.get_color_space(resources, name),
                    &warning_sink,
                    &cache,
                    false,
                    transfer_function,
                ) {
                    draw_image_xobject(&x_object, context, device);
                }
            }
            TypedInstruction::TextRise(t) => {
                context.get_mut().text_state.rise = t.0.as_f32();
            }
            TypedInstruction::Shading(s) => {
                if !context.ocg_state.is_visible() {
                    continue;
                }

                let transfer_function = context.get().graphics_state.transfer_function.clone();

                if let Some(sp) = resources
                    .get_shading(s.0)
                    .and_then(|o| {
                        let (dict, stream) = dict_or_stream(&o)?;
                        Shading::new(dict, stream, &context.interpreter_cache.object_cache)
                    })
                    .map(|s| {
                        Pattern::Shading(ShadingPattern {
                            shading: Arc::new(s),
                            matrix: Affine::IDENTITY,
                            opacity: context.get().graphics_state.non_stroke_alpha,
                            transfer_function: transfer_function.clone(),
                        })
                    })
                {
                    context.save_state();
                    context.push_root_transform();
                    let st = context.get_mut();
                    st.graphics_state.non_stroke_pattern = Some(sp);
                    st.graphics_state.none_stroke_cs = ColorSpace::pattern();

                    device.set_soft_mask(st.graphics_state.soft_mask.clone());
                    device.set_blend_mode(st.graphics_state.blend_mode);

                    let bbox = context.bbox().to_path(0.1);
                    let inverted_bbox = context.get().ctm.inverse() * bbox;
                    fill_path_impl(context, device, FillRule::NonZero, Some(&inverted_bbox));

                    context.pop_root_transform();
                    context.restore_state(device);
                } else {
                    warn!("failed to process shading");
                }
            }
            TypedInstruction::BeginCompatibility(_) => {}
            TypedInstruction::EndCompatibility(_) => {}
            TypedInstruction::ColorGlyph(_) => {}
            TypedInstruction::ShowTextWithParameters(t) => {
                context.get_mut().text_state.word_space = t.0.as_f32();
                context.get_mut().text_state.char_space = t.1.as_f32();
                text::next_line(context, 0.0, -context.get().text_state.leading as f64);
                text::show_text_string(context, device, resources, t.2);
            }
            _ => {
                warn!("failed to read an operator");
            }
        }
    }

    // Close the marked-content sequences this stream left open, innermost first, as if
    // the stream had ended with the missing `EMC`s. Otherwise an unbalanced `BDC` in a
    // Form XObject would stay open after the form: a hidden optional-content span would
    // hide the rest of the page, the form's own `/OC` entry would be popped in its place,
    // and the device would see the sequence swallow content it does not belong to. This
    // also runs when the budget cut the stream short.
    for _ in 0..open_marked_content {
        context.ocg_state.end_marked_content();
        device.end_marked_content();
    }

    while context.num_states() > num_states {
        context.restore_state(device);
    }
}
