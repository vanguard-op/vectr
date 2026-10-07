//! Flattens the drawing primitives into closed contours for boolean work.
//!
//! Boolean composition (union, subtract, intersect) and outline offsetting act
//! on filled regions, so every primitive is reduced to one or more closed
//! point rings. Curves and arcs are sampled at a fixed resolution, which keeps
//! a given scene byte-identical from run to run (NFR-010). This is a
//! resolution over the primitive layer (C-003), not a lossless conversion: a
//! contour is only ever fed to the geometry engine, never emitted in place of
//! the exact shape.

use crate::primitives::{Ellipse, Path, Polygon, Rect, Segment, Shape};

/// A closed ring of points; the first point is not repeated at the end.
pub type Contour = Vec<[f64; 2]>;

/// The closed rings that fill one shape.
pub type Contours = Vec<Contour>;

/// Straight segments used to sample one Bézier curve.
const CURVE_STEPS: usize = 24;

/// Points used to sample a full ellipse.
const ELLIPSE_STEPS: usize = 96;

/// Points used to sample one quadrant of a rounded rectangle corner.
const CORNER_STEPS: usize = 12;

/// Sampled points per degree of an elliptical arc.
const ARC_POINTS_PER_DEGREE: f64 = 0.25;

/// Reduces one shape to its filled contours.
///
/// A line carries no area and contributes nothing; an untransformed shape is
/// flattened exactly as authored.
pub fn flatten_shape(shape: &Shape) -> Contours {
    match shape {
        Shape::Rect(rect) => vec![flatten_rect(rect)],
        Shape::Ellipse(ellipse) => vec![flatten_ellipse(ellipse)],
        Shape::Polygon(polygon) => flatten_polygon(polygon),
        Shape::Line(_) => Vec::new(),
        Shape::Path(path) => flatten_path(path),
    }
}

fn flatten_rect(rect: &Rect) -> Contour {
    let rx = rect.rx.clamp(0.0, rect.width / 2.0);
    let ry = rect.ry.clamp(0.0, rect.height / 2.0);
    if rx == 0.0 || ry == 0.0 {
        return vec![
            [rect.x, rect.y],
            [rect.x + rect.width, rect.y],
            [rect.x + rect.width, rect.y + rect.height],
            [rect.x, rect.y + rect.height],
        ];
    }

    let mut points = Vec::with_capacity(4 * (CORNER_STEPS + 1));
    let right = rect.x + rect.width;
    let bottom = rect.y + rect.height;
    // Corners, clockwise from top-right.
    arc_quadrant(&mut points, right - rx, rect.y + ry, -90.0, 0.0, rx, ry);
    arc_quadrant(&mut points, right - rx, bottom - ry, 0.0, 90.0, rx, ry);
    arc_quadrant(&mut points, rect.x + rx, bottom - ry, 90.0, 180.0, rx, ry);
    arc_quadrant(&mut points, rect.x + rx, rect.y + ry, 180.0, 270.0, rx, ry);
    dedupe(points)
}

fn arc_quadrant(
    points: &mut Vec<[f64; 2]>,
    cx: f64,
    cy: f64,
    from: f64,
    to: f64,
    rx: f64,
    ry: f64,
) {
    for step in 0..=CORNER_STEPS {
        let t = from + (to - from) * (step as f64 / CORNER_STEPS as f64);
        let (sin, cos) = t.to_radians().sin_cos();
        points.push([cx + rx * cos, cy + ry * sin]);
    }
}

fn flatten_ellipse(ellipse: &Ellipse) -> Contour {
    let mut points = Vec::with_capacity(ELLIPSE_STEPS);
    for step in 0..ELLIPSE_STEPS {
        let angle = std::f64::consts::TAU * (step as f64 / ELLIPSE_STEPS as f64);
        let (sin, cos) = angle.sin_cos();
        points.push([ellipse.cx + ellipse.rx * cos, ellipse.cy + ellipse.ry * sin]);
    }
    points
}

fn flatten_polygon(polygon: &Polygon) -> Contours {
    if polygon.points.len() < 3 {
        return Vec::new();
    }
    let mut points = polygon.points.clone();
    dedupe_close(&mut points);
    if points.len() < 3 {
        return Vec::new();
    }
    vec![points]
}

/// Flattens a path into open polylines, one per subpath, each beginning at its
/// start point.
///
/// Used where the path is a guide rather than a filled region — placing copies
/// along it (FEAT-003) — so subpaths with only a move are skipped and open
/// subpaths are kept as written.
pub fn flatten_subpaths(path: &Path) -> Vec<Vec<[f64; 2]>> {
    let mut polylines = Vec::new();
    for subpath in &path.subpaths {
        if subpath.segments.is_empty() {
            continue;
        }
        let mut points = vec![subpath.start];
        let mut current = subpath.start;
        for segment in &subpath.segments {
            flatten_segment(current, segment, &mut points);
            current = segment_end(segment);
        }
        polylines.push(points);
    }
    polylines
}

fn flatten_path(path: &Path) -> Contours {
    let mut contours = Vec::new();
    for subpath in &path.subpaths {
        if subpath.segments.is_empty() {
            continue;
        }
        let mut points = vec![subpath.start];
        let mut current = subpath.start;
        for segment in &subpath.segments {
            flatten_segment(current, segment, &mut points);
            current = segment_end(segment);
        }
        // A closed subpath must not repeat its start; the engine closes it.
        if points.len() > 1 && points.first() == points.last() {
            points.pop();
        }
        if points.len() >= 3 {
            contours.push(points);
        }
    }
    contours
}

fn flatten_segment(from: [f64; 2], segment: &Segment, points: &mut Vec<[f64; 2]>) {
    match segment {
        Segment::Line { to } => points.push(*to),
        Segment::Cubic { ctrl1, ctrl2, to } => {
            for step in 1..=CURVE_STEPS {
                let t = step as f64 / CURVE_STEPS as f64;
                points.push(cubic_at(from, *ctrl1, *ctrl2, *to, t));
            }
        }
        Segment::Quadratic { ctrl, to } => {
            for step in 1..=CURVE_STEPS {
                let t = step as f64 / CURVE_STEPS as f64;
                points.push(quadratic_at(from, *ctrl, *to, t));
            }
        }
        Segment::Arc {
            rx,
            ry,
            x_rotation,
            large_arc,
            sweep,
            to,
        } => {
            samples_of_arc(from, *to, *rx, *ry, *x_rotation, *large_arc, *sweep, points);
        }
    }
}

fn segment_end(segment: &Segment) -> [f64; 2] {
    match *segment {
        Segment::Line { to }
        | Segment::Cubic { to, .. }
        | Segment::Quadratic { to, .. }
        | Segment::Arc { to, .. } => to,
    }
}

fn cubic_at(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let (u2, t2) = (u * u, t * t);
    let (uuu, uut, utt, ttt) = (u2 * u, u2 * t, u * t2, t2 * t);
    [
        uuu * p0[0] + 3.0 * uut * p1[0] + 3.0 * utt * p2[0] + ttt * p3[0],
        uuu * p0[1] + 3.0 * uut * p1[1] + 3.0 * utt * p2[1] + ttt * p3[1],
    ]
}

fn quadratic_at(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    [
        u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0],
        u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1],
    ]
}

/// Samples the endpoint-parameterised elliptical arc of the SVG spec.
#[allow(clippy::too_many_arguments)]
fn samples_of_arc(
    from: [f64; 2],
    to: [f64; 2],
    rx: f64,
    ry: f64,
    x_rotation: f64,
    large_arc: bool,
    sweep: bool,
    points: &mut Vec<[f64; 2]>,
) {
    if rx == 0.0 || ry == 0.0 || from == to {
        points.push(to);
        return;
    }

    let phi = x_rotation.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let dx = (from[0] - to[0]) / 2.0;
    let dy = (from[1] - to[1]) / 2.0;
    let x1 = cos_phi * dx + sin_phi * dy;
    let y1 = -sin_phi * dx + cos_phi * dy;

    let mut rx = rx.abs();
    let mut ry = ry.abs();
    let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }

    let numerator = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let sign = if large_arc != sweep { 1.0 } else { -1.0 };
    let coefficient = if denominator == 0.0 {
        0.0
    } else {
        sign * (numerator / denominator).sqrt()
    };

    let cxp = coefficient * rx * y1 / ry;
    let cyp = coefficient * -ry * x1 / rx;
    let cx = cos_phi * cxp - sin_phi * cyp + (from[0] + to[0]) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (from[1] + to[1]) / 2.0;

    let start = ((y1 - cyp) / ry).atan2((x1 - cxp) / rx);
    let end = ((-y1 - cyp) / ry).atan2((-x1 - cxp) / rx);
    let mut sweep_angle = end - start;
    if !sweep && sweep_angle > 0.0 {
        sweep_angle -= std::f64::consts::TAU;
    } else if sweep && sweep_angle < 0.0 {
        sweep_angle += std::f64::consts::TAU;
    }

    let steps = ((sweep_angle.abs().to_degrees() * ARC_POINTS_PER_DEGREE).ceil() as usize).max(2);
    for step in 1..=steps {
        let angle = start + sweep_angle * (step as f64 / steps as f64);
        let (sin_a, cos_a) = angle.sin_cos();
        points.push([
            cx + rx * cos_a * cos_phi - ry * sin_a * sin_phi,
            cy + rx * cos_a * sin_phi + ry * sin_a * cos_phi,
        ]);
    }
}

/// Removes consecutive duplicate points and a repeated closing point.
fn dedupe(points: Vec<[f64; 2]>) -> Contour {
    let mut points = points;
    dedupe_close(&mut points);
    points
}

fn dedupe_close(points: &mut Contour) {
    points.dedup();
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{Line, Path, SubPath};

    #[test]
    fn a_rectangle_flattens_to_four_corners() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 5.0,
            rx: 0.0,
            ry: 0.0,
        };
        assert_eq!(
            flatten_shape(&Shape::Rect(rect)),
            vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0],]]
        );
    }

    #[test]
    fn a_rounded_rectangle_stays_within_its_bounds() {
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rx: 3.0,
            ry: 3.0,
        };
        let contours = flatten_shape(&Shape::Rect(rect));
        let contour = &contours[0];
        assert!(contour.len() > 4);
        for [x, y] in contour {
            assert!((0.0..=10.0).contains(x) && (0.0..=10.0).contains(y));
        }
    }

    #[test]
    fn a_circle_flattens_to_a_closed_ring_of_the_right_radius() {
        let ellipse = Ellipse {
            cx: 5.0,
            cy: 5.0,
            rx: 5.0,
            ry: 5.0,
        };
        let contours = flatten_shape(&Shape::Ellipse(ellipse));
        assert_eq!(contours[0].len(), ELLIPSE_STEPS);
        for [x, y] in &contours[0] {
            let radius = ((x - 5.0).powi(2) + (y - 5.0).powi(2)).sqrt();
            assert!((radius - 5.0).abs() < 1e-6, "radius {radius}");
        }
    }

    #[test]
    fn a_polygon_flattens_to_its_vertices() {
        let polygon = Polygon {
            points: vec![[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]],
        };
        assert_eq!(
            flatten_shape(&Shape::Polygon(polygon)),
            vec![vec![[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]]]
        );
    }

    #[test]
    fn a_line_carries_no_filled_area() {
        let line = Line {
            points: vec![[0.0, 0.0], [1.0, 1.0]],
        };
        assert!(flatten_shape(&Shape::Line(line)).is_empty());
    }

    #[test]
    fn a_path_subpath_flattens_including_curves() {
        let path = Path {
            subpaths: vec![SubPath {
                start: [0.0, 0.0],
                segments: vec![
                    Segment::Line { to: [10.0, 0.0] },
                    Segment::Cubic {
                        ctrl1: [20.0, 0.0],
                        ctrl2: [20.0, 10.0],
                        to: [10.0, 10.0],
                    },
                ],
                closed: true,
            }],
        };
        let contours = flatten_shape(&Shape::Path(path));
        assert_eq!(contours.len(), 1);
        assert!(contours[0].len() >= 2 + CURVE_STEPS);
    }

    #[test]
    fn flattening_is_deterministic() {
        let ellipse = Ellipse {
            cx: 0.0,
            cy: 0.0,
            rx: 4.0,
            ry: 3.0,
        };
        let once = flatten_shape(&Shape::Ellipse(ellipse));
        let twice = flatten_shape(&Shape::Ellipse(ellipse));
        assert_eq!(once, twice);
    }
}
