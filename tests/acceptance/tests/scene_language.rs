//! Acceptance tests for the scene markup language (FEAT-001, C-001).
//!
//! Verifies that a scene parses to addressable, named elements; that it
//! round-trips without loss; that a text revision shows up as a diff on named
//! elements; and the language's failure states — unsupported format version,
//! duplicate identifier, and the empty document.

mod common;

use common::*;
use serde_json::json;
use vectr_core::{DiagnosticCode, ElementKind};

#[test]
fn every_element_is_addressable_by_a_stable_identifier() {
    let document = scene(vec![
        group("mark", 0, Some("Mark")),
        {
            let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
            card["parentId"] = json!("mark");
            card
        },
        {
            let mut dot = element(
                "dot",
                1,
                "ellipse",
                json!({ "x": 0.0, "y": 0.0, "width": 5.0, "height": 5.0 }),
            );
            dot["parentId"] = json!("mark");
            dot
        },
    ]);

    let parsed = parse_scene(&document);
    assert_eq!(
        parsed.element("mark").map(|e| e.kind),
        Some(ElementKind::Group)
    );
    assert_eq!(
        parsed.element("card").map(|e| e.kind),
        Some(ElementKind::Rect)
    );
    assert_eq!(
        parsed.element("dot").and_then(|e| e.parent_id.as_deref()),
        Some("mark")
    );
    assert!(parsed.element("absent").is_none());
}

#[test]
fn a_scene_round_trips_without_loss() {
    let document = scene(vec![
        group("mark", 0, Some("Mark")),
        {
            let mut card = rect("card", 0, 1.0, 2.0, 30.0, 40.0);
            card["parentId"] = json!("mark");
            card["fillToken"] = json!("accent");
            card
        },
        text("wordmark", 1, 5.0, 6.0, "Hi", 24.0),
    ]);

    let original = parse_scene(&document);
    let serialized = original.to_json_string().expect("serializable");
    let round_tripped = vectr_core::parse(&serialized).expect("the serialized scene is valid");
    assert_eq!(original, round_tripped);
}

#[test]
fn a_text_revision_diff_lands_on_the_named_element_not_an_opaque_blob() {
    let before = parse_scene(&scene(vec![rect("card", 0, 0.0, 0.0, 10.0, 10.0)]));

    let changed_document = scene(vec![rect("card", 0, 0.0, 0.0, 20.0, 10.0)]);
    let after = parse_scene(&changed_document);

    let before_text = before.to_json_pretty().expect("serializable");
    let after_text = after.to_json_pretty().expect("serializable");

    let before_lines: Vec<&str> = before_text.lines().collect();
    let after_lines: Vec<&str> = after_text.lines().collect();
    let differing: Vec<&str> = before_lines
        .iter()
        .zip(after_lines.iter())
        .filter(|(left, right)| left != right)
        .map(|(_, right)| *right)
        .collect();

    assert_eq!(differing.len(), 1, "one field changed: {differing:?}");
    assert!(
        differing[0].contains("\"width\""),
        "the changed line names the field: {}",
        differing[0]
    );
    assert!(
        after_text.contains("\"id\": \"card\""),
        "the element stays named and addressable: {after_text}"
    );
}

#[test]
fn an_unsupported_format_version_is_refused_naming_the_version_and_the_range() {
    let mut document = scene(vec![rect("e1", 0, 0.0, 0.0, 10.0, 10.0)]);
    document["formatVersion"] = json!("9.9");

    let diagnostics = vectr_core::parse(&document.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, DiagnosticCode::FORMAT_VERSION);
    assert!(error.message.contains("9.9"), "{}", error.message);
    assert!(
        error.message.contains("0.1")
            || error
                .message
                .contains(&vectr_core::scene::supported_range()),
        "names the supported range: {}",
        error.message
    );
}

#[test]
fn a_duplicate_element_identifier_is_refused_naming_the_duplicate() {
    let document = scene(vec![
        rect("card", 0, 0.0, 0.0, 10.0, 10.0),
        rect("card", 1, 20.0, 20.0, 10.0, 10.0),
    ]);

    let diagnostics = vectr_core::parse(&document.to_string()).expect_err("refused");
    let error = diagnostics
        .errors()
        .find(|error| error.code == DiagnosticCode::DUPLICATE_ID)
        .expect("a duplicate-id error");
    assert!(error.message.contains("card"), "{}", error.message);
}

#[test]
fn an_empty_document_compiles_to_an_empty_canvas() {
    let document = scene(Vec::new());
    let parsed = parse_scene(&document);
    assert!(parsed.elements.is_empty());

    let model = compile_doc(&document);
    assert!(model.nodes.is_empty());
    assert!(close(model.canvas.width, 400.0));
    assert_eq!(model.canvas.background, "#ffffff");

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("viewBox=\"0 0 400 400\""), "{svg}");
}
