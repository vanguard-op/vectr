//! Acceptance tests for gradient fills and the unified paint model (FEAT-027).
//!
//! Verifies that an element's fill or stroke can reference a named linear or
//! radial gradient whose stops resolve to the palette tokens they name, that a
//! token change restyles every referencing element in one recompile, and the
//! feature's failure states — a missing stop token, too few stops, and an unused
//! gradient.

mod common;

use common::*;
use serde_json::json;
use vectr_core::style::{
    validate_gradient, GRADIENT, UNDEFINED_GRADIENT, UNDEFINED_TOKEN, UNUSED_GRADIENT,
};
use vectr_core::{Gradient, StyleContext};

/// A gradient document with the given stops, as `(offset, token)` pairs.
fn gradient_document(id: &str, kind: &str, stops: &[(f64, &str)]) -> String {
    let stops: Vec<serde_json::Value> = stops
        .iter()
        .map(|(offset, token)| json!({ "offset": offset, "token": token }))
        .collect();
    json!({
        "id": id,
        "projectId": "p",
        "name": id,
        "type": kind,
        "stops": stops
    })
    .to_string()
}

fn parse_gradient(id: &str, kind: &str, stops: &[(f64, &str)]) -> Gradient {
    vectr_core::parse_gradient(&gradient_document(id, kind, stops)).expect("a gradient")
}

fn brand() -> vectr_core::Palette {
    vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#e94560"), ("ink", "#1a1a2e")],
    ))
    .expect("a palette")
}

#[test]
fn a_linear_gradient_fill_exports_stops_that_resolve_to_palette_tokens() {
    let mut card = rect("card", 0, 0.0, 0.0, 100.0, 100.0);
    card["fill"] = gradient_paint("fade");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette = brand();
    let gradients = [parse_gradient(
        "fade",
        "linear",
        &[(0.0, "accent"), (1.0, "ink")],
    )];
    let style = StyleContext {
        palette: Some(&palette),
        strokes: &[],
        gradients: &gradients,
        fonts: &[],
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let fill = model.nodes[0].paint.fill.as_ref().expect("a fill");
    let vectr_core::render::Paint::Gradient(gradient) = fill else {
        panic!("expected a gradient fill, got {fill:?}");
    };
    assert_eq!(gradient.stops[0].color, "#e94560");
    assert_eq!(gradient.stops[1].color, "#1a1a2e");

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("<linearGradient"), "{svg}");
    assert!(svg.contains("stop-color=\"#e94560\""), "{svg}");
    assert!(svg.contains("stop-color=\"#1a1a2e\""), "{svg}");
    assert!(svg.contains("fill=\"url(#gradient-1)\""), "{svg}");
}

#[test]
fn a_radial_gradient_stroke_is_painted_with_a_radial_gradient() {
    let outline =
        vectr_core::parse_stroke_profile(&stroke_profile("outline", 4.0, "round", "round"))
            .expect("a profile");
    let mut card = rect("card", 0, 0.0, 0.0, 100.0, 100.0);
    card["stroke"] = json!({ "profileId": "outline", "paint": gradient_paint("glow") });
    let document = scene_with(vec![card], None, Some("brand"));

    let palette = brand();
    let gradients = [parse_gradient(
        "glow",
        "radial",
        &[(0.0, "accent"), (1.0, "ink")],
    )];
    let style = StyleContext {
        palette: Some(&palette),
        strokes: std::slice::from_ref(&outline),
        gradients: &gradients,
        fonts: &[],
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let stroke = model.nodes[0].paint.stroke.as_ref().expect("a stroke");
    let vectr_core::render::Paint::Gradient(gradient) = &stroke.paint else {
        panic!("expected a gradient stroke");
    };
    assert_eq!(gradient.gradient_type, vectr_core::GradientType::Radial);

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("<radialGradient"), "{svg}");
    assert!(svg.contains("stroke=\"url(#gradient-1)\""), "{svg}");
    assert!(svg.contains("stroke-width=\"4\""), "{svg}");
}

#[test]
fn a_gradient_stop_naming_a_missing_token_is_an_error_naming_the_token() {
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["fill"] = gradient_paint("fade");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette = brand();
    let gradients = [parse_gradient(
        "fade",
        "linear",
        &[(0.0, "accent"), (1.0, "missing")],
    )];
    let style = StyleContext {
        palette: Some(&palette),
        strokes: &[],
        gradients: &gradients,
        fonts: &[],
        recipe: None,
        definitions: &[],
    };

    let diagnostics = compile_with(&document, &style).expect_err("refused");
    let error = diagnostics
        .errors()
        .find(|error| error.code == UNDEFINED_TOKEN)
        .expect("an undefined-token error");
    assert!(error.message.contains("missing"), "{}", error.message);
    assert!(error.message.contains("fade"), "{}", error.message);
}

#[test]
fn an_element_referencing_an_undefined_gradient_is_an_error_naming_it() {
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["fill"] = gradient_paint("ghost");
    let document = scene_with(vec![card], None, Some("brand"));

    // The project supplies a different gradient, so `ghost` is a missing
    // reference rather than a gradient merely left unresolved (C-002).
    let palette = brand();
    let gradients = [parse_gradient(
        "fade",
        "linear",
        &[(0.0, "accent"), (1.0, "ink")],
    )];
    let style = StyleContext {
        palette: Some(&palette),
        strokes: &[],
        gradients: &gradients,
        fonts: &[],
        recipe: None,
        definitions: &[],
    };

    let diagnostics = compile_with(&document, &style).expect_err("refused");
    let error = diagnostics
        .errors()
        .find(|error| error.code == UNDEFINED_GRADIENT)
        .expect("an undefined-gradient error");
    assert!(error.message.contains("ghost"), "{}", error.message);
}

#[test]
fn changing_a_token_restyles_every_element_using_the_gradient_in_one_recompile() {
    let mut first = rect("first", 0, 0.0, 0.0, 10.0, 10.0);
    first["fill"] = gradient_paint("fade");
    let mut second = rect("second", 1, 20.0, 0.0, 10.0, 10.0);
    second["fill"] = gradient_paint("fade");
    let document = scene_with(vec![first, second], None, Some("brand"));

    let gradients = [parse_gradient(
        "fade",
        "linear",
        &[(0.0, "accent"), (1.0, "ink")],
    )];
    let before = brand();
    let after = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#0000ff"), ("ink", "#1a1a2e")],
    ))
    .expect("a palette");

    let compile = |palette: &vectr_core::Palette| {
        let style = StyleContext {
            palette: Some(palette),
            strokes: &[],
            gradients: &gradients,
            fonts: &[],
            recipe: None,
            definitions: &[],
        };
        compile_with(&document, &style).expect("compiles")
    };

    for model in [compile(&before), compile(&after)] {
        assert_eq!(model.nodes.len(), 2);
    }
    let before_model = compile(&before);
    let after_model = compile(&after);
    for node in &before_model.nodes {
        let vectr_core::render::Paint::Gradient(gradient) =
            node.paint.fill.as_ref().expect("a fill")
        else {
            panic!("expected a gradient");
        };
        assert_eq!(gradient.stops[0].color, "#e94560");
    }
    for node in &after_model.nodes {
        let vectr_core::render::Paint::Gradient(gradient) =
            node.paint.fill.as_ref().expect("a fill")
        else {
            panic!("expected a gradient");
        };
        assert_eq!(gradient.stops[0].color, "#0000ff");
    }
}

#[test]
fn a_gradient_fill_rasterizes_its_stop_colours_in_png() {
    let mut card = rect("card", 0, 0.0, 0.0, 100.0, 100.0);
    card["fill"] = gradient_paint("fade");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette = brand();
    let gradients = [parse_gradient(
        "fade",
        "linear",
        &[(0.0, "accent"), (1.0, "ink")],
    )];
    let style = StyleContext {
        palette: Some(&palette),
        strokes: &[],
        gradients: &gradients,
        fonts: &[],
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let bytes =
        vectr_core::export_png(&model, &vectr_core::RasterOptions::default()).expect("exports");
    let image = tiny_skia::Pixmap::decode_png(&bytes).expect("a PNG");
    assert_eq!((image.width(), image.height()), (400, 400));

    // The default linear gradient runs left to right across the rect's bounding
    // box (0..100 in x), so sampling inside the rect shows the two ends differ:
    // the left approaches the `accent` stop, the right the `ink` stop.
    let left = image.pixel(10, 50).expect("in bounds").demultiply();
    let right = image.pixel(90, 50).expect("in bounds").demultiply();
    assert!(
        left.red() > right.red(),
        "the gradient shades between its stops: left {left:?}, right {right:?}"
    );
}

#[test]
fn a_gradient_type_that_is_neither_linear_nor_radial_is_not_expressible() {
    let document = json!({
        "id": "fade",
        "projectId": "p",
        "name": "Fade",
        "type": "conic",
        "stops": [
            { "offset": 0, "token": "accent" },
            { "offset": 1, "token": "ink" }
        ]
    })
    .to_string();

    let diagnostics = vectr_core::parse_gradient(&document).expect_err("refused");
    assert!(
        diagnostics.has_errors(),
        "the language defines no conic or mesh gradient: {diagnostics:?}"
    );
}

#[test]
fn a_gradient_with_fewer_than_two_stops_is_an_error_naming_the_gradient() {
    let gradient = parse_gradient("fade", "linear", &[(0.0, "accent")]);
    let diagnostics = validate_gradient(&gradient);
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, GRADIENT);
    assert!(error.message.contains("fade"), "{}", error.message);
}

#[test]
fn an_unused_gradient_is_a_warning_not_an_error() {
    let document = scene(vec![rect("card", 0, 0.0, 0.0, 10.0, 10.0)]);
    let palette = brand();
    let gradients = [parse_gradient(
        "fade",
        "linear",
        &[(0.0, "accent"), (1.0, "ink")],
    )];
    let style = StyleContext {
        palette: Some(&palette),
        strokes: &[],
        gradients: &gradients,
        fonts: &[],
        recipe: None,
        definitions: &[],
    };

    let model = compile_with(&document, &style).expect("compiles");
    assert!(!model.diagnostics.has_errors());
    assert!(
        model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == UNUSED_GRADIENT && warning.message.contains("fade")),
        "{:?}",
        model.diagnostics
    );
}
