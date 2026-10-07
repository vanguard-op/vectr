//! Vectr scene engine: parse, validate, resolve, style, compile, and export.
//!
//! The [`scene`] module owns the scene document model and its strict JSON
//! reading and writing (C-001); [`primitives`] resolves the elemental shapes a
//! scene draws (FEAT-002). Later modules — style, compiler, render, export —
//! build on the types re-exported here.

pub mod primitives;
pub mod scene;

pub use primitives::{Ellipse, Line, Polygon, Rect, Shape};
pub use scene::{
    parse, validate, Canvas, Constraint, ConstraintKind, Diagnostic, DiagnosticCode, Diagnostics,
    Element, ElementKind, Geometry, Location, Scene, Severity, Transform, MAX_SCENE_BYTES,
};
