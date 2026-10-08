//! Boolean composition of filled shapes: union, subtract, and intersect
//! (FEAT-003).
//!
//! Operands are flattened to closed contours and combined with the geometry
//! engine, which cleans self-intersections and returns well-formed rings with
//! outer contours counter-clockwise and holes clockwise. The result is a single
//! concrete path, so a render node carries no unresolved reference (C-003).
//!
//! A boolean that removes everything is not an error: it produces no shape and
//! reports nothing, because an empty result is a valid outcome (FEAT-003).

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use crate::primitives::{Path, Segment, Shape, SubPath};
use crate::scene::{BooleanOperation, Diagnostic, Diagnostics, Element, Location};

use super::flatten::{flatten_shape, Contours};
use super::COMPOSITION;

/// Combines two or more shapes with a boolean operation.
///
/// Returns `None` when the operation leaves no filled region — no intersection,
/// or a subtraction that removes the subject. That is a result, not a failure,
/// so no diagnostic is recorded.
pub fn combine(
    operation: BooleanOperation,
    operands: &[Shape],
    element: &Element,
    diagnostics: &mut Diagnostics,
) -> Option<Shape> {
    if operands.len() < 2 {
        reject(
            diagnostics,
            element,
            format!(
                "boolean `{}` needs at least two operands, found {}",
                element.id,
                operands.len()
            ),
        );
        return None;
    }

    let rule = match operation {
        BooleanOperation::Union => OverlayRule::Union,
        BooleanOperation::Subtract => OverlayRule::Difference,
        BooleanOperation::Intersect => OverlayRule::Intersect,
    };

    let mut subject: Vec<Contours> = vec![flatten_shape(&operands[0])];
    for operand in &operands[1..] {
        let clip: Contours = flatten_shape(operand);
        if clip.is_empty() {
            // Union and subtract are unchanged by an empty operand; an
            // intersection with nothing is empty.
            if operation == BooleanOperation::Intersect {
                return None;
            }
            continue;
        }
        subject = subject.overlay(&clip, rule, FillRule::NonZero);
    }

    shapes_to_path(subject)
}

/// Converts the engine's shapes into one concrete path.
///
/// Every contour becomes a closed subpath; outer contours precede their holes,
/// and their engine-guaranteed orientation lets a nonzero fill rule reproduce
/// the result.
fn shapes_to_path(shapes: Vec<Contours>) -> Option<Shape> {
    let mut subpaths = Vec::new();
    for shape in shapes {
        for contour in shape {
            if contour.len() < 3 {
                continue;
            }
            let start = contour[0];
            let segments = contour
                .windows(2)
                .map(|pair| Segment::Line { to: pair[1] })
                .collect();
            subpaths.push(SubPath {
                start,
                segments,
                closed: true,
            });
        }
    }
    if subpaths.is_empty() {
        None
    } else {
        Some(Shape::Path(Path { subpaths }))
    }
}

fn reject(diagnostics: &mut Diagnostics, element: &Element, message: String) {
    diagnostics.push(
        Diagnostic::error(COMPOSITION, message)
            .with_location(Location::element(element.id.clone())),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{Ellipse, Polygon, Rect};
    use crate::scene::{Geometry, Transform};

    fn element(kind: crate::scene::ElementKind) -> Element {
        Element {
            id: "b1".to_string(),
            scene_id: "s1".to_string(),
            parent_id: None,
            order: 0,
            name: None,
            kind,
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
            fill_token: None,
            stroke_profile_id: None,
            stroke_token: None,
            font_id: None,
            opacity: 1.0,
            visible: true,
        }
    }

    fn square(x: f64, y: f64, size: f64) -> Shape {
        Shape::Rect(Rect {
            x,
            y,
            width: size,
            height: size,
            rx: 0.0,
            ry: 0.0,
        })
    }

    fn area(shape: &Shape) -> f64 {
        // Shoelace over the flattened contour; sufficient to compare results.
        super::super::flatten::flatten_shape(shape)
            .iter()
            .map(|contour| {
                let mut sum = 0.0;
                for pair in contour.windows(2) {
                    sum += pair[0][0] * pair[1][1] - pair[1][0] * pair[0][1];
                }
                sum += contour[contour.len() - 1][0] * contour[0][1]
                    - contour[0][0] * contour[contour.len() - 1][1];
                sum / 2.0
            })
            .sum()
    }

    #[test]
    fn subtract_removes_the_clip_from_the_subject() {
        let mut diagnostics = Diagnostics::new();
        let result = combine(
            BooleanOperation::Subtract,
            &[square(0.0, 0.0, 10.0), square(2.0, 2.0, 3.0)],
            &element(crate::scene::ElementKind::Boolean),
            &mut diagnostics,
        )
        .expect("a result");
        assert!(diagnostics.is_empty());
        assert!(
            (area(&result) - (100.0 - 9.0)).abs() < 1e-6,
            "{}",
            area(&result)
        );
    }

    #[test]
    fn union_covers_both_operands_once() {
        let mut diagnostics = Diagnostics::new();
        let result = combine(
            BooleanOperation::Union,
            &[square(0.0, 0.0, 10.0), square(5.0, 0.0, 10.0)],
            &element(crate::scene::ElementKind::Boolean),
            &mut diagnostics,
        )
        .expect("a result");
        assert!((area(&result) - 150.0).abs() < 1e-6, "{}", area(&result));
    }

    #[test]
    fn intersect_keeps_only_the_overlap() {
        let mut diagnostics = Diagnostics::new();
        let result = combine(
            BooleanOperation::Intersect,
            &[square(0.0, 0.0, 10.0), square(5.0, 0.0, 10.0)],
            &element(crate::scene::ElementKind::Boolean),
            &mut diagnostics,
        )
        .expect("a result");
        assert!((area(&result) - 50.0).abs() < 1e-6, "{}", area(&result));
    }

    #[test]
    fn an_intersection_with_no_overlap_is_empty_not_an_error() {
        let mut diagnostics = Diagnostics::new();
        let result = combine(
            BooleanOperation::Intersect,
            &[square(0.0, 0.0, 2.0), square(50.0, 50.0, 2.0)],
            &element(crate::scene::ElementKind::Boolean),
            &mut diagnostics,
        );
        assert!(result.is_none());
        assert!(!diagnostics.has_errors());
    }

    #[test]
    fn a_boolean_without_enough_operands_is_rejected_naming_the_element() {
        let mut diagnostics = Diagnostics::new();
        let result = combine(
            BooleanOperation::Union,
            &[square(0.0, 0.0, 1.0)],
            &element(crate::scene::ElementKind::Boolean),
            &mut diagnostics,
        );
        assert!(result.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, COMPOSITION);
        assert!(error.message.contains("b1"), "{}", error.message);
    }

    #[test]
    fn a_rectangular_result_is_a_closed_path() {
        let mut diagnostics = Diagnostics::new();
        let result = combine(
            BooleanOperation::Subtract,
            &[square(0.0, 0.0, 10.0), square(4.0, 4.0, 2.0)],
            &element(crate::scene::ElementKind::Boolean),
            &mut diagnostics,
        )
        .expect("a result");
        let Shape::Path(path) = result else {
            panic!("expected a concrete path");
        };
        assert_eq!(path.subpaths.len(), 2, "outer contour and hole");
        assert!(path.subpaths.iter().all(|subpath| subpath.closed));
    }

    #[test]
    fn boolean_is_deterministic() {
        let a = Shape::Ellipse(Ellipse {
            cx: 0.0,
            cy: 0.0,
            rx: 5.0,
            ry: 5.0,
        });
        let b = Shape::Polygon(Polygon {
            points: vec![[-1.0, -1.0], [6.0, -1.0], [6.0, 6.0], [-1.0, 6.0]],
        });
        let mut first = Diagnostics::new();
        let mut second = Diagnostics::new();
        let once = combine(
            BooleanOperation::Union,
            &[a.clone(), b.clone()],
            &element(crate::scene::ElementKind::Boolean),
            &mut first,
        );
        let twice = combine(
            BooleanOperation::Union,
            &[a, b],
            &element(crate::scene::ElementKind::Boolean),
            &mut second,
        );
        assert_eq!(once, twice);
    }
}
