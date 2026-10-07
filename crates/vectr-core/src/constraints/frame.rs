//! Element frames: the axis-aligned bounds constraints reason about.
//!
//! A constraint relates elements by position, so resolution works on each
//! element's axis-aligned bounding box in scene coordinates. The box is
//! computed from the element's local geometry and then transformed by its
//! declared transform, so rotation and scale are honoured. An element whose
//! geometry this layer cannot bound — a group or composition, or a primitive
//! with no geometry — contributes a degenerate frame at its transformed origin,
//! which is the deterministic default the resolver documents.

use crate::composition::Affine;
use crate::primitives::Shape;
use crate::scene::{Element, ElementKind};

/// A point in scene units.
pub type Point = [f64; 2];

/// An axis-aligned bounding box in scene coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// The lower corner.
    pub min: Point,
    /// The upper corner.
    pub max: Point,
}

impl Frame {
    /// A frame between two corners.
    pub fn new(min: Point, max: Point) -> Self {
        Self { min, max }
    }

    /// A degenerate frame at a single point.
    pub fn point(point: Point) -> Self {
        Self {
            min: point,
            max: point,
        }
    }

    /// The frame's centre.
    pub fn center(&self) -> Point {
        [
            (self.min[0] + self.max[0]) / 2.0,
            (self.min[1] + self.max[1]) / 2.0,
        ]
    }

    /// The frame's extent along one axis (`0` for x, `1` for y).
    pub fn extent(&self, axis: usize) -> f64 {
        self.max[axis] - self.min[axis]
    }

    /// The frame shifted by a delta.
    pub fn translated(&self, delta: Point) -> Self {
        Self {
            min: [self.min[0] + delta[0], self.min[1] + delta[1]],
            max: [self.max[0] + delta[0], self.max[1] + delta[1]],
        }
    }
}

/// The frame an element occupies in scene coordinates.
///
/// `affine` is the element's resolved transform; when the geometry cannot be
/// bounded the frame degenerates to the transformed origin.
pub fn element_frame(element: &Element, affine: Affine) -> Frame {
    match local_bounds(element) {
        Some(local) => transform_frame(local, affine),
        None => Frame::point(affine.apply([0.0, 0.0])),
    }
}

/// The element's bounds in its own local coordinates.
fn local_bounds(element: &Element) -> Option<Frame> {
    let geometry = &element.geometry;
    match element.kind {
        ElementKind::Rect | ElementKind::Ellipse => {
            let x = geometry.x.unwrap_or(0.0);
            let y = geometry.y.unwrap_or(0.0);
            let width = geometry.width.unwrap_or(0.0);
            let height = geometry.height.unwrap_or(0.0);
            Some(Frame::new(
                [x.min(x + width), y.min(y + height)],
                [x.max(x + width), y.max(y + height)],
            ))
        }
        ElementKind::Polygon | ElementKind::Line => {
            let points = geometry.points.as_deref()?;
            bbox_of_points(points)
        }
        ElementKind::Path => {
            let path = crate::primitives::parse(geometry.path_data.as_deref()?).ok()?;
            bbox_of_contours(&crate::composition::flatten_shape(&Shape::Path(path)))
        }
        _ => None,
    }
}

fn bbox_of_points(points: &[Point]) -> Option<Frame> {
    let first = *points.first()?;
    let mut min = first;
    let mut max = first;
    for point in points {
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    Some(Frame::new(min, max))
}

fn bbox_of_contours(contours: &[Vec<Point>]) -> Option<Frame> {
    let mut frame: Option<Frame> = None;
    for contour in contours {
        for point in contour {
            frame = Some(match frame {
                Some(current) => Frame::new(
                    [current.min[0].min(point[0]), current.min[1].min(point[1])],
                    [current.max[0].max(point[0]), current.max[1].max(point[1])],
                ),
                None => Frame::point(*point),
            });
        }
    }
    frame
}

fn transform_frame(local: Frame, affine: Affine) -> Frame {
    let corners = [
        [local.min[0], local.min[1]],
        [local.max[0], local.min[1]],
        [local.max[0], local.max[1]],
        [local.min[0], local.max[1]],
    ];
    let mut min = affine.apply(corners[0]);
    let mut max = min;
    for corner in corners {
        let point = affine.apply(corner);
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    Frame::new(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Geometry, Transform};

    fn element(kind: ElementKind, geometry: Geometry) -> Element {
        Element {
            id: "e".to_string(),
            scene_id: "s".to_string(),
            parent_id: None,
            order: 0,
            name: None,
            kind,
            geometry,
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
            opacity: 1.0,
            visible: true,
        }
    }

    #[test]
    fn a_rect_frame_is_its_bounding_box() {
        let frame = element_frame(
            &element(
                ElementKind::Rect,
                Geometry {
                    x: Some(2.0),
                    y: Some(3.0),
                    width: Some(10.0),
                    height: Some(4.0),
                    ..Geometry::default()
                },
            ),
            Affine::IDENTITY,
        );
        assert_eq!(frame.min, [2.0, 3.0]);
        assert_eq!(frame.max, [12.0, 7.0]);
        assert_eq!(frame.center(), [7.0, 5.0]);
    }

    #[test]
    fn a_polygon_frame_spans_its_vertices() {
        let frame = element_frame(
            &element(
                ElementKind::Polygon,
                Geometry {
                    points: Some(vec![[0.0, 0.0], [10.0, -5.0], [4.0, 8.0]]),
                    ..Geometry::default()
                },
            ),
            Affine::IDENTITY,
        );
        assert_eq!(frame.min, [0.0, -5.0]);
        assert_eq!(frame.max, [10.0, 8.0]);
    }

    #[test]
    fn a_translated_element_moves_its_frame() {
        let mut element = element(
            ElementKind::Rect,
            Geometry {
                width: Some(10.0),
                height: Some(10.0),
                ..Geometry::default()
            },
        );
        element.transform.translate_x = 100.0;
        element.transform.translate_y = 20.0;
        let frame = element_frame(&element, Affine::from_scene(&element.transform).unwrap());
        assert_eq!(frame.min, [100.0, 20.0]);
        assert_eq!(frame.max, [110.0, 30.0]);
    }

    #[test]
    fn a_group_degrades_to_its_transformed_origin() {
        let mut element = element(ElementKind::Group, Geometry::default());
        element.transform.translate_x = 7.0;
        element.transform.translate_y = 9.0;
        let frame = element_frame(&element, Affine::from_scene(&element.transform).unwrap());
        assert_eq!(frame.min, frame.max);
        assert_eq!(frame.center(), [7.0, 9.0]);
    }
}
