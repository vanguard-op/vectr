//! Reusable-definition expansion: lowers every instance in a scene into the
//! concrete elements its definition provides (FEAT-030).
//!
//! The compiler's seam is a flat, concrete element tree. Rather than teach every
//! emission path about a second element universe, this pass runs first: it walks
//! the scene, and wherever an `instance` element places a definition it splices
//! the definition's elements in, substituting the instance's parameter bindings,
//! and wrapping the result in a `group` that carries the instance's identity and
//! transform. The compiler then sees only primitives, compositions and groups,
//! and the render model has no unresolved definition reference (C-003, FEAT-011,
//! FEAT-030).
//!
//! Resolution is deterministic (NFR-010): elements are walked in document order,
//! identifiers are derived from the instance path, and no unordered collection
//! feeds the output. A reference cycle among definitions, an unresolved
//! definition, a binding or parameter reference that does not match the placed
//! definition, and an expansion past the supported size limit are each a located
//! error naming the definition; nothing is silently dropped or truncated
//! (NFR-011).

use std::collections::{HashMap, HashSet};

use crate::scene::{
    Binding, BindingValue, BoolValue, Definition, Diagnostic, DiagnosticCode, Diagnostics, Element,
    ElementKind, Geometry, Location, NumberValue, Paint, PaintKind, PaintValue, ParameterType,
    ParameterValue, Scene, StringValue, Transform,
};

/// A definition reference that does not resolve to a definition in the context.
pub const UNRESOLVED_DEFINITION: DiagnosticCode = DiagnosticCode::new("E_DEFINITION");

/// Two definitions place each other, directly or through a chain.
pub const DEFINITION_CYCLE: DiagnosticCode = DiagnosticCode::new("E_DEFINITION_CYCLE");

/// An instance's binding or a parameter reference does not match the placed
/// definition.
pub const INVALID_BINDING: DiagnosticCode = DiagnosticCode::new("E_BINDING");

/// A definition in the context is never instantiated by the scene.
pub const UNUSED_DEFINITION: DiagnosticCode = DiagnosticCode::new("W_UNUSED_DEFINITION");

/// The most elements one definition expansion may produce; beyond it the
/// expansion is refused with a defined size limit rather than allowed to grow
/// without bound (NFR-021). Mirrors the compiler's render-node limit.
pub const MAX_EXPANDED_ELEMENTS: usize = 1_000_000;

/// A scene with its instances expanded, plus the warnings expansion produced.
#[derive(Debug, Clone)]
pub struct Expansion {
    /// The scene's elements with every instance resolved to concrete elements.
    pub scene: Scene,
    /// Warnings recorded while expanding; errors are returned instead.
    pub diagnostics: Diagnostics,
}

/// Expands every instance in `scene` against `definitions` (FEAT-030).
///
/// On success the returned scene carries no `instance` element. On failure every
/// finding is returned and no scene is produced, so a caller never compiles a
/// partially expanded scene (NFR-011).
pub fn expand(scene: &Scene, definitions: &[Definition]) -> Result<Expansion, Diagnostics> {
    let mut expander = Expander {
        scene_id: scene.id.clone(),
        definitions: definitions
            .iter()
            .map(|definition| (definition.id.as_str(), definition))
            .collect(),
        diagnostics: Diagnostics::new(),
        out: Vec::new(),
        used_ids: HashSet::new(),
        referenced: HashSet::new(),
        stack: Vec::new(),
    };

    expander.expand_scene_elements(scene);

    if expander.diagnostics.has_errors() {
        return Err(expander.diagnostics);
    }

    // A definition the scene never instantiates renders nothing and is reported
    // rather than passed off as a successful compile (FEAT-030).
    for definition in definitions {
        if !expander.referenced.contains(definition.id.as_str()) {
            expander.diagnostics.push(Diagnostic::warning(
                UNUSED_DEFINITION,
                format!(
                    "definition `{}` is never instantiated; it renders nothing",
                    definition.id
                ),
            ));
        }
    }

    let mut expanded = scene.clone();
    expanded.elements = expander.out;
    Ok(Expansion {
        scene: expanded,
        diagnostics: expander.diagnostics,
    })
}

/// Checks a scene's instances against the definitions a project provides,
/// without producing an expanded scene (C-002, FEAT-030).
///
/// Used by the project loader so `validate` reports an unresolved definition, a
/// binding mismatch, and a definition cycle before anything is compiled.
pub fn validate_instances(scene: &Scene, definitions: &[Definition]) -> Diagnostics {
    match expand(scene, definitions) {
        Ok(expansion) => expansion.diagnostics,
        Err(diagnostics) => diagnostics,
    }
}

/// A resolved parameter value.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Number(f64),
    Text(String),
    Boolean(bool),
    Token(String),
}

impl Value {
    fn type_name(&self) -> &'static str {
        match self {
            Value::Number(_) => "number",
            Value::Text(_) => "string",
            Value::Boolean(_) => "boolean",
            Value::Token(_) => "token",
        }
    }

    fn matches(&self, expected: ParameterType) -> bool {
        matches!(
            (expected, self),
            (ParameterType::Number, Value::Number(_))
                | (ParameterType::String, Value::Text(_))
                | (ParameterType::Boolean, Value::Boolean(_))
                | (ParameterType::Token, Value::Token(_))
        )
    }
}

/// The values bound to one definition's parameters.
type Bindings = HashMap<String, Value>;

struct Expander<'a> {
    scene_id: String,
    definitions: HashMap<&'a str, &'a Definition>,
    diagnostics: Diagnostics,
    out: Vec<Element>,
    used_ids: HashSet<String>,
    referenced: HashSet<String>,
    stack: Vec<String>,
}

impl Expander<'_> {
    /// Expands the scene's own elements, replacing each instance in place.
    ///
    /// Elements are visited in document order and their `parentId` is preserved,
    /// so a dangling parent or a cycle still reaches the compiler's own
    /// structural check rather than being dropped by this pass. A definition
    /// element is namespaced by the instance that placed it, so the same
    /// definition placed twice yields distinct identifiers.
    fn expand_scene_elements(&mut self, scene: &Scene) {
        let instances: HashSet<&str> = scene
            .elements
            .iter()
            .filter(|element| element.kind == ElementKind::Instance)
            .map(|element| element.id.as_str())
            .collect();

        for element in &scene.elements {
            if element.kind == ElementKind::Instance {
                self.expand_placement(
                    &element.id,
                    element.order,
                    element.definition_ref.as_deref(),
                    element.bindings.as_deref(),
                    None,
                    element.transform.clone(),
                    element.opacity(),
                    element.visible(),
                    element.name.clone(),
                    element.parent_id.as_deref(),
                    None,
                );
                continue;
            }

            if let Some(parent) = &element.parent_id {
                if instances.contains(parent.as_str()) {
                    self.diagnostics.push(
                        Diagnostic::error(
                            INVALID_BINDING,
                            format!(
                                "element `{}` is a child of instance `{parent}`; an instance has no children of its own",
                                element.id
                            ),
                        )
                        .with_location(Location::element(element.id.clone())),
                    );
                }
            }

            let new_id = self.unique(&element.id, None);
            let mut copy = element.clone();
            copy.id = new_id;
            self.out.push(copy);
        }
    }

    /// Splices one placed definition into the output, wrapped in a group that
    /// carries the instance's identity, transform, opacity and visibility.
    #[allow(clippy::too_many_arguments)]
    fn expand_placement(
        &mut self,
        instance_id: &str,
        order: i64,
        definition_ref: Option<&str>,
        bindings: Option<&[Binding]>,
        outer: Option<&Bindings>,
        transform: Transform,
        opacity: f64,
        visible: bool,
        name: Option<String>,
        parent: Option<&str>,
        prefix: Option<&str>,
    ) {
        let Some(definition_ref) = definition_ref.filter(|id| !id.is_empty()) else {
            self.diagnostics.push(
                Diagnostic::error(UNRESOLVED_DEFINITION, "an instance places no definition")
                    .with_location(Location::element(instance_id)),
            );
            return;
        };
        let Some(definition) = self.definitions.get(definition_ref).copied() else {
            self.diagnostics.push(
                Diagnostic::error(
                    UNRESOLVED_DEFINITION,
                    format!(
                        "instance references definition `{definition_ref}`, which does not resolve"
                    ),
                )
                .with_location(Location::element(instance_id)),
            );
            return;
        };
        self.referenced.insert(definition.id.clone());

        if let Some(position) = self.stack.iter().position(|id| id == &definition.id) {
            let mut chain: Vec<&str> = self.stack[position..].iter().map(String::as_str).collect();
            chain.push(&definition.id);
            self.diagnostics.push(
                Diagnostic::error(
                    DEFINITION_CYCLE,
                    format!("definition cycle: {}", chain.join(" -> ")),
                )
                .with_location(Location::element(instance_id)),
            );
            return;
        }

        let values = match resolve_bindings(definition, bindings, outer) {
            Ok(values) => values,
            Err(findings) => {
                self.diagnostics.extend(findings);
                return;
            }
        };

        // The instance's identity travels as a group: it contributes no node of
        // its own, but its id and name join the ancestor chain of every node the
        // definition resolves to (C-003, FEAT-030).
        let group_id = self.unique(instance_id, prefix);
        let group = Element {
            id: group_id.clone(),
            scene_id: Some(self.scene_id.clone()),
            definition_id: None,
            parent_id: parent.map(str::to_string),
            order,
            name,
            kind: ElementKind::Group,
            geometry: Geometry::default(),
            transform,
            fill: None,
            stroke: None,
            font_id: None,
            opacity: NumberValue::Literal(opacity),
            visible: BoolValue::Literal(visible),
            definition_ref: None,
            bindings: None,
        };
        self.out.push(group);

        self.stack.push(definition.id.clone());
        self.instantiate_definition(definition, &values, &group_id);
        self.stack.pop();
    }

    /// Emits a definition's element tree with its parameters resolved.
    fn instantiate_definition(&mut self, definition: &Definition, values: &Bindings, prefix: &str) {
        let children = template_children(&definition.elements);
        let mut roots: Vec<&Element> = definition
            .elements
            .iter()
            .filter(|element| element.parent_id.is_none())
            .collect();
        roots.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.id.cmp(&right.id))
        });

        // A definition whose tree a root cannot reach (a dangling parent or a
        // cycle) is refused rather than silently dropping the unreachable
        // elements (NFR-011).
        let reachable = reachable_templates(&definition.elements, &children, &roots);
        if reachable < definition.elements.len() {
            self.diagnostics.push(Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!(
                    "definition `{}` has elements no root reaches; a dangling parent or a cycle in its tree",
                    definition.id
                ),
            ));
            return;
        }

        for root in roots {
            self.instantiate_element(
                root,
                Some(prefix),
                values,
                prefix,
                &definition.id,
                &children,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn instantiate_element(
        &mut self,
        template: &Element,
        parent: Option<&str>,
        values: &Bindings,
        prefix: &str,
        definition_id: &str,
        children: &HashMap<&str, Vec<&Element>>,
    ) {
        if self.out.len() >= MAX_EXPANDED_ELEMENTS {
            self.diagnostics.push(Diagnostic::error(
                DiagnosticCode::SIZE_LIMIT,
                format!(
                    "definition `{definition_id}` expands beyond the {MAX_EXPANDED_ELEMENTS}-element limit; it is refused rather than truncated"
                ),
            ));
            return;
        }

        if template.kind == ElementKind::Instance {
            let transform = match resolve_transform(&template.transform, values) {
                Ok(transform) => transform,
                Err(findings) => {
                    self.diagnostics.extend(findings);
                    return;
                }
            };
            let opacity = match resolve_number(&template.opacity, values, "/opacity") {
                Ok(value) => value,
                Err(findings) => {
                    self.diagnostics.extend(findings);
                    return;
                }
            };
            let visible = match resolve_bool(&template.visible, values, "/visible") {
                Ok(value) => value,
                Err(findings) => {
                    self.diagnostics.extend(findings);
                    return;
                }
            };
            self.expand_placement(
                &template.id,
                template.order,
                template.definition_ref.as_deref(),
                template.bindings.as_deref(),
                Some(values),
                transform,
                opacity,
                visible,
                template.name.clone(),
                parent,
                Some(prefix),
            );
            return;
        }

        let mut element = match resolve_element(template, &self.scene_id, values) {
            Ok(element) => element,
            Err(findings) => {
                self.diagnostics.extend(findings);
                return;
            }
        };
        let new_id = self.unique(&template.id, Some(prefix));
        element.id = new_id.clone();
        element.parent_id = parent.map(str::to_string);
        self.out.push(element);

        if let Some(list) = children.get(template.id.as_str()) {
            let mut list = list.clone();
            sort_by_order(&mut list, |child| (child.order, child.id.as_str()));
            for child in list {
                self.instantiate_element(
                    child,
                    Some(&new_id),
                    values,
                    prefix,
                    definition_id,
                    children,
                );
            }
        }
    }

    /// A stable identifier unique across the expanded scene.
    ///
    /// A definition element is namespaced by the instance that placed it, so the
    /// same definition placed twice yields distinct identifiers; a collision
    /// with an existing identifier is resolved by a deterministic suffix.
    fn unique(&mut self, base: &str, prefix: Option<&str>) -> String {
        let candidate = match prefix {
            Some(prefix) => format!("{base}~{prefix}"),
            None => base.to_string(),
        };
        if self.used_ids.insert(candidate.clone()) {
            return candidate;
        }
        let mut suffix = 1usize;
        loop {
            let alternative = format!("{candidate}~{suffix}");
            if self.used_ids.insert(alternative.clone()) {
                return alternative;
            }
            suffix += 1;
        }
    }
}

/// Resolves an instance's bindings against the definition's parameters.
///
/// A declared parameter with neither a binding nor a default is an error; a
/// binding for a parameter the definition does not declare is an error; a value
/// whose type does not match the parameter is an error (FEAT-030).
fn resolve_bindings(
    definition: &Definition,
    bindings: Option<&[Binding]>,
    outer: Option<&Bindings>,
) -> Result<Bindings, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    let mut values = Bindings::new();

    for parameter in &definition.parameters {
        let binding = bindings.and_then(|bindings| {
            bindings
                .iter()
                .find(|binding| binding.name == parameter.name)
        });
        let value = match binding {
            Some(binding) => {
                match resolve_binding_value(&binding.value, parameter.value_type, outer) {
                    Ok(value) => value,
                    Err(message) => {
                        diagnostics.push(
                            Diagnostic::error(
                                INVALID_BINDING,
                                format!(
                                    "definition `{}` parameter `{}`: {message}",
                                    definition.id, parameter.name
                                ),
                            )
                            .with_location(Location::element(definition.id.clone())),
                        );
                        continue;
                    }
                }
            }
            None => match &parameter.default {
                Some(default) => default_value(parameter.value_type, default),
                None => {
                    diagnostics.push(
                        Diagnostic::error(
                            INVALID_BINDING,
                            format!(
                                "definition `{}` parameter `{}` has neither a binding nor a default",
                                definition.id, parameter.name
                            ),
                        )
                        .with_location(Location::element(definition.id.clone())),
                    );
                    continue;
                }
            },
        };
        values.insert(parameter.name.clone(), value);
    }

    if let Some(bindings) = bindings {
        for binding in bindings {
            if definition.parameter(&binding.name).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        INVALID_BINDING,
                        format!(
                            "definition `{}` does not declare parameter `{}`",
                            definition.id, binding.name
                        ),
                    )
                    .with_location(Location::element(definition.id.clone())),
                );
            }
        }
    }

    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(values)
    }
}

fn resolve_binding_value(
    value: &BindingValue,
    expected: ParameterType,
    outer: Option<&Bindings>,
) -> Result<Value, String> {
    let resolved = match value {
        BindingValue::Number(number) => Value::Number(*number),
        BindingValue::Text(text) => match expected {
            ParameterType::Token => Value::Token(text.clone()),
            _ => Value::Text(text.clone()),
        },
        BindingValue::Boolean(boolean) => Value::Boolean(*boolean),
        BindingValue::Param(reference) => match outer.and_then(|outer| outer.get(&reference.param))
        {
            Some(value) => value.clone(),
            None => {
                return Err(format!(
                    "forwards `{}`, which the enclosing definition does not declare",
                    reference.param
                ))
            }
        },
    };
    if resolved.matches(expected) {
        Ok(resolved)
    } else {
        Err(format!(
            "a {} value does not match the declared {}",
            resolved.type_name(),
            expected.as_str()
        ))
    }
}

fn default_value(expected: ParameterType, default: &ParameterValue) -> Value {
    match (expected, default) {
        (ParameterType::Number, ParameterValue::Number(value)) => Value::Number(*value),
        (ParameterType::String, ParameterValue::Text(value)) => Value::Text(value.clone()),
        (ParameterType::Token, ParameterValue::Text(value)) => Value::Token(value.clone()),
        (ParameterType::Boolean, ParameterValue::Boolean(value)) => Value::Boolean(*value),
        _ => Value::Text(String::new()),
    }
}

/// Resolves one definition element into a concrete scene element (FEAT-030).
///
/// Every parameter reference is substituted for the value the instance binds,
/// so the result carries literal values only and the compiler sees a concrete
/// element.
fn resolve_element(
    template: &Element,
    scene_id: &str,
    values: &Bindings,
) -> Result<Element, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    let geometry = &template.geometry;

    let element = Element {
        id: template.id.clone(),
        scene_id: Some(scene_id.to_string()),
        definition_id: None,
        parent_id: template.parent_id.clone(),
        order: template.order,
        name: template.name.clone(),
        kind: template.kind,
        geometry: Geometry {
            x: opt_number(&geometry.x, values, "/geometry/x", &mut diagnostics),
            y: opt_number(&geometry.y, values, "/geometry/y", &mut diagnostics),
            width: opt_number(&geometry.width, values, "/geometry/width", &mut diagnostics),
            height: opt_number(
                &geometry.height,
                values,
                "/geometry/height",
                &mut diagnostics,
            ),
            rx: opt_number(&geometry.rx, values, "/geometry/rx", &mut diagnostics),
            ry: opt_number(&geometry.ry, values, "/geometry/ry", &mut diagnostics),
            points: geometry.points.clone(),
            path_data: opt_string(
                &geometry.path_data,
                values,
                "/geometry/pathData",
                &mut diagnostics,
            ),
            text: opt_string(&geometry.text, values, "/geometry/text", &mut diagnostics),
            font_size: opt_number(
                &geometry.font_size,
                values,
                "/geometry/fontSize",
                &mut diagnostics,
            ),
            align: geometry.align,
            line_height: opt_number(
                &geometry.line_height,
                values,
                "/geometry/lineHeight",
                &mut diagnostics,
            ),
            letter_spacing: opt_number(
                &geometry.letter_spacing,
                values,
                "/geometry/letterSpacing",
                &mut diagnostics,
            ),
            count: geometry.count.as_ref().and_then(|value| {
                number_or_report(value, values, "/geometry/count", &mut diagnostics)
                    .map(|value| NumberValue::Literal(value.max(0.0)))
            }),
            spacing: opt_number(
                &geometry.spacing,
                values,
                "/geometry/spacing",
                &mut diagnostics,
            ),
            operation: geometry.operation,
            distance: opt_number(
                &geometry.distance,
                values,
                "/geometry/distance",
                &mut diagnostics,
            ),
            axis: geometry.axis,
        },
        transform: resolve_transform(&template.transform, values).unwrap_or_else(|findings| {
            diagnostics.extend(findings);
            Transform {
                translate_x: NumberValue::Literal(0.0),
                translate_y: NumberValue::Literal(0.0),
                rotate: NumberValue::Literal(0.0),
                scale_x: NumberValue::Literal(1.0),
                scale_y: NumberValue::Literal(1.0),
                skew_x: None,
                skew_y: None,
            }
        }),
        fill: resolve_paint_field(template.fill.as_ref(), values, "/fill", &mut diagnostics),
        stroke: template.stroke.as_ref().and_then(|stroke| {
            resolve_paint_field(
                Some(&stroke.paint),
                values,
                "/stroke/paint",
                &mut diagnostics,
            )
            .map(|paint| crate::scene::Stroke {
                profile_id: stroke.profile_id.clone(),
                paint,
            })
        }),
        font_id: template.font_id.clone(),
        opacity: NumberValue::Literal(
            number_or_report(&template.opacity, values, "/opacity", &mut diagnostics)
                .unwrap_or(1.0),
        ),
        visible: BoolValue::Literal(
            bool_or_report(&template.visible, values, "/visible", &mut diagnostics).unwrap_or(true),
        ),
        definition_ref: template.definition_ref.clone(),
        bindings: template.bindings.clone(),
    };

    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(element)
    }
}

/// Resolves a definition element's fill or stroke paint, substituting a token
/// parameter reference for the token the instance binds (FEAT-030).
fn resolve_paint_field(
    value: Option<&PaintValue>,
    values: &Bindings,
    path: &str,
    diagnostics: &mut Diagnostics,
) -> Option<PaintValue> {
    match value? {
        PaintValue::Paint(paint) => Some(PaintValue::Paint(paint.clone())),
        PaintValue::Param(reference) => {
            match lookup(&reference.param, values, ParameterType::Token, path) {
                Ok(Value::Token(token)) => Some(PaintValue::Paint(Paint {
                    kind: PaintKind::Token,
                    reference: token,
                })),
                Ok(other) => {
                    diagnostics.extend(type_error(
                        &reference.param,
                        other.type_name(),
                        "token",
                        path,
                    ));
                    None
                }
                Err(findings) => {
                    diagnostics.extend(findings);
                    None
                }
            }
        }
    }
}

fn opt_number(
    value: &Option<NumberValue>,
    values: &Bindings,
    path: &str,
    diagnostics: &mut Diagnostics,
) -> Option<NumberValue> {
    value
        .as_ref()
        .and_then(|value| number_or_report(value, values, path, diagnostics))
        .map(NumberValue::Literal)
}

fn opt_string(
    value: &Option<StringValue>,
    values: &Bindings,
    path: &str,
    diagnostics: &mut Diagnostics,
) -> Option<StringValue> {
    value
        .as_ref()
        .and_then(|value| string_or_report(value, values, path, diagnostics))
        .map(StringValue::Literal)
}

fn number_or_report(
    value: &NumberValue,
    values: &Bindings,
    path: &str,
    diagnostics: &mut Diagnostics,
) -> Option<f64> {
    match value {
        NumberValue::Literal(value) => Some(*value),
        NumberValue::Param(reference) => {
            match lookup(&reference.param, values, ParameterType::Number, path) {
                Ok(Value::Number(value)) => Some(value),
                Ok(other) => {
                    diagnostics.extend(type_error(
                        &reference.param,
                        other.type_name(),
                        "number",
                        path,
                    ));
                    None
                }
                Err(findings) => {
                    diagnostics.extend(findings);
                    None
                }
            }
        }
    }
}

fn bool_or_report(
    value: &BoolValue,
    values: &Bindings,
    path: &str,
    diagnostics: &mut Diagnostics,
) -> Option<bool> {
    match value {
        BoolValue::Literal(value) => Some(*value),
        BoolValue::Param(reference) => {
            match lookup(&reference.param, values, ParameterType::Boolean, path) {
                Ok(Value::Boolean(value)) => Some(value),
                Ok(other) => {
                    diagnostics.extend(type_error(
                        &reference.param,
                        other.type_name(),
                        "boolean",
                        path,
                    ));
                    None
                }
                Err(findings) => {
                    diagnostics.extend(findings);
                    None
                }
            }
        }
    }
}

fn string_or_report(
    value: &StringValue,
    values: &Bindings,
    path: &str,
    diagnostics: &mut Diagnostics,
) -> Option<String> {
    match value {
        StringValue::Literal(value) => Some(value.clone()),
        StringValue::Param(reference) => {
            match lookup(&reference.param, values, ParameterType::String, path) {
                Ok(Value::Text(value)) => Some(value),
                Ok(other) => {
                    diagnostics.extend(type_error(
                        &reference.param,
                        other.type_name(),
                        "string",
                        path,
                    ));
                    None
                }
                Err(findings) => {
                    diagnostics.extend(findings);
                    None
                }
            }
        }
    }
}

fn resolve_number(value: &NumberValue, values: &Bindings, path: &str) -> Result<f64, Diagnostics> {
    match value {
        NumberValue::Literal(value) => Ok(*value),
        NumberValue::Param(reference) => {
            let value = lookup(&reference.param, values, ParameterType::Number, path)?;
            match value {
                Value::Number(value) => Ok(value),
                other => Err(type_error(
                    &reference.param,
                    other.type_name(),
                    "number",
                    path,
                )),
            }
        }
    }
}

fn resolve_bool(value: &BoolValue, values: &Bindings, path: &str) -> Result<bool, Diagnostics> {
    match value {
        BoolValue::Literal(value) => Ok(*value),
        BoolValue::Param(reference) => {
            let value = lookup(&reference.param, values, ParameterType::Boolean, path)?;
            match value {
                Value::Boolean(value) => Ok(value),
                other => Err(type_error(
                    &reference.param,
                    other.type_name(),
                    "boolean",
                    path,
                )),
            }
        }
    }
}

/// Looks a parameter's resolved value up and checks its type.
fn lookup(
    name: &str,
    values: &Bindings,
    expected: ParameterType,
    path: &str,
) -> Result<Value, Diagnostics> {
    match values.get(name) {
        Some(value) if value.matches(expected) => Ok(value.clone()),
        Some(value) => Err(type_error(name, value.type_name(), expected.as_str(), path)),
        None => Err(type_error(name, "<unbound>", expected.as_str(), path)),
    }
}

fn type_error(name: &str, found: &str, expected: &str, path: &str) -> Diagnostics {
    Diagnostics::from(
        Diagnostic::error(
            INVALID_BINDING,
            format!("parameter `{name}` is a {found}, but the field at {path} needs a {expected}"),
        )
        .at_path(path),
    )
}

/// Resolves a definition element's transform, substituting parameter references.
fn resolve_transform(transform: &Transform, values: &Bindings) -> Result<Transform, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    let number = |value: &NumberValue, path: &str| resolve_number(value, values, path);
    let resolved = Transform {
        translate_x: NumberValue::Literal(
            number(&transform.translate_x, "/transform/translateX").unwrap_or_else(|findings| {
                diagnostics.extend(findings);
                0.0
            }),
        ),
        translate_y: NumberValue::Literal(
            number(&transform.translate_y, "/transform/translateY").unwrap_or_else(|findings| {
                diagnostics.extend(findings);
                0.0
            }),
        ),
        rotate: NumberValue::Literal(
            number(&transform.rotate, "/transform/rotate").unwrap_or_else(|findings| {
                diagnostics.extend(findings);
                0.0
            }),
        ),
        scale_x: NumberValue::Literal(
            number(&transform.scale_x, "/transform/scaleX").unwrap_or_else(|findings| {
                diagnostics.extend(findings);
                1.0
            }),
        ),
        scale_y: NumberValue::Literal(
            number(&transform.scale_y, "/transform/scaleY").unwrap_or_else(|findings| {
                diagnostics.extend(findings);
                1.0
            }),
        ),
        skew_x: match &transform.skew_x {
            Some(value) => number(value, "/transform/skewX")
                .map(|value| Some(NumberValue::Literal(value)))
                .unwrap_or_else(|findings| {
                    diagnostics.extend(findings);
                    None
                }),
            None => None,
        },
        skew_y: match &transform.skew_y {
            Some(value) => number(value, "/transform/skewY")
                .map(|value| Some(NumberValue::Literal(value)))
                .unwrap_or_else(|findings| {
                    diagnostics.extend(findings);
                    None
                }),
            None => None,
        },
    };
    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(resolved)
    }
}

/// How many of a definition's elements a root reaches through `parentId`.
fn reachable_templates(
    elements: &[Element],
    children: &HashMap<&str, Vec<&Element>>,
    roots: &[&Element],
) -> usize {
    let mut visited: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&Element> = roots.to_vec();
    while let Some(element) = stack.pop() {
        if !visited.insert(element.id.as_str()) {
            continue;
        }
        if let Some(list) = children.get(element.id.as_str()) {
            stack.extend(list.iter().copied());
        }
    }
    visited.len().min(elements.len())
}

fn template_children(elements: &[Element]) -> HashMap<&str, Vec<&Element>> {
    let mut children: HashMap<&str, Vec<&Element>> = HashMap::new();
    for element in elements {
        if let Some(parent) = &element.parent_id {
            children.entry(parent.as_str()).or_default().push(element);
        }
    }
    children
}

fn sort_by_order<T, K: Ord>(list: &mut [T], key: impl Fn(&T) -> K) {
    list.sort_by_key(|item| key(item));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::{compile_with_style, StyleContext};
    use crate::primitives::Shape;
    use crate::scene::{parse as parse_scene, parse_definition};

    const TRANSFORM: &str =
        r#""transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}"#;

    fn definition(source: &str) -> Definition {
        parse_definition(source).expect("a valid definition")
    }

    fn scene(elements: &str) -> Scene {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{{"width":100,"height":100,"background":"transparent"}},"elements":{elements}}}"#
        );
        parse_scene(&source).expect("a valid scene")
    }

    fn instance(id: &str, order: i64, reference: &str, extra: &str) -> String {
        format!(
            r#"{{"id":"{id}","sceneId":"s","order":{order},"kind":"instance","geometry":{{}},{TRANSFORM},"definitionRef":"{reference}","opacity":1,"visible":true{extra}}}"#
        )
    }

    fn compile(
        scene: &Scene,
        definitions: &[Definition],
    ) -> Result<crate::render::RenderModel, Diagnostics> {
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: None,
            definitions,
        };
        compile_with_style(scene, &style)
    }

    /// A definition placing one 10x10 rect named `body`.
    fn chip() -> Definition {
        definition(&format!(
            r#"{{"id":"chip","projectId":"p","name":"Chip","parameters":[],"origin":{{"x":0,"y":0}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"x":0,"y":0,"width":10,"height":10}},{TRANSFORM},"opacity":1,"visible":true}}]}}"#
        ))
    }

    fn width(model: &crate::render::RenderModel, id: &str) -> f64 {
        match &model.node(id).expect("the node").geometry {
            Some(Shape::Rect(rect)) => rect.width,
            other => panic!("expected a rect, got {other:?}"),
        }
    }

    #[test]
    fn an_instance_renders_the_definitions_elements() {
        let definitions = [chip()];
        let scene = scene(&format!("[{}]", instance("i1", 0, "chip", "")));
        let model = compile(&scene, &definitions).expect("compiles");
        assert_eq!(model.nodes.len(), 1);
        assert_eq!(width(&model, "body~i1"), 10.0);
    }

    #[test]
    fn an_instance_contributes_its_identity_to_the_ancestor_chain() {
        let definitions = [chip()];
        let mut scene = scene(&format!("[{}]", instance("i1", 0, "chip", "")));
        scene.elements[0].name = Some("Placed".to_string());
        let model = compile(&scene, &definitions).expect("compiles");
        let node = model.node("body~i1").expect("the node");
        assert_eq!(node.groups.len(), 1);
        assert_eq!(node.groups[0].id, "i1");
        assert_eq!(node.groups[0].name.as_deref(), Some("Placed"));
    }

    #[test]
    fn a_definition_placed_twice_renders_at_each_placement() {
        let definitions = [chip()];
        let first = instance("i1", 0, "chip", "").replace(r#""translateX":0"#, r#""translateX":5"#);
        let second =
            instance("i2", 1, "chip", "").replace(r#""translateX":0"#, r#""translateX":50"#);
        let scene = scene(&format!("[{first},{second}]"));
        let model = compile(&scene, &definitions).expect("compiles");
        assert_eq!(model.nodes.len(), 2);
        let positions: Vec<f64> = model
            .nodes
            .iter()
            .map(|node| node.transform.apply([0.0, 0.0])[0])
            .collect();
        assert_eq!(positions, vec![5.0, 50.0]);
    }

    #[test]
    fn a_bound_parameter_changes_the_field_it_occupies() {
        let definitions = [definition(&format!(
            r#"{{"id":"chip","projectId":"p","name":"Chip","parameters":[{{"name":"w","type":"number"}}],"origin":{{"x":0,"y":0}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"x":0,"y":0,"width":{{"param":"w"}},"height":10}},{TRANSFORM},"opacity":1,"visible":true}}]}}"#
        ))];
        let scene = scene(&format!(
            "[{}]",
            instance("i1", 0, "chip", r#","bindings":[{"name":"w","value":42}]"#)
        ));
        let model = compile(&scene, &definitions).expect("compiles");
        assert_eq!(width(&model, "body~i1"), 42.0);
    }

    #[test]
    fn an_unbound_parameter_takes_its_declared_default() {
        let definitions = [definition(&format!(
            r#"{{"id":"chip","projectId":"p","name":"Chip","parameters":[{{"name":"w","type":"number","default":30}}],"origin":{{"x":0,"y":0}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"x":0,"y":0,"width":{{"param":"w"}},"height":10}},{TRANSFORM},"opacity":1,"visible":true}}]}}"#
        ))];
        let scene = scene(&format!("[{}]", instance("i1", 0, "chip", "")));
        let model = compile(&scene, &definitions).expect("compiles");
        assert_eq!(width(&model, "body~i1"), 30.0);
    }

    #[test]
    fn a_nested_definition_resolves_to_concrete_geometry() {
        let inner = chip();
        let outer = definition(&format!(
            r#"{{"id":"outer","projectId":"p","name":"Outer","parameters":[],"origin":{{"x":0,"y":0}},"elements":[{{"id":"place","definitionId":"outer","order":0,"kind":"instance","geometry":{{}},{TRANSFORM},"definitionRef":"chip","opacity":1,"visible":true}}]}}"#
        ));
        let scene = scene(&format!("[{}]", instance("i1", 0, "outer", "")));
        let model = compile(&scene, &[inner, outer]).expect("compiles");
        assert_eq!(model.nodes.len(), 1);
        assert!(model.node("body~place~i1").is_some(), "{:?}", model.nodes);
    }

    #[test]
    fn a_definition_cycle_is_refused_naming_the_cycle() {
        let a = definition(&format!(
            r#"{{"id":"a","projectId":"p","name":"A","parameters":[],"origin":{{"x":0,"y":0}},"elements":[{{"id":"pa","definitionId":"a","order":0,"kind":"instance","geometry":{{}},{TRANSFORM},"definitionRef":"b","opacity":1,"visible":true}}]}}"#
        ));
        let b = definition(&format!(
            r#"{{"id":"b","projectId":"p","name":"B","parameters":[],"origin":{{"x":0,"y":0}},"elements":[{{"id":"pb","definitionId":"b","order":0,"kind":"instance","geometry":{{}},{TRANSFORM},"definitionRef":"a","opacity":1,"visible":true}}]}}"#
        ));
        let scene = scene(&format!("[{}]", instance("i1", 0, "a", "")));
        let diagnostics = compile(&scene, &[a, b]).expect_err("a cycle is refused");
        let error = diagnostics
            .errors()
            .find(|error| error.code == DEFINITION_CYCLE)
            .expect("a cycle error");
        assert!(
            error.message.contains('a') && error.message.contains('b'),
            "{}",
            error.message
        );
    }

    #[test]
    fn an_unresolved_definition_is_refused() {
        let scene = scene(&format!("[{}]", instance("i1", 0, "ghost", "")));
        let diagnostics = compile(&scene, &[]).expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.code == UNRESOLVED_DEFINITION && error.message.contains("ghost")));
    }

    #[test]
    fn a_binding_for_an_undeclared_parameter_is_refused() {
        let definitions = [chip()];
        let scene = scene(&format!(
            "[{}]",
            instance(
                "i1",
                0,
                "chip",
                r#","bindings":[{"name":"ghost","value":1}]"#
            )
        ));
        let diagnostics = compile(&scene, &definitions).expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.code == INVALID_BINDING && error.message.contains("ghost")));
    }

    #[test]
    fn a_parameter_with_neither_a_binding_nor_a_default_is_refused() {
        let definitions = [definition(&format!(
            r#"{{"id":"chip","projectId":"p","name":"Chip","parameters":[{{"name":"w","type":"number"}}],"origin":{{"x":0,"y":0}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"x":0,"y":0,"width":{{"param":"w"}},"height":10}},{TRANSFORM},"opacity":1,"visible":true}}]}}"#
        ))];
        let scene = scene(&format!("[{}]", instance("i1", 0, "chip", "")));
        let diagnostics = compile(&scene, &definitions).expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.code == INVALID_BINDING));
    }

    #[test]
    fn a_definition_never_instantiated_warns_and_renders_nothing() {
        let definitions = [chip()];
        let scene = scene("[]");
        let model = compile(&scene, &definitions).expect("compiles");
        assert!(model.nodes.is_empty());
        assert!(model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == UNUSED_DEFINITION && warning.message.contains("chip")));
    }

    #[test]
    fn expansion_is_deterministic() {
        let definitions = [chip()];
        let scene = scene(&format!(
            "[{},{}]",
            instance("i1", 0, "chip", ""),
            instance("i2", 1, "chip", "")
        ));
        let first = compile(&scene, &definitions).expect("compiles");
        let second = compile(&scene, &definitions).expect("compiles");
        assert_eq!(first, second, "expansion must be byte-identical (NFR-010)");
    }

    #[test]
    fn a_parameter_reference_the_definition_does_not_declare_is_refused_at_parse() {
        let diagnostics = parse_definition(&format!(
            r#"{{"id":"chip","projectId":"p","name":"Chip","parameters":[],"origin":{{"x":0,"y":0}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"width":{{"param":"ghost"}},"height":10}},{TRANSFORM},"opacity":1,"visible":true}}]}}"#
        ))
        .expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.message.contains("ghost")));
    }

    #[test]
    fn a_binding_can_forward_an_enclosing_definitions_parameter() {
        let inner = definition(&format!(
            r#"{{"id":"chip","projectId":"p","name":"Chip","parameters":[{{"name":"w","type":"number"}}],"origin":{{"x":0,"y":0}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"x":0,"y":0,"width":{{"param":"w"}},"height":10}},{TRANSFORM},"opacity":1,"visible":true}}]}}"#
        ));
        let outer = definition(&format!(
            r#"{{"id":"outer","projectId":"p","name":"Outer","parameters":[{{"name":"n","type":"number"}}],"origin":{{"x":0,"y":0}},"elements":[{{"id":"place","definitionId":"outer","order":0,"kind":"instance","geometry":{{}},{TRANSFORM},"definitionRef":"chip","bindings":[{{"name":"w","value":{{"param":"n"}}}}],"opacity":1,"visible":true}}]}}"#
        ));
        let scene = scene(&format!(
            "[{}]",
            instance("i1", 0, "outer", r#","bindings":[{"name":"n","value":77}]"#)
        ));
        let model = compile(&scene, &[inner, outer]).expect("compiles");
        assert_eq!(width(&model, "body~place~i1"), 77.0);
    }

    #[test]
    fn a_token_parameter_default_resolves_to_the_named_token() {
        let definitions = [definition(&format!(
            r#"{{"id":"chip","projectId":"p","name":"Chip","parameters":[{{"name":"tint","type":"token","default":"accent"}}],"origin":{{"x":0,"y":0}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"x":0,"y":0,"width":10,"height":10}},{TRANSFORM},"fill":{{"param":"tint"}},"opacity":1,"visible":true}}]}}"#
        ))];
        let scene = scene(&format!("[{}]", instance("i1", 0, "chip", "")));
        let model = compile(&scene, &definitions).expect("compiles");
        assert_eq!(
            model.node("body~i1").unwrap().paint.fill,
            Some(crate::render::Paint::Color {
                value: "accent".to_string()
            })
        );
    }

    #[test]
    fn a_definition_round_trips_without_loss() {
        let original = definition(&format!(
            r#"{{"id":"chip","projectId":"p","name":"Chip","parameters":[{{"name":"w","type":"number","default":30}},{{"name":"tint","type":"token","default":"accent"}}],"origin":{{"x":1,"y":2}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"x":0,"y":0,"width":{{"param":"w"}},"height":10}},{TRANSFORM},"fill":{{"param":"tint"}},"opacity":1,"visible":true}}]}}"#
        ));
        let text = original.to_json_string().expect("serializable");
        assert_eq!(
            parse_definition(&text).expect("round-trips"),
            original,
            "a definition round-trips without loss"
        );
    }

    #[test]
    fn a_scene_element_that_belongs_to_a_definition_is_refused() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{{"width":10,"height":10,"background":"transparent"}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"width":1,"height":1}},{TRANSFORM},"opacity":1,"visible":true}}]}}"#
        );
        let diagnostics = parse_scene(&source).expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.message.contains("belongs to a definition")));
    }

    #[test]
    fn a_parameter_reference_whose_type_mismatches_the_field_is_refused_at_parse() {
        let diagnostics = parse_definition(&format!(
            r#"{{"id":"chip","projectId":"p","name":"Chip","parameters":[{{"name":"s","type":"string"}}],"origin":{{"x":0,"y":0}},"elements":[{{"id":"body","definitionId":"chip","order":0,"kind":"rect","geometry":{{"width":{{"param":"s"}},"height":10}},{TRANSFORM},"opacity":1,"visible":true}}]}}"#
        ))
        .expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.message.contains("string")));
    }
}
