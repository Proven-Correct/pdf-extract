//! Tests of fork change H10g: `InterpretBudget::cancel`, a flag that stops
//! interpretation at the next operator, recorded as
//! `BudgetExhaustion::Cancelled` (spec §3.2, T34, oracle §4.9b).
//!
//! The text device raises it when the page reaches its character limit, so
//! that the rest of the page costs no further operators or time.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::{raw_pdf, stream};
use hayro_interpret::font::Glyph;
use hayro_interpret::{
    BlendMode, BudgetExhaustion, ClipPath, Context, Device, GlyphDrawMode, Image, InterpretBudget,
    InterpretStats, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
    interpret_page,
};
use hayro_syntax::Pdf;
use kurbo::{Affine, BezPath, Rect};

/// Counts glyphs and paths, runs Type 3 glyph procedures, and raises the
/// cancel flag once `stop_after` glyphs or paths (whichever it counts) were
/// seen.
struct Stopper {
    cancel: Arc<AtomicBool>,
    glyphs: usize,
    paths: usize,
    stop_after_glyphs: Option<usize>,
    stop_after_paths: Option<usize>,
}

impl<'a> Device<'a> for Stopper {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {
        self.paths += 1;
        if Some(self.paths) == self.stop_after_paths {
            self.cancel.store(true, Ordering::Relaxed);
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
        self.glyphs += 1;
        if Some(self.glyphs) == self.stop_after_glyphs {
            self.cancel.store(true, Ordering::Relaxed);
        }
        if let Glyph::Type3(t3) = glyph {
            t3.interpret(self, transform, glyph_transform, paint);
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// Interprets page 0 of `bytes` with `max_ops` and a cancel flag shared with
/// the device.
fn run(
    bytes: Vec<u8>,
    max_ops: Option<u64>,
    stop_after_glyphs: Option<usize>,
    stop_after_paths: Option<usize>,
) -> (Stopper, InterpretStats) {
    let pdf = Pdf::new(bytes).expect("opens");
    let page = &pdf.pages()[0];
    let cancel = Arc::new(AtomicBool::new(false));
    let mut budget = InterpretBudget::new(max_ops, None);
    budget.cancel = Some(cancel.clone());
    let settings = InterpreterSettings {
        budget,
        ..InterpreterSettings::default()
    };
    let cache = InterpreterCache::new();
    let mut context = Context::new(
        Affine::IDENTITY,
        Rect::new(-1e5, -1e5, 1e5, 1e5),
        &cache,
        pdf.xref(),
        settings,
    );
    let mut device = Stopper {
        cancel,
        glyphs: 0,
        paths: 0,
        stop_after_glyphs,
        stop_after_paths,
    };
    interpret_page(page, &mut context, &mut device);
    let stats = context.stats();
    (device, stats)
}

/// Ten one-glyph `Tj`s, then a flood of 10,000 `q Q` pairs.
fn glyphs_then_flood() -> Vec<u8> {
    let mut content = String::from("BT /F 11 Tf 10 100 Td ");
    for c in 'A'..='J' {
        content.push_str(&format!("({c}) Tj "));
    }
    content.push_str("ET ");
    content.push_str(&"q Q ".repeat(10_000));
    raw_pdf(&[
        "<< /Font << /F 6 0 R >> >>".to_owned(),
        stream("", &content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ])
}

#[test]
fn the_flag_stops_interpretation_at_the_next_operator() {
    let (device, stats) = run(glyphs_then_flood(), None, Some(5), None);
    assert_eq!(device.glyphs, 5);
    assert_eq!(stats.exhausted, Some(BudgetExhaustion::Cancelled));
    // BT, Tf, Td, five Tj, and the refused sixth Tj: far below the page's
    // 20,000-odd operators, and well within one deadline check interval.
    assert_eq!(stats.ops, 9);
    assert!(stats.ops < u64::from(InterpretBudget::DEFAULT_CHECK_INTERVAL));
}

#[test]
fn an_unset_flag_changes_nothing() {
    let (device, stats) = run(glyphs_then_flood(), None, None, None);
    assert_eq!(device.glyphs, 10);
    assert_eq!(stats.exhausted, None);
    assert_eq!(stats.ops, 3 + 10 + 1 + 20_000);
}

#[test]
fn whichever_comes_first_is_the_reason() {
    // The flag is raised while operators remain: cancelled.
    let (device, stats) = run(glyphs_then_flood(), Some(6), Some(1), None);
    assert_eq!(stats.exhausted, Some(BudgetExhaustion::Cancelled));
    assert_eq!(device.glyphs, 1);
    // The operator limit runs out before the device would raise the flag:
    // the reason stays `Ops`, and nothing after it runs.
    let (device, stats) = run(glyphs_then_flood(), Some(4), Some(5), None);
    assert_eq!(stats.exhausted, Some(BudgetExhaustion::Ops));
    assert_eq!(device.glyphs, 1);
}

#[test]
fn a_type3_glyph_procedure_is_cut_too() {
    // The flag is shared with the nested context the Type 3 procedure runs
    // in: the 5th of its 10,000 paths raises it and the procedure stops.
    let procedure = format!("1000 0 d0 {}", "0 0 1 1 re f ".repeat(10_000));
    let bytes = raw_pdf(&[
        "<< /Font << /T3 6 0 R >> >>".to_owned(),
        stream("", "BT /T3 10 Tf 10 100 Td (aa) Tj ET"),
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] \
         /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 7 0 R >> \
         /Encoding << /Differences [97 /a] >> /FirstChar 97 /LastChar 97 /Widths [1000] >>"
            .to_owned(),
        stream("", &procedure),
    ]);
    let (device, stats) = run(bytes, None, None, Some(5));
    assert_eq!(device.paths, 5);
    assert_eq!(stats.exhausted, Some(BudgetExhaustion::Cancelled));
    // The second glyph is still handed to the device (it was being shown
    // when the flag was raised), but its procedure draws nothing.
    assert_eq!(device.glyphs, 2);
}
