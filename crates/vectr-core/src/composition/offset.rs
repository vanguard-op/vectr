//! Outline offsetting: grow or shrink a shape's filled region (FEAT-003).
//!
//! Each contour is offset along its edge normals with mitered joins, then
//! re-simplified by the geometry engine so a self-crossing offset resolves to
//! clean contours. A positive distance grows each enclosed region and a
//! negative one shrinks it, so an outer boundary expands while a hole (whose
//! ring winds the other way) grows into the material — the usual offset
//! semantics.
//!
//! The result is a concrete path, so a render node carries no unresolved
//! reference (C-003).

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use crate::primitives::{Path, Segment, Shape, SubPath};
use crate::scene::{Diagnostic, Diagnostics, Element, Location};

use super::flatten::{flatten_shape, Contour};
use super::COMPOSITION;

/// The longest a miter join may stretch before it is cut to a bevel.
const MITER_LIMIT: f64 = 4.0;

/// Offsets a shape's outline by `distance`.
///
/// Returns `None` when the shape has no filled region or the offset leaves
/// none. A non-finite distance is a located error.
pub fn offset_shape(
    shape: &Shape,
    distance: f64,
    element: &Element,
    diagnostics: &mut Diagnostics,
) -> Option<Shape> {
    if !distance.is_finite() {
        diagnostics.push(
            Diagnostic::error(
                COMPOSITION,
                format!("offset `{}` must have a finite distance", element.id),
            )
            .with_location(Location::element(element.id.clone())),
        );
        return None;
    }
    if distance == 0.0 {
        return Some(shape.clone());
    }

    let mut subpaths = Vec::new();
    for contour in flatten_shape(shape) {
        let ring = offset_contour(&contour, distance);
        if ring.len() < 3 {
            continue;
        }
        for cleaned in simplify(ring) {
            if cleaned.len() >= 3 {
                subpaths.push(contour_to_subpath(&cleaned));
            }
        }
    }

    if subpaths.is_empty() {
        None
    } else {
        Some(Shape::Path(Path { subpaths }))
    }
}

fn offset_contour(contour: &Contour, distance: f64) -> Contour {
    let count = contour.len();
    if count < 3 {
        return contour.clone();
    }

    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let previous = contour[(index + count - 1) % count];
        let current = contour[index];
        let next = contour[(index + 1) % count];

        let into = normalize([current[0] - previous[0], current[1] - previous[1]]);
        let out = normalize([next[0] - current[0], next[1] - current[1]]);
        if into == [0.0, 0.0] || out == [0.0, 0.0] {
            continue;
        }

        let start = offset_point(previous, into, distance);
        let end = offset_point(current, out, distance);
        let cross = into[0] * out[1] - into[1] * out[0];
        if cross.abs() < 1e-12 {
            result.push(end);
            continue;
        }

        let diff = [end[0] - start[0], end[1] - start[1]];
        let t = (diff[0] * out[1] - diff[1] * out[0]) / cross;
        let miter = [start[0] + into[0] * t, start[1] + into[1] * t];
        if distance_between(miter, current) > MITER_LIMIT * distance.abs() {
            result.push(start);
            result.push(end);
        } else {
            result.push(miter);
        }
    }
    result
}

/// The point `distance` along the left normal of a directed edge.
fn offset_point(origin: [f64; 2], direction: [f64; 2], distance: f64) -> [f64; 2] {
    let normal = [direction[1], -direction[0]];
    [
        origin[0] + normal[0] * distance,
        origin[1] + normal[1] * distance,
    ]
}

/// Resolves a self-crossing ring into clean, non-overlapping contours.
fn simplify(ring: Contour) -> Vec<Contour> {
    let shapes = vec![ring];
    let result = shapes.overlay(&shapes, OverlayRule::Union, FillRule::NonZero);
    result
        .into_iter()
        .flatten()
        .filter(|contour| contour.len() >= 3)
        .collect()
}

fn contour_to_subpath(contour: &Contour) -> SubPath {
    let segments = contour
        .windows(2)
        .map(|pair| Segment::Line { to: pair[1] })
        .collect();
    SubPath {
        start: contour[0],
        segments,
        closed: true,
    }
}

fn normalize(vector: [f64; 2]) -> [f64; 2] {
    let length = distance_between(vector, [0.0, 0.0]);
    if length <= f64::EPSILON {
        [0.0, 0.0]
    } else {
        [vector[0] / length, vector[1] / length]
    }
}

fn distance_between(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::flatten_shape;
    use crate::primitives::{Line, Rect};
    use crate::scene::{ElementKind, Geometry, Transform};

    fn element() -> Element {
        Element {
            id: "o1".to_string(),
            scene_id: Some("s1".to_string()),
            definition_id: None,
            parent_id: None,
            order: 0,
            name: None,
            kind: ElementKind::Offset,
            geometry: Geometry::default(),
            transform: Transform {
                translate_x: 0.0,
                translate_y: 0.0,
                rotate: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
                skew_x: None,
                skew_y: None,
            },
            fill: None,
            stroke: None,
            font_id: None,
            opacity: 1.0,
            visible: true,
            definition_ref: None,
            bindings: None,
            overrides: None,
        }
    }

    fn square(size: f64) -> Shape {
        Shape::Rect(Rect {
            x: 0.0,
            y: 0.0,
            width: size,
            height: size,
            rx: 0.0,
            ry: 0.0,
        })
    }

    fn area(shape: &Shape) -> f64 {
        flatten_shape(shape)
            .iter()
            .map(|contour| {
                let n = contour.len();
                let mut sum = 0.0;
                for index in 0..n {
                    let a = contour[index];
                    let b = contour[(index + 1) % n];
                    sum += a[0] * b[1] - b[0] * a[1];
                }
                sum / 2.0
            })
            .sum()
    }

    #[test]
    fn a_positive_offset_grows_a_square() {
        let mut diagnostics = Diagnostics::new();
        let result =
            offset_shape(&square(10.0), 1.0, &element(), &mut diagnostics).expect("a result");
        assert!(diagnostics.is_empty());
        assert!((area(&result) - 144.0).abs() < 1e-6, "{}", area(&result));
    }

    #[test]
    fn a_negative_offset_shrinks_a_square() {
        let mut diagnostics = Diagnostics::new();
        let result =
            offset_shape(&square(10.0), -1.0, &element(), &mut diagnostics).expect("a result");
        assert!((area(&result) - 64.0).abs() < 1e-6, "{}", area(&result));
    }

    #[test]
    fn a_zero_offset_is_the_identity() {
        let mut diagnostics = Diagnostics::new();
        let result =
            offset_shape(&square(10.0), 0.0, &element(), &mut diagnostics).expect("a result");
        assert_eq!(result, square(10.0));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn a_non_finite_distance_is_a_located_error() {
        let mut diagnostics = Diagnostics::new();
        let result = offset_shape(&square(10.0), f64::NAN, &element(), &mut diagnostics);
        assert!(result.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, COMPOSITION);
        assert!(error.message.contains("o1"), "{}", error.message);
    }

    #[test]
    fn a_shape_without_area_offsets_to_nothing() {
        let line = Shape::Line(Line {
            points: vec![[0.0, 0.0], [10.0, 0.0]],
        });
        let mut diagnostics = Diagnostics::new();
        assert!(offset_shape(&line, 1.0, &element(), &mut diagnostics).is_none());
        assert!(!diagnostics.has_errors());
    }

    #[test]
    fn a_concave_outline_offsets_without_error() {
        let concave = Shape::Polygon(crate::primitives::Polygon {
            points: vec![
                [0.0, 0.0],
                [10.0, 0.0],
                [10.0, 4.0],
                [4.0, 4.0],
                [4.0, 10.0],
                [0.0, 10.0],
            ],
        });
        let mut diagnostics = Diagnostics::new();
        let result = offset_shape(&concave, 1.0, &element(), &mut diagnostics).expect("a result");
        assert!(diagnostics.is_empty());
        assert!(area(&result) > 64.0, "{}", area(&result));
    }
}
