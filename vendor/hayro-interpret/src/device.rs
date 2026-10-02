use crate::font::Glyph;
use crate::soft_mask::SoftMask;
use crate::{BlendMode, ClipPath, Image};
use crate::{GlyphDrawMode, Paint, PathDrawMode};
use hayro_syntax::object::{Dict, ObjectIdentifier};
use kurbo::{Affine, BezPath, Rect, Shape};

/// A trait for a device that can be used to process PDF drawing instructions.
pub trait Device<'a> {
    /// Set the properties for future stroking operations.
    /// Set a soft mask to be used for future drawing instructions.
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>);
    /// Set the blend mode that should be used for rendering operations.
    fn set_blend_mode(&mut self, blend_mode: BlendMode);
    /// Draw a path.
    fn draw_path(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &PathDrawMode,
    );
    /// Push a new clip path to the clip stack.
    fn push_clip_path(&mut self, clip_path: &ClipPath);
    /// Push a new transparency group to the blend stack.
    fn push_transparency_group(
        &mut self,
        opacity: f32,
        mask: Option<SoftMask<'a>>,
        blend_mode: BlendMode,
    );
    /// Draw a glyph.
    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &Paint<'a>,
        // TODO: Move this into outline glyph.
        draw_mode: &GlyphDrawMode,
    );
    /// Draw an image.
    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine);
    /// Pop the last clip path from the clip stack.
    fn pop_clip_path(&mut self);
    /// Pop the last transparency group from the blend stack.
    fn pop_transparency_group(&mut self);
    /// Draw a rectangle directly, without going through the general path pipeline.
    fn draw_rect(
        &mut self,
        rect: &Rect,
        transform: Affine,
        paint: &Paint<'a>,
        draw_mode: &PathDrawMode,
    ) {
        self.draw_path(&rect.to_path(0.1), transform, paint, draw_mode);
    }
    /// Called at the beginning of a marked content sequence (BMC/BDC).
    ///
    /// The tag is the marked content tag (e.g. b"P", b"Span"). The mcid is
    /// the marked content identifier from the properties dict, if present.
    ///
    /// `properties` is the property list of a `BDC` operator: the inline
    /// dictionary, or the entry of the resources' `/Properties` dictionary
    /// that a name operand refers to. It is `None` for `BMC` and when a name
    /// does not resolve to a dictionary. Devices use it for entries such as
    /// `/ActualText`, `/Alt` or `/Lang`. An inline dictionary lives in the
    /// content stream being interpreted, which may be shorter-lived than the
    /// document, so the dictionary is only borrowed for the duration of the
    /// call.
    fn begin_marked_content(
        &mut self,
        _tag: &[u8],
        _mcid: Option<i32>,
        _properties: Option<&Dict<'_>>,
    ) {
    }
    /// Called at the end of a marked content sequence (EMC).
    fn end_marked_content(&mut self) {}
    /// Called before the content of a Form `XObject` is drawn, with the form's object
    /// identifier (`None` for a form that is not an indirect object).
    ///
    /// Every call is matched by one [`end_form`](Device::end_form) after the form's
    /// content; forms drawn inside the form (and annotation appearance streams, which are
    /// forms) nest. A form that is not drawn at all (hidden by optional content, skipped
    /// because it draws itself or is nested too deeply, or after the interpretation
    /// budget ran out) causes no call. Text extraction uses the pair as a boundary
    /// between the text inside the form and the text around it.
    fn begin_form(&mut self, _id: Option<ObjectIdentifier>) {}
    /// Called after the content of a Form `XObject` announced by
    /// [`begin_form`](Device::begin_form) was drawn.
    fn end_form(&mut self) {}
    /// Called before the appearance stream of an annotation is drawn, with the
    /// annotation's `/Subtype` (empty if it has none).
    ///
    /// Matched by one [`end_annotation`](Device::end_annotation). The appearance stream
    /// itself is a form, so a [`begin_form`](Device::begin_form)/
    /// [`end_form`](Device::end_form) pair normally follows inside, unless the form is
    /// not drawn. Annotations that are not drawn (hidden, without an appearance stream,
    /// or of a subtype that is never drawn) cause no call.
    fn begin_annotation(&mut self, _subtype: &[u8]) {}
    /// Called after the appearance stream of an annotation announced by
    /// [`begin_annotation`](Device::begin_annotation) was drawn.
    fn end_annotation(&mut self) {}
}

/// A device that discards all drawing operations.
pub struct DummyDevice;

impl Device<'_> for DummyDevice {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
    fn draw_glyph(
        &mut self,
        _: &Glyph<'_>,
        _: Affine,
        _: Affine,
        _: &Paint<'_>,
        _: &GlyphDrawMode,
    ) {
    }
    fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}
