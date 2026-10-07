//! The scene document model and its strict JSON reading, writing, and
//! validation (C-001).
//!
//! [`parse`] reads a scene from JSON; [`validate`] checks a scene already in
//! memory. Parsing is strict: a property the language does not declare, a
//! missing required field, or an invalid value is a located error rather than
//! something silently dropped (FEAT-001).

mod diagnostic;
mod model;
mod version;

pub use diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Location, Severity};
pub use model::{
    Axis, BooleanOperation, Canvas, Constraint, ConstraintKind, Element, ElementKind, Geometry,
    ProjectionAxis, Scene, Transform,
};
pub use version::{
    is_supported, is_supported_version, parse_version, supported_range, CURRENT_FORMAT_VERSION,
    MAX_SUPPORTED_FORMAT_VERSION, MIN_SUPPORTED_FORMAT_VERSION,
};

use std::collections::HashSet;

/// The largest scene document the parser accepts, in bytes.
///
/// The input is untrusted (NFR-021); a document larger than this is refused
/// with a defined size diagnostic rather than read. The bound covers the
/// documented 50,000-element large scene while capping memory.
pub const MAX_SCENE_BYTES: usize = 64 * 1024 * 1024;

/// Reads a scene from a JSON document.
///
/// On success the returned [`Scene`] is structurally valid and passes
/// [`validate`]; parsing refuses rather than returning a scene carrying an
/// unsupported format version or a duplicate identifier (NFR-011).
pub fn parse(source: &str) -> Result<Scene, Diagnostics> {
    ensure_within_size(source.len())?;

    // Syntax first: bad JSON, or JSON whose top level is not a scene object.
    let value: serde_json::Value = serde_json::from_str(source)
        .map_err(|error| diagnostics_from_serde(DiagnosticCode::PARSE, &error))?;
    if !value.is_object() {
        return Err(Diagnostics::from(Diagnostic::error(
            DiagnosticCode::PARSE,
            "not a scene document: the top level must be a JSON object",
        )));
    }

    // Typed parse, which enforces the declared shape and reports line/column.
    let scene: Scene = serde_json::from_str(source)
        .map_err(|error| diagnostics_from_serde(DiagnosticCode::SCHEMA, &error))?;

    let findings = validate(&scene);
    if findings.has_errors() {
        return Err(findings);
    }
    Ok(scene)
}

/// Refuses a document longer than [`MAX_SCENE_BYTES`].
fn ensure_within_size(byte_len: usize) -> Result<(), Diagnostics> {
    if byte_len > MAX_SCENE_BYTES {
        return Err(Diagnostics::from(Diagnostic::error(
            DiagnosticCode::SIZE_LIMIT,
            format!(
                "scene document is {byte_len} bytes, exceeding the {MAX_SCENE_BYTES}-byte limit"
            ),
        )));
    }
    Ok(())
}

/// Checks a parsed scene against the language contract.
///
/// Returns every finding, errors and warnings alike, in a deterministic order.
pub fn validate(scene: &Scene) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();

    validate_format_version(&mut diagnostics, scene);
    validate_name(&mut diagnostics, &scene.name, "/name", "scene name");
    validate_required_number(
        &mut diagnostics,
        scene.canvas.width,
        "/canvas/width",
        "width",
    );
    validate_required_number(
        &mut diagnostics,
        scene.canvas.height,
        "/canvas/height",
        "height",
    );

    validate_elements(&mut diagnostics, scene);
    validate_constraints(&mut diagnostics, scene);

    diagnostics
}

fn validate_format_version(diagnostics: &mut Diagnostics, scene: &Scene) {
    match parse_version(&scene.format_version) {
        None => diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!(
                    "invalid formatVersion `{}`: expected a `major.minor` version",
                    scene.format_version
                ),
            )
            .at_path("/formatVersion"),
        ),
        Some(version) if !is_supported_version(version) => diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::FORMAT_VERSION,
                format!(
                    "unsupported scene format version `{}`; supported range {}",
                    scene.format_version,
                    supported_range()
                ),
            )
            .at_path("/formatVersion"),
        ),
        Some(_) => {}
    }
}

fn validate_name(diagnostics: &mut Diagnostics, value: &str, path: &str, field: &str) {
    if value.chars().count() > 120 {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!("{field} must be at most 120 characters"),
            )
            .at_path(path),
        );
    }
}

fn validate_required_number(diagnostics: &mut Diagnostics, value: f64, path: &str, field: &str) {
    if !value.is_finite() {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!("`{field}` must be a finite number"),
            )
            .at_path(path),
        );
    } else if value < 0.0 {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!("`{field}` must be zero or greater"),
            )
            .at_path(path),
        );
    }
}

fn validate_optional_number(
    diagnostics: &mut Diagnostics,
    value: Option<f64>,
    path: &str,
    field: &str,
) {
    if let Some(value) = value {
        if !value.is_finite() {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!("`{field}` must be a finite number"),
                )
                .at_path(path),
            );
        }
    }
}

fn validate_non_negative_number(
    diagnostics: &mut Diagnostics,
    value: Option<f64>,
    path: &str,
    field: &str,
) {
    if let Some(value) = value {
        if !value.is_finite() {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!("`{field}` must be a finite number"),
                )
                .at_path(path),
            );
        } else if value < 0.0 {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!("`{field}` must be zero or greater"),
                )
                .at_path(path),
            );
        }
    }
}

fn validate_elements(diagnostics: &mut Diagnostics, scene: &Scene) {
    let mut seen: HashSet<&str> = HashSet::with_capacity(scene.elements.len());

    for (index, element) in scene.elements.iter().enumerate() {
        let base = format!("/elements/{index}");

        if !seen.insert(element.id.as_str()) {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::DUPLICATE_ID,
                    format!("duplicate element identifier `{}`", element.id),
                )
                .with_location(Location::element_at(
                    element.id.clone(),
                    format!("{base}/id"),
                )),
            );
        }

        if element.order < 0 {
            diagnostics.push(
                Diagnostic::error(DiagnosticCode::SCHEMA, "`order` must be zero or greater")
                    .at_path(format!("{base}/order")),
            );
        }

        if !element.opacity.is_finite() || !(0.0..=1.0).contains(&element.opacity) {
            diagnostics.push(
                Diagnostic::error(DiagnosticCode::SCHEMA, "`opacity` must be between 0 and 1")
                    .at_path(format!("{base}/opacity")),
            );
        }

        validate_geometry(diagnostics, &element.geometry, &base);
        validate_transform(diagnostics, &element.transform, &base);
    }
}

fn validate_geometry(diagnostics: &mut Diagnostics, geometry: &Geometry, base: &str) {
    validate_optional_number(diagnostics, geometry.x, &format!("{base}/geometry/x"), "x");
    validate_optional_number(diagnostics, geometry.y, &format!("{base}/geometry/y"), "y");
    validate_non_negative_number(
        diagnostics,
        geometry.width,
        &format!("{base}/geometry/width"),
        "width",
    );
    validate_non_negative_number(
        diagnostics,
        geometry.height,
        &format!("{base}/geometry/height"),
        "height",
    );
    validate_non_negative_number(
        diagnostics,
        geometry.rx,
        &format!("{base}/geometry/rx"),
        "rx",
    );
    validate_non_negative_number(
        diagnostics,
        geometry.ry,
        &format!("{base}/geometry/ry"),
        "ry",
    );
    validate_optional_number(
        diagnostics,
        geometry.spacing,
        &format!("{base}/geometry/spacing"),
        "spacing",
    );
    validate_optional_number(
        diagnostics,
        geometry.distance,
        &format!("{base}/geometry/distance"),
        "distance",
    );

    if let Some(points) = &geometry.points {
        for (point_index, point) in points.iter().enumerate() {
            for (axis_index, coordinate) in point.iter().enumerate() {
                if !coordinate.is_finite() {
                    diagnostics.push(
                        Diagnostic::error(
                            DiagnosticCode::SCHEMA,
                            "point coordinates must be finite numbers",
                        )
                        .at_path(format!("{base}/geometry/points/{point_index}/{axis_index}")),
                    );
                }
            }
        }
    }
}

fn validate_transform(diagnostics: &mut Diagnostics, transform: &Transform, base: &str) {
    let fields = [
        ("translateX", transform.translate_x),
        ("translateY", transform.translate_y),
        ("rotate", transform.rotate),
        ("scaleX", transform.scale_x),
        ("scaleY", transform.scale_y),
    ];
    for (field, value) in fields {
        validate_optional_number(
            diagnostics,
            Some(value),
            &format!("{base}/transform/{field}"),
            field,
        );
    }
    validate_optional_number(
        diagnostics,
        transform.skew_x,
        &format!("{base}/transform/skewX"),
        "skewX",
    );
    validate_optional_number(
        diagnostics,
        transform.skew_y,
        &format!("{base}/transform/skewY"),
        "skewY",
    );
}

fn validate_constraints(diagnostics: &mut Diagnostics, scene: &Scene) {
    let Some(constraints) = &scene.constraints else {
        return;
    };

    for (index, constraint) in constraints.iter().enumerate() {
        let base = format!("/constraints/{index}");
        if constraint.element_ids.len() < 2 {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    "a constraint must reference at least two elements",
                )
                .at_path(format!("{base}/elementIds")),
            );
        }
        validate_optional_number(
            diagnostics,
            constraint.value,
            &format!("{base}/value"),
            "value",
        );
    }
}

fn diagnostics_from_serde(code: DiagnosticCode, error: &serde_json::Error) -> Diagnostics {
    // serde_json appends " at line N column M" to its Display; the position is
    // carried structurally instead, so drop the suffix from the message.
    let raw = error.to_string();
    let message = raw.split(" at line ").next().unwrap_or(&raw).to_string();

    let mut diagnostic = Diagnostic::error(code, message);
    if error.line() > 0 {
        diagnostic = diagnostic.with_location(Location::line_column(error.line(), error.column()));
    }
    Diagnostics::from(diagnostic)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIPPED_VERSION: &str = CURRENT_FORMAT_VERSION;

    /// A complete, valid scene exercising every field the contract declares.
    fn full_scene() -> String {
        format!(
            r##"{{
  "id": "scene-1",
  "projectId": "project-1",
  "name": "Habit logo",
  "formatVersion": "{SHIPPED_VERSION}",
  "canvas": {{ "width": 512, "height": 512, "background": "#ffffff" }},
  "paletteId": "palette-1",
  "recipeId": "flat",
  "title": "Habit logo",
  "description": "Mark and wordmark",
  "elements": [
    {{
      "id": "mark",
      "sceneId": "scene-1",
      "parentId": null,
      "order": 0,
      "name": "Mark",
      "kind": "rect",
      "geometry": {{ "x": 0, "y": 0, "width": 120, "height": 120, "rx": 8, "ry": 8 }},
      "transform": {{ "translateX": 12, "translateY": 8, "rotate": 45, "scaleX": 1, "scaleY": 1, "skewX": 0, "skewY": 0 }},
      "fillToken": "accent",
      "strokeProfileId": "stroke-1",
      "opacity": 1,
      "visible": true
    }},
    {{
      "id": "wordmark",
      "sceneId": "scene-1",
      "parentId": "mark",
      "order": 1,
      "kind": "path",
      "geometry": {{ "pathData": "M0 0 L10 10" }},
      "transform": {{ "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 }},
      "fillToken": null,
      "strokeProfileId": null,
      "opacity": 0.5,
      "visible": false
    }}
  ],
  "constraints": [
    {{ "id": "c1", "sceneId": "scene-1", "kind": "align", "elementIds": ["mark", "wordmark"], "axis": "x", "value": null }}
  ]
}}"##
        )
    }

    fn parse_error(source: &str) -> Diagnostics {
        parse(source).expect_err("expected the scene to be refused")
    }

    #[test]
    fn parses_a_complete_scene() {
        let scene = parse(&full_scene()).expect("valid scene");
        assert_eq!(scene.id, "scene-1");
        assert_eq!(scene.elements.len(), 2);
        assert_eq!(scene.elements[0].kind, ElementKind::Rect);
        assert_eq!(scene.constraints.as_ref().map(Vec::len), Some(1));
    }

    #[test]
    fn round_trips_without_loss() {
        let original = parse(&full_scene()).expect("valid scene");
        let text = original.to_json_string().expect("serializable");
        let reparsed = parse(&text).expect("serialized scene is valid");
        assert_eq!(original, reparsed);
    }

    #[test]
    fn canonical_form_is_stable() {
        let once = parse(&full_scene())
            .expect("valid scene")
            .to_json_string()
            .unwrap();
        let twice = parse(&once).expect("valid scene").to_json_string().unwrap();
        assert_eq!(once, twice, "serialization must be deterministic (NFR-010)");
    }

    #[test]
    fn every_element_is_addressable_by_id() {
        let scene = parse(&full_scene()).expect("valid scene");
        assert_eq!(scene.element("mark").map(|e| e.order), Some(0));
        assert_eq!(
            scene.element("wordmark").map(|e| e.parent_id.as_deref()),
            Some(Some("mark"))
        );
        assert!(scene.element("missing").is_none());
    }

    #[test]
    fn empty_scene_is_a_valid_empty_canvas() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Empty","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":0,"height":0,"background":"transparent"}}}}"#
        );
        let scene = parse(&source).expect("an empty scene is valid");
        assert!(scene.elements.is_empty());
        let reparsed = parse(&scene.to_json_string().unwrap()).expect("round-trips");
        assert_eq!(scene, reparsed);
    }

    #[test]
    fn element_optional_fields_may_be_omitted() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Scene","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":100,"height":100,"background":"transparent"}},"elements":[{{"id":"e1","sceneId":"s","order":0,"kind":"ellipse","geometry":{{}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}]}}"#
        );
        let scene = parse(&source).expect("optional element fields may be omitted");
        let element = scene.element("e1").expect("element present");
        assert_eq!(element.parent_id, None);
        assert_eq!(element.name, None);
        assert_eq!(element.fill_token, None);
        assert_eq!(element.stroke_profile_id, None);

        let text = scene.to_json_string().expect("serializable");
        assert!(
            !text.contains("parentId"),
            "an omitted optional field stays omitted"
        );
        assert!(!text.contains("fillToken"));
    }

    #[test]
    fn size_limit_is_sixty_four_mib() {
        assert_eq!(MAX_SCENE_BYTES, 64 * 1024 * 1024);
    }

    #[test]
    fn document_at_the_limit_is_allowed() {
        assert!(ensure_within_size(MAX_SCENE_BYTES).is_ok());
    }

    #[test]
    fn oversized_document_is_refused_with_a_size_diagnostic() {
        let diagnostics = ensure_within_size(MAX_SCENE_BYTES + 1).expect_err("over the limit");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::SIZE_LIMIT);
        assert!(
            error.message.contains(&MAX_SCENE_BYTES.to_string()),
            "names the limit: {}",
            error.message
        );
    }

    #[test]
    fn geometry_carries_offset_distance_and_projection_axis() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Scene","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":100,"height":100,"background":"transparent"}},"elements":[{{"id":"o1","sceneId":"s","order":0,"kind":"offset","geometry":{{"distance":4.5}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}},{{"id":"p1","sceneId":"s","order":1,"kind":"projection","geometry":{{"axis":"isometric"}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}]}}"#
        );
        let scene = parse(&source).expect("the revised geometry parses");
        assert_eq!(scene.element("o1").unwrap().geometry.distance, Some(4.5));
        assert_eq!(
            scene.element("p1").unwrap().geometry.axis,
            Some(ProjectionAxis::Isometric)
        );

        let text = scene.to_json_string().expect("serializable");
        assert!(text.contains("\"distance\":4.5"));
        assert!(text.contains("\"axis\":\"isometric\""));
        assert_eq!(parse(&text).expect("round-trips"), scene);
    }

    #[test]
    fn negative_offset_distance_is_valid() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Scene","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":1,"height":1,"background":"transparent"}},"elements":[{{"id":"o1","sceneId":"s","order":0,"kind":"offset","geometry":{{"distance":-2}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}]}}"#
        );
        let scene = parse(&source).expect("an inward offset is valid");
        assert_eq!(scene.element("o1").unwrap().geometry.distance, Some(-2.0));
    }

    #[test]
    fn a_non_finite_offset_distance_is_rejected() {
        let mut scene = parse(&full_scene()).expect("valid scene");
        scene.elements[0].geometry.distance = Some(f64::INFINITY);

        let diagnostics = validate(&scene);
        let error = diagnostics
            .errors()
            .find(|d| {
                d.location.as_ref().and_then(|l| l.json_path.as_deref())
                    == Some("/elements/0/geometry/distance")
            })
            .expect("a located error");
        assert_eq!(error.code, DiagnosticCode::SCHEMA);
    }

    #[test]
    fn geometry_axis_rejects_the_constraint_axis_value() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Scene","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":1,"height":1,"background":"transparent"}},"elements":[{{"id":"p1","sceneId":"s","order":0,"kind":"projection","geometry":{{"axis":"both"}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}]}}"#
        );
        let diagnostics = parse_error(&source);
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }

    #[test]
    fn projection_axis_names_match_the_language() {
        for (axis, name) in [
            (ProjectionAxis::X, "x"),
            (ProjectionAxis::Y, "y"),
            (ProjectionAxis::Isometric, "isometric"),
        ] {
            assert_eq!(serde_json::to_string(&axis).unwrap(), format!("\"{name}\""));
            assert_eq!(ProjectionAxis::from_name(name), Some(axis));
            assert_eq!(axis.as_str(), name);
        }
        assert_eq!(ProjectionAxis::from_name("both"), None);
    }

    #[test]
    fn malformed_json_is_a_parse_error() {
        let diagnostics = parse_error("{ not json");
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::PARSE)
        );
    }

    #[test]
    fn non_object_document_is_a_parse_error() {
        let diagnostics = parse_error("[1, 2, 3]");
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::PARSE)
        );
    }

    #[test]
    fn unknown_property_is_a_schema_error_with_a_location() {
        let source = full_scene().replace(
            "\"visible\": true",
            "\"visible\": true, \"colour\": \"red\"",
        );
        let diagnostics = parse_error(&source);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::SCHEMA);
        assert!(
            !error.message.contains(" at line "),
            "position is carried in the location, not the message"
        );
        assert!(
            error.location.as_ref().and_then(|l| l.line).is_some(),
            "schema errors carry a location"
        );
    }

    #[test]
    fn missing_required_field_is_a_schema_error() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"n","formatVersion":"{SHIPPED_VERSION}"}}"#
        );
        let diagnostics = parse_error(&source);
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }

    #[test]
    fn unsupported_format_version_is_refused_by_name() {
        let source = full_scene().replace(SHIPPED_VERSION, "9.9");
        let diagnostics = parse_error(&source);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::FORMAT_VERSION);
        assert!(
            error.message.contains("9.9"),
            "names the declared version: {}",
            error.message
        );
        assert!(
            error.message.contains(SHIPPED_VERSION),
            "names the supported range: {}",
            error.message
        );
    }

    #[test]
    fn malformed_format_version_is_a_schema_error() {
        let source = full_scene().replace(SHIPPED_VERSION, "v1");
        let diagnostics = parse_error(&source);
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }

    #[test]
    fn duplicate_element_identifier_is_reported() {
        let source = full_scene().replace("\"id\": \"wordmark\"", "\"id\": \"mark\"");
        let diagnostics = parse_error(&source);
        let error = diagnostics
            .errors()
            .find(|d| d.code == DiagnosticCode::DUPLICATE_ID)
            .expect("a duplicate-id error");
        assert!(error.message.contains("mark"));
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|l| l.element_id.as_deref()),
            Some("mark")
        );
    }

    #[test]
    fn duplicate_identifier_is_found_by_validate_alone() {
        let scene = parse(&full_scene()).unwrap();
        let mut copy = scene.clone();
        copy.elements[1].id = copy.elements[0].id.clone();
        let diagnostics = validate(&copy);
        assert!(diagnostics
            .errors()
            .any(|d| d.code == DiagnosticCode::DUPLICATE_ID));
    }

    #[test]
    fn opacity_out_of_range_is_rejected() {
        let source = full_scene().replace("\"opacity\": 0.5", "\"opacity\": 1.5");
        let diagnostics = parse_error(&source);
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }

    #[test]
    fn negative_order_is_rejected() {
        let source = full_scene().replace("\"order\": 1", "\"order\": -1");
        let diagnostics = parse_error(&source);
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }

    #[test]
    fn constraint_needs_two_elements() {
        let source = full_scene().replace(
            "\"elementIds\": [\"mark\", \"wordmark\"]",
            "\"elementIds\": [\"mark\"]",
        );
        let diagnostics = parse_error(&source);
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }

    #[test]
    fn valid_scene_produces_no_findings() {
        let scene = parse(&full_scene()).unwrap();
        assert!(validate(&scene).is_empty());
    }
}
