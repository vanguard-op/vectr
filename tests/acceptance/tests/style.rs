//! Acceptance tests for the style and palette system (FEAT-005).
//!
//! Verifies that palette tokens and stroke profiles are defined once and shared,
//! that a token change restyles every referencing element in a single recompile,
//! and the system's failure states — an undefined token, a stroke missing half
//! its definition, a redefined token, and an unused token.

mod common;

use common::*;
use serde_json::json;
use vectr_core::style::{
    validate_palette, validate_palette_usage, StrokeCap, StrokeJoin, UNDEFINED_STROKE,
    UNDEFINED_TOKEN,
};
use vectr_core::{DiagnosticCode, StyleContext};

fn style_with(
    palette_text: &str,
    stroke_text: &str,
) -> (vectr_core::Palette, Vec<vectr_core::StrokeProfile>) {
    let palette = vectr_core::parse_palette(palette_text).expect("a palette");
    let stroke = vectr_core::parse_stroke_profile(stroke_text).expect("a profile");
    (palette, vec![stroke])
}

#[test]
fn changing_a_palette_token_restyles_every_referencing_element() {
    let mut first = rect("first", 0, 0.0, 0.0, 10.0, 10.0);
    first["fillToken"] = json!("accent");
    let mut second = rect("second", 1, 20.0, 0.0, 10.0, 10.0);
    second["fillToken"] = json!("accent");
    let document = scene_with(vec![first, second], None, Some("brand"));

    let before =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#111111")])).expect("a palette");
    let after =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#222222")])).expect("a palette");

    let before_style = StyleContext {
        palette: Some(&before),
        strokes: &[],
        fonts: &[],
    };
    let after_style = StyleContext {
        palette: Some(&after),
        strokes: &[],
        fonts: &[],
    };

    let before_model = compile_with(&document, &before_style).expect("compiles");
    let after_model = compile_with(&document, &after_style).expect("compiles");

    for node in &before_model.nodes {
        assert_eq!(node.paint.fill.as_deref(), Some("#111111"));
    }
    for node in &after_model.nodes {
        assert_eq!(node.paint.fill.as_deref(), Some("#222222"));
    }
}

#[test]
fn a_stroke_profile_is_shared_by_every_referencing_primitive() {
    let (palette, strokes) = style_with(
        &palette("brand", &[("accent", "#0000ff")]),
        &stroke_profile("outline", 4.0, "round", "bevel"),
    );
    let style = StyleContext {
        palette: Some(&palette),
        strokes: &strokes,
        fonts: &[],
    };

    let mut first = rect("first", 0, 0.0, 0.0, 10.0, 10.0);
    first["strokeProfileId"] = json!("outline");
    first["strokeToken"] = json!("accent");
    let mut second = element(
        "second",
        1,
        "ellipse",
        json!({ "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0 }),
    );
    second["strokeProfileId"] = json!("outline");
    second["strokeToken"] = json!("accent");
    let document = scene_with(vec![first, second], None, Some("brand"));

    let model = compile_with(&document, &style).expect("compiles");
    for node in &model.nodes {
        let stroke = node.paint.stroke.as_ref().expect("a shared stroke");
        assert_eq!(stroke.width, 4.0);
        assert_eq!(stroke.cap, StrokeCap::Round);
        assert_eq!(stroke.join, StrokeJoin::Bevel);
        assert_eq!(
            stroke.value, "#0000ff",
            "colour comes from the token it names"
        );
    }
}

#[test]
fn an_undefined_palette_token_is_an_error_naming_the_missing_token() {
    let palette =
        vectr_core::parse_palette(&palette("brand", &[("other", "#ffffff")])).expect("a palette");
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["fillToken"] = json!("accent");
    let document = scene_with(vec![card], None, Some("brand"));
    let style = StyleContext {
        palette: Some(&palette),
        strokes: &[],
        fonts: &[],
    };

    let diagnostics = compile_with(&document, &style).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, UNDEFINED_TOKEN);
    assert!(error.message.contains("accent"), "{}", error.message);
}

#[test]
fn a_stroke_profile_without_a_colour_token_is_refused() {
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["strokeProfileId"] = json!("outline");
    let document = scene(vec![card]);

    let diagnostics = vectr_core::parse(&document.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, DiagnosticCode::SCHEMA);
    assert!(
        error
            .location
            .as_ref()
            .and_then(|location| location.json_path.as_deref())
            .is_some_and(|path| path.ends_with("/strokeToken")),
        "{:?}",
        error.location
    );
}

#[test]
fn a_stroke_colour_token_without_a_profile_is_refused() {
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["strokeToken"] = json!("accent");
    let document = scene(vec![card]);

    let diagnostics = vectr_core::parse(&document.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, DiagnosticCode::SCHEMA);
    assert!(
        error
            .location
            .as_ref()
            .and_then(|location| location.json_path.as_deref())
            .is_some_and(|path| path.ends_with("/strokeProfileId")),
        "{:?}",
        error.location
    );
}

#[test]
fn an_undefined_stroke_profile_is_an_error_naming_it() {
    let palette =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#0000ff")])).expect("a palette");
    let outline =
        vectr_core::parse_stroke_profile(&stroke_profile("outline", 4.0, "round", "bevel"))
            .expect("a profile");
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["strokeProfileId"] = json!("ghost");
    card["strokeToken"] = json!("accent");
    let document = scene_with(vec![card], None, Some("brand"));
    let style = StyleContext {
        palette: Some(&palette),
        strokes: std::slice::from_ref(&outline),
        fonts: &[],
    };

    let diagnostics = compile_with(&document, &style).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, UNDEFINED_STROKE);
    assert!(error.message.contains("ghost"), "{}", error.message);
}

#[test]
fn a_token_redefined_mid_document_uses_the_later_value_and_warns() {
    let palette = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#111111"), ("accent", "#222222")],
    ))
    .expect("a palette");

    let warnings = validate_palette(&palette);
    assert!(
        warnings
            .warnings()
            .any(|warning| warning.code == vectr_core::style::REDEFINED_TOKEN),
        "a redefinition is a warning: {warnings:?}"
    );

    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["fillToken"] = json!("accent");
    let document = scene_with(vec![card], None, Some("brand"));
    let style = StyleContext {
        palette: Some(&palette),
        strokes: &[],
        fonts: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");
    assert_eq!(
        model.nodes[0].paint.fill.as_deref(),
        Some("#222222"),
        "the later definition wins"
    );
}

#[test]
fn an_unused_token_is_a_warning_not_an_error() {
    let palette = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#111111"), ("ink", "#000000")],
    ))
    .expect("a palette");
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["fillToken"] = json!("accent");
    let document = scene_with(vec![card], None, Some("brand"));
    let scene = parse_scene(&document);

    let findings = validate_palette_usage(&scene, &palette);
    assert!(
        !findings.has_errors(),
        "an unused token is not an error: {findings:?}"
    );
    assert!(
        findings
            .warnings()
            .any(|warning| warning.code == vectr_core::style::UNUSED_TOKEN
                && warning.message.contains("ink")),
        "{findings:?}"
    );
}
