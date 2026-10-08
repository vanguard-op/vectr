//! Acceptance tests for constraint and geometry resolution (FEAT-004).
//!
//! Verifies that stated relationships resolve into concrete geometry — equal
//! spacing, attachment, and alignment — that an unsatisfiable system names its
//! conflict, and that resolution is deterministic.

mod common;

use common::*;
use serde_json::json;
use vectr_core::Shape;

/// A rect placed by its transform, so the constraint solver's resolved
/// translation is the element's position (geometry sits at the local origin).
fn placed(id: &str, order: i64, x: f64, y: f64, width: f64, height: f64) -> serde_json::Value {
    let mut value = rect(id, order, 0.0, 0.0, width, height);
    value["transform"]["translateX"] = json!(x);
    value["transform"]["translateY"] = json!(y);
    value
}

#[test]
fn equal_spacing_makes_the_gaps_equal_within_tolerance() {
    let document = scene_with(
        vec![
            placed("a", 0, 0.0, 0.0, 10.0, 10.0),
            placed("b", 1, 50.0, 0.0, 10.0, 10.0),
            placed("c", 2, 200.0, 0.0, 10.0, 10.0),
        ],
        Some(json!([{
            "id": "sp", "sceneId": "s", "kind": "equalSpacing",
            "elementIds": ["a", "b", "c"], "axis": "x"
        }])),
        None,
    );

    let model = compile_doc(&document);
    let origin = |id: &str| model.node(id).expect("a node").transform.apply([0.0, 0.0])[0];
    let (a, b, c) = (origin("a"), origin("b"), origin("c"));

    assert!(close(a, 0.0), "the first element stays put: {a}");
    assert!(close(c, 200.0), "the last element stays put: {c}");
    let first_gap = b - (a + 10.0);
    let second_gap = c - (b + 10.0);
    assert!(
        close(first_gap, second_gap),
        "gaps are equal: {first_gap} vs {second_gap}"
    );
}

#[test]
fn a_connector_attached_to_two_anchors_meets_both_resolved_anchor_points() {
    let document = scene_with(
        vec![
            line("wire", 0, json!([[0.0, 0.0], [10.0, 0.0]])),
            rect("left", 1, 0.0, 0.0, 20.0, 20.0),
            rect("right", 2, 100.0, 40.0, 20.0, 20.0),
        ],
        Some(json!([{
            "id": "link", "sceneId": "s", "kind": "attach",
            "elementIds": ["wire", "left", "right"]
        }])),
        None,
    );

    let model = compile_doc(&document);
    let geometry = model
        .node("wire")
        .expect("the connector")
        .geometry
        .as_ref()
        .expect("connector geometry");
    let Shape::Line(line) = geometry else {
        panic!("the connector is a line: {geometry:?}");
    };
    assert_eq!(line.points.len(), 2);
    assert!(
        close(line.points[0][0], 10.0) && close(line.points[0][1], 10.0),
        "the connector starts at the left anchor's centre: {:?}",
        line.points[0]
    );
    assert!(
        close(line.points[1][0], 110.0) && close(line.points[1][1], 50.0),
        "the connector ends at the right anchor's centre: {:?}",
        line.points[1]
    );
}

#[test]
fn an_unsatisfiable_constraint_fails_naming_the_conflicting_constraint() {
    let document = scene_with(
        vec![
            rect("a", 0, 0.0, 0.0, 10.0, 10.0),
            rect("b", 1, 50.0, 0.0, 10.0, 10.0),
        ],
        Some(json!([
            { "id": "c1", "sceneId": "s", "kind": "align", "elementIds": ["a", "b"], "axis": "x", "value": 0.0 },
            { "id": "c2", "sceneId": "s", "kind": "align", "elementIds": ["a", "b"], "axis": "x", "value": 50.0 }
        ])),
        None,
    );

    let scene = parse_scene(&document);
    let diagnostics = vectr_core::compile(&scene).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, vectr_core::constraints::CONFLICT);
    assert!(
        error.message.contains("c1") && error.message.contains("c2"),
        "names both constraints: {}",
        error.message
    );
}

#[test]
fn an_under_constrained_system_gets_a_deterministic_default_placement() {
    let document = scene_with(
        vec![
            placed("a", 0, 0.0, 0.0, 10.0, 10.0),
            placed("b", 1, 50.0, 0.0, 10.0, 10.0),
        ],
        Some(json!([{
            "id": "align", "sceneId": "s", "kind": "align",
            "elementIds": ["a", "b"], "axis": "x"
        }])),
        None,
    );

    let first = compile_doc(&document);
    let second = compile_doc(&document);
    assert_eq!(first, second, "the default placement is deterministic");

    // `align` without a value aligns to the first element's centre.
    let center = |id: &str| first.node(id).expect("a node").transform.apply([5.0, 5.0]);
    assert!(
        close(center("a")[0], center("b")[0]),
        "both centres align on x: {:?} vs {:?}",
        center("a"),
        center("b")
    );
}

#[test]
fn resolution_is_stable_across_repeated_runs() {
    let document = scene_with(
        vec![
            placed("a", 0, 0.0, 0.0, 10.0, 10.0),
            placed("b", 1, 50.0, 0.0, 10.0, 10.0),
            placed("c", 2, 200.0, 0.0, 10.0, 10.0),
        ],
        Some(json!([{
            "id": "sp", "sceneId": "s", "kind": "equalSpacing",
            "elementIds": ["a", "b", "c"], "axis": "x"
        }])),
        None,
    );

    let first = compile_doc(&document)
        .to_json_string()
        .expect("serializable");
    let second = compile_doc(&document)
        .to_json_string()
        .expect("serializable");
    assert_eq!(
        first, second,
        "identical input yields identical geometry (NFR-010)"
    );
}

#[test]
fn a_constraint_naming_an_unknown_element_is_refused_naming_it() {
    let document = scene_with(
        vec![
            rect("a", 0, 0.0, 0.0, 10.0, 10.0),
            rect("b", 1, 50.0, 0.0, 10.0, 10.0),
        ],
        Some(json!([{
            "id": "align", "sceneId": "s", "kind": "align",
            "elementIds": ["a", "ghost"], "axis": "x"
        }])),
        None,
    );

    let scene = parse_scene(&document);
    let diagnostics = vectr_core::compile(&scene).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, vectr_core::constraints::CONSTRAINT);
    assert!(error.message.contains("ghost"), "{}", error.message);
}
