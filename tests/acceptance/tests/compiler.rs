//! Acceptance tests for the scene compiler and its render model (FEAT-011, C-003).
//!
//! Verifies determinism, that an invalid scene fails naming its location, that
//! the model is fully resolved with named-group nesting and text runs intact,
//! and the compiler's failure states.

mod common;

use common::*;
use serde_json::json;
use vectr_core::{FontAsset, StyleContext};

#[test]
fn compiling_the_same_scene_twice_yields_an_identical_model() {
    let mut card = rect("card", 0, 10.0, 20.0, 30.0, 40.0);
    card["fillToken"] = json!("accent");
    let document = scene_with(
        vec![
            group("mark", 0, Some("Mark")),
            {
                card["parentId"] = json!("mark");
                card
            },
            text("wordmark", 1, 5.0, 6.0, "Hi", 24.0),
        ],
        None,
        Some("brand"),
    );
    let brand =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#ff0000")])).expect("a palette");
    let style = StyleContext {
        palette: Some(&brand),
        strokes: &[],
        fonts: &[],
    };

    let first = compile_with(&document, &style).expect("compiles");
    let second = compile_with(&document, &style).expect("compiles");
    assert_eq!(first, second, "the render model is deterministic (NFR-010)");
    assert_eq!(
        first.to_json_string().expect("serializable"),
        second.to_json_string().expect("serializable"),
    );
}

#[test]
fn an_invalid_scene_fails_naming_the_location_of_the_error() {
    let document = scene(vec![rect("card", 0, 0.0, 0.0, 10.0, 10.0)]);
    let mut scene = parse_scene(&document);
    scene.elements[0].opacity = 1.5;

    let diagnostics = vectr_core::compile(&scene).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(
        error
            .location
            .as_ref()
            .and_then(|location| location.json_path.as_deref()),
        Some("/elements/0/opacity")
    );
}

#[test]
fn a_circular_reference_is_an_error_naming_the_cycle() {
    let mut first = rect("a", 0, 0.0, 0.0, 10.0, 10.0);
    first["parentId"] = json!("b");
    let mut second = rect("b", 1, 0.0, 0.0, 10.0, 10.0);
    second["parentId"] = json!("a");
    let document = scene(vec![first, second]);

    let scene = parse_scene(&document);
    let diagnostics = vectr_core::compile(&scene).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, vectr_core::compiler::CYCLE);
    assert!(error.message.contains("circular"), "{}", error.message);
    assert!(
        error.message.contains("a") && error.message.contains("b"),
        "names the cycle: {}",
        error.message
    );
}

#[test]
fn the_render_model_retains_each_named_group_and_its_nesting() {
    let mut inner = group("inner", 0, Some("Inner"));
    inner["parentId"] = json!("outer");
    let mut leaf = rect("leaf", 0, 0.0, 0.0, 10.0, 10.0);
    leaf["parentId"] = json!("inner");
    leaf["name"] = json!("Leaf");
    let document = scene(vec![group("outer", 0, Some("Outer")), inner, leaf]);

    let model = compile_doc(&document);
    let node = model.node("leaf").expect("the leaf");
    let chain: Vec<(&str, Option<&str>)> = node
        .groups
        .iter()
        .map(|group| (group.id.as_str(), group.name.as_deref()))
        .collect();
    assert_eq!(
        chain,
        vec![("outer", Some("Outer")), ("inner", Some("Inner"))],
        "outermost first"
    );
    assert_eq!(node.name.as_deref(), Some("Leaf"));
}

#[test]
fn a_text_element_carries_its_string_resolved_font_and_layout() {
    let mut wordmark = text("wordmark", 0, 10.0, 60.0, "Hi", 48.0);
    wordmark["geometry"]["align"] = json!("start");
    let document = scene(vec![wordmark]);
    let fonts = [FontAsset::new(
        vectr_core::DEFAULT_FONT_ID,
        "Inter",
        font_bytes("Inter.ttf"),
    )];
    let style = StyleContext {
        palette: None,
        strokes: &[],
        fonts: &fonts,
    };

    let model = compile_with(&document, &style).expect("compiles");
    let node = model.node("wordmark").expect("the text node");
    assert!(node.geometry.is_none(), "a text node has no geometry");
    let run = node.text.as_ref().expect("a text run");
    assert_eq!(run.value, "Hi");
    assert_eq!(run.font_id, "default");
    assert_eq!(run.font_size, 48.0);
    assert_eq!(
        run.line_height, 48.0,
        "line height defaults to the font size"
    );
    assert_eq!(
        model
            .fonts
            .iter()
            .map(|font| font.id.as_str())
            .collect::<Vec<_>>(),
        vec!["default"],
        "the resolved font travels with the model"
    );
}

#[test]
fn an_unsupported_feature_is_refused_by_name() {
    let document = scene(vec![element("r1", 0, "raster", json!({}))]);
    let scene = parse_scene(&document);
    let diagnostics = vectr_core::compile(&scene).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, vectr_core::compiler::UNSUPPORTED);
    assert!(error.message.contains("raster"), "{}", error.message);
    assert!(error.message.contains("r1"), "{}", error.message);
}

#[test]
fn the_render_model_round_trips_through_its_json_representation() {
    let mut card = rect("card", 0, 1.0, 2.0, 30.0, 40.0);
    card["fillToken"] = json!("accent");
    let document = scene_with(vec![card], None, Some("brand"));
    let brand =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#ff0000")])).expect("a palette");
    let style = StyleContext {
        palette: Some(&brand),
        strokes: &[],
        fonts: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let text = model.to_json_string().expect("serializable");
    let reparsed = vectr_core::render::parse(&text).expect("deserializable");
    assert_eq!(model, reparsed);
    assert!(text.contains("\"fill\":\"#ff0000\""), "{text}");
}
