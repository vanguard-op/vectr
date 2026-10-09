//! Acceptance tests for font handling (FEAT-024, NFR-040).
//!
//! Verifies that text renders with the bundled open-licensed default or a
//! user-supplied font, that output outlines glyphs so it does not depend on an
//! installed font, that a missing font is named, and that a missing glyph falls
//! back rather than drawing a blank box.

mod common;

use common::*;
use serde_json::json;
use vectr_core::{DiagnosticCode, FontAsset, StyleContext, FALLBACK_FONT_ID};

fn text_scene(font_id: Option<&str>, value: &str) -> serde_json::Value {
    let mut wordmark = text("wordmark", 0, 5.0, 45.0, value, 40.0);
    wordmark["name"] = json!("Wordmark");
    if let Some(font_id) = font_id {
        wordmark["fontId"] = json!(font_id);
    }
    let mut document = scene(vec![wordmark]);
    document["canvas"] = json!({ "width": 128.0, "height": 64.0, "background": "#ffffff" });
    document
}

fn default_font() -> FontAsset {
    FontAsset::new(
        vectr_core::DEFAULT_FONT_ID,
        "Inter",
        font_bytes("Inter.ttf"),
    )
}

fn fallback_font() -> FontAsset {
    FontAsset::new(FALLBACK_FONT_ID, "Noto Sans", font_bytes("NotoSans.ttf"))
}

#[test]
fn a_text_element_with_no_font_renders_with_the_open_licensed_default() {
    let document = text_scene(None, "Hi");
    let fonts = [default_font()];
    let style = StyleContext {
        palette: None,
        strokes: &[],
        gradients: &[],
        fonts: &fonts,
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let run = model
        .node("wordmark")
        .expect("the text node")
        .text
        .as_ref()
        .unwrap();
    assert_eq!(run.font_id, "default");
    let resolved = model
        .fonts
        .iter()
        .find(|font| font.id == "default")
        .expect("default font");
    assert_eq!(resolved.data, font_bytes("Inter.ttf"));

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("<path"), "{svg}");
    assert!(
        !svg.contains("<text"),
        "glyphs are outlined (FEAT-024): {svg}"
    );
}

#[test]
fn a_text_element_naming_a_user_supplied_font_uses_that_font() {
    let document = text_scene(Some("brand"), "Hi");
    let fonts = [
        default_font(),
        FontAsset::new("brand", "Brand", font_bytes("Inter.ttf")),
    ];
    let style = StyleContext {
        palette: None,
        strokes: &[],
        gradients: &[],
        fonts: &fonts,
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");

    let run = model
        .node("wordmark")
        .expect("the text node")
        .text
        .as_ref()
        .unwrap();
    assert_eq!(run.font_id, "brand");
    assert!(
        model.fonts.iter().any(|font| font.id == "brand"),
        "{:?}",
        model.fonts
    );
}

#[test]
fn exported_output_outlines_text_and_redistributes_no_font_file() {
    let document = text_scene(Some("brand"), "Hi");
    let fonts = [
        default_font(),
        FontAsset::new("brand", "Brand", font_bytes("Inter.ttf")),
    ];
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

    assert!(svg.contains("<path"), "{svg}");
    assert!(!svg.contains("<text"), "{svg}");
    assert!(!svg.contains("@font-face"), "no embedded font: {svg}");
    assert!(
        !svg.contains("font-family"),
        "output does not depend on an installed font: {svg}"
    );
    assert!(!svg.contains("data:font"), "{svg}");
}

#[test]
fn a_missing_font_is_an_error_naming_the_font() {
    let document = text_scene(Some("ghost"), "Hi");
    let fonts = [default_font()];
    let style = StyleContext {
        palette: None,
        strokes: &[],
        gradients: &[],
        fonts: &fonts,
        recipe: None,
        definitions: &[],
    };
    let diagnostics = compile_with(&document, &style).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, vectr_core::compiler::FONT);
    assert!(error.message.contains("ghost"), "{}", error.message);
}

#[test]
fn a_glyph_the_chosen_font_lacks_is_substituted_from_the_fallback() {
    // U+0149 (ŉ) is absent from Inter and present in Noto Sans.
    let document = text_scene(None, "\u{149}");
    let fonts = [default_font(), fallback_font()];
    let style = StyleContext {
        palette: None,
        strokes: &[],
        gradients: &[],
        fonts: &fonts,
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");
    assert!(
        model.fonts.iter().any(|font| font.id == FALLBACK_FONT_ID),
        "the fallback travels with the model: {:?}",
        model.fonts
    );

    let export = vectr_core::export_svg_reporting(&model, &Default::default()).expect("exports");
    assert!(export.svg.contains("<path"), "{}", export.svg);
    assert!(
        !export
            .diagnostics
            .warnings()
            .any(|warning| warning.code == vectr_core::fonts::MISSING_GLYPH),
        "the fallback covers the glyph: {:?}",
        export.diagnostics
    );
}

#[test]
fn a_glyph_no_available_font_covers_is_reported_rather_than_drawn_as_a_blank_box() {
    // U+10FFFF is a noncharacter no font carries.
    let document = text_scene(None, "\u{10FFFF}");
    let fonts = [default_font(), fallback_font()];
    let style = StyleContext {
        palette: None,
        strokes: &[],
        gradients: &[],
        fonts: &fonts,
        recipe: None,
        definitions: &[],
    };
    let model = compile_with(&document, &style).expect("compiles");
    let export = vectr_core::export_svg_reporting(&model, &Default::default()).expect("exports");
    assert!(
        export
            .diagnostics
            .warnings()
            .any(|warning| warning.code == vectr_core::fonts::MISSING_GLYPH),
        "an uncovered glyph is reported: {:?}",
        export.diagnostics
    );
}

#[test]
fn a_font_asset_referenced_by_a_non_text_element_is_refused_naming_the_element() {
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["fontId"] = json!("brand");
    let document = scene(vec![card]);

    let diagnostics = vectr_core::parse(&document.to_string()).expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, DiagnosticCode::SCHEMA);
    assert!(error.message.contains("card"), "{}", error.message);
    assert!(
        error.message.contains("not a text element"),
        "{}",
        error.message
    );
}
