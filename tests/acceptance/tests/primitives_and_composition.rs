//! Acceptance tests for drawing primitives and composition (FEAT-002, FEAT-003).
//!
//! Covers the elemental shapes with their paint and paint order, text placement,
//! the composition primitives (repeat, boolean, group transform), and the edge
//! cases each feature names.

mod common;

use common::*;
use serde_json::json;
use vectr_core::style::{StrokeCap, StrokeJoin};
use vectr_core::{DiagnosticCode, ElementKind, Rect, Shape, StyleContext};

fn palette_and_stroke() -> (String, String) {
    (
        palette("brand", &[("accent", "#ff0000"), ("ink", "#000000")]),
        stroke_profile("outline", 3.0, "round", "bevel"),
    )
}

#[test]
fn a_rectangle_renders_at_its_position_and_size_with_fill_and_stroke() {
    let (palette_text, stroke_text) = palette_and_stroke();
    let brand = vectr_core::parse_palette(&palette_text).expect("a palette");
    let outline = vectr_core::parse_stroke_profile(&stroke_text).expect("a profile");

    let mut card = rect("card", 0, 10.0, 20.0, 30.0, 40.0);
    card["fill"] = token_paint("accent");
    card["stroke"] = stroke("outline", "accent");
    let document = scene_with(vec![card], None, Some("brand"));

    let style = StyleContext {
        palette: Some(&brand),
        strokes: std::slice::from_ref(&outline),
        gradients: &[],
        fonts: &[],
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let node = &model.nodes[0];
    assert_eq!(
        node.geometry,
        Some(Shape::Rect(Rect {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
            rx: 0.0,
            ry: 0.0,
        }))
    );
    assert_eq!(fill_color(node), Some("#ff0000"));
    let stroke = node.paint.stroke.as_ref().expect("a stroke");
    assert_eq!(color(&stroke.paint), Some("#ff0000"));
    assert_eq!(stroke.width, 3.0);
    assert_eq!(stroke.cap, StrokeCap::Round);
    assert_eq!(stroke.join, StrokeJoin::Bevel);

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(
        svg.contains("<rect x=\"10\" y=\"20\" width=\"30\" height=\"40\""),
        "{svg}"
    );
    assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
    assert!(svg.contains("stroke=\"#ff0000\""), "{svg}");
    assert!(svg.contains("stroke-width=\"3\""), "{svg}");
    assert!(svg.contains("stroke-linecap=\"round\""), "{svg}");
    assert!(svg.contains("stroke-linejoin=\"bevel\""), "{svg}");
}

#[test]
fn a_path_with_fill_and_stroke_renders_both_and_honours_cap_and_join() {
    let (palette_text, stroke_text) = palette_and_stroke();
    let brand = vectr_core::parse_palette(&palette_text).expect("a palette");
    let outline = vectr_core::parse_stroke_profile(&stroke_text).expect("a profile");

    let mut path = element("p1", 0, "path", json!({ "pathData": "M0 0 L10 0" }));
    path["fill"] = token_paint("accent");
    path["stroke"] = stroke("outline", "accent");
    let document = scene_with(vec![path], None, Some("brand"));

    let style = StyleContext {
        palette: Some(&brand),
        strokes: std::slice::from_ref(&outline),
        gradients: &[],
        fonts: &[],
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let node = &model.nodes[0];
    assert!(matches!(node.geometry, Some(Shape::Path(_))));
    assert_eq!(fill_color(node), Some("#ff0000"));
    let stroke = node.paint.stroke.as_ref().expect("a stroke");
    assert_eq!(stroke.width, 3.0);
    assert_eq!(stroke.cap, StrokeCap::Round);
    assert_eq!(stroke.join, StrokeJoin::Bevel);

    // The path is open, so its caps are applied at the open ends: the emitted
    // path is not closed and carries the round cap.
    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("d=\"M 0 0 L 10 0\""), "{svg}");
    assert!(
        !svg.contains("d=\"M 0 0 L 10 0 Z\""),
        "the open path is not closed: {svg}"
    );
    assert!(svg.contains("stroke-linecap=\"round\""), "{svg}");
}

#[test]
fn paint_order_follows_document_order() {
    let document = scene(vec![
        rect("under", 0, 0.0, 0.0, 100.0, 100.0),
        rect("over", 1, 50.0, 50.0, 100.0, 100.0),
    ]);

    let model = compile_doc(&document);
    let ids: Vec<&str> = model.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(ids, vec!["under", "over"]);

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    let under = svg.find("<g id=\"under\"").expect("the first shape");
    let over = svg.find("<g id=\"over\"").expect("the second shape");
    assert!(under < over, "document order is paint order: {svg}");
}

#[test]
fn a_text_element_renders_at_its_anchor_with_alignment_spacing_and_fill() {
    let brand =
        vectr_core::parse_palette(&palette("brand", &[("ink", "#123456")])).expect("a palette");

    let mut wordmark = text("wordmark", 0, 100.0, 50.0, "Hi", 24.0);
    wordmark["geometry"]["align"] = json!("center");
    wordmark["geometry"]["lineHeight"] = json!(30.0);
    wordmark["geometry"]["letterSpacing"] = json!(1.5);
    wordmark["fill"] = token_paint("ink");
    let document = scene_with(vec![wordmark], None, Some("brand"));

    let style = StyleContext {
        palette: Some(&brand),
        strokes: &[],
        gradients: &[],
        fonts: &[],
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let node = &model.nodes[0];
    assert_eq!(node.kind, "text");
    assert!(node.geometry.is_none(), "a text node carries no geometry");
    let run = node.text.as_ref().expect("a text run");
    assert_eq!(run.value, "Hi");
    assert_eq!(run.font_size, 24.0);
    assert_eq!(run.line_height, 30.0);
    assert_eq!(run.letter_spacing, 1.5);
    assert_eq!(run.align.as_str(), "center");
    assert_eq!(fill_color(node), Some("#123456"));
    // The anchor is baked into the resolved transform.
    let origin = node.transform.apply([0.0, 0.0]);
    assert!(
        close(origin[0], 100.0) && close(origin[1], 50.0),
        "{origin:?}"
    );
}

#[test]
fn a_rectangle_with_zero_extent_is_refused_naming_the_primitive() {
    let document = scene(vec![rect("card", 0, 0.0, 0.0, 0.0, 40.0)]);
    let scene = parse_scene(&document);
    let diagnostics = vectr_core::compile(&scene).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, vectr_core::primitives::PRIMITIVE);
    assert!(error.message.contains("card"), "{}", error.message);
}

#[test]
fn a_negative_dimension_is_refused_at_validation() {
    let document = scene(vec![rect("card", 0, 0.0, 0.0, -5.0, 40.0)]);
    let diagnostics = vectr_core::parse(&document.to_string()).expect_err("refused");
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
fn an_empty_path_renders_nothing_and_raises_a_warning() {
    let document = scene(vec![element(
        "p1",
        0,
        "path",
        json!({ "pathData": "M0 0" }),
    )]);
    let model = compile_doc(&document);
    assert!(model.nodes.is_empty(), "{:?}", model.nodes);
    assert!(
        model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == vectr_core::primitives::EMPTY_PATH),
        "{:?}",
        model.diagnostics
    );
}

#[test]
fn a_text_element_without_a_string_or_a_size_is_refused_naming_the_element() {
    let no_text = scene(vec![element("t1", 0, "text", json!({ "fontSize": 24.0 }))]);
    let diagnostics = vectr_core::parse(&no_text.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert!(error.message.contains("t1"), "{}", error.message);

    let no_size = scene(vec![element("t1", 0, "text", json!({ "text": "Hi" }))]);
    let diagnostics = vectr_core::parse(&no_size.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert!(error.message.contains("t1"), "{}", error.message);
}

#[test]
fn a_text_element_as_a_composition_operand_is_refused_naming_the_combination() {
    let boolean = element("b1", 0, "boolean", json!({ "operation": "union" }));
    let mut child = text("t1", 0, 0.0, 0.0, "Hi", 12.0);
    child["parentId"] = json!("b1");
    let document = scene(vec![boolean, child]);

    let diagnostics = vectr_core::parse(&document.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert!(
        error.message.contains("text element `t1`"),
        "{}",
        error.message
    );
    assert!(error.message.contains("boolean"), "{}", error.message);
}

#[test]
fn a_repeat_renders_n_copies_with_the_specified_spacing() {
    let mut repeat = element("row", 0, "repeat", json!({ "count": 3, "spacing": 25.0 }));
    repeat["name"] = json!("Row");
    let mut child = rect("cell", 0, 0.0, 0.0, 10.0, 10.0);
    child["parentId"] = json!("row");
    let document = scene(vec![repeat, child]);

    let model = compile_doc(&document);
    assert_eq!(model.nodes.len(), 3, "{:?}", model.nodes);
    let mut xs: Vec<f64> = model
        .nodes
        .iter()
        .map(|node| node.transform.apply([0.0, 0.0])[0])
        .collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(
        close(xs[0], 0.0) && close(xs[1], 25.0) && close(xs[2], 50.0),
        "{xs:?}"
    );
}

#[test]
fn a_boolean_subtract_removes_the_second_shape_from_the_first() {
    let boolean = element("cut", 0, "boolean", json!({ "operation": "subtract" }));
    let mut base = rect("base", 0, 0.0, 0.0, 100.0, 100.0);
    base["parentId"] = json!("cut");
    let mut bite = rect("bite", 1, 0.0, 0.0, 50.0, 100.0);
    bite["parentId"] = json!("cut");
    let document = scene(vec![boolean, base, bite]);

    let model = compile_doc(&document);
    assert_eq!(model.nodes.len(), 1, "{:?}", model.nodes);
    let geometry = model.nodes[0].geometry.as_ref().expect("resolved geometry");
    assert!(
        close(shape_area(geometry), 5_000.0),
        "A minus B leaves half of A: {}",
        shape_area(geometry)
    );
}

#[test]
fn a_group_rotation_rotates_every_child_about_the_group_origin() {
    let mut pivot = group("pivot", 0, Some("Pivot"));
    pivot["transform"] = json!({
        "translateX": 100.0, "translateY": 0.0, "rotate": 90.0, "scaleX": 1.0, "scaleY": 1.0
    });
    let mut first = rect("first", 0, 0.0, 0.0, 10.0, 10.0);
    first["parentId"] = json!("pivot");
    first["transform"]["translateX"] = json!(10.0);
    let mut second = rect("second", 1, 0.0, 0.0, 10.0, 10.0);
    second["parentId"] = json!("pivot");
    second["transform"]["translateY"] = json!(10.0);
    let document = scene(vec![pivot, first, second]);

    let model = compile_doc(&document);
    let first_origin = model
        .node("first")
        .expect("the first child")
        .transform
        .apply([0.0, 0.0]);
    let second_origin = model
        .node("second")
        .expect("the second child")
        .transform
        .apply([0.0, 0.0]);
    assert!(
        close(first_origin[0], 100.0) && close(first_origin[1], 10.0),
        "local (10,0) pivots about (100,0): {first_origin:?}"
    );
    assert!(
        close(second_origin[0], 90.0) && close(second_origin[1], 0.0),
        "local (0,10) pivots about (100,0): {second_origin:?}"
    );
}

#[test]
fn a_boolean_with_no_intersection_yields_an_empty_result_not_an_error() {
    let boolean = element("overlap", 0, "boolean", json!({ "operation": "intersect" }));
    let mut left = rect("left", 0, 0.0, 0.0, 10.0, 10.0);
    left["parentId"] = json!("overlap");
    let mut right = rect("right", 1, 100.0, 100.0, 10.0, 10.0);
    right["parentId"] = json!("overlap");
    let document = scene(vec![boolean, left, right]);

    let model = compile_doc(&document);
    assert!(!model.diagnostics.has_errors(), "{:?}", model.diagnostics);
    let visible_area: f64 = model
        .nodes
        .iter()
        .filter_map(|node| node.geometry.as_ref())
        .map(shape_area)
        .sum();
    assert!(
        visible_area < 1e-6,
        "disjoint intersect is empty: {visible_area}"
    );
}

#[test]
fn a_repeat_count_of_zero_renders_nothing_and_warns() {
    let repeat = element("row", 0, "repeat", json!({ "count": 0, "spacing": 10.0 }));
    let mut child = rect("cell", 0, 0.0, 0.0, 10.0, 10.0);
    child["parentId"] = json!("row");
    let document = scene(vec![repeat, child]);

    let model = compile_doc(&document);
    assert!(model.nodes.is_empty(), "{:?}", model.nodes);
    assert!(
        model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == vectr_core::composition::COUNT_ZERO),
        "{:?}",
        model.diagnostics
    );
}

#[test]
fn a_composition_that_would_expand_without_bound_reports_a_defined_size_limit() {
    // A repeat count above the compiler's node budget is refused before the
    // expansion is materialised, rather than allowed to grow without bound
    // (FEAT-011, NFR-021).
    let repeat = element(
        "row",
        0,
        "repeat",
        json!({ "count": vectr_core::compiler::MAX_RENDER_NODES as u64 + 1, "spacing": 1.0 }),
    );
    let mut child = rect("cell", 0, 0.0, 0.0, 10.0, 10.0);
    child["parentId"] = json!("row");
    let document = scene(vec![repeat, child]);

    let scene = parse_scene(&document);
    let diagnostics = vectr_core::compile(&scene).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, DiagnosticCode::SIZE_LIMIT);
    assert!(error.message.contains("limit"), "{}", error.message);
}

#[test]
fn an_invalid_transform_is_a_validation_error() {
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["transform"]["rotate"] = json!("sideways");
    let document = scene(vec![card]);

    let diagnostics = vectr_core::parse(&document.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, DiagnosticCode::SCHEMA);
    // The malformed transform is a located schema error: the offending value is
    // named in the message and positioned in the document.
    assert!(error.message.contains("sideways"), "{}", error.message);
    let location = error.location.as_ref().expect("a located error");
    assert!(
        location.line.is_some() || location.json_path.is_some(),
        "the malformed transform is located: {location:?}"
    );
}

#[test]
fn a_composition_lowers_to_concrete_geometry_with_no_unresolved_reference() {
    let mut repeat = element("row", 0, "repeat", json!({ "count": 2, "spacing": 20.0 }));
    repeat["fill"] = token_paint("accent");
    let mut child = rect("cell", 0, 0.0, 0.0, 10.0, 10.0);
    child["parentId"] = json!("row");
    child["fill"] = token_paint("accent");
    let document = scene_with(vec![repeat, child], None, Some("brand"));
    let brand =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#abcdef")])).expect("a palette");
    let style = StyleContext {
        palette: Some(&brand),
        strokes: &[],
        gradients: &[],
        fonts: &[],
        recipe: None,
        definitions: &[],
    };

    let model = compile_with(&document, &style).expect("compiles");
    assert_eq!(model.nodes.len(), 2);
    for node in &model.nodes {
        assert!(
            node.geometry.is_some(),
            "every node carries concrete geometry"
        );
        assert_eq!(fill_color(node), Some("#abcdef"));
        assert_eq!(node.kind, "rect");
        assert_ne!(
            node.kind, "repeat",
            "the composition itself is lowered away"
        );
    }
}

#[test]
fn every_primitive_kind_lowers_to_its_concrete_shape() {
    let document = scene(vec![
        rect("r", 0, 1.0, 2.0, 3.0, 4.0),
        element(
            "e",
            1,
            "ellipse",
            json!({ "x": 10.0, "y": 20.0, "width": 6.0, "height": 8.0 }),
        ),
        element(
            "g",
            2,
            "polygon",
            json!({ "points": [[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]] }),
        ),
        line("l", 3, json!([[0.0, 0.0], [10.0, 10.0]])),
        element("p", 4, "path", json!({ "pathData": "M0 0 L10 0" })),
    ]);

    let model = compile_doc(&document);
    let kinds: Vec<&str> = model
        .nodes
        .iter()
        .map(|node| node.geometry.as_ref().expect("geometry").kind())
        .collect();
    assert_eq!(kinds, vec!["rect", "ellipse", "polygon", "line", "path"]);
    assert_eq!(model.nodes[1].kind, ElementKind::Ellipse.as_str());
}
