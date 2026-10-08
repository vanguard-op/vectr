//! Acceptance tests for schema discovery (FEAT-017, C-004).
//!
//! Drives the built `vectr` binary: `vectr schema` prints the complete
//! machine-readable language contract, `--type` prints one type's properties
//! and allowed values, `--compact` serves the same contract minified for
//! machine consumption, and an unknown type is refused with the closest names.
//! The served contract declares the format version the tool itself accepts, so a
//! model that reads the schema authors against the build it has.

mod common;

use common::*;
use serde_json::{json, Map, Value};

/// Runs `vectr schema` with the given flags from a scratch directory.
fn schema(args: &[&str]) -> (i32, String, String) {
    let dir = TempDir::new("schema");
    let mut full = vec!["schema"];
    full.extend_from_slice(args);
    let output = run_vectr(dir.path(), &full);
    (code(&output), stdout(&output), stderr(&output))
}

#[test]
fn the_full_contract_is_printed_as_machine_readable_json() {
    let (code, out, err) = schema(&[]);
    assert_eq!(code, 0, "{err}");
    let contract: Value = serde_json::from_str(&out).expect("the whole contract is JSON");

    // The document is a scene contract with a definition for every entity the
    // language names, so a model can discover all of it from `$defs`.
    assert_eq!(contract["$ref"], "#/$defs/Scene");
    let definitions = contract["$defs"]
        .as_object()
        .expect("the contract carries $defs");
    for entity in [
        "Scene", "Element", "Canvas", "Transform", "Paint", "Stroke", "Palette", "StrokeProfile",
        "StyleRecipe", "Gradient", "Constraint", "Asset",
    ] {
        assert!(definitions.contains_key(entity), "missing `{entity}`");
    }

    // Every declared type resolves to a definition, so discovery is complete.
    let types = contract["x-vectr-types"]
        .as_array()
        .expect("the contract lists its types");
    assert!(!types.is_empty());
    for name in types {
        let name = name.as_str().expect("a type name");
        assert!(
            definitions.contains_key(name),
            "declared type `{name}` has no definition"
        );
    }
}

#[test]
fn a_single_type_request_prints_its_properties_and_allowed_values() {
    let (code, out, err) = schema(&["--type", "Element"]);
    assert_eq!(code, 0, "{err}");
    let element: Value = serde_json::from_str(&out).expect("a schema document");
    assert_eq!(element["title"], "Element");
    assert_eq!(element["$ref"], "#/$defs/Element");
    assert!(
        element["$defs"]["Element"]["properties"]["geometry"].is_object(),
        "the type's properties are present: {element}"
    );

    // The allowed values are discoverable: the element kinds and the paint kinds.
    let kinds: Vec<&str> = element["$defs"]["ElementKind"]["enum"]
        .as_array()
        .expect("an allowed-value enum")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(kinds.contains(&"rect") && kinds.contains(&"text"), "{kinds:?}");
    assert!(
        element["$defs"]["PaintKind"]["enum"].is_array(),
        "paint's allowed values are present"
    );

    // Type names are matched case-insensitively.
    let (code, lower, _) = schema(&["--type", "element"]);
    assert_eq!(code, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&lower).expect("JSON")["title"],
        "Element"
    );
}

#[test]
fn an_unknown_type_is_refused_with_the_closest_names() {
    let (code, _, err) = schema(&["--type", "Palete"]);
    assert_eq!(code, 2, "an unknown type is a usage-class failure");
    assert!(err.contains("E_SCHEMA_TYPE"), "{err}");
    assert!(err.contains("Palete"), "names the request: {err}");
    assert!(err.contains("Palette"), "lists the similar type: {err}");
}

#[test]
fn a_compact_form_is_available_and_carries_the_same_contract() {
    let (code, compact, err) = schema(&["--compact"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        !compact.trim().contains('\n'),
        "the compact form is minified for machine consumption"
    );

    let (_, full, _) = schema(&[]);
    // Same document, different whitespace: compare the parsed values.
    let compact_value: Value = serde_json::from_str(&compact).expect("compact JSON");
    let full_value: Value = serde_json::from_str(&full).expect("full JSON");
    assert_eq!(compact_value, full_value);
}

#[test]
fn a_minimal_scene_authored_from_the_contract_alone_validates() {
    let (_, out, _) = schema(&[]);
    let contract: Value = serde_json::from_str(&out).expect("JSON");
    let declared = contract["x-vectr-formatVersion"]
        .as_str()
        .expect("the contract declares its format version")
        .to_string();

    let required = |name: &str| -> Vec<String> {
        contract["$defs"][name]["required"]
            .as_array()
            .unwrap_or_else(|| panic!("`{name}` publishes its required fields"))
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    };
    let scene_required = required("Scene");
    let element_required = required("Element");
    let transform_required = required("Transform");
    for field in ["id", "projectId", "name", "formatVersion", "canvas"] {
        assert!(scene_required.iter().any(|f| f == field), "Scene requires `{field}`");
    }
    for field in [
        "id", "sceneId", "order", "kind", "geometry", "transform", "opacity", "visible",
    ] {
        assert!(
            element_required.iter().any(|f| f == field),
            "Element requires `{field}`"
        );
    }

    // The element's transform carries exactly the fields the contract requires.
    let mut transform = Map::new();
    for field in &transform_required {
        let value = match field.as_str() {
            "scaleX" | "scaleY" => json!(1),
            _ => json!(0),
        };
        transform.insert(field.clone(), value);
    }

    // The contract's own description states an ellipse is placed by x/y and
    // sized by width/height, so a model reading only the schema can size it.
    let geometry_description = contract["$defs"]["Geometry"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(
        geometry_description.contains("ellipse") && geometry_description.contains("width"),
        "the geometry description guides an ellipse's size: {geometry_description}"
    );

    // Built from the contract's own required lists, with no example in hand.
    let document = json!({
        "id": "s",
        "projectId": "p",
        "name": "Minimal",
        "formatVersion": declared,
        "canvas": { "width": 10, "height": 10, "background": "transparent" },
        "elements": [{
            "id": "e1",
            "sceneId": "s",
            "order": 0,
            "kind": "ellipse",
            "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "transform": Value::Object(transform),
            "opacity": 1,
            "visible": true
        }]
    });

    let dir = TempDir::new("schema-minimal");
    dir.write("vectr.project.json", "{}");
    dir.write("scenes/minimal.json", &document.to_string());
    let validate = run_vectr(dir.path(), &["validate", "scenes/minimal.json"]);
    assert_eq!(code(&validate), 0, "{}", stderr(&validate));
    let compile = run_vectr(dir.path(), &["compile", "scenes/minimal.json", "--check"]);
    assert_eq!(code(&compile), 0, "{}", stderr(&compile));
}

#[test]
fn the_served_contract_declares_the_version_the_tool_accepts() {
    let (_, out, _) = schema(&[]);
    let contract: Value = serde_json::from_str(&out).expect("JSON");
    let declared = contract["x-vectr-formatVersion"]
        .as_str()
        .expect("the contract declares its format version");
    assert_eq!(declared, VERSION, "the contract and the tool agree");

    // A scene at the declared version validates; one at another is refused with
    // the supported range, so a mismatch between a model's scene and the tool
    // surfaces rather than being worked around (FEAT-017 edge case).
    let dir = TempDir::new("schema-version");
    dir.write("vectr.project.json", "{}");

    let matching = scene(vec![rect("r1", 0, 0.0, 0.0, 10.0, 10.0)]);
    let matching_path = dir.write("scenes/ok.json", &matching.to_string());
    let output = run_vectr(
        dir.path(),
        &["validate", matching_path.to_str().expect("utf-8")],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let mut mismatched = scene(vec![rect("r1", 0, 0.0, 0.0, 10.0, 10.0)]);
    mismatched["formatVersion"] = serde_json::json!("0.1");
    dir.write("scenes/bad.json", &mismatched.to_string());
    let output = run_vectr(dir.path(), &["validate", "scenes/bad.json"]);
    assert_eq!(code(&output), 1);
    let err = stderr(&output);
    assert!(err.contains("E_FORMAT_VERSION"), "{err}");
    assert!(err.contains("0.1"), "names the version: {err}");
    assert!(err.contains(VERSION), "names the supported range: {err}");
}
