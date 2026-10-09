//! Placement along a path: copies of one shape distributed along a guide
//! (FEAT-003).
//!
//! The guide is flattened to polylines, copies are spaced evenly by arc length,
//! and each is rotated to the guide's local direction. The result is one
//! transform per copy, so the compiler keeps a single child definition.

use crate::primitives::{Path, EMPTY_PATH};
use crate::scene::{Diagnostic, Diagnostics, Element, Location};

use super::flatten::flatten_subpaths;
use super::transform::Affine;
use super::COUNT_ZERO;

/// The per-copy placements along a guide path.
///
/// `count` copies are distributed over the whole guide: the first at its start,
/// the last at its end (a single copy sits at the midpoint). A count of zero
/// renders nothing and warns; a guide that draws no length cannot place copies
/// and warns.
pub fn placements(
    guide: &Path,
    count: u32,
    element: &Element,
    diagnostics: &mut Diagnostics,
) -> Vec<Affine> {
    if count == 0 {
        diagnostics.push(
            Diagnostic::warning(
                COUNT_ZERO,
                format!(
                    "alongPath `{}` has a count of zero and renders nothing",
                    element.id
                ),
            )
            .with_location(Location::element(element.id.clone())),
        );
        return Vec::new();
    }

    let edges = edges_of(guide);
    let total: f64 = edges.iter().map(|edge| edge.length).sum();
    if edges.is_empty() || total <= f64::EPSILON {
        diagnostics.push(
            Diagnostic::warning(
                EMPTY_PATH,
                format!(
                    "alongPath `{}` has no guide length and places nothing",
                    element.id
                ),
            )
            .with_location(Location::element(element.id.clone())),
        );
        return Vec::new();
    }

    (0..count)
        .map(|index| {
            let fraction = if count == 1 {
                0.5
            } else {
                index as f64 / (count - 1) as f64
            };
            sample(&edges, fraction * total)
        })
        .collect()
}

struct Edge {
    from: [f64; 2],
    to: [f64; 2],
    start: f64,
    length: f64,
}

fn edges_of(guide: &Path) -> Vec<Edge> {
    let mut edges = Vec::new();
    let mut start = 0.0;
    for polyline in flatten_subpaths(guide) {
        for pair in polyline.windows(2) {
            let length = distance(pair[0], pair[1]);
            if length <= f64::EPSILON {
                continue;
            }
            edges.push(Edge {
                from: pair[0],
                to: pair[1],
                start,
                length,
            });
            start += length;
        }
    }
    edges
}

fn sample(edges: &[Edge], target: f64) -> Affine {
    let edge = edges
        .iter()
        .find(|edge| target <= edge.start + edge.length)
        .unwrap_or_else(|| edges.last().expect("a non-empty guide"));
    let t = ((target - edge.start) / edge.length).clamp(0.0, 1.0);
    let point = [
        edge.from[0] + (edge.to[0] - edge.from[0]) * t,
        edge.from[1] + (edge.to[1] - edge.from[1]) * t,
    ];
    let angle = (edge.to[1] - edge.from[1])
        .atan2(edge.to[0] - edge.from[0])
        .to_degrees();
    Affine::translate(point[0], point[1]).then(Affine::rotate(angle))
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{parse, Segment, SubPath};
    use crate::scene::{BoolValue, ElementKind, Geometry, NumberValue, Transform};

    fn element() -> Element {
        Element {
            id: "p1".to_string(),
            scene_id: Some("s1".to_string()),
            definition_id: None,
            parent_id: None,
            order: 0,
            name: None,
            accessible_name: None,
            kind: ElementKind::AlongPath,
            geometry: Geometry::default(),
            transform: Transform {
                translate_x: NumberValue::Literal(0.0),
                translate_y: NumberValue::Literal(0.0),
                rotate: NumberValue::Literal(0.0),
                scale_x: NumberValue::Literal(1.0),
                scale_y: NumberValue::Literal(1.0),
                skew_x: None,
                skew_y: None,
            },
            fill: None,
            stroke: None,
            font_id: None,
            opacity: NumberValue::Literal(1.0),
            visible: BoolValue::Literal(true),
            definition_ref: None,
            bindings: None,
        }
    }

    fn horizontal(length: f64) -> Path {
        Path {
            subpaths: vec![SubPath {
                start: [0.0, 0.0],
                segments: vec![Segment::Line { to: [length, 0.0] }],
                closed: false,
            }],
        }
    }

    #[test]
    fn copies_are_distributed_over_the_whole_guide() {
        let mut diagnostics = Diagnostics::new();
        let placements = placements(&horizontal(30.0), 3, &element(), &mut diagnostics);
        assert_eq!(placements.len(), 3);
        assert_eq!(placements[0].apply([0.0, 0.0]), [0.0, 0.0]);
        assert_eq!(placements[1].apply([0.0, 0.0]), [15.0, 0.0]);
        assert_eq!(placements[2].apply([0.0, 0.0]), [30.0, 0.0]);
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn copies_follow_the_guide_direction() {
        let guide = parse("M0 0 L0 10").expect("valid path");
        let mut diagnostics = Diagnostics::new();
        let placements = placements(&guide, 2, &element(), &mut diagnostics);
        // Point the copy's local +X along the guide: it should now face down.
        let tip = placements[1].apply([1.0, 0.0]);
        assert!((tip[0] - 0.0).abs() < 1e-9, "{tip:?}");
        assert!((tip[1] - 11.0).abs() < 1e-9, "{tip:?}");
    }

    #[test]
    fn a_single_copy_sits_at_the_midpoint() {
        let mut diagnostics = Diagnostics::new();
        let placements = placements(&horizontal(20.0), 1, &element(), &mut diagnostics);
        assert_eq!(placements[0].apply([0.0, 0.0]), [10.0, 0.0]);
    }

    #[test]
    fn a_zero_count_renders_nothing_and_warns() {
        let mut diagnostics = Diagnostics::new();
        let placements = placements(&horizontal(10.0), 0, &element(), &mut diagnostics);
        assert!(placements.is_empty());
        assert!(!diagnostics.has_errors());
        assert_eq!(
            diagnostics.warnings().next().map(|d| d.code.clone()),
            Some(COUNT_ZERO)
        );
    }

    #[test]
    fn a_guide_without_length_warns_and_places_nothing() {
        let guide = Path {
            subpaths: vec![SubPath {
                start: [5.0, 5.0],
                segments: vec![],
                closed: false,
            }],
        };
        let mut diagnostics = Diagnostics::new();
        let placements = placements(&guide, 3, &element(), &mut diagnostics);
        assert!(placements.is_empty());
        assert_eq!(
            diagnostics.warnings().next().map(|d| d.code.clone()),
            Some(EMPTY_PATH)
        );
        assert!(!diagnostics.has_errors());
    }
}
