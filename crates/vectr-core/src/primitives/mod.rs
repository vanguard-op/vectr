//! Drawing primitives: the elemental shapes a scene is built from (FEAT-002).
//!
//! Resolution turns one primitive [`Element`] into its concrete [`Shape`],
//! ready for a render-model node (C-003). It is the geometry half of that
//! seam: fill, stroke, transform, opacity and paint order are applied by the
//! compiler, not here.
//!
//! A primitive is refused at resolution when its geometry cannot draw:
//! a rectangle or ellipse with zero or negative extent, a point list with no
//! points, or malformed path data. Each error names the primitive and carries
//! the element's location. An empty path is not an error: it renders nothing
//! and is reported as a warning (FEAT-002).

pub mod path;
mod shape;

pub use path::{parse, Path, PathError, Segment, SubPath};
pub use shape::{Ellipse, Line, Polygon, Rect, Shape};

use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, Element, ElementKind, Location};

/// A primitive's geometry is invalid and blocks compilation.
pub const PRIMITIVE: DiagnosticCode = DiagnosticCode::new("E_PRIMITIVE");

/// A path draws nothing, so no node is emitted.
pub const EMPTY_PATH: DiagnosticCode = DiagnosticCode::new("W_EMPTY_PATH");

/// Whether an element kind is one of the drawing primitives this module owns.
pub fn is_primitive(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Rect
            | ElementKind::Ellipse
            | ElementKind::Polygon
            | ElementKind::Line
            | ElementKind::Path
    )
}

/// Resolves one element into its concrete shape.
///
/// Returns `Some` when the element contributes geometry. Returns `None` when it
/// contributes nothing — an empty path, or invalid geometry — having recorded
/// the reason (`EMPTY_PATH` for an empty path; `PRIMITIVE`, an error, for
/// invalid geometry). A caller checks `diagnostics.has_errors()` to tell a
/// warning from a refusal.
pub fn resolve(element: &Element, diagnostics: &mut Diagnostics) -> Option<Shape> {
    match element.kind {
        ElementKind::Rect => resolve_rect(element, diagnostics).map(Shape::Rect),
        ElementKind::Ellipse => resolve_ellipse(element, diagnostics).map(Shape::Ellipse),
        ElementKind::Polygon => resolve_points(element, diagnostics, "polygon")
            .map(|points| Shape::Polygon(Polygon { points })),
        ElementKind::Line => {
            resolve_points(element, diagnostics, "line").map(|points| Shape::Line(Line { points }))
        }
        ElementKind::Path => resolve_path(element, diagnostics).map(Shape::Path),
        other => {
            reject(
                diagnostics,
                element,
                format!("kind `{}` is not a drawing primitive", kind_name(other)),
            );
            None
        }
    }
}

fn resolve_rect(element: &Element, diagnostics: &mut Diagnostics) -> Option<Rect> {
    let geometry = &element.geometry;
    let (Some(width), Some(height)) = (geometry.width(), geometry.height()) else {
        reject_positive_extent(diagnostics, element, "rect");
        return None;
    };
    if !is_positive_extent(width, height) {
        reject_positive_extent(diagnostics, element, "rect");
        return None;
    }
    let x = geometry.x().unwrap_or(0.0);
    let y = geometry.y().unwrap_or(0.0);
    if !x.is_finite() || !y.is_finite() {
        reject_finite_origin(diagnostics, element, "rect");
        return None;
    }
    Some(Rect {
        x,
        y,
        width,
        height,
        rx: non_negative(geometry.rx()),
        ry: non_negative(geometry.ry()),
    })
}

fn resolve_ellipse(element: &Element, diagnostics: &mut Diagnostics) -> Option<Ellipse> {
    let geometry = &element.geometry;
    let (Some(width), Some(height)) = (geometry.width(), geometry.height()) else {
        reject_positive_extent(diagnostics, element, "ellipse");
        return None;
    };
    if !is_positive_extent(width, height) {
        reject_positive_extent(diagnostics, element, "ellipse");
        return None;
    }
    let x = geometry.x().unwrap_or(0.0);
    let y = geometry.y().unwrap_or(0.0);
    if !x.is_finite() || !y.is_finite() {
        reject_finite_origin(diagnostics, element, "ellipse");
        return None;
    }
    // The ellipse is inscribed in the bounding box whose origin is (x, y); its
    // radii are the half extents, so a circle is a square box.
    Some(Ellipse {
        cx: x + width / 2.0,
        cy: y + height / 2.0,
        rx: width / 2.0,
        ry: height / 2.0,
    })
}

fn resolve_points(
    element: &Element,
    diagnostics: &mut Diagnostics,
    primitive: &str,
) -> Option<Vec<[f64; 2]>> {
    let Some(points) = element.geometry.points.as_ref() else {
        reject(
            diagnostics,
            element,
            format!("{primitive} `{}` must declare points", element.id),
        );
        return None;
    };
    if points.is_empty() {
        reject(
            diagnostics,
            element,
            format!("{primitive} `{}` must declare points", element.id),
        );
        return None;
    }
    if points.iter().flatten().any(|value| !value.is_finite()) {
        reject(
            diagnostics,
            element,
            format!(
                "{primitive} `{}` must have finite point coordinates",
                element.id
            ),
        );
        return None;
    }
    Some(points.clone())
}

fn resolve_path(element: &Element, diagnostics: &mut Diagnostics) -> Option<Path> {
    let Some(data) = element.geometry.path_data() else {
        warn_empty_path(diagnostics, element);
        return None;
    };
    if data.trim().is_empty() {
        warn_empty_path(diagnostics, element);
        return None;
    }
    match parse(data) {
        Ok(parsed) if !parsed.is_empty() => Some(parsed),
        Ok(_) => {
            warn_empty_path(diagnostics, element);
            None
        }
        Err(error) => {
            reject(
                diagnostics,
                element,
                format!("path `{}` has invalid path data: {error}", element.id),
            );
            None
        }
    }
}

fn is_positive_extent(width: f64, height: f64) -> bool {
    width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0
}

fn non_negative(value: Option<f64>) -> f64 {
    value.filter(|v| v.is_finite()).unwrap_or(0.0).max(0.0)
}

fn reject_positive_extent(diagnostics: &mut Diagnostics, element: &Element, primitive: &str) {
    reject(
        diagnostics,
        element,
        format!(
            "{primitive} `{}` must have a positive width and height",
            element.id
        ),
    );
}

fn reject_finite_origin(diagnostics: &mut Diagnostics, element: &Element, primitive: &str) {
    reject(
        diagnostics,
        element,
        format!("{primitive} `{}` must have finite coordinates", element.id),
    );
}

fn warn_empty_path(diagnostics: &mut Diagnostics, element: &Element) {
    diagnostics.push(
        Diagnostic::warning(
            EMPTY_PATH,
            format!("path `{}` is empty and renders nothing", element.id),
        )
        .with_location(Location::element(element.id.clone())),
    );
}

fn reject(diagnostics: &mut Diagnostics, element: &Element, message: String) {
    diagnostics.push(
        Diagnostic::error(PRIMITIVE, message).with_location(Location::element(element.id.clone())),
    );
}

fn kind_name(kind: ElementKind) -> &'static str {
    kind.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{BoolValue, Geometry, NumberValue, StringValue, Transform};

    fn n(value: f64) -> NumberValue {
        NumberValue::Literal(value)
    }

    fn s(value: &str) -> StringValue {
        StringValue::Literal(value.to_string())
    }

    fn element(kind: ElementKind, geometry: Geometry) -> Element {
        Element {
            id: "e1".to_string(),
            scene_id: Some("s1".to_string()),
            definition_id: None,
            parent_id: None,
            order: 0,
            name: None,
            accessible_name: None,
            kind,
            geometry,
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

    fn resolve_one(kind: ElementKind, geometry: Geometry) -> (Option<Shape>, Diagnostics) {
        let mut diagnostics = Diagnostics::new();
        let shape = resolve(&element(kind, geometry), &mut diagnostics);
        (shape, diagnostics)
    }

    fn box_geometry(width: f64, height: f64) -> Geometry {
        Geometry {
            x: Some(n(10.0)),
            y: Some(n(20.0)),
            width: Some(n(width)),
            height: Some(n(height)),
            ..Geometry::default()
        }
    }

    #[test]
    fn a_rectangle_resolves_at_its_position_and_size() {
        let geometry = Geometry {
            x: Some(n(4.0)),
            y: Some(n(8.0)),
            width: Some(n(120.0)),
            height: Some(n(60.0)),
            rx: Some(n(6.0)),
            ry: Some(n(6.0)),
            ..Geometry::default()
        };
        let (shape, diagnostics) = resolve_one(ElementKind::Rect, geometry);
        assert_eq!(
            shape,
            Some(Shape::Rect(Rect {
                x: 4.0,
                y: 8.0,
                width: 120.0,
                height: 60.0,
                rx: 6.0,
                ry: 6.0,
            }))
        );
        assert!(diagnostics.is_empty());
        assert!(shape.unwrap().is_closed());
    }

    #[test]
    fn a_rectangle_without_an_origin_sits_at_zero() {
        let geometry = Geometry {
            width: Some(n(10.0)),
            height: Some(n(10.0)),
            ..Geometry::default()
        };
        let (shape, _) = resolve_one(ElementKind::Rect, geometry);
        assert_eq!(
            shape,
            Some(Shape::Rect(Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
            }))
        );
    }

    #[test]
    fn a_zero_width_rectangle_is_rejected_naming_the_primitive() {
        let (shape, diagnostics) = resolve_one(ElementKind::Rect, box_geometry(0.0, 10.0));
        assert!(shape.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, PRIMITIVE);
        assert!(error.message.contains("rect"), "{}", error.message);
        assert!(error.message.contains("e1"), "{}", error.message);
    }

    #[test]
    fn a_negative_height_rectangle_is_rejected() {
        let (shape, diagnostics) = resolve_one(ElementKind::Rect, box_geometry(10.0, -1.0));
        assert!(shape.is_none());
        assert!(diagnostics.has_errors());
    }

    #[test]
    fn a_rectangle_missing_its_size_is_rejected() {
        let (shape, diagnostics) = resolve_one(ElementKind::Rect, Geometry::default());
        assert!(shape.is_none());
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(PRIMITIVE)
        );
    }

    #[test]
    fn an_ellipse_is_inscribed_in_its_bounding_box() {
        let (shape, diagnostics) = resolve_one(ElementKind::Ellipse, box_geometry(80.0, 40.0));
        assert_eq!(
            shape,
            Some(Shape::Ellipse(Ellipse {
                cx: 50.0,
                cy: 40.0,
                rx: 40.0,
                ry: 20.0,
            }))
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn a_zero_extent_ellipse_is_rejected_naming_the_primitive() {
        let (shape, diagnostics) = resolve_one(ElementKind::Ellipse, box_geometry(80.0, 0.0));
        assert!(shape.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, PRIMITIVE);
        assert!(error.message.contains("ellipse"), "{}", error.message);
    }

    #[test]
    fn a_polygon_resolves_to_a_closed_point_list() {
        let geometry = Geometry {
            points: Some(vec![[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]]),
            ..Geometry::default()
        };
        let (shape, diagnostics) = resolve_one(ElementKind::Polygon, geometry);
        assert_eq!(
            shape,
            Some(Shape::Polygon(Polygon {
                points: vec![[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]],
            }))
        );
        assert!(shape.unwrap().is_closed());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn a_line_resolves_to_an_open_point_list() {
        let geometry = Geometry {
            points: Some(vec![[0.0, 0.0], [10.0, 10.0]]),
            ..Geometry::default()
        };
        let (shape, diagnostics) = resolve_one(ElementKind::Line, geometry);
        assert_eq!(
            shape,
            Some(Shape::Line(Line {
                points: vec![[0.0, 0.0], [10.0, 10.0]],
            }))
        );
        assert!(!shape.unwrap().is_closed());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn a_point_list_without_points_is_rejected_naming_the_primitive() {
        let (shape, diagnostics) = resolve_one(ElementKind::Polygon, Geometry::default());
        assert!(shape.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert!(error.message.contains("polygon"), "{}", error.message);
    }

    #[test]
    fn a_non_finite_point_is_rejected() {
        let geometry = Geometry {
            points: Some(vec![[0.0, 0.0], [f64::NAN, 1.0]]),
            ..Geometry::default()
        };
        let (shape, diagnostics) = resolve_one(ElementKind::Line, geometry);
        assert!(shape.is_none());
        assert!(diagnostics.has_errors());
    }

    #[test]
    fn a_path_resolves_to_concrete_subpaths() {
        let geometry = Geometry {
            path_data: Some(s("M0 0 L10 0 L10 10 Z")),
            ..Geometry::default()
        };
        let (shape, diagnostics) = resolve_one(ElementKind::Path, geometry);
        let Some(Shape::Path(path)) = shape else {
            panic!("expected a concrete path");
        };
        assert_eq!(path.subpaths.len(), 1);
        assert!(path.is_closed());
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn an_open_path_is_open_so_a_stroke_caps_its_ends() {
        let geometry = Geometry {
            path_data: Some(s("M0 0 L10 10")),
            ..Geometry::default()
        };
        let (shape, _) = resolve_one(ElementKind::Path, geometry);
        assert!(!shape.unwrap().is_closed());
    }

    #[test]
    fn an_empty_path_renders_nothing_and_warns() {
        for data in [None, Some(""), Some("   ")] {
            let geometry = Geometry {
                path_data: data.map(s),
                ..Geometry::default()
            };
            let (shape, diagnostics) = resolve_one(ElementKind::Path, geometry);
            assert!(shape.is_none());
            assert!(!diagnostics.has_errors());
            let warning = diagnostics.warnings().next().expect("a warning");
            assert_eq!(warning.code, EMPTY_PATH);
        }
    }

    #[test]
    fn a_move_only_path_renders_nothing_and_warns() {
        let geometry = Geometry {
            path_data: Some(s("M 10 10")),
            ..Geometry::default()
        };
        let (shape, diagnostics) = resolve_one(ElementKind::Path, geometry);
        assert!(shape.is_none());
        assert_eq!(
            diagnostics.warnings().next().map(|d| d.code.clone()),
            Some(EMPTY_PATH)
        );
    }

    #[test]
    fn malformed_path_data_is_rejected_naming_the_primitive() {
        let geometry = Geometry {
            path_data: Some(s("M0 0 Q")),
            ..Geometry::default()
        };
        let (shape, diagnostics) = resolve_one(ElementKind::Path, geometry);
        assert!(shape.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, PRIMITIVE);
        assert!(error.message.contains("path"), "{}", error.message);
    }

    #[test]
    fn a_composition_kind_is_not_a_drawing_primitive() {
        assert!(!is_primitive(ElementKind::Group));
        let (shape, diagnostics) = resolve_one(ElementKind::Group, Geometry::default());
        assert!(shape.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, PRIMITIVE);
        assert!(error.message.contains("group"), "{}", error.message);
    }

    #[test]
    fn every_drawing_kind_is_a_primitive() {
        for kind in [
            ElementKind::Rect,
            ElementKind::Ellipse,
            ElementKind::Polygon,
            ElementKind::Line,
            ElementKind::Path,
        ] {
            assert!(is_primitive(kind));
        }
    }

    #[test]
    fn shape_kinds_use_the_scene_language_names() {
        let rect = Shape::Rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            rx: 0.0,
            ry: 0.0,
        });
        assert_eq!(rect.kind(), "rect");
        assert_eq!(Shape::Line(Line { points: vec![] }).kind(), "line");
    }
}
