//! The scene document model and its strict JSON reading, writing, and
//! validation (C-001).
//!
//! [`parse`] reads a scene from JSON; [`validate`] checks a scene already in
//! memory. Parsing is strict: a property the language does not declare, a
//! missing required field, or an invalid value is a located error rather than
//! something silently dropped (FEAT-001).

mod color;
mod diagnostic;
mod model;
mod version;

pub use color::{is_color, validate_color, INVALID_COLOR};
pub use diagnostic::{Diagnostic, DiagnosticCode, Diagnostics, Location, Severity};
pub use model::{
    Axis, Binding, BindingValue, BoolValue, BooleanOperation, Canvas, Constraint, ConstraintKind,
    Definition, Element, ElementKind, Geometry, NumberValue, Origin, Paint, PaintKind, PaintValue,
    ParamRef, Parameter, ParameterType, ParameterValue, ProjectionAxis, Scene, StringValue, Stroke,
    TextAlign, Transform,
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

/// Reads a reusable definition from a JSON document (C-002, FEAT-030).
///
/// On success the returned [`Definition`] is structurally valid and passes
/// [`validate_definition`]; parsing refuses rather than returning one carrying a
/// duplicate identifier or an unsupported shape (NFR-011).
pub fn parse_definition(source: &str) -> Result<Definition, Diagnostics> {
    ensure_within_size(source.len())?;

    let value: serde_json::Value = serde_json::from_str(source)
        .map_err(|error| diagnostics_from_serde(DiagnosticCode::PARSE, &error))?;
    if !value.is_object() {
        return Err(Diagnostics::from(Diagnostic::error(
            DiagnosticCode::PARSE,
            "not a definition document: the top level must be a JSON object",
        )));
    }

    let definition: Definition = serde_json::from_str(source)
        .map_err(|error| diagnostics_from_serde(DiagnosticCode::SCHEMA, &error))?;

    let findings = validate_definition(&definition);
    if findings.has_errors() {
        return Err(findings);
    }
    Ok(definition)
}

/// Checks a parsed definition against the language contract (C-002, FEAT-030).
///
/// Returns every finding, errors and warnings alike, in a deterministic order.
/// The definition's element tree is validated with the same structural rules a
/// scene's is; a parameter reference is checked against the declared parameters
/// and the field it occupies (FEAT-018).
pub fn validate_definition(definition: &Definition) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();

    validate_name(
        &mut diagnostics,
        &definition.name,
        "/name",
        "definition name",
    );
    if definition.id.is_empty() {
        diagnostics.push(
            Diagnostic::error(DiagnosticCode::SCHEMA, "`id` must not be empty").at_path("/id"),
        );
    }

    let mut seen: HashSet<&str> = HashSet::with_capacity(definition.parameters.len());
    for (index, parameter) in definition.parameters.iter().enumerate() {
        let base = format!("/parameters/{index}");
        if parameter.name.is_empty() {
            diagnostics.push(
                Diagnostic::error(DiagnosticCode::SCHEMA, "a parameter name must not be empty")
                    .at_path(format!("{base}/name")),
            );
        }
        if !seen.insert(parameter.name.as_str()) {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::DUPLICATE_ID,
                    format!("duplicate parameter name `{}`", parameter.name),
                )
                .at_path(format!("{base}/name")),
            );
        }
        if let Some(default) = &parameter.default {
            if !parameter_type_matches(parameter.value_type, default) {
                diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::SCHEMA,
                        format!(
                            "parameter `{}` has a default that is not a {}",
                            parameter.name,
                            parameter.value_type.as_str()
                        ),
                    )
                    .at_path(format!("{base}/default")),
                );
            }
        }
    }

    if !definition.origin.x.is_finite() || !definition.origin.y.is_finite() {
        diagnostics.push(
            Diagnostic::error(DiagnosticCode::SCHEMA, "`origin` must be finite").at_path("/origin"),
        );
    }

    // Each element must name this definition, and each parameter reference must
    // resolve to a declared parameter of the matching type.
    for (index, element) in definition.elements.iter().enumerate() {
        let base = format!("/elements/{index}");
        if element.definition_id.as_deref() != Some(definition.id.as_str()) {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!(
                        "element `{}` belongs to definition `{}`, not `{}`",
                        element.id,
                        element.definition_id.as_deref().unwrap_or("<none>"),
                        definition.id
                    ),
                )
                .with_location(Location::element_at(
                    element.id.clone(),
                    format!("{base}/definitionId"),
                )),
            );
        }
        validate_element_params(&mut diagnostics, definition, element, &base);
    }

    // The definition's element tree is checked with the same structural rules a
    // scene's is; a parameter reference is a well-formed value whose binding is
    // checked by expansion (FEAT-018, FEAT-030).
    validate_elements(&mut diagnostics, &definition.elements);
    validate_element_identity(&mut diagnostics, &definition.elements);

    diagnostics
}

/// Whether a literal default matches its parameter's declared type.
fn parameter_type_matches(value_type: ParameterType, value: &ParameterValue) -> bool {
    matches!(
        (value_type, value),
        (ParameterType::Number, ParameterValue::Number(_))
            | (ParameterType::String, ParameterValue::Text(_))
            | (ParameterType::Token, ParameterValue::Text(_))
            | (ParameterType::Boolean, ParameterValue::Boolean(_))
    )
}

/// Checks every parameter reference a definition element carries (FEAT-030).
///
/// A reference names a parameter the definition must declare, and its type must
/// match the field it occupies; a mismatch is reported at the reference's
/// location (FEAT-018, FEAT-030).
fn validate_element_params(
    diagnostics: &mut Diagnostics,
    definition: &Definition,
    element: &Element,
    base: &str,
) {
    let geometry = &element.geometry;
    let numeric_geometry = [
        ("x", geometry.x.as_ref()),
        ("y", geometry.y.as_ref()),
        ("width", geometry.width.as_ref()),
        ("height", geometry.height.as_ref()),
        ("rx", geometry.rx.as_ref()),
        ("ry", geometry.ry.as_ref()),
        ("fontSize", geometry.font_size.as_ref()),
        ("lineHeight", geometry.line_height.as_ref()),
        ("letterSpacing", geometry.letter_spacing.as_ref()),
        ("count", geometry.count.as_ref()),
        ("spacing", geometry.spacing.as_ref()),
        ("distance", geometry.distance.as_ref()),
    ];
    for (field, value) in numeric_geometry {
        if let Some(value) = value {
            check_param(
                diagnostics,
                definition,
                element,
                value.param(),
                ParameterType::Number,
                &format!("{base}/geometry/{field}"),
            );
        }
    }
    for (field, value) in [
        ("pathData", geometry.path_data.as_ref()),
        ("text", geometry.text.as_ref()),
    ] {
        if let Some(value) = value {
            check_param(
                diagnostics,
                definition,
                element,
                value.param(),
                ParameterType::String,
                &format!("{base}/geometry/{field}"),
            );
        }
    }

    let transform = &element.transform;
    let numeric_transform = [
        ("translateX", &transform.translate_x),
        ("translateY", &transform.translate_y),
        ("rotate", &transform.rotate),
        ("scaleX", &transform.scale_x),
        ("scaleY", &transform.scale_y),
    ];
    for (field, value) in numeric_transform {
        check_param(
            diagnostics,
            definition,
            element,
            value.param(),
            ParameterType::Number,
            &format!("{base}/transform/{field}"),
        );
    }
    for (field, value) in [
        ("skewX", transform.skew_x.as_ref()),
        ("skewY", transform.skew_y.as_ref()),
    ] {
        if let Some(value) = value {
            check_param(
                diagnostics,
                definition,
                element,
                value.param(),
                ParameterType::Number,
                &format!("{base}/transform/{field}"),
            );
        }
    }

    check_param(
        diagnostics,
        definition,
        element,
        element.opacity.param(),
        ParameterType::Number,
        &format!("{base}/opacity"),
    );
    check_param(
        diagnostics,
        definition,
        element,
        element.visible.param(),
        ParameterType::Boolean,
        &format!("{base}/visible"),
    );

    if let Some(fill) = &element.fill {
        check_param(
            diagnostics,
            definition,
            element,
            fill.param(),
            ParameterType::Token,
            &format!("{base}/fill"),
        );
    }
    if let Some(stroke) = &element.stroke {
        check_param(
            diagnostics,
            definition,
            element,
            stroke.paint.param(),
            ParameterType::Token,
            &format!("{base}/stroke/paint"),
        );
    }

    // A binding that forwards a parameter must name one this definition
    // declares; the forwarded type is checked against the nested definition's
    // parameter when the placement resolves.
    if let Some(bindings) = &element.bindings {
        for (index, binding) in bindings.iter().enumerate() {
            let BindingValue::Param(reference) = &binding.value else {
                continue;
            };
            if definition.parameter(&reference.param).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::SCHEMA,
                        format!(
                            "element `{}` forwards parameter `{}`, which definition `{}` does not declare",
                            element.id, reference.param, definition.id
                        ),
                    )
                    .with_location(Location::element_at(
                        element.id.clone(),
                        format!("{base}/bindings/{index}/value"),
                    )),
                );
            }
        }
    }
}

/// Reports a parameter reference that is absent from the definition or whose
/// declared type does not match the field (FEAT-030).
fn check_param(
    diagnostics: &mut Diagnostics,
    definition: &Definition,
    element: &Element,
    reference: Option<&str>,
    expected: ParameterType,
    path: &str,
) {
    let Some(name) = reference else {
        return;
    };
    match definition.parameter(name) {
        None => diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!(
                    "element `{}` references parameter `{name}`, which definition `{}` does not declare",
                    element.id, definition.id
                ),
            )
            .with_location(Location::element_at(element.id.clone(), path)),
        ),
        Some(parameter) if parameter.value_type != expected => diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!(
                    "element `{}` references `{name}` (a {}) in a field that needs a {}",
                    element.id,
                    parameter.value_type.as_str(),
                    expected.as_str()
                ),
            )
            .with_location(Location::element_at(element.id.clone(), path)),
        ),
        Some(_) => {}
    }
}

/// Checks that an element names exactly one owner, and that the instance-only
/// fields appear only on an instance (C-001, FEAT-030).
pub(crate) fn validate_element_identity(diagnostics: &mut Diagnostics, elements: &[Element]) {
    for (index, element) in elements.iter().enumerate() {
        let base = format!("/elements/{index}");
        match (&element.scene_id, &element.definition_id) {
            (Some(_), Some(_)) => diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!(
                        "element `{}` sets both sceneId and definitionId; exactly one is required",
                        element.id
                    ),
                )
                .with_location(Location::element_at(element.id.clone(), base.clone())),
            ),
            (None, None) => diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!(
                        "element `{}` sets neither sceneId nor definitionId; exactly one is required",
                        element.id
                    ),
                )
                .with_location(Location::element_at(element.id.clone(), base.clone())),
            ),
            _ => {}
        }

        if element.kind == ElementKind::Instance {
            if element.definition_ref.as_deref().is_none_or(str::is_empty) {
                diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::SCHEMA,
                        format!("instance `{}` requires a definitionRef", element.id),
                    )
                    .with_location(Location::element_at(
                        element.id.clone(),
                        format!("{base}/definitionRef"),
                    )),
                );
            }
        } else if element.definition_ref.is_some() {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!(
                        "element `{}` declares definitionRef but is not an instance",
                        element.id
                    ),
                )
                .with_location(Location::element_at(
                    element.id.clone(),
                    format!("{base}/definitionRef"),
                )),
            );
        }

        if element.kind != ElementKind::Instance && element.bindings.is_some() {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!(
                        "element `{}` declares bindings but is not an instance",
                        element.id
                    ),
                )
                .with_location(Location::element_at(
                    element.id.clone(),
                    format!("{base}/bindings"),
                )),
            );
        }
    }
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
    validate_color(
        &mut diagnostics,
        &scene.canvas.background,
        "canvas background",
        "/canvas/background",
    );

    validate_elements(&mut diagnostics, &scene.elements);
    validate_element_identity(&mut diagnostics, &scene.elements);
    validate_scene_param_refs(&mut diagnostics, scene);
    validate_scene_ownership(&mut diagnostics, scene);
    validate_constraints(&mut diagnostics, scene);

    diagnostics
}

/// A scene declares no parameters, so a parameter reference in a scene element
/// is a located error (C-001, FEAT-018).
fn validate_scene_param_refs(diagnostics: &mut Diagnostics, scene: &Scene) {
    for (index, element) in scene.elements.iter().enumerate() {
        let base = format!("/elements/{index}");
        let geometry = &element.geometry;
        let numeric = [
            ("x", geometry.x.as_ref()),
            ("y", geometry.y.as_ref()),
            ("width", geometry.width.as_ref()),
            ("height", geometry.height.as_ref()),
            ("rx", geometry.rx.as_ref()),
            ("ry", geometry.ry.as_ref()),
            ("fontSize", geometry.font_size.as_ref()),
            ("lineHeight", geometry.line_height.as_ref()),
            ("letterSpacing", geometry.letter_spacing.as_ref()),
            ("count", geometry.count.as_ref()),
            ("spacing", geometry.spacing.as_ref()),
            ("distance", geometry.distance.as_ref()),
        ];
        for (field, value) in numeric {
            if value.and_then(NumberValue::param).is_some() {
                reject_scene_param(diagnostics, element, &format!("{base}/geometry/{field}"));
            }
        }
        for (field, value) in [
            ("pathData", geometry.path_data.as_ref()),
            ("text", geometry.text.as_ref()),
        ] {
            if value.and_then(StringValue::param).is_some() {
                reject_scene_param(diagnostics, element, &format!("{base}/geometry/{field}"));
            }
        }
        let transform = &element.transform;
        for (field, value) in [
            ("translateX", Some(&transform.translate_x)),
            ("translateY", Some(&transform.translate_y)),
            ("rotate", Some(&transform.rotate)),
            ("scaleX", Some(&transform.scale_x)),
            ("scaleY", Some(&transform.scale_y)),
            ("skewX", transform.skew_x.as_ref()),
            ("skewY", transform.skew_y.as_ref()),
        ] {
            if value.and_then(NumberValue::param).is_some() {
                reject_scene_param(diagnostics, element, &format!("{base}/transform/{field}"));
            }
        }
        if element.opacity.param().is_some() {
            reject_scene_param(diagnostics, element, &format!("{base}/opacity"));
        }
        if element.visible.param().is_some() {
            reject_scene_param(diagnostics, element, &format!("{base}/visible"));
        }
        if element.fill.as_ref().and_then(PaintValue::param).is_some() {
            reject_scene_param(diagnostics, element, &format!("{base}/fill"));
        }
        if let Some(stroke) = &element.stroke {
            if stroke.paint.param().is_some() {
                reject_scene_param(diagnostics, element, &format!("{base}/stroke/paint"));
            }
        }
    }
}

/// Reports a parameter reference in a scene element at its location.
fn reject_scene_param(diagnostics: &mut Diagnostics, element: &Element, path: &str) {
    diagnostics.push(
        Diagnostic::error(
            DiagnosticCode::SCHEMA,
            format!(
                "element `{}` holds a parameter reference in a scene; a scene declares no parameters",
                element.id
            ),
        )
        .with_location(Location::element_at(element.id.clone(), path.to_string())),
    );
}

/// A scene element must belong to the scene, not to a definition (C-001).
fn validate_scene_ownership(diagnostics: &mut Diagnostics, scene: &Scene) {
    for (index, element) in scene.elements.iter().enumerate() {
        if element.definition_id.is_some() {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!(
                        "element `{}` belongs to a definition and cannot appear in a scene",
                        element.id
                    ),
                )
                .with_location(Location::element_at(
                    element.id.clone(),
                    format!("/elements/{index}/definitionId"),
                )),
            );
        }
    }
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

fn validate_elements(diagnostics: &mut Diagnostics, elements: &[Element]) {
    let mut seen: HashSet<&str> = HashSet::with_capacity(elements.len());
    let mut index_of: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::with_capacity(elements.len());
    for (index, element) in elements.iter().enumerate() {
        index_of.insert(element.id.as_str(), index);
    }

    for (index, element) in elements.iter().enumerate() {
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

        if let Some(opacity) = element.opacity.literal() {
            if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                diagnostics.push(
                    Diagnostic::error(DiagnosticCode::SCHEMA, "`opacity` must be between 0 and 1")
                        .at_path(format!("{base}/opacity")),
                );
            }
        }

        validate_geometry(diagnostics, &element.geometry, &base);
        validate_transform(diagnostics, &element.transform, &base);
        validate_text(diagnostics, element, &base);
        validate_text_operand(diagnostics, elements, element, &base, &index_of);
    }
}

/// A text element needs a string and a positive size, and a font reference
/// belongs only to a text element (FEAT-002, FEAT-024).
fn validate_text(diagnostics: &mut Diagnostics, element: &Element, base: &str) {
    if element.kind == ElementKind::Text {
        // A parameter reference is a well-formed value whose binding is checked
        // separately; only a literal must be a non-empty string and a positive
        // size (FEAT-030).
        let text_ok = match &element.geometry.text {
            None => false,
            Some(StringValue::Literal(text)) => !text.is_empty(),
            Some(StringValue::Param(_)) => true,
        };
        if !text_ok {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!("text element `{}` requires geometry.text", element.id),
                )
                .with_location(Location::element_at(
                    element.id.clone(),
                    format!("{base}/geometry/text"),
                )),
            );
        }
        let size_ok = match &element.geometry.font_size {
            None => false,
            Some(NumberValue::Literal(size)) => size.is_finite() && *size > 0.0,
            Some(NumberValue::Param(_)) => true,
        };
        if !size_ok {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!(
                        "text element `{}` requires geometry.fontSize greater than zero",
                        element.id
                    ),
                )
                .with_location(Location::element_at(
                    element.id.clone(),
                    format!("{base}/geometry/fontSize"),
                )),
            );
        }
    } else if element.font_id.is_some() {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!(
                    "element `{}` declares fontId but is not a text element",
                    element.id
                ),
            )
            .with_location(Location::element_at(
                element.id.clone(),
                format!("{base}/fontId"),
            )),
        );
    }
}

/// A text element cannot be an operand of a boolean, offset, projection,
/// repeat, or alongPath element (FEAT-002, FEAT-011).
fn validate_text_operand(
    diagnostics: &mut Diagnostics,
    elements: &[Element],
    element: &Element,
    base: &str,
    index_of: &std::collections::HashMap<&str, usize>,
) {
    if element.kind != ElementKind::Text {
        return;
    }
    let Some(parent_id) = element.parent_id.as_deref() else {
        return;
    };
    let Some(&parent_index) = index_of.get(parent_id) else {
        return;
    };
    let parent_kind = elements[parent_index].kind;
    if is_text_operand_kind(parent_kind) {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!(
                    "text element `{}` is not a valid operand of a {} element",
                    element.id,
                    parent_kind.as_str()
                ),
            )
            .with_location(Location::element_at(
                element.id.clone(),
                format!("{base}/parentId"),
            )),
        );
    }
}

/// The composition kinds a text element cannot be lowered into.
fn is_text_operand_kind(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::Boolean
            | ElementKind::Offset
            | ElementKind::Projection
            | ElementKind::Repeat
            | ElementKind::AlongPath
    )
}

fn validate_geometry(diagnostics: &mut Diagnostics, geometry: &Geometry, base: &str) {
    // Accessors return only literals, so a parameter reference is left to the
    // parameter check (FEAT-030).
    validate_optional_number(
        diagnostics,
        geometry.x(),
        &format!("{base}/geometry/x"),
        "x",
    );
    validate_optional_number(
        diagnostics,
        geometry.y(),
        &format!("{base}/geometry/y"),
        "y",
    );
    validate_non_negative_number(
        diagnostics,
        geometry.width(),
        &format!("{base}/geometry/width"),
        "width",
    );
    validate_non_negative_number(
        diagnostics,
        geometry.height(),
        &format!("{base}/geometry/height"),
        "height",
    );
    validate_non_negative_number(
        diagnostics,
        geometry.rx(),
        &format!("{base}/geometry/rx"),
        "rx",
    );
    validate_non_negative_number(
        diagnostics,
        geometry.ry(),
        &format!("{base}/geometry/ry"),
        "ry",
    );
    validate_optional_number(
        diagnostics,
        geometry.spacing(),
        &format!("{base}/geometry/spacing"),
        "spacing",
    );
    validate_optional_number(
        diagnostics,
        geometry.distance(),
        &format!("{base}/geometry/distance"),
        "distance",
    );
    validate_non_negative_number(
        diagnostics,
        geometry.font_size(),
        &format!("{base}/geometry/fontSize"),
        "fontSize",
    );
    validate_non_negative_number(
        diagnostics,
        geometry.line_height(),
        &format!("{base}/geometry/lineHeight"),
        "lineHeight",
    );
    validate_optional_number(
        diagnostics,
        geometry.letter_spacing(),
        &format!("{base}/geometry/letterSpacing"),
        "letterSpacing",
    );

    // A repeat's count is a whole number (schema.md, "Element").
    if let Some(count) = geometry.count.as_ref().and_then(NumberValue::literal) {
        if !count.is_finite() || count < 0.0 || count.fract() != 0.0 {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    "`count` must be a whole number zero or greater",
                )
                .at_path(format!("{base}/geometry/count")),
            );
        }
    }

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
    // Accessors return only literals, so a parameter reference is left to the
    // parameter check (FEAT-030).
    let fields = [
        ("translateX", transform.translate_x()),
        ("translateY", transform.translate_y()),
        ("rotate", transform.rotate()),
        ("scaleX", transform.scale_x()),
        ("scaleY", transform.scale_y()),
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
        transform.skew_x(),
        &format!("{base}/transform/skewX"),
        "skewX",
    );
    validate_optional_number(
        diagnostics,
        transform.skew_y(),
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
      "fill": {{ "kind": "token", "ref": "accent" }},
      "stroke": {{ "profileId": "stroke-1", "paint": {{ "kind": "token", "ref": "accent" }} }},
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
      "fill": null,
      "stroke": null,
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
        assert_eq!(element.fill, None);
        assert_eq!(element.stroke, None);

        let text = scene.to_json_string().expect("serializable");
        assert!(
            !text.contains("parentId"),
            "an omitted optional field stays omitted"
        );
        assert!(!text.contains("\"fill\""));
        assert!(!text.contains("\"stroke\""));
    }

    #[test]
    fn a_stroke_round_trips_with_its_profile_and_paint() {
        let scene = parse(&full_scene()).expect("valid scene");
        let stroke = scene
            .element("mark")
            .unwrap()
            .stroke
            .as_ref()
            .expect("a stroke");
        assert_eq!(stroke.profile_id, "stroke-1");
        assert_eq!(stroke.paint.literal().unwrap().kind, PaintKind::Token);
        assert_eq!(stroke.paint.literal().unwrap().reference, "accent");
        let text = scene.to_json_string().expect("serializable");
        assert!(
            text.contains(
                r#""stroke":{"profileId":"stroke-1","paint":{"kind":"token","ref":"accent"}}"#
            ),
            "{text}"
        );
        assert_eq!(parse(&text).expect("round-trips"), scene);
    }

    #[test]
    fn a_stroke_without_a_paint_is_refused() {
        let source = full_scene().replace(r#", "paint": { "kind": "token", "ref": "accent" }"#, "");
        let diagnostics = parse_error(&source);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::SCHEMA);
    }

    #[test]
    fn a_stroke_without_a_profile_is_refused() {
        let source = full_scene().replace(r#""profileId": "stroke-1", "#, "");
        let diagnostics = parse_error(&source);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::SCHEMA);
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
        assert_eq!(scene.element("o1").unwrap().geometry.distance(), Some(4.5));
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
        assert_eq!(scene.element("o1").unwrap().geometry.distance(), Some(-2.0));
    }

    #[test]
    fn a_non_finite_offset_distance_is_rejected() {
        let mut scene = parse(&full_scene()).expect("valid scene");
        scene.elements[0].geometry.distance = Some(NumberValue::Literal(f64::INFINITY));

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

    #[test]
    fn a_canvas_background_that_is_not_a_colour_is_reported_with_its_location() {
        let source = full_scene().replace(
            r##""background": "#ffffff""##,
            r##""background": "not-a-colour""##,
        );
        let diagnostics = parse_error(&source);
        let error = diagnostics
            .errors()
            .find(|diagnostic| diagnostic.code == INVALID_COLOR)
            .expect("an invalid-colour error");
        assert!(
            error.message.contains("canvas background"),
            "{}",
            error.message
        );
        assert!(error.message.contains("not-a-colour"), "{}", error.message);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/canvas/background")
        );
    }

    #[test]
    fn every_svg_colour_format_is_accepted_for_the_canvas() {
        for background in [
            "#fff",
            "#ffff",
            "#ffffff",
            "#ffffffff",
            "rgb(1, 2, 3)",
            "rgba(1, 2, 3, 0.5)",
            "hsl(120, 50%, 50%)",
            "hsla(120, 50%, 50%, 0.25)",
            "red",
            "transparent",
        ] {
            let source = full_scene().replace(
                r##""background": "#ffffff""##,
                &format!("\"background\": \"{background}\""),
            );
            let scene = parse(&source).unwrap_or_else(|diagnostics| {
                panic!("expected {background:?} to be a colour: {diagnostics}")
            });
            assert_eq!(scene.canvas.background, background);
        }
    }

    #[test]
    fn the_canvas_background_is_checked_by_validate_alone() {
        let mut scene = parse(&full_scene()).unwrap();
        scene.canvas.background = "#12345".to_string();
        let diagnostics = validate(&scene);
        assert!(diagnostics.errors().any(|d| d.code == INVALID_COLOR));
    }

    fn text_scene(geometry: &str, extra: &str) -> String {
        format!(
            r#"{{"id":"s","projectId":"p","name":"Scene","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":200,"height":100,"background":"transparent"}},"elements":[{{"id":"t1","sceneId":"s","order":0,"kind":"text","geometry":{geometry},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}}{extra},"opacity":1,"visible":true}}]}}"#
        )
    }

    #[test]
    fn parses_a_text_element_with_its_run_fields() {
        let source = text_scene(
            r#"{"x":10,"y":20,"text":"Hello","fontSize":24,"align":"center","lineHeight":30,"letterSpacing":1.5,"width":100}"#,
            r#","fontId":"font-1""#,
        );
        let scene = parse(&source).expect("a valid text scene");
        let element = scene.element("t1").expect("the text element");
        assert_eq!(element.kind, ElementKind::Text);
        assert_eq!(element.geometry.text(), Some("Hello"));
        assert_eq!(element.geometry.font_size(), Some(24.0));
        assert_eq!(element.geometry.align, Some(TextAlign::Center));
        assert_eq!(element.geometry.line_height(), Some(30.0));
        assert_eq!(element.geometry.letter_spacing(), Some(1.5));
        assert_eq!(element.geometry.width(), Some(100.0));
        assert_eq!(element.font_id.as_deref(), Some("font-1"));

        let text = scene.to_json_string().expect("serializable");
        assert!(text.contains(r#""kind":"text""#), "{text}");
        assert!(text.contains(r#""fontId":"font-1""#), "{text}");
        assert_eq!(parse(&text).expect("round-trips"), scene);
    }

    #[test]
    fn a_text_element_without_a_string_or_size_is_refused_naming_the_element() {
        let no_text = text_scene(r#"{"fontSize":24}"#, "");
        let diagnostics = parse_error(&no_text);
        let error = diagnostics.errors().next().expect("an error");
        assert!(error.message.contains("t1"), "{}", error.message);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/elements/0/geometry/text")
        );

        let no_size = text_scene(r#"{"text":"Hello"}"#, "");
        let diagnostics = parse_error(&no_size);
        let error = diagnostics.errors().next().expect("an error");
        assert!(error.message.contains("t1"), "{}", error.message);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/elements/0/geometry/fontSize")
        );
    }

    #[test]
    fn a_font_reference_on_a_non_text_element_is_refused_naming_the_element() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Scene","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":1,"height":1,"background":"transparent"}},"elements":[{{"id":"r1","sceneId":"s","order":0,"kind":"rect","geometry":{{"width":10,"height":10}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"fontId":"font-1","opacity":1,"visible":true}}]}}"#
        );
        let diagnostics = parse_error(&source);
        let error = diagnostics.errors().next().expect("an error");
        assert!(error.message.contains("r1"), "{}", error.message);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/elements/0/fontId")
        );
    }

    #[test]
    fn a_text_element_as_a_composition_operand_is_refused() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Scene","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":1,"height":1,"background":"transparent"}},"elements":[{{"id":"b1","sceneId":"s","order":0,"kind":"boolean","geometry":{{"operation":"union"}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}},{{"id":"t1","sceneId":"s","order":0,"kind":"text","parentId":"b1","geometry":{{"text":"Hi","fontSize":12}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}]}}"#
        );
        let diagnostics = parse_error(&source);
        let error = diagnostics.errors().next().expect("an error");
        assert!(
            error.message.contains("text element `t1`"),
            "{}",
            error.message
        );
        assert!(error.message.contains("boolean"), "{}", error.message);
    }

    #[test]
    fn text_alignment_names_match_the_language() {
        for (align, name) in [
            (TextAlign::Start, "start"),
            (TextAlign::Center, "center"),
            (TextAlign::End, "end"),
        ] {
            assert_eq!(
                serde_json::to_string(&align).unwrap(),
                format!("\"{name}\"")
            );
            assert_eq!(TextAlign::from_name(name), Some(align));
            assert_eq!(align.as_str(), name);
        }
        assert_eq!(TextAlign::from_name("middle"), None);
    }

    #[test]
    fn a_scene_element_holding_a_parameter_reference_is_refused() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Scene","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":1,"height":1,"background":"transparent"}},"elements":[{{"id":"r1","sceneId":"s","order":0,"kind":"rect","geometry":{{"width":{{"param":"w"}},"height":1}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}]}}"#
        );
        let diagnostics = parse_error(&source);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::SCHEMA);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/elements/0/geometry/width")
        );
    }

    #[test]
    fn a_definition_element_may_hold_a_parameter_reference() {
        let definition = parse_definition(
            r#"{"id":"chip","projectId":"p","name":"Chip","parameters":[{"name":"w","type":"number"}],"origin":{"x":0,"y":0},"elements":[{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{"width":{"param":"w"},"height":10},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"opacity":1,"visible":true}]}"#,
        )
        .expect("a definition element may hold a parameter reference");
        assert_eq!(definition.elements.len(), 1);
        assert_eq!(
            definition.elements[0]
                .geometry
                .width
                .as_ref()
                .and_then(NumberValue::param),
            Some("w")
        );
        let text = definition.to_json_string().expect("serializable");
        assert_eq!(parse_definition(&text).expect("round-trips"), definition);
    }

    #[test]
    fn an_invalid_value_for_a_parameter_capable_field_names_the_value() {
        let source = full_scene().replace("\"rotate\": 45", "\"rotate\": \"sideways\"");
        let diagnostics = parse_error(&source);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::SCHEMA);
        assert!(
            error.message.contains("sideways"),
            "names the invalid value: {}",
            error.message
        );
    }

    #[test]
    fn a_repeat_count_must_be_a_whole_number() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Scene","formatVersion":"{SHIPPED_VERSION}","canvas":{{"width":1,"height":1,"background":"transparent"}},"elements":[{{"id":"r1","sceneId":"s","order":0,"kind":"repeat","geometry":{{"count":2.5}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}]}}"#
        );
        let diagnostics = parse_error(&source);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::SCHEMA);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/elements/0/geometry/count")
        );
    }

    #[test]
    fn an_overrides_field_is_an_unknown_property() {
        // Parameters are the only use-adjustment mechanism, so the removed
        // per-use override is not part of the element shape (D-038).
        let source = full_scene().replace(
            r#""opacity": 0.5"#,
            r#""opacity": 0.5, "overrides": [{"target": "mark"}]"#,
        );
        let diagnostics = parse_error(&source);
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }
}
