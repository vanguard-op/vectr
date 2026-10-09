//! Vectr scene engine: parse, validate, resolve, style, compile, and export.
//!
//! The [`scene`] module owns the scene document model and its strict JSON
//! reading and writing (C-001); [`primitives`] resolves the elemental shapes a
//! scene draws (FEAT-002), [`composition`] composes them into concrete
//! placements and geometry (FEAT-003), [`constraints`] resolves stated
//! relationships into concrete placements (FEAT-004), [`style`] holds the
//! palettes, stroke profiles, gradients and recipes applied to elements
//! (FEAT-005, FEAT-027),
//! [`compiler`] compiles a validated scene into one [`render`] model shared by
//! every exporter (FEAT-011), [`render`] defines that model (C-003), [`fonts`]
//! finalizes a text node's glyph geometry (FEAT-024), [`schema`] publishes the
//! language contract for discovery (FEAT-017), and [`export`] writes that model
//! to an output format (FEAT-012).
//!
//! # Embedding
//!
//! This crate is the embeddable engine (FEAT-021): an application links it
//! directly and compiles and exports in process, with no separate process and
//! no filesystem or network access of its own. The entry points are [`parse`]
//! and [`validate`] for the document, [`compile`] and [`compile_with_style`]
//! for the render model, and [`export_svg`] and [`export_png`] for output. A
//! failure is always a [`Diagnostics`] carrying a severity, a stable code, and
//! a location — the same detail the command line prints — never a panic and
//! never a partial result (NFR-011).
//!
//! The API holds no shared mutable state, so it is safe to call from multiple
//! threads at once; identical input and seed yield byte-identical output
//! (NFR-010). PNG export sits behind the default `rasterizer` feature, so a
//! build without it reports the missing capability rather than failing
//! silently.

pub mod compiler;
pub mod composition;
pub mod constraints;

/// The export layer: a compiled model rendered to an output format.
///
/// Declared inline so the SVG subtree is addressed by its own path
/// (`export/svg/`); later exporters add sibling modules here.
pub mod export {
    pub mod pdf;
    pub mod png;
    pub mod svg;
}

pub mod fonts;
pub mod primitives;
pub mod render;
pub mod scene;
pub mod schema;
pub mod style;

pub use compiler::expand::{
    expand, validate_instances, Expansion, DEFINITION_CYCLE, INVALID_BINDING,
    UNRESOLVED_DEFINITION, UNUSED_DEFINITION,
};
pub use compiler::{
    compile, compile_definition, compile_subtree, compile_with_style, FontAsset, StyleContext,
    DEFAULT_FONT_ID, EMPTY_PART_FRAME, PART, PART_FRAME_FALLBACK,
};
pub use composition::{flatten_shape, is_composition, projection_for, resolve_transform, Affine};
pub use constraints::{resolve as resolve_constraints, Attachment, Frame, Placement, Resolution};
pub use export::pdf::{export_pdf, export_pdf_reporting, PdfExport, PdfOptions};
pub use export::png::{export_png, export_png_reporting, PngExport, RasterOptions};
pub use export::svg::{export_svg, export_svg_reporting, SvgExport, SvgOptions};
pub use fonts::{
    outline_text, FontLibrary, OutlinedText, FALLBACK_FONT_ID, FONT_MISSING, MISSING_GLYPH,
};
pub use primitives::{Ellipse, Line, Polygon, Rect, Shape};
pub use render::{
    GradientPaint, NodePaint, NodeStroke, Paint, RenderCanvas, RenderMeta, RenderModel,
    ResolvedFont, ResolvedNode, ResolvedStop, TextRun,
};
pub use scene::{
    is_color, parse, parse_definition, validate, validate_color, validate_definition, Binding,
    BindingValue, BoolValue, Canvas, Constraint, ConstraintKind, Definition, Diagnostic,
    DiagnosticCode, Diagnostics, Element, ElementKind, Geometry, Location, NumberValue, Origin,
    PaintKind, PaintValue, ParamRef, Parameter, ParameterType, ParameterValue, Procedure,
    ProjectionAxis, Scene, Severity, StringValue, TextAlign, Transform, CURRENT_FORMAT_VERSION,
    MAX_SCENE_BYTES,
};
pub use schema::{schema, schema_for, SchemaForm, SCHEMA_VERSION, UNKNOWN_SCHEMA_TYPE};
pub use style::{
    parse_gradient, parse_palette, parse_stroke_profile, parse_style_recipe, resolve_fill,
    resolve_gradient, resolve_stroke, resolve_stroke_paint, validate_gradient,
    validate_gradient_usage, validate_palette, validate_palette_usage, validate_stroke_profile,
    validate_style_recipe, Gradient, GradientStop, GradientType, LightDirection, Palette,
    PaletteToken, RecipeName, RecipeParameters, ResolvedStroke, Shading, Spread, StrokeCap,
    StrokeJoin, StrokeProfile, StyleRecipe, FREEFORM_CURVE, GRID_SNAPPED, GRID_TOO_FINE,
    ISOMETRIC_OFF_AXIS, LINE_ART_EMPTY, MIN_GRID_SIZE, MIN_STROKE_WEIGHT, STROKE_WEIGHT_CLAMPED,
};
