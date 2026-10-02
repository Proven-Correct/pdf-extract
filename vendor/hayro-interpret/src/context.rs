use crate::cache::{Cache, CacheKey};
use crate::color::ColorSpace;
use crate::convert::convert_transform;
use crate::font::{Font, StandardFont};
use crate::interpret::state::{ClipType, State, TextStateFont};
use crate::ocg::OcgState;
use crate::util::{BezPathExt, Float64Ext};
use crate::{ClipPath, Device, FillRule, InterpretStats, InterpreterSettings, StrokeProps};
use hayro_syntax::content::ops::Transform;
use hayro_syntax::object::Dict;
use hayro_syntax::object::Name;
use hayro_syntax::object::ObjectIdentifier;
use hayro_syntax::page::Resources;
use hayro_syntax::xref::XRef;
use kurbo::{Affine, BezPath, PathEl, Point, Rect, Shape};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Maximum nesting depth for interpreting `XObject`'s/patterns/streams.
pub(crate) const MAX_NESTED_INTERPRETATION_DEPTH: u32 = 50;

/// A cache used by the interpreter.
///
/// Ideally, such a cache should be constructed once per PDF and then reused across
/// multiple interpreter invocations on the same document.
#[derive(Clone)]
pub struct InterpreterCache<'a> {
    pub(crate) font_cache: Rc<RefCell<HashMap<u128, Option<Font<'a>>>>>,
    pub(crate) object_cache: Cache,
}

impl<'a> Default for InterpreterCache<'a> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> InterpreterCache<'a> {
    /// Create a new interpreter cache.
    pub fn new() -> Self {
        Self {
            font_cache: Rc::new(RefCell::new(HashMap::new())),
            object_cache: Cache::new(),
        }
    }
}

/// Resolved named property lists: `/Properties` dictionary (data address, length) → name →
/// the entry, if it resolved to a dictionary.
type PropertyCache<'a> = HashMap<(usize, usize), HashMap<Box<[u8]>, Option<Dict<'a>>>>;

/// A per-page interpretation context that borrows shared data from an [`InterpreterCache`].
pub struct Context<'a> {
    states: Vec<State<'a>>,
    path: BezPath,
    sub_path_start: Point,
    last_point: Point,
    clip: Option<FillRule>,
    root_transforms: Vec<Affine>,
    bbox: Vec<Rect>,
    pub(crate) settings: InterpreterSettings,
    pub(crate) interpreter_cache: InterpreterCache<'a>,
    pub(crate) xref: &'a XRef,
    pub(crate) ocg_state: OcgState,
    nesting_depth: u32,
    // The Form XObjects currently being drawn, innermost last. Forms are always indirect
    // objects, so the identifier tells a form that draws itself (directly or through other
    // forms) from one that is merely drawn several times.
    form_stack: Vec<ObjectIdentifier>,
    // Named property lists (`/Name BDC`) already resolved, keyed by the `/Properties`
    // dictionary they were looked up in (its data's address and length) and then by name.
    // Resolving an entry parses the referenced object, which can be large (a long
    // `/ActualText`), and a content stream can refer to the same entry with every `BDC`.
    // The data is borrowed for `'a`, so its address cannot be reused by a different
    // dictionary while this context exists.
    property_cache: PropertyCache<'a>,
}

impl<'a> Context<'a> {
    /// Create a new context.
    pub fn new(
        initial_transform: Affine,
        bbox: Rect,
        cache: &InterpreterCache<'a>,
        xref: &'a XRef,
        settings: InterpreterSettings,
    ) -> Self {
        let state = State::new(initial_transform);

        Self::new_with(initial_transform, bbox, cache, xref, settings, state, 0)
    }

    pub(crate) fn new_with(
        initial_transform: Affine,
        bbox: Rect,
        cache: &InterpreterCache<'a>,
        xref: &'a XRef,
        settings: InterpreterSettings,
        state: State<'a>,
        nesting_depth: u32,
    ) -> Self {
        let ocg_state = {
            let root_ref = xref.root_id();
            xref.get::<Dict<'_>>(root_ref)
                .map(|catalog| OcgState::from_catalog(&catalog))
                .unwrap_or_default()
        };

        Self {
            states: vec![state],
            settings,
            xref,
            root_transforms: vec![initial_transform],
            last_point: Point::default(),
            sub_path_start: Point::default(),
            clip: None,
            bbox: vec![bbox],
            path: BezPath::new(),
            interpreter_cache: cache.clone(),
            ocg_state,
            nesting_depth,
            form_stack: Vec::new(),
            property_cache: HashMap::new(),
        }
    }

    /// The dictionary that `name` refers to in the `/Properties` resource dictionary
    /// `properties`, resolved once per context (see `property_cache`).
    pub(crate) fn named_property(
        &mut self,
        properties: &Dict<'a>,
        name: &Name<'_>,
    ) -> Option<Dict<'a>> {
        if properties.is_empty() {
            return None;
        }

        let data = properties.data();
        let by_name = self
            .property_cache
            .entry((data.as_ptr() as usize, data.len()))
            .or_default();

        if let Some(cached) = by_name.get(name.as_ref()) {
            return cached.clone();
        }

        property_hooks::note_resolution();
        let resolved = properties.get::<Dict<'_>>(name.as_ref());
        by_name.insert(name.as_ref().into(), resolved.clone());

        resolved
    }

    pub(crate) fn save_state(&mut self) {
        let Some(cur) = self.states.last().cloned() else {
            warn!("attempted to save state without existing state");
            return;
        };

        self.states.push(cur);
    }

    pub(crate) fn bbox(&self) -> Rect {
        self.bbox.last().copied().unwrap_or_else(|| {
            warn!("failed to get a bbox");

            Rect::new(0.0, 0.0, 1.0, 1.0)
        })
    }

    fn push_bbox(&mut self, bbox: Rect) {
        let new = self.bbox().intersect(bbox);
        self.bbox.push(new);
    }

    pub(crate) fn push_clip_path(
        &mut self,
        clip_path: BezPath,
        fill: FillRule,
        device: &mut impl Device<'a>,
    ) {
        if let Some(clip_rect) = path_as_rect(&clip_path) {
            let cur_bbox = self.bbox();

            // If the clip path is a rect and completely covers the current bbox, don't emit it.
            if cur_bbox
                .min_x()
                .is_nearly_greater_or_equal(clip_rect.min_x())
                && cur_bbox
                    .min_y()
                    .is_nearly_greater_or_equal(clip_rect.min_y())
                && cur_bbox.max_x().is_nearly_less_or_equal(clip_rect.max_x())
                && cur_bbox.max_y().is_nearly_less_or_equal(clip_rect.max_y())
            {
                self.get_mut().clips.push(ClipType::Dummy);
                return;
            }
        }

        let bbox = clip_path.bounding_box();
        device.push_clip_path(&ClipPath {
            path: clip_path,
            fill,
        });
        self.push_bbox(bbox);
        self.get_mut().clips.push(ClipType::Real);
    }

    pub(crate) fn pop_clip_path(&mut self, device: &mut impl Device<'a>) {
        if let Some(ClipType::Real) = self.get_mut().clips.pop() {
            device.pop_clip_path();
            self.pop_bbox();
        }
    }

    fn pop_bbox(&mut self) {
        self.bbox.pop();
    }

    pub(crate) fn push_root_transform(&mut self) {
        self.root_transforms.push(self.get().ctm);
    }

    pub(crate) fn pop_root_transform(&mut self) {
        self.root_transforms.pop();
    }

    pub(crate) fn root_transform(&self) -> Affine {
        self.root_transforms
            .last()
            .copied()
            .unwrap_or(Affine::IDENTITY)
    }

    pub(crate) fn restore_state(&mut self, device: &mut impl Device<'a>) {
        let Some(target_clips) = self
            .states
            .get(self.states.len().saturating_sub(2))
            .map(|s| s.clips.len())
        else {
            warn!("underflowed graphics state");
            return;
        };

        while self.get().clips.len() > target_clips {
            self.pop_clip_path(device);
        }

        // The first state should never be popped.
        if self.states.len() > 1 {
            self.states.pop();
        }

        device.set_soft_mask(
            self.states
                .last()
                .and_then(|l| l.graphics_state.soft_mask.clone()),
        );
    }

    pub(crate) fn path(&self) -> &BezPath {
        &self.path
    }

    pub(crate) fn path_mut(&mut self) -> &mut BezPath {
        &mut self.path
    }

    pub(crate) fn sub_path_start(&self) -> &Point {
        &self.sub_path_start
    }

    pub(crate) fn sub_path_start_mut(&mut self) -> &mut Point {
        &mut self.sub_path_start
    }

    pub(crate) fn last_point(&self) -> &Point {
        &self.last_point
    }

    pub(crate) fn last_point_mut(&mut self) -> &mut Point {
        &mut self.last_point
    }

    pub(crate) fn clip(&self) -> &Option<FillRule> {
        &self.clip
    }

    pub(crate) fn clip_mut(&mut self) -> &mut Option<FillRule> {
        &mut self.clip
    }

    pub(crate) fn get(&self) -> &State<'a> {
        self.states.last().unwrap()
    }

    pub(crate) fn get_mut(&mut self) -> &mut State<'a> {
        self.states.last_mut().unwrap()
    }

    pub(crate) fn pre_concat_transform(&mut self, transform: Transform) {
        self.pre_concat_affine(convert_transform(transform));
    }

    pub(crate) fn pre_concat_affine(&mut self, transform: Affine) {
        self.get_mut().ctm *= transform;
    }

    pub(crate) fn get_color_space(
        &mut self,
        resources: &Resources<'_>,
        name: &Name<'_>,
    ) -> Option<ColorSpace> {
        let cs_object = resources.get_color_space(name)?;
        self.interpreter_cache
            .object_cache
            .get_or_insert_with(cs_object.cache_key(), || {
                ColorSpace::new(cs_object.clone(), &self.interpreter_cache.object_cache)
            })
    }

    pub(crate) fn stroke_props(&self) -> StrokeProps {
        self.get().graphics_state.stroke_props.clone()
    }

    pub(crate) fn num_states(&self) -> usize {
        self.states.len()
    }

    pub(crate) fn nesting_depth(&self) -> u32 {
        self.nesting_depth
    }

    /// Account for one operator against the budget in the settings.
    ///
    /// Returns `false` once the budget is exhausted.
    pub(crate) fn budget_tick(&self) -> bool {
        self.settings.budget.tick()
    }

    /// The counters of the [`InterpretBudget`] in the settings: operators interpreted, whether
    /// and why interpretation was cut short, and how often nested content was skipped.
    ///
    /// The counters are shared with every clone of the budget, so they cover all
    /// interpretation done with these settings, nested contexts included.
    ///
    /// [`InterpretBudget`]: crate::InterpretBudget
    pub fn stats(&self) -> InterpretStats {
        self.settings.budget.stats()
    }

    /// Whether interpretation was cut short because the budget was exhausted. When this
    /// returns `true`, the device has received only part of the content.
    pub fn budget_exhausted(&self) -> bool {
        self.stats().exhausted.is_some()
    }

    pub(crate) fn begin_nested_interpretation(&mut self) -> bool {
        if self.nesting_depth >= MAX_NESTED_INTERPRETATION_DEPTH {
            self.settings.budget.note_nesting_cap_hit();

            return false;
        }

        self.nesting_depth += 1;

        true
    }

    /// Whether the form `id` is currently being drawn.
    pub(crate) fn is_drawing_form(&self, id: ObjectIdentifier) -> bool {
        self.form_stack.contains(&id)
    }

    pub(crate) fn push_form(&mut self, id: ObjectIdentifier) {
        self.form_stack.push(id);
    }

    pub(crate) fn pop_form(&mut self) {
        self.form_stack.pop();
    }

    pub(crate) fn end_nested_interpretation(&mut self) {
        self.nesting_depth = self.nesting_depth.saturating_sub(1);
    }
    pub(crate) fn resolve_font(&mut self, font_dict: &Dict<'a>) -> Option<TextStateFont<'a>> {
        let cache_key = font_dict.cache_key();

        let mut loaded_now = false;
        let resolved = {
            let mut font_cache = self.interpreter_cache.font_cache.borrow_mut();
            font_cache
                .entry(cache_key)
                .or_insert_with(|| {
                    loaded_now = true;
                    Font::new(
                        font_dict,
                        &self.settings.font_resolver,
                        &self.settings.cmap_resolver,
                    )
                })
                .clone()
        };

        if let Some(resolved) = resolved {
            Some(TextStateFont::Font(resolved))
        } else {
            let fallback = Font::new_standard(StandardFont::Helvetica, &self.settings.font_resolver)
                .map(TextStateFont::Fallback);
            // Counted once per font, when it failed to load, like the substitutions
            // inside the font constructors.
            if loaded_now && fallback.is_some() {
                crate::font::note_font_substitution();
            }
            fallback
        }
    }
}

pub(crate) fn path_as_rect(path: &BezPath) -> Option<Rect> {
    // One MoveTo, three LineTo, one ClosePath
    if path.elements().len() != 5 {
        return None;
    }

    let bbox = path.fast_bounding_box();
    let (min_x, min_y, max_x, max_y) = (bbox.min_x(), bbox.min_y(), bbox.max_x(), bbox.max_y());
    let mut corners = [false; 4];

    let mut check_point = |p: Point| {
        corners[0] |= p.x.is_nearly_equal(min_x) && p.y.is_nearly_equal(min_y);
        corners[1] |= p.x.is_nearly_equal(min_x) && p.y.is_nearly_equal(max_y);
        corners[2] |= p.x.is_nearly_equal(max_x) && p.y.is_nearly_equal(min_y);
        corners[3] |= p.x.is_nearly_equal(max_x) && p.y.is_nearly_equal(max_y);
    };

    for (idx, el) in path.elements().iter().enumerate() {
        match el {
            PathEl::MoveTo(p) => {
                if idx != 0 {
                    return None;
                }

                check_point(*p);
            }
            PathEl::LineTo(l) => check_point(*l),
            PathEl::QuadTo(_, _) => return None,
            PathEl::CurveTo(_, _, _) => return None,
            PathEl::ClosePath => {}
        }
    }

    if corners[0] && corners[1] && corners[2] && corners[3] {
        Some(bbox)
    } else {
        None
    }
}

#[doc(hidden)]
/// Instrumentation for tests: how often a named property list was resolved rather than
/// taken from a context's cache. Not part of the supported API.
pub mod property_hooks {
    use std::cell::Cell;

    std::thread_local! {
        static RESOLUTIONS: Cell<u64> = const { Cell::new(0) };
    }

    /// Named property lists resolved on this thread so far.
    pub fn resolutions() -> u64 {
        RESOLUTIONS.with(Cell::get)
    }

    pub(crate) fn note_resolution() {
        RESOLUTIONS.with(|n| n.set(n.get().saturating_add(1)));
    }
}
