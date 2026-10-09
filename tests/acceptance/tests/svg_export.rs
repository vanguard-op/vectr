//! Acceptance tests for SVG export (FEAT-012, NFR-023).
//!
//! Verifies that named groups and their nesting survive, that shapes match the
//! render model, that text is outlined with its name and accessible text, that
//! export is deterministic, and the target's degradation behaviour.

mod common;

use common::*;
use serde_json::json;
use vectr_core::export::svg::{export_svg_reporting, UNSUPPORTED};
use vectr_core::render::{NodePaint, RenderCanvas, RenderMeta, RenderModel, ResolvedNode};
use vectr_core::{Affine, Diagnostics, FontAsset, Rect, Shape, StyleContext};

fn named_nested_groups() -> serde_json::Value {
    let mut inner = group("inner", 0, Some("Inner"));
    inner["parentId"] = json!("outer");
    let mut leaf = rect("leaf", 0, 0.0, 0.0, 10.0, 10.0);
    leaf["parentId"] = json!("inner");
    leaf["name"] = json!("Leaf");
    scene(vec![group("outer", 0, Some("Outer")), inner, leaf])
}

fn manual_model(nodes: Vec<ResolvedNode>) -> RenderModel {
    RenderModel {
        canvas: RenderCanvas {
            width: 100.0,
            height: 50.0,
            background: "#ffffff".to_string(),
        },
        nodes,
        meta: RenderMeta::default(),
        diagnostics: Diagnostics::new(),
        fonts: Vec::new(),
    }
}

fn plain_node(id: &str, kind: &str, geometry: Shape) -> ResolvedNode {
    ResolvedNode {
        id: id.to_string(),
        name: None,
        accessible_name: None,
        order: 0,
        kind: kind.to_string(),
        groups: Vec::new(),
        geometry: Some(geometry),
        text: None,
        transform: Affine::IDENTITY,
        paint: NodePaint::default(),
        opacity: 1.0,
        visible: true,
    }
}

#[test]
fn named_groups_survive_with_their_nesting_and_named_shapes_carry_their_names() {
    let model = compile_doc(&named_nested_groups());
    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");

    let outer = svg
        .find("<g id=\"outer\" data-name=\"Outer\">")
        .expect("outer group");
    let inner = svg
        .find("<g id=\"inner\" data-name=\"Inner\">")
        .expect("inner group");
    let leaf = svg
        .find("<g id=\"leaf\" data-name=\"Leaf\">")
        .expect("named shape");
    assert!(outer < inner && inner < leaf, "nesting is preserved: {svg}");
    assert_eq!(svg.matches("</g>").count(), 3, "{svg}");
}

#[test]
fn stroked_and_filled_shapes_match_the_render_model() {
    let brand =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#ff0000")])).expect("a palette");
    let outline =
        vectr_core::parse_stroke_profile(&stroke_profile("outline", 2.5, "round", "bevel"))
            .expect("a profile");
    let mut card = rect("card", 0, 1.0, 2.0, 30.0, 40.0);
    card["name"] = json!("Box");
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

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(
        svg.contains("<rect x=\"1\" y=\"2\" width=\"30\" height=\"40\""),
        "{svg}"
    );
    assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
    assert!(svg.contains("stroke=\"#ff0000\""), "{svg}");
    assert!(svg.contains("stroke-width=\"2.5\""), "{svg}");
    assert!(svg.contains("stroke-linecap=\"round\""), "{svg}");
    assert!(svg.contains("stroke-linejoin=\"bevel\""), "{svg}");
    assert!(svg.contains("data-name=\"Box\""), "{svg}");
}

#[test]
fn text_is_emitted_as_outlined_paths_with_its_name_and_accessible_text() {
    let mut wordmark = text("wordmark", 0, 10.0, 60.0, "Hi", 48.0);
    wordmark["name"] = json!("Wordmark");
    let document = scene(vec![wordmark]);
    let fonts = [FontAsset::new(
        vectr_core::DEFAULT_FONT_ID,
        "Inter",
        font_bytes("Inter.ttf"),
    )];
    let style = StyleContext {
        palette: None,
        strokes: &[],
        gradients: &[],
        fonts: &fonts,
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("<path"), "glyphs become paths: {svg}");
    assert!(
        !svg.contains("<text"),
        "text is outlined, not left font-dependent: {svg}"
    );
    assert!(
        svg.contains("id=\"wordmark\" data-name=\"Wordmark\""),
        "{svg}"
    );
    assert!(svg.contains("<title>Wordmark</title>"), "{svg}");
    assert!(svg.contains("<desc>Hi</desc>"), "{svg}");
}

#[test]
fn exporting_the_same_scene_twice_is_identical() {
    let model = compile_doc(&named_nested_groups());
    let first = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    let second = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert_eq!(
        first, second,
        "identical input yields identical SVG (NFR-010)"
    );
}

#[test]
fn a_feature_the_target_cannot_represent_is_omitted_with_a_warning() {
    let raster = plain_node(
        "r1",
        "raster",
        Shape::Rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            rx: 0.0,
            ry: 0.0,
        }),
    );
    let export =
        export_svg_reporting(&manual_model(vec![raster]), &Default::default()).expect("exports");
    assert!(
        !export.svg.contains("r1"),
        "the layer is omitted: {}",
        export.svg
    );
    assert_eq!(
        export
            .diagnostics
            .warnings()
            .next()
            .map(|warning| warning.code.clone()),
        Some(UNSUPPORTED)
    );
}

#[test]
fn coordinates_outside_the_usual_range_still_produce_a_valid_document() {
    let extreme = plain_node(
        "e1",
        "rect",
        Shape::Rect(Rect {
            x: 1e300,
            y: -1e300,
            width: 1e300,
            height: 1e300,
            rx: 0.0,
            ry: 0.0,
        }),
    );
    let svg =
        vectr_core::export_svg(&manual_model(vec![extreme]), &Default::default()).expect("exports");
    assert!(svg.contains("1e300"), "{svg}");
    assert!(!svg.contains("inf") && !svg.contains("NaN"), "{svg}");
    assert!(svg.starts_with("<?xml"), "{svg}");
    assert!(svg.trim_end().ends_with("</svg>"), "{svg}");
}

#[test]
fn emitted_svg_is_inert_with_no_script_or_event_handler() {
    let brand = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "\"><script>alert(1)</script>")],
    ))
    .expect("a palette");
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["name"] = json!("<script>alert(1)</script>");
    card["fill"] = token_paint("accent");
    let document = scene_with(vec![card], None, Some("brand"));
    let style = StyleContext {
        palette: Some(&brand),
        strokes: &[],
        gradients: &[],
        fonts: &[],
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(!svg.contains("<script>"), "no script element: {svg}");
    assert!(
        !svg.contains("onload=") && !svg.contains("onerror="),
        "{svg}"
    );
    assert!(
        svg.contains("&lt;script&gt;"),
        "the payload is escaped: {svg}"
    );
}
