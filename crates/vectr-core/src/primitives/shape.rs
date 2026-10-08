//! Concrete geometry for the drawing primitives (C-003).
//!
//! Resolving an element yields one [`Shape`]; a render-model node carries it as
//! its concrete geometry. Coordinates are in local scene units — the element
//! transform is applied later, when the scene is compiled, so nothing here
//! reads it.

use serde::{Deserialize, Serialize};

use super::path::Path as PathGeometry;

/// A concrete drawing primitive.
///
/// Serializes as a `kind`-tagged object — `{"kind":"rect", ...}` — so a render
/// model carries its concrete geometry across a process boundary as data
/// (D-014). The tag names match [`Shape::kind`], the scene language's names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Shape {
    /// An axis-aligned rectangle, optionally with rounded corners.
    Rect(Rect),
    /// An ellipse inscribed in a bounding box.
    Ellipse(Ellipse),
    /// A closed point list.
    Polygon(Polygon),
    /// An open point list.
    Line(Line),
    /// A freeform path.
    Path(PathGeometry),
}

impl Shape {
    /// The primitive's name as it appears in the scene language.
    pub fn kind(&self) -> &'static str {
        match self {
            Shape::Rect(_) => "rect",
            Shape::Ellipse(_) => "ellipse",
            Shape::Polygon(_) => "polygon",
            Shape::Line(_) => "line",
            Shape::Path(_) => "path",
        }
    }

    /// Whether the shape's outline is closed.
    ///
    /// A stroke applies caps only at the ends of an open shape (FEAT-002); a
    /// path is closed only when every one of its subpaths is.
    pub fn is_closed(&self) -> bool {
        match self {
            Shape::Rect(_) | Shape::Ellipse(_) | Shape::Polygon(_) => true,
            Shape::Line(_) => false,
            Shape::Path(path) => path.is_closed(),
        }
    }
}

/// An axis-aligned rectangle with optional corner radii.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    /// The rectangle's X origin.
    pub x: f64,
    /// The rectangle's Y origin.
    pub y: f64,
    /// The rectangle's width.
    pub width: f64,
    /// The rectangle's height.
    pub height: f64,
    /// The corner radius in X.
    pub rx: f64,
    /// The corner radius in Y.
    pub ry: f64,
}

/// An ellipse, carried as its centre and its two radii.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ellipse {
    /// The centre's X coordinate.
    pub cx: f64,
    /// The centre's Y coordinate.
    pub cy: f64,
    /// The X radius.
    pub rx: f64,
    /// The Y radius.
    pub ry: f64,
}

/// A closed point list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Polygon {
    /// The vertices, in order.
    pub points: Vec<[f64; 2]>,
}

/// An open point list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    /// The vertices, in order.
    pub points: Vec<[f64; 2]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(shape: &Shape) -> Shape {
        let text = serde_json::to_string(shape).expect("serializable");
        serde_json::from_str(&text).expect("deserializable")
    }

    #[test]
    fn a_shape_is_tagged_by_its_scene_kind() {
        let rect = Shape::Rect(Rect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
            rx: 0.0,
            ry: 0.0,
        });
        let text = serde_json::to_string(&rect).unwrap();
        assert!(text.contains("\"kind\":\"rect\""), "{text}");
        assert_eq!(round_trip(&rect), rect);
    }

    #[test]
    fn every_geometry_kind_round_trips() {
        let shapes = [
            Shape::Rect(Rect {
                x: 1.0,
                y: 2.0,
                width: 3.0,
                height: 4.0,
                rx: 1.0,
                ry: 1.0,
            }),
            Shape::Ellipse(Ellipse {
                cx: 5.0,
                cy: 6.0,
                rx: 7.0,
                ry: 8.0,
            }),
            Shape::Polygon(Polygon {
                points: vec![[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]],
            }),
            Shape::Line(Line {
                points: vec![[0.0, 0.0], [10.0, 10.0]],
            }),
        ];
        for shape in shapes {
            assert_eq!(round_trip(&shape), shape);
        }
    }
}
