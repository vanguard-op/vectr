//! Acceptance tests for reusable part definitions (FEAT-030).
//!
//! Drives the shipped library through its public API (C-002) and the `vectr`
//! binary as a subprocess (C-004). A definition is a project-scoped reusable
//! part: a scene places it with an `instance` element, binds its declared
//! parameters, and the compiler lowers every instance to concrete elements
//! before it walks the tree, so the render model (C-003) carries no unresolved
//! definition reference. These tests prove the acceptance criteria and edge
//! cases on the render model, the exported SVG, and a project's compiled output.

mod common;

use common::*;
use serde_json::{json, Value};
use vectr_core::compiler::expand::MAX_EXPANDED_ELEMENTS;
use vectr_core::{
    Definition, DiagnosticCode, ResolvedNode, Shape, StyleContext, DEFINITION_CYCLE,
    INVALID_BINDING, UNRESOLVED_DEFINITION, UNUSED_DEFINITION,
};

// ---------------------------------------------------------------------------
// Builders for the one-element model
// ---------------------------------------------------------------------------

/// The transform every element in these tests declares.
fn identity() -> Value {
    json!({
        "translateX": 0.0,
        "translateY": 0.0,
        "rotate": 0.0,
        "scaleX": 1.0,
        "scaleY": 1.0
    })
}

/// One element owned by a definition: the same `Element` shape a scene's is,
/// addressed by `definitionId` instead of `sceneId` (C-001).
fn def_element(id: &str, definition: &str, order: i64, kind: &str, geometry: Value) -> Value {
    json!({
        "id": id,
        "definitionId": definition,
        "order": order,
        "kind": kind,
        "geometry": geometry,
        "transform": identity(),
        "opacity": 1.0,
        "visible": true
    })
}

/// A rect owned by a definition.
fn def_rect(id: &str, definition: &str, order: i64, width: f64, height: f64) -> Value {
    def_element(
        id,
        definition,
        order,
        "rect",
        json!({ "x": 0.0, "y": 0.0, "width": width, "height": height }),
    )
}

/// An instance element owned by a definition, placing `reference`.
fn def_instance(id: &str, definition: &str, order: i64, reference: &str) -> Value {
    let mut value = def_element(id, definition, order, "instance", json!({}));
    value["definitionRef"] = json!(reference);
    value
}

/// A Definition document from its parameters and elements (C-001, FEAT-030).
fn definition(id: &str, parameters: Value, elements: Vec<Value>) -> Value {
    json!({
        "id": id,
        "projectId": "p",
        "name": id,
        "parameters": parameters,
        "origin": { "x": 0.0, "y": 0.0 },
        "elements": elements
    })
}

/// Parses a definition, panicking with the diagnostics on failure.
fn parse_definition(document: &Value) -> Definition {
    vectr_core::parse_definition(&document.to_string()).unwrap_or_else(|diagnostics| {
        panic!("expected a valid definition, got: {diagnostics}");
    })
}

/// A scene instance element placing `reference`, with optional bindings.
fn instance(id: &str, order: i64, reference: &str, bindings: Option<Value>) -> Value {
    let mut value = json!({
        "id": id,
        "sceneId": SCENE_ID,
        "order": order,
        "kind": "instance",
        "geometry": {},
        "definitionRef": reference,
        "transform": identity(),
        "opacity": 1.0,
        "visible": true
    });
    if let Some(bindings) = bindings {
        value["bindings"] = bindings;
    }
    value
}

/// A style context carrying only reusable definitions.
fn with_definitions(definitions: &[Definition]) -> StyleContext<'_> {
    StyleContext {
        palette: None,
        strokes: &[],
        gradients: &[],
        fonts: &[],
        recipe: None,
        definitions,
    }
}

/// The node with `id`, panicking when the model does not carry it.
fn node<'a>(model: &'a vectr_core::RenderModel, id: &str) -> &'a ResolvedNode {
    model
        .node(id)
        .unwrap_or_else(|| panic!("expected node `{id}` in {:?}", model.nodes))
}

/// The width of a resolved rect node.
fn rect_width(model: &vectr_core::RenderModel, id: &str) -> f64 {
    match &node(model, id).geometry {
        Some(Shape::Rect(rect)) => rect.width,
        other => panic!("expected a rect at `{id}`, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Acceptance criteria
// ---------------------------------------------------------------------------

/// A definition authored once renders its elements where an instance places it.
#[test]
fn an_instance_renders_the_elements_of_the_definition_it_places() {
    let definitions = [parse_definition(&definition(
        "chip",
        json!([]),
        vec![def_rect("body", "chip", 0, 10.0, 10.0)],
    ))];
    let document = scene(vec![instance("i1", 0, "chip", None)]);

    let model = compile_with(&document, &with_definitions(&definitions)).expect("compiles");
    assert_eq!(model.nodes.len(), 1, "{:?}", model.nodes);
    assert_eq!(rect_width(&model, "body~i1"), 10.0);
    // No unresolved reference survives into the render model (C-003).
    assert!(
        model.nodes.iter().all(|node| node.kind != "instance"),
        "{:?}",
        model.nodes
    );
}

/// A definition referenced many times in one scene renders at each placement.
#[test]
fn each_instance_renders_at_its_own_placement() {
    let definitions = [parse_definition(&definition(
        "chip",
        json!([]),
        vec![def_rect("body", "chip", 0, 10.0, 10.0)],
    ))];
    let mut first = instance("i1", 0, "chip", None);
    first["transform"]["translateX"] = json!(5.0);
    let mut second = instance("i2", 1, "chip", None);
    second["transform"]["translateX"] = json!(50.0);
    let document = scene(vec![first, second]);

    let model = compile_with(&document, &with_definitions(&definitions)).expect("compiles");
    assert_eq!(model.nodes.len(), 2, "{:?}", model.nodes);
    let positions: Vec<f64> = model
        .nodes
        .iter()
        .map(|node| node.transform.apply([0.0, 0.0])[0])
        .collect();
    assert_eq!(positions, vec![5.0, 50.0]);
}

/// A declared parameter an instance binds takes the bound value in the field the
/// definition references it from.
#[test]
fn a_bound_parameter_takes_the_value_the_instance_binds() {
    let definitions = [parse_definition(&definition(
        "chip",
        json!([{ "name": "w", "type": "number" }]),
        vec![def_element(
            "body",
            "chip",
            0,
            "rect",
            json!({ "x": 0.0, "y": 0.0, "width": { "param": "w" }, "height": 10.0 }),
        )],
    ))];
    let document = scene(vec![instance(
        "i1",
        0,
        "chip",
        Some(json!([{ "name": "w", "value": 42 }])),
    )]);

    let model = compile_with(&document, &with_definitions(&definitions)).expect("compiles");
    assert_eq!(rect_width(&model, "body~i1"), 42.0);
}

/// A parameter an instance does not bind takes its declared default.
#[test]
fn an_unbound_parameter_takes_its_declared_default() {
    let definitions = [parse_definition(&definition(
        "chip",
        json!([{ "name": "w", "type": "number", "default": 30 }]),
        vec![def_element(
            "body",
            "chip",
            0,
            "rect",
            json!({ "x": 0.0, "y": 0.0, "width": { "param": "w" }, "height": 10.0 }),
        )],
    ))];
    let document = scene(vec![instance("i1", 0, "chip", None)]);

    let model = compile_with(&document, &with_definitions(&definitions)).expect("compiles");
    assert_eq!(rect_width(&model, "body~i1"), 30.0);
}

/// Two instances of one definition that bind different values render their own
/// appearance.
#[test]
fn two_instances_binding_different_values_render_their_own_appearance() {
    let definitions = [parse_definition(&definition(
        "swatch",
        json!([{ "name": "tint", "type": "token", "default": "accent" }]),
        vec![{
            let mut body = def_rect("body", "swatch", 0, 10.0, 10.0);
            body["fill"] = json!({ "param": "tint" });
            body
        }],
    ))];
    let document = scene_with(
        vec![
            instance(
                "i1",
                0,
                "swatch",
                Some(json!([{ "name": "tint", "value": "accent" }])),
            ),
            instance(
                "i2",
                1,
                "swatch",
                Some(json!([{ "name": "tint", "value": "ink" }])),
            ),
        ],
        None,
        Some("brand"),
    );
    let palette = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#e94560"), ("ink", "#1a1a2e")],
    ))
    .expect("a palette");
    let style = StyleContext {
        palette: Some(&palette),
        definitions: &definitions,
        ..Default::default()
    };

    let model = compile_with(&document, &style).expect("compiles");
    assert_eq!(fill_color(node(&model, "body~i1")), Some("#e94560"));
    assert_eq!(fill_color(node(&model, "body~i2")), Some("#1a1a2e"));
}

/// A definition that places another definition resolves to concrete geometry
/// with no unresolved reference.
#[test]
fn a_nested_definition_resolves_to_concrete_geometry_with_no_unresolved_reference() {
    let inner = parse_definition(&definition(
        "chip",
        json!([]),
        vec![def_rect("body", "chip", 0, 10.0, 10.0)],
    ));
    let outer = parse_definition(&definition(
        "frame",
        json!([]),
        vec![def_instance("place", "frame", 0, "chip")],
    ));
    let document = scene(vec![instance("i1", 0, "frame", None)]);

    let model = compile_with(&document, &with_definitions(&[inner, outer])).expect("compiles");
    assert_eq!(model.nodes.len(), 1, "{:?}", model.nodes);
    assert_eq!(rect_width(&model, "body~place~i1"), 10.0);
    // The nested placement lowered to concrete elements; no instance node and no
    // `definitionRef` survives (C-003).
    assert!(
        model.nodes.iter().all(|node| node.kind != "instance"),
        "{:?}",
        model.nodes
    );
    let json = model.to_json_string().expect("serializable");
    assert!(
        !json.contains("definitionRef") && !json.contains("\"instance\""),
        "{json}"
    );
}

/// The same definition and bindings compile to identical output on every run.
#[test]
fn the_same_definition_and_bindings_compile_identically() {
    let definitions = [parse_definition(&definition(
        "chip",
        json!([{ "name": "w", "type": "number", "default": 12 }]),
        vec![def_element(
            "body",
            "chip",
            0,
            "rect",
            json!({ "x": 0.0, "y": 0.0, "width": { "param": "w" }, "height": 10.0 }),
        )],
    ))];
    let document = scene(vec![
        instance("i1", 0, "chip", None),
        instance("i2", 1, "chip", Some(json!([{ "name": "w", "value": 7 }]))),
    ]);

    let first = compile_with(&document, &with_definitions(&definitions)).expect("compiles");
    let second = compile_with(&document, &with_definitions(&definitions)).expect("compiles");
    assert_eq!(first, second, "expansion is deterministic (NFR-010)");
    assert_eq!(
        first.to_json_string().expect("serializable"),
        second.to_json_string().expect("serializable"),
    );
}

/// Editing a definition is reflected by every instance in one recompile.
#[test]
fn editing_a_definition_reflects_in_every_instance_in_one_recompile() {
    let before = parse_definition(&definition(
        "chip",
        json!([]),
        vec![def_rect("body", "chip", 0, 10.0, 10.0)],
    ));
    let after = parse_definition(&definition(
        "chip",
        json!([]),
        vec![def_rect("body", "chip", 0, 40.0, 40.0)],
    ));
    let document = scene(vec![
        instance("i1", 0, "chip", None),
        instance("i2", 1, "chip", None),
    ]);

    let initial = compile_with(&document, &with_definitions(&[before])).expect("compiles");
    assert_eq!(rect_width(&initial, "body~i1"), 10.0);
    assert_eq!(rect_width(&initial, "body~i2"), 10.0);

    let edited = compile_with(&document, &with_definitions(&[after])).expect("recompiles");
    assert_eq!(rect_width(&edited, "body~i1"), 40.0);
    assert_eq!(rect_width(&edited, "body~i2"), 40.0);
}

/// A definition placed by more than one scene reflects an edit in every placing
/// scene on its next compile.
#[test]
fn editing_a_definition_reflects_in_every_scene_that_places_it() {
    let before = parse_definition(&definition(
        "chip",
        json!([]),
        vec![def_rect("body", "chip", 0, 10.0, 10.0)],
    ));
    let after = parse_definition(&definition(
        "chip",
        json!([]),
        vec![def_rect("body", "chip", 0, 40.0, 40.0)],
    ));
    let scene_a = scene(vec![instance("a", 0, "chip", None)]);
    let scene_b = scene(vec![instance("b", 0, "chip", None)]);

    for document in [&scene_a, &scene_b] {
        let initial = compile_with(document, &with_definitions(std::slice::from_ref(&before)))
            .expect("compiles before the edit");
        assert_eq!(initial.nodes.len(), 1);
        let edited = compile_with(document, &with_definitions(std::slice::from_ref(&after)))
            .expect("recompiles after the edit");
        assert_eq!(
            rect_width(&edited, &format!("body~{}", first_instance_id(document))),
            40.0,
            "every placing scene reflects the shared edit"
        );
    }
}

/// The identifier of the single instance in a test scene.
fn first_instance_id(document: &Value) -> String {
    document["elements"][0]["id"]
        .as_str()
        .expect("an instance id")
        .to_string()
}

// ---------------------------------------------------------------------------
// Edge cases and failure states
// ---------------------------------------------------------------------------

/// A reference to a definition that does not resolve is a located validation
/// error naming the reference.
#[test]
fn an_unresolved_definition_is_a_located_validation_error() {
    let document = scene(vec![instance("i1", 0, "ghost", None)]);

    let diagnostics = compile_with(&document, &with_definitions(&[])).expect_err("refused");
    let error = diagnostics
        .errors()
        .find(|error| error.code == UNRESOLVED_DEFINITION)
        .expect("an unresolved-definition error");
    assert!(error.message.contains("ghost"), "{}", error.message);
    assert_eq!(
        error
            .location
            .as_ref()
            .and_then(|location| location.element_id.as_deref()),
        Some("i1"),
        "the error names the reference's location"
    );
}

/// A reference cycle among definitions is a validation error naming the cycle.
#[test]
fn a_definition_cycle_is_a_validation_error_naming_the_cycle() {
    let a = parse_definition(&definition(
        "a",
        json!([]),
        vec![def_instance("pa", "a", 0, "b")],
    ));
    let b = parse_definition(&definition(
        "b",
        json!([]),
        vec![def_instance("pb", "b", 0, "a")],
    ));
    let document = scene(vec![instance("i1", 0, "a", None)]);

    let diagnostics = compile_with(&document, &with_definitions(&[a, b])).expect_err("refused");
    let error = diagnostics
        .errors()
        .find(|error| error.code == DEFINITION_CYCLE)
        .expect("a definition-cycle error");
    assert!(
        error.message.contains('a') && error.message.contains('b'),
        "the cycle names its definitions: {}",
        error.message
    );
}

/// A binding for a parameter the definition does not declare is a validation
/// error naming the parameter.
#[test]
fn a_binding_for_an_undeclared_parameter_is_a_located_validation_error() {
    let definitions = [parse_definition(&definition(
        "chip",
        json!([]),
        vec![def_rect("body", "chip", 0, 10.0, 10.0)],
    ))];
    let document = scene(vec![instance(
        "i1",
        0,
        "chip",
        Some(json!([{ "name": "ghost", "value": 1 }])),
    )]);

    let diagnostics =
        compile_with(&document, &with_definitions(&definitions)).expect_err("refused");
    let error = diagnostics
        .errors()
        .find(|error| error.code == INVALID_BINDING)
        .expect("an invalid-binding error");
    assert!(error.message.contains("ghost"), "{}", error.message);
}

/// An element that references a parameter the definition does not declare is
/// refused, naming the reference and its location.
#[test]
fn a_reference_to_an_undeclared_parameter_is_refused_naming_the_reference() {
    let document = definition(
        "chip",
        json!([]),
        vec![def_element(
            "body",
            "chip",
            0,
            "rect",
            json!({ "x": 0.0, "y": 0.0, "width": { "param": "ghost" }, "height": 10.0 }),
        )],
    );

    let diagnostics = vectr_core::parse_definition(&document.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert!(error.message.contains("ghost"), "{}", error.message);
    assert_eq!(
        error
            .location
            .as_ref()
            .and_then(|location| location.element_id.as_deref()),
        Some("body"),
        "the error names the reference's location"
    );
}

/// A reference whose type does not match the field it occupies is refused.
#[test]
fn a_reference_whose_type_does_not_match_the_field_is_refused() {
    let document = definition(
        "chip",
        json!([{ "name": "s", "type": "string" }]),
        vec![def_element(
            "body",
            "chip",
            0,
            "rect",
            json!({ "x": 0.0, "y": 0.0, "width": { "param": "s" }, "height": 10.0 }),
        )],
    );

    let diagnostics = vectr_core::parse_definition(&document.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert!(error.message.contains("string"), "{}", error.message);
    assert_eq!(
        error
            .location
            .as_ref()
            .and_then(|location| location.element_id.as_deref()),
        Some("body"),
        "the error names the reference's location"
    );
}

/// A parameter with neither a binding nor a default is a validation error naming
/// the parameter.
#[test]
fn a_parameter_with_neither_a_binding_nor_a_default_is_a_located_validation_error() {
    let definitions = [parse_definition(&definition(
        "chip",
        json!([{ "name": "w", "type": "number" }]),
        vec![def_element(
            "body",
            "chip",
            0,
            "rect",
            json!({ "x": 0.0, "y": 0.0, "width": { "param": "w" }, "height": 10.0 }),
        )],
    ))];
    let document = scene(vec![instance("i1", 0, "chip", None)]);

    let diagnostics =
        compile_with(&document, &with_definitions(&definitions)).expect_err("refused");
    let error = diagnostics
        .errors()
        .find(|error| error.code == INVALID_BINDING)
        .expect("an invalid-binding error");
    assert!(error.message.contains('w'), "{}", error.message);
}

/// A scene element whose field holds a parameter reference is a located
/// validation error: a scene declares no parameters.
#[test]
fn a_scene_element_holding_a_parameter_reference_is_a_located_validation_error() {
    let mut element = rect("body", 0, 0.0, 0.0, 10.0, 10.0);
    element["geometry"]["width"] = json!({ "param": "w" });
    let document = scene(vec![element]);

    let diagnostics = vectr_core::parse(&document.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert!(
        error.message.contains("parameter reference"),
        "{}",
        error.message
    );
    assert_eq!(
        error
            .location
            .as_ref()
            .and_then(|location| location.json_path.as_deref()),
        Some("/elements/0/geometry/width"),
        "the error names the reference's location"
    );
}

/// A definition that is never instantiated renders nothing and warns; it is not
/// an error.
#[test]
fn a_definition_never_instantiated_renders_nothing_and_warns() {
    let definitions = [parse_definition(&definition(
        "chip",
        json!([]),
        vec![def_rect("body", "chip", 0, 10.0, 10.0)],
    ))];
    let document = scene(vec![]);

    let model = compile_with(&document, &with_definitions(&definitions)).expect("compiles");
    assert!(model.nodes.is_empty(), "{:?}", model.nodes);
    assert!(
        model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == UNUSED_DEFINITION && warning.message.contains("chip")),
        "{:?}",
        model.diagnostics
    );
    assert!(!model.diagnostics.has_errors(), "{:?}", model.diagnostics);
}

/// An edit to a shared definition that introduces a cycle fails in every scene
/// that places it, not only in the scene being edited.
#[test]
fn a_shared_definition_edit_that_introduces_a_cycle_fails_in_every_placing_scene() {
    let a = parse_definition(&definition(
        "a",
        json!([]),
        vec![def_instance("pa", "a", 0, "b")],
    ));
    let b = parse_definition(&definition(
        "b",
        json!([]),
        vec![def_instance("pb", "b", 0, "a")],
    ));
    let scene_a = scene(vec![instance("a", 0, "a", None)]);
    let scene_b = scene(vec![instance("b", 0, "a", None)]);

    for document in [&scene_a, &scene_b] {
        let diagnostics = compile_with(document, &with_definitions(&[a.clone(), b.clone()]))
            .expect_err("every scene that places the edited definition fails");
        assert!(
            diagnostics
                .errors()
                .any(|error| error.code == DEFINITION_CYCLE),
            "{diagnostics}"
        );
    }
}

/// Expansion beyond the supported size limit is refused with a defined size
/// limit naming the definition; it is never silently dropped or truncated.
#[test]
fn an_expansion_beyond_the_size_limit_is_refused_naming_the_definition() {
    let depth = 20;
    let definitions = doubling_chain(depth);
    let document = scene(vec![instance("i1", 0, "d0", None)]);

    let diagnostics =
        compile_with(&document, &with_definitions(&definitions)).expect_err("refused");
    let error = diagnostics
        .errors()
        .find(|error| error.code == DiagnosticCode::SIZE_LIMIT)
        .expect("a defined size-limit error");
    assert!(
        error.message.contains("definition `") && error.message.contains("expands beyond"),
        "the limit names the definition: {}",
        error.message
    );
    assert!(
        error.message.contains(&format!("{MAX_EXPANDED_ELEMENTS}")),
        "the limit names the figure: {}",
        error.message
    );
}

/// A chain of `depth` definitions where each level places the next twice, so a
/// single placement expands to more than the supported element limit.
fn doubling_chain(depth: usize) -> Vec<Definition> {
    let mut definitions = Vec::new();
    for level in 0..depth {
        let id = format!("d{level}");
        let elements = if level + 1 == depth {
            vec![def_rect("leaf", &id, 0, 1.0, 1.0)]
        } else {
            let next = format!("d{}", level + 1);
            vec![
                def_instance("a", &id, 0, &next),
                def_instance("b", &id, 1, &next),
            ]
        };
        definitions.push(parse_definition(&definition(&id, json!([]), elements)));
    }
    definitions
}

// ---------------------------------------------------------------------------
// Project-scoped behaviour through the CLI (C-004)
// ---------------------------------------------------------------------------

/// A project holding a set of definition documents and scene documents.
fn project(tag: &str, definitions: &[(&str, Value)], scenes: &[(&str, Value)]) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", "{}");
    for (id, document) in definitions {
        dir.write(&format!("definitions/{id}.json"), &document.to_string());
    }
    for (id, document) in scenes {
        dir.write(&format!("scenes/{id}.json"), &document.to_string());
    }
    dir
}

/// A scene document whose single instance places `reference`.
fn placing_scene(id: &str, reference: &str) -> Value {
    let mut document = scene(vec![instance("i1", 0, reference, None)]);
    document["id"] = json!(id);
    document["elements"][0]["sceneId"] = json!(id);
    document
}

/// The width of the first rect node in a render model written to `path`.
fn compiled_rect_width(path: &std::path::Path) -> f64 {
    let text = std::fs::read_to_string(path).expect("the render model was written");
    let model = vectr_core::render::parse(&text).expect("a render model");
    match &model.nodes[0].geometry {
        Some(Shape::Rect(rect)) => rect.width,
        other => panic!("expected a rect, got {other:?}"),
    }
}

/// A definition in a project renders in every scene that places it.
#[test]
fn a_project_definition_renders_in_every_scene_that_places_it() {
    let dir = project(
        "definition-project",
        &[(
            "chip",
            definition(
                "chip",
                json!([]),
                vec![def_rect("body", "chip", 0, 10.0, 10.0)],
            ),
        )],
        &[
            ("a", placing_scene("a", "chip")),
            ("b", placing_scene("b", "chip")),
        ],
    );

    for scene_id in ["a", "b"] {
        let output = run_vectr(
            dir.path(),
            &["compile", scene_id, "--out", &format!("{scene_id}.json")],
        );
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        assert_eq!(
            compiled_rect_width(&dir.path().join(format!("{scene_id}.json"))),
            10.0
        );
    }
}

/// Editing a project definition reflects in every placing scene on its next
/// compile.
#[test]
fn editing_a_project_definition_reflects_in_every_placing_scene_on_recompile() {
    let dir = project(
        "definition-edit-project",
        &[(
            "chip",
            definition(
                "chip",
                json!([]),
                vec![def_rect("body", "chip", 0, 10.0, 10.0)],
            ),
        )],
        &[
            ("a", placing_scene("a", "chip")),
            ("b", placing_scene("b", "chip")),
        ],
    );

    for scene_id in ["a", "b"] {
        let output = run_vectr(
            dir.path(),
            &["compile", scene_id, "--out", &format!("{scene_id}.json")],
        );
        assert_eq!(code(&output), 0, "{}", stderr(&output));
    }

    // One edit to the shared definition, then a recompile of every scene.
    dir.write(
        "definitions/chip.json",
        &definition(
            "chip",
            json!([]),
            vec![def_rect("body", "chip", 0, 40.0, 40.0)],
        )
        .to_string(),
    );
    for scene_id in ["a", "b"] {
        let output = run_vectr(
            dir.path(),
            &["compile", scene_id, "--out", &format!("{scene_id}.json")],
        );
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        assert_eq!(
            compiled_rect_width(&dir.path().join(format!("{scene_id}.json"))),
            40.0,
            "scene `{scene_id}` reflects the shared edit on its next compile"
        );
    }
}

/// A project definition's instance identity survives into exported SVG as a
/// named group wrapping the definition's elements (C-003, FEAT-012).
#[test]
fn the_exported_svg_carries_the_instance_identity_as_a_named_group() {
    let dir = project(
        "definition-svg",
        &[(
            "chip",
            definition(
                "chip",
                json!([]),
                vec![def_rect("body", "chip", 0, 10.0, 10.0)],
            ),
        )],
        &[("a", {
            let mut document = placing_scene("a", "chip");
            document["elements"][0]["name"] = json!("Placed");
            document
        })],
    );

    let output = run_vectr(
        dir.path(),
        &["export", "a", "--format", "svg", "--out", "a.svg"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let svg = std::fs::read_to_string(dir.path().join("a.svg")).expect("the svg");
    assert!(
        svg.contains("<g id=\"i1\" data-name=\"Placed\">"),
        "the instance's identity is a named group: {svg}"
    );
    assert!(svg.contains("<rect"), "{svg}");
}

/// A definition cycle introduced into a project fails every scene that places
/// it, not only the scene being compiled.
#[test]
fn a_project_definition_cycle_fails_every_scene_that_places_it() {
    let dir = project(
        "definition-cycle-project",
        &[
            (
                "a",
                definition("a", json!([]), vec![def_instance("pa", "a", 0, "b")]),
            ),
            (
                "b",
                definition("b", json!([]), vec![def_instance("pb", "b", 0, "a")]),
            ),
        ],
        &[
            ("one", placing_scene("one", "a")),
            ("two", placing_scene("two", "a")),
        ],
    );

    for scene_id in ["one", "two"] {
        let output = run_vectr(dir.path(), &["compile", scene_id, "--check"]);
        assert_eq!(code(&output), 3, "{}", stderr(&output));
        let err = stderr(&output);
        assert!(err.contains("E_DEFINITION_CYCLE"), "{err}");
    }
}
