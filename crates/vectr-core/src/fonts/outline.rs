//! Glyph outline collection for the Font & Asset Manager (FEAT-024).
//!
//! `ttf-parser` walks a glyph's contours through the [`OutlineBuilder`] trait,
//! reporting points in font units with the y-axis pointing up. This builder
//! turns each command into the engine's concrete [`SubPath`]/[`Segment`]
//! geometry, scaled into scene units and flipped to the y-down axis the render
//! model and every exporter use, so the result drops straight into the same
//! path representation the rest of the pipeline carries.

use ttf_parser::OutlineBuilder;

use crate::primitives::{Segment, SubPath};

/// Collects one glyph's contours as concrete subpaths in scene units.
///
/// The glyph is placed by its origin: `dx` is the pen position plus the glyph's
/// horizontal offset and `baseline` is the line's baseline minus its vertical
/// offset, both in scene units. A font-unit point `(x, y)` becomes
/// `(dx + x·scale, baseline - y·scale)`, which both scales the em and flips the
/// font's y-up axis to the y-down axis the render model uses.
pub(super) struct GlyphOutline {
    pub(super) subpaths: Vec<SubPath>,
    current: Option<usize>,
    scale: f64,
    dx: f64,
    baseline: f64,
}

impl GlyphOutline {
    /// A collector placing one glyph at the given origin and scale.
    pub(super) fn new(scale: f64, dx: f64, baseline: f64) -> Self {
        Self {
            subpaths: Vec::new(),
            current: None,
            scale,
            dx,
            baseline,
        }
    }

    fn point(&self, x: f32, y: f32) -> [f64; 2] {
        [
            self.dx + f64::from(x) * self.scale,
            self.baseline - f64::from(y) * self.scale,
        ]
    }

    fn push(&mut self, segment: Segment) {
        if let Some(index) = self.current {
            self.subpaths[index].segments.push(segment);
        }
    }
}

impl OutlineBuilder for GlyphOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        let start = self.point(x, y);
        self.subpaths.push(SubPath {
            start,
            segments: Vec::new(),
            closed: false,
        });
        self.current = Some(self.subpaths.len() - 1);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.point(x, y);
        self.push(Segment::Line { to });
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let ctrl = self.point(x1, y1);
        let to = self.point(x, y);
        self.push(Segment::Quadratic { ctrl, to });
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let ctrl1 = self.point(x1, y1);
        let ctrl2 = self.point(x2, y2);
        let to = self.point(x, y);
        self.push(Segment::Cubic { ctrl1, ctrl2, to });
    }

    fn close(&mut self) {
        if let Some(index) = self.current {
            self.subpaths[index].closed = true;
        }
    }
}
