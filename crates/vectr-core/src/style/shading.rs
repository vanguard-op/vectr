//! Single-layer shading: deriving the flat tonal region a filled shape gains on
//! the side facing away from the scene's light (FEAT-028).
//!
//! The region is a property of a resolved fill area, not of the element that
//! authored it: it is the part of the fill area on the far side of the straight
//! line through the centre of the area's axis-aligned bounding box,
//! perpendicular to the light vector, bounded by the shape's own outline and cut
//! by that one hard edge (docs/Vectr/features/FEAT-028.md). Deriving it here
//! keeps the style system the single owner of the look, and the result is
//! deterministic and pure vector (NFR-010).
//!
//! The shape and the light are both read in scene coordinates — the light is
//! one of eight compass directions and the bounding box is the shape's
//! axis-aligned box as it is placed — so the region is derived in the world
//! frame and carried back into the shape's own local frame, ready to follow its
//! base node under the same transform (C-003).

use crate::composition::{flatten_shape, Affine};
use crate::primitives::{Path, Segment, Shape, SubPath};

use super::recipe::LightDirection;

/// The unit light vector for a compass direction, in scene coordinates, with x
/// to the right and y downward (FEAT-028).
pub fn light_vector(direction: LightDirection) -> [f64; 2] {
    // The four diagonal directions are the axes rotated a half step, so each
    // component is 1/√2.
    const DIAGONAL: f64 = std::f64::consts::FRAC_1_SQRT_2;
    match direction {
        LightDirection::Top => [0.0, -1.0],
        LightDirection::TopRight => [DIAGONAL, -DIAGONAL],
        LightDirection::Right => [1.0, 0.0],
        LightDirection::BottomRight => [DIAGONAL, DIAGONAL],
        LightDirection::Bottom => [0.0, 1.0],
        LightDirection::BottomLeft => [-DIAGONAL, DIAGONAL],
        LightDirection::Left => [-1.0, 0.0],
        LightDirection::TopLeft => [-DIAGONAL, -DIAGONAL],
    }
}

/// Why a filled shape's shading region could not be derived (FEAT-028).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// The shape encloses no fill area, so it has no outline to clip against.
    NoOutline,
    /// The derived region is empty: the fill area lies entirely on the lit side.
    EmptyRegion,
    /// The shape's transform cannot be inverted to carry the region home.
    Degenerate,
}

impl Unavailable {
    /// The cause, phrased for a diagnostic message.
    pub fn reason(self) -> &'static str {
        match self {
            Unavailable::NoOutline => "the shape encloses no fill area",
            Unavailable::EmptyRegion => {
                "the shading region is empty; the shape lies entirely on the side toward the light"
            }
            Unavailable::Degenerate => "the shape's transform cannot be inverted",
        }
    }
}

/// Derives the shading region for one filled shape (FEAT-028).
///
/// `geometry` is the node's local fill area and `transform` its resolved world
/// transform; the returned shape is in the same local coordinates, ready to
/// follow its base node under that transform. The region is the fill area on the
/// far side of the straight line through the centre of its axis-aligned bounding
/// box, perpendicular to `light`, so it is always a subset of the fill area and
/// never covers the side toward the light.
///
/// A shape with no enclosed area, a region that comes out empty, and a transform
/// that cannot be inverted are reported rather than guessed, so the caller can
/// name the cause instead of silently leaving the shape unshaded.
pub fn region(geometry: &Shape, transform: &Affine, light: [f64; 2]) -> Result<Shape, Unavailable> {
    let contours = flatten_shape(geometry);
    if contours.iter().all(|contour| contour.len() < 3) {
        return Err(Unavailable::NoOutline);
    }

    let inverse = inverse(*transform).ok_or(Unavailable::Degenerate)?;

    // The fill area as it is placed, so the bounding box and the clip both live
    // in scene coordinates.
    let world: Vec<Vec<[f64; 2]>> = contours
        .iter()
        .map(|contour| {
            contour
                .iter()
                .map(|point| transform.apply(*point))
                .collect()
        })
        .collect();
    let (min, max) = world_bounds(&world).ok_or(Unavailable::NoOutline)?;
    let boundary = [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0];

    let mut subpaths = Vec::new();
    for contour in &world {
        let clipped = clip_far_side(contour, boundary, light);
        if clipped.len() < 3 || area(&clipped) == 0.0 {
            continue;
        }
        // Carry the region back into the shape's own frame so the base node's
        // transform places it exactly where the clip put it.
        let points: Vec<[f64; 2]> = clipped.iter().map(|point| inverse.apply(*point)).collect();
        if let Some(subpath) = closed_subpath(points) {
            subpaths.push(subpath);
        }
    }

    if subpaths.is_empty() {
        return Err(Unavailable::EmptyRegion);
    }
    Ok(Shape::Path(Path { subpaths }))
}

/// The axis-aligned bounds every point of the placed fill area spans.
fn world_bounds(contours: &[Vec<[f64; 2]>]) -> Option<([f64; 2], [f64; 2])> {
    let mut bounds: Option<([f64; 2], [f64; 2])> = None;
    for point in contours.iter().flatten() {
        if !point[0].is_finite() || !point[1].is_finite() {
            continue;
        }
        bounds = Some(match bounds {
            Some((min, max)) => (
                [min[0].min(point[0]), min[1].min(point[1])],
                [max[0].max(point[0]), max[1].max(point[1])],
            ),
            None => (*point, *point),
        });
    }
    bounds
}

/// Clips one closed ring to the far side of the light line (FEAT-028).
///
/// A point is on the far side when `(p − b)·light ≤ 0`, with `b` the boundary
/// point. The ring is walked in order and the crossings with the line are
/// interpolated, so the result is a closed ring again.
fn clip_far_side(ring: &[[f64; 2]], boundary: [f64; 2], light: [f64; 2]) -> Vec<[f64; 2]> {
    let signed =
        |point: [f64; 2]| (point[0] - boundary[0]) * light[0] + (point[1] - boundary[1]) * light[1];

    let mut out = Vec::with_capacity(ring.len() + 1);
    for index in 0..ring.len() {
        let current = ring[index];
        let next = ring[(index + 1) % ring.len()];
        let current_signed = signed(current);
        let next_signed = signed(next);
        if current_signed <= 0.0 {
            out.push(current);
        }
        // The line is crossed between the two points: add the intersection.
        if (current_signed < 0.0 && next_signed > 0.0)
            || (current_signed > 0.0 && next_signed < 0.0)
        {
            let t = current_signed / (current_signed - next_signed);
            out.push([
                current[0] + (next[0] - current[0]) * t,
                current[1] + (next[1] - current[1]) * t,
            ]);
        }
    }
    out
}

/// Builds a closed subpath from a ring of points, dropping a repeated closing
/// point. A ring with fewer than three points is not a region.
fn closed_subpath(mut points: Vec<[f64; 2]>) -> Option<SubPath> {
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    if points.len() < 3 {
        return None;
    }
    let start = points[0];
    let segments = points[1..]
        .iter()
        .map(|point| Segment::Line { to: *point })
        .collect();
    Some(SubPath {
        start,
        segments,
        closed: true,
    })
}

/// The signed area of a ring, via the shoelace formula. Zero means the ring
/// encloses nothing.
fn area(points: &[[f64; 2]]) -> f64 {
    let mut sum = 0.0;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        sum += current[0] * next[1] - next[0] * current[1];
    }
    sum / 2.0
}

/// The inverse of an affine transform, or `None` when it is singular.
fn inverse(transform: Affine) -> Option<Affine> {
    let determinant = transform.a * transform.d - transform.b * transform.c;
    if !determinant.is_finite() || determinant.abs() < 1e-12 {
        return None;
    }
    let a = transform.d / determinant;
    let b = -transform.b / determinant;
    let c = -transform.c / determinant;
    let d = transform.a / determinant;
    let e = -(a * transform.e + c * transform.f);
    let f = -(b * transform.e + d * transform.f);
    Some(Affine { a, b, c, d, e, f })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{Polygon, Rect};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Shape {
        Shape::Rect(Rect {
            x,
            y,
            width,
            height,
            rx: 0.0,
            ry: 0.0,
        })
    }

    /// The world points of a derived region under its base transform.
    fn world_points(region: &Shape, transform: Affine) -> Vec<[f64; 2]> {
        flatten_shape(region)
            .into_iter()
            .flatten()
            .map(|point| transform.apply(point))
            .collect()
    }

    #[test]
    fn every_direction_is_a_unit_vector_with_the_documented_components() {
        let root_half = std::f64::consts::FRAC_1_SQRT_2;
        let expected = [
            (LightDirection::Top, [0.0, -1.0]),
            (LightDirection::TopRight, [root_half, -root_half]),
            (LightDirection::Right, [1.0, 0.0]),
            (LightDirection::BottomRight, [root_half, root_half]),
            (LightDirection::Bottom, [0.0, 1.0]),
            (LightDirection::BottomLeft, [-root_half, root_half]),
            (LightDirection::Left, [-1.0, 0.0]),
            (LightDirection::TopLeft, [-root_half, -root_half]),
        ];
        for (direction, vector) in expected {
            let actual = light_vector(direction);
            assert!((actual[0] - vector[0]).abs() < 1e-12, "{direction:?}");
            assert!((actual[1] - vector[1]).abs() < 1e-12, "{direction:?}");
            let length = (actual[0].powi(2) + actual[1].powi(2)).sqrt();
            assert!((length - 1.0).abs() < 1e-12, "{direction:?}");
        }
    }

    #[test]
    fn the_region_is_the_far_half_of_a_shape_under_a_top_light() {
        let shape = rect(0.0, 0.0, 10.0, 10.0);
        let derived =
            region(&shape, &Affine::IDENTITY, light_vector(LightDirection::Top)).expect("a region");
        // Light from the top falls on the bottom half: y from 5 to 10.
        let points = world_points(&derived, Affine::IDENTITY);
        assert!(
            points.iter().all(|point| point[1] >= 5.0 - 1e-9),
            "{points:?}"
        );
        assert!(
            points.iter().all(|point| point[1] <= 10.0 + 1e-9),
            "{points:?}"
        );
        assert!((area(&flatten_shape(&derived)[0]).abs() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn a_light_from_the_right_shades_the_left_half() {
        let shape = rect(0.0, 0.0, 10.0, 10.0);
        let derived = region(
            &shape,
            &Affine::IDENTITY,
            light_vector(LightDirection::Right),
        )
        .expect("a region");
        let points = world_points(&derived, Affine::IDENTITY);
        // Light on the right leaves the left side facing away from it.
        assert!(
            points.iter().all(|point| point[0] <= 5.0 + 1e-9),
            "{points:?}"
        );
    }

    #[test]
    fn the_far_side_never_covers_the_side_toward_the_light() {
        // The boundary passes through the bounding-box centre, so every point of
        // the region lies on the far side of the light line (FEAT-028).
        let shape = Shape::Polygon(Polygon {
            points: vec![[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]],
        });
        let light = light_vector(LightDirection::TopRight);
        let derived = region(&shape, &Affine::IDENTITY, light).expect("a region");
        let center = [5.0, 4.0];
        for point in world_points(&derived, Affine::IDENTITY) {
            let signed = (point[0] - center[0]) * light[0] + (point[1] - center[1]) * light[1];
            assert!(signed <= 1e-9, "point {point:?} sits on the lit side");
        }
        // The region is a subset of the shape, so it encloses less area.
        assert!(area(&flatten_shape(&derived)[0]).abs() < 40.0);
    }

    #[test]
    fn a_translation_places_the_region_with_the_shape() {
        // Translating a shape translates its region by the same amount
        // (FEAT-028).
        let shape = rect(0.0, 0.0, 10.0, 10.0);
        let transform = Affine::translate(100.0, 40.0);
        let derived =
            region(&shape, &transform, light_vector(LightDirection::Top)).expect("a region");
        for point in world_points(&derived, transform) {
            assert!((45.0 - 1e-9..=50.0 + 1e-9).contains(&point[1]), "{point:?}");
            assert!(
                (100.0 - 1e-9..=110.0 + 1e-9).contains(&point[0]),
                "{point:?}"
            );
        }
    }

    #[test]
    fn a_rotated_shape_shades_against_the_scene_light_not_its_own_axis() {
        // The light is fixed in scene coordinates, so a shape turned 90° is cut
        // on the world-side away from the light (FEAT-028).
        let shape = rect(0.0, 0.0, 10.0, 10.0);
        let transform = Affine::rotate(90.0);
        let derived =
            region(&shape, &transform, light_vector(LightDirection::Top)).expect("a region");
        for point in world_points(&derived, transform) {
            // Rotation about the origin moves the box to x in [-10, 0] and
            // y in [0, 10]; its world bounding-box centre is (-5, 5), and a top
            // light shades the far side, which is the world-bottom half.
            assert!(point[1] >= 5.0 - 1e-9, "{point:?}");
        }
    }

    #[test]
    fn a_line_has_no_outline_to_shade() {
        use crate::primitives::Line;
        let line = Shape::Line(Line {
            points: vec![[0.0, 0.0], [10.0, 10.0]],
        });
        assert_eq!(
            region(&line, &Affine::IDENTITY, light_vector(LightDirection::Top)),
            Err(Unavailable::NoOutline)
        );
    }

    #[test]
    fn a_region_that_encloses_nothing_is_reported() {
        // A degenerate triangle has no area on either side of the line.
        let shape = Shape::Polygon(Polygon {
            points: vec![[0.0, 0.0], [10.0, 0.0], [20.0, 0.0]],
        });
        assert_eq!(
            region(&shape, &Affine::IDENTITY, light_vector(LightDirection::Top)),
            Err(Unavailable::EmptyRegion)
        );
    }

    #[test]
    fn a_singular_transform_is_reported_rather_than_guessed() {
        let shape = rect(0.0, 0.0, 10.0, 10.0);
        let degenerate = Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
            e: 0.0,
            f: 0.0,
        };
        assert_eq!(
            region(&shape, &degenerate, light_vector(LightDirection::Top)),
            Err(Unavailable::Degenerate)
        );
    }

    #[test]
    fn deriving_a_region_is_deterministic() {
        let shape = Shape::Polygon(Polygon {
            points: vec![[0.0, 0.0], [30.0, 4.0], [22.0, 20.0], [4.0, 14.0]],
        });
        let transform = Affine::translate(3.0, 7.0);
        let light = light_vector(LightDirection::BottomLeft);
        let once = region(&shape, &transform, light).expect("a region");
        let twice = region(&shape, &transform, light).expect("a region");
        assert_eq!(once, twice, "the region is deterministic (NFR-010)");
    }
}
