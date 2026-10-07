//! Concrete geometry for the drawing primitives (C-003).
//!
//! Resolving an element yields one [`Shape`]; a render-model node carries it as
//! its concrete geometry. Coordinates are in local scene units — the element
//! transform is applied later, when the scene is compiled, so nothing here
//! reads it.

use super::path::Path as PathGeometry;

/// A concrete drawing primitive.
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
pub struct Polygon {
    /// The vertices, in order.
    pub points: Vec<[f64; 2]>,
}

/// An open point list.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// The vertices, in order.
    pub points: Vec<[f64; 2]>,
}
