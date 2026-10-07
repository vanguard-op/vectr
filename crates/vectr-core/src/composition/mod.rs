//! Composition primitives: grouping, transforms, repetition, booleans, paths
//! and projections (FEAT-003).
//!
//! These primitives lower a composition into the concrete placements and
//! geometry the render model carries (C-003). They are the pieces the compiler
//! composes over the element tree: [`resolve_transform`] and [`inherit`] flatten
//! a hierarchy, [`placements`] expands a repeat, [`combine`] folds a boolean,
//! and [`projection_for`] maps children onto a projection axis.
//!
//! Every operation here is pure and deterministic (NFR-010): the same scene
//! always yields the same placements and geometry, and a failure is reported
//! with the element's location rather than producing partial output (NFR-011).

pub mod along_path;
pub mod boolean;
pub mod flatten;
pub mod offset;
pub mod repeat;
pub mod transform;

pub use along_path::placements as along_path_placements;
pub use boolean::combine;
pub use flatten::{flatten_shape, flatten_subpaths, Contour, Contours};
pub use offset::offset_shape;
pub use repeat::placements;
pub use transform::Affine;

use crate::scene::{
    Diagnostic, DiagnosticCode, Diagnostics, Element, ElementKind, Location, ProjectionAxis,
};

/// A composition is malformed: a boolean with too few operands, or invalid
/// composition geometry.
pub const COMPOSITION: DiagnosticCode = DiagnosticCode::new("E_COMPOSITION");

/// An element declares a transform that cannot be applied.
pub const TRANSFORM: DiagnosticCode = DiagnosticCode::new("E_TRANSFORM");

/// A repeat has a count of zero, so it renders nothing.
pub const COUNT_ZERO: DiagnosticCode = DiagnosticCode::new("W_COUNT_ZERO");

/// Whether an element kind is a composition operation.
pub fn is_composition(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Group
            | ElementKind::Repeat
            | ElementKind::Boolean
            | ElementKind::AlongPath
            | ElementKind::Offset
            | ElementKind::Projection
    )
}

/// Resolves an element's declared transform into a matrix.
///
/// A field that is not a finite number is refused: an invalid transform is a
/// located error naming the malformed field, never a silent identity
/// (FEAT-003).
pub fn resolve_transform(element: &Element, diagnostics: &mut Diagnostics) -> Option<Affine> {
    match Affine::from_scene(&element.transform) {
        Ok(affine) => Some(affine),
        Err(field) => {
            diagnostics.push(
                Diagnostic::error(
                    TRANSFORM,
                    format!(
                        "element `{}` has a malformed transform: `{field}` is not a finite number",
                        element.id
                    ),
                )
                .with_location(Location::element_at(
                    element.id.clone(),
                    format!("/transform/{field}"),
                )),
            );
            None
        }
    }
}

/// The world transform of a child inside a composition.
///
/// The parent composes over the child, so a group carrying a rotation turns
/// every child about the group's own origin (FEAT-003).
pub fn inherit(parent: Affine, child: Affine) -> Affine {
    parent.then(child)
}

/// The isometric projection helper (FEAT-003).
pub fn projection() -> Affine {
    Affine::isometric()
}

/// The projection helper for a given axis (FEAT-003).
///
/// The `projection` element's axis selects the mapping: the x and y axes
/// flatten children onto that axis, and `isometric` maps them onto the scene's
/// isometric axes.
pub fn projection_for(axis: ProjectionAxis) -> Affine {
    Affine::projection(axis)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Geometry, Transform};

    fn element(kind: ElementKind, transform: Transform) -> Element {
        Element {
            id: "g1".to_string(),
            scene_id: "s1".to_string(),
            parent_id: None,
            order: 0,
            name: None,
            kind,
            geometry: Geometry::default(),
            transform,
            fill_token: None,
            stroke_profile_id: None,
            opacity: 1.0,
            visible: true,
        }
    }

    fn identity() -> Transform {
        Transform {
            translate_x: 0.0,
            translate_y: 0.0,
            rotate: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            skew_x: None,
            skew_y: None,
        }
    }

    #[test]
    fn a_group_composes_over_its_children_about_its_origin() {
        let parent = resolve_transform(
            &element(
                ElementKind::Group,
                Transform {
                    translate_x: 100.0,
                    rotate: 90.0,
                    ..identity()
                },
            ),
            &mut Diagnostics::new(),
        )
        .unwrap();
        let child = Affine::translate(10.0, 0.0);
        let world = inherit(parent, child).apply([0.0, 0.0]);
        assert!((world[0] - 100.0).abs() < 1e-9, "{world:?}");
        assert!((world[1] - 10.0).abs() < 1e-9, "{world:?}");
    }

    #[test]
    fn a_malformed_transform_is_a_located_error_naming_the_field() {
        let mut diagnostics = Diagnostics::new();
        let resolved = resolve_transform(
            &element(
                ElementKind::Group,
                Transform {
                    skew_x: Some(f64::INFINITY),
                    ..identity()
                },
            ),
            &mut diagnostics,
        );
        assert!(resolved.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, TRANSFORM);
        assert!(error.message.contains("skewX"), "{}", error.message);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/transform/skewX")
        );
    }

    #[test]
    fn the_composition_kinds_are_recognised() {
        for kind in [
            ElementKind::Group,
            ElementKind::Repeat,
            ElementKind::Boolean,
            ElementKind::AlongPath,
            ElementKind::Offset,
            ElementKind::Projection,
        ] {
            assert!(is_composition(kind));
        }
        assert!(!is_composition(ElementKind::Rect));
    }

    #[test]
    fn the_isometric_projection_lifts_both_axes() {
        let x_axis = projection().apply([1.0, 0.0]);
        let y_axis = projection().apply([0.0, 1.0]);
        assert!(x_axis[0] > 0.0 && x_axis[1] > 0.0);
        assert!(y_axis[0] < 0.0 && y_axis[1] > 0.0);
    }

    #[test]
    fn the_axis_projections_flatten_onto_their_axis() {
        assert_eq!(
            projection_for(ProjectionAxis::X).apply([3.0, 4.0]),
            [3.0, 0.0]
        );
        assert_eq!(
            projection_for(ProjectionAxis::Y).apply([3.0, 4.0]),
            [0.0, 4.0]
        );
    }
}
