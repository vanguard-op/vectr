//! Vectr scene engine: parse, validate, resolve, style, compile, and export.
//!
//! The [`scene`] module owns the scene document model and its strict JSON
//! reading and writing (C-001); [`primitives`] resolves the elemental shapes a
//! scene draws (FEAT-002), [`composition`] composes them into concrete
//! placements and geometry (FEAT-003), [`constraints`] resolves stated
//! relationships into concrete placements (FEAT-004), [`style`] holds the
//! palettes, stroke profiles and recipes applied to elements (FEAT-005),
//! [`compiler`] compiles a validated scene into one [`render`] model shared by
//! every exporter (FEAT-011), [`render`] defines that model (C-003), and
//! [`export`] writes that model to an output format (FEAT-012).

pub mod compiler;
pub mod composition;
pub mod constraints;

/// The export layer: a compiled model rendered to an output format.
///
/// Declared inline so the SVG subtree is addressed by its own path
/// (`export/svg/`); later exporters add sibling modules here.
pub mod export {
    pub mod png;
    pub mod svg;
}

pub mod primitives;
pub mod render;
pub mod scene;
pub mod style;

pub use compiler::{compile, compile_with_style, FontAsset, StyleContext, DEFAULT_FONT_ID};
pub use composition::{flatten_shape, is_composition, projection_for, resolve_transform, Affine};
pub use constraints::{resolve as resolve_constraints, Attachment, Frame, Placement, Resolution};
pub use export::png::{export_png, export_png_reporting, PngExport, RasterOptions};
pub use export::svg::{export_svg, export_svg_reporting, SvgExport, SvgOptions};
pub use primitives::{Ellipse, Line, Polygon, Rect, Shape};
pub use render::{
    NodeStroke, Paint, RenderCanvas, RenderMeta, RenderModel, ResolvedFont, ResolvedNode, TextRun,
};
pub use scene::{
    parse, validate, Canvas, Constraint, ConstraintKind, Diagnostic, DiagnosticCode, Diagnostics,
    Element, ElementKind, Geometry, Location, ProjectionAxis, Scene, Severity, TextAlign,
    Transform, MAX_SCENE_BYTES,
};
pub use style::{
    parse_palette, parse_stroke_profile, parse_style_recipe, resolve_fill, resolve_stroke,
    resolve_stroke_color, validate_palette, validate_palette_usage, validate_stroke_profile,
    validate_style_recipe, Palette, PaletteToken, RecipeName, RecipeParameters, ResolvedStroke,
    Shading, StrokeCap, StrokeJoin, StrokeProfile, StyleRecipe,
};
