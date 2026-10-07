//! Vectr scene engine: parse, validate, resolve, style, compile, and export.
//!
//! The [`scene`] module owns the scene document model and its strict JSON
//! reading and writing (C-001); [`primitives`] resolves the elemental shapes a
//! scene draws (FEAT-002) and [`composition`] composes them into concrete
//! placements and geometry (FEAT-003). Later modules — style, compiler, render,
//! export — build on the types re-exported here.

pub mod composition;
pub mod primitives;
pub mod scene;

pub use composition::{flatten_shape, is_composition, projection_for, resolve_transform, Affine};
pub use primitives::{Ellipse, Line, Polygon, Rect, Shape};
pub use scene::{
    parse, validate, Canvas, Constraint, ConstraintKind, Diagnostic, DiagnosticCode, Diagnostics,
    Element, ElementKind, Geometry, Location, ProjectionAxis, Scene, Severity, Transform,
    MAX_SCENE_BYTES,
};
