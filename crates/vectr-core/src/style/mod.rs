//! The style system: palettes, stroke profiles and style recipes, and applying
//! them to elements (FEAT-005).
//!
//! A scene keeps its structure and its look separate: elements reference a
//! palette token by name and a stroke profile by id, and a scene may select a
//! style recipe. Resolving those references is what lets a single value change
//! restyle every element that uses it (FEAT-005).
//!
//! Each entity is its own strict-JSON document, read with [`parse_palette`],
//! [`parse_stroke_profile`] and [`parse_style_recipe`] and checked with the
//! matching `validate_*` functions. A reference that does not resolve is an
//! error naming it, never a silent fallback; a token that is redefined or
//! unused is a warning.

pub mod palette;
pub mod recipe;
pub mod stroke;

pub use palette::{Palette, PaletteToken};
pub use recipe::{RecipeName, RecipeParameters, Shading, StyleRecipe};
pub use stroke::{ResolvedStroke, StrokeCap, StrokeJoin, StrokeProfile};

pub use palette::validate_usage as validate_palette_usage;
pub use palette::{parse as parse_palette, validate as validate_palette};
pub use recipe::{parse as parse_style_recipe, validate as validate_style_recipe};
pub use stroke::{parse as parse_stroke_profile, validate as validate_stroke_profile};

use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, Element, Location};

/// An element references a palette token no definition provides.
pub const UNDEFINED_TOKEN: DiagnosticCode = DiagnosticCode::new("E_UNDEFINED_TOKEN");

/// An element references a stroke profile that does not exist.
pub const UNDEFINED_STROKE: DiagnosticCode = DiagnosticCode::new("E_UNDEFINED_STROKE");

/// A palette defines the same token name more than once.
pub const REDEFINED_TOKEN: DiagnosticCode = DiagnosticCode::new("W_REDEFINED_TOKEN");

/// A palette token nothing references.
pub const UNUSED_TOKEN: DiagnosticCode = DiagnosticCode::new("W_UNUSED_TOKEN");

/// Resolves an element's fill against a palette.
///
/// Returns the resolved color, or `None` when the element declares no fill.
/// When the element names a token the palette does not define — including when
/// no palette is supplied — an [`UNDEFINED_TOKEN`] error naming the token is
/// recorded and `None` is returned (FEAT-005).
pub fn resolve_fill(
    element: &Element,
    palette: Option<&Palette>,
    diagnostics: &mut Diagnostics,
) -> Option<String> {
    let token = element.fill_token.as_deref()?;
    match palette.and_then(|palette| palette.resolve(token)) {
        Some(value) => Some(value.to_string()),
        None => {
            diagnostics.push(
                Diagnostic::error(
                    UNDEFINED_TOKEN,
                    format!(
                        "element `{}` references undefined palette token `{token}`",
                        element.id
                    ),
                )
                .with_location(Location::element_at(element.id.clone(), "/fillToken")),
            );
            None
        }
    }
}

/// Resolves an element's stroke against the available profiles.
///
/// Returns the profile's settings, or `None` when the element declares no
/// stroke. When the element names a profile that is not present, an
/// [`UNDEFINED_STROKE`] error naming the id is recorded and `None` is returned.
pub fn resolve_stroke(
    element: &Element,
    profiles: &[StrokeProfile],
    diagnostics: &mut Diagnostics,
) -> Option<ResolvedStroke> {
    let id = element.stroke_profile_id.as_deref()?;
    match profiles.iter().find(|profile| profile.id == id) {
        Some(profile) => Some(profile.resolved()),
        None => {
            diagnostics.push(
                Diagnostic::error(
                    UNDEFINED_STROKE,
                    format!(
                        "element `{}` references undefined stroke profile `{id}`",
                        element.id
                    ),
                )
                .with_location(Location::element_at(element.id.clone(), "/strokeProfileId")),
            );
            None
        }
    }
}

/// Resolves an element's stroke colour against a palette.
///
/// Returns the resolved colour, or `None` when the element declares no stroke
/// colour. A stroke carries its colour the same way a fill does, through a
/// palette token the element names; when the token is not defined — including
/// when no palette is supplied — an [`UNDEFINED_TOKEN`] error naming the token
/// is recorded and `None` is returned, never a fallback colour (FEAT-005).
pub fn resolve_stroke_color(
    element: &Element,
    palette: Option<&Palette>,
    diagnostics: &mut Diagnostics,
) -> Option<String> {
    let token = element.stroke_token.as_deref()?;
    match palette.and_then(|palette| palette.resolve(token)) {
        Some(value) => Some(value.to_string()),
        None => {
            diagnostics.push(
                Diagnostic::error(
                    UNDEFINED_TOKEN,
                    format!(
                        "element `{}` references undefined palette token `{token}`",
                        element.id
                    ),
                )
                .with_location(Location::element_at(element.id.clone(), "/strokeToken")),
            );
            None
        }
    }
}

/// Reads any style document's JSON, refusing a top level that is not an object.
pub(crate) fn parse_document<T>(source: &str, kind: &str) -> Result<T, Diagnostics>
where
    T: serde::de::DeserializeOwned,
{
    let value: serde_json::Value =
        serde_json::from_str(source).map_err(|error| from_serde(DiagnosticCode::PARSE, &error))?;
    if !value.is_object() {
        return Err(Diagnostics::from(Diagnostic::error(
            DiagnosticCode::PARSE,
            format!("not a {kind} document: the top level must be a JSON object"),
        )));
    }
    serde_json::from_str(source).map_err(|error| from_serde(DiagnosticCode::SCHEMA, &error))
}

/// Serializes a style document, reporting a serialization failure as an error.
pub(crate) fn to_json<T: serde::Serialize>(document: &T) -> Result<String, Diagnostics> {
    serde_json::to_string(document).map_err(|error| {
        Diagnostics::from(Diagnostic::error(
            DiagnosticCode::SCHEMA,
            format!("could not serialize the document: {error}"),
        ))
    })
}

fn from_serde(code: DiagnosticCode, error: &serde_json::Error) -> Diagnostics {
    // serde_json appends " at line N column M" to its Display; the position is
    // carried structurally instead, so drop the suffix from the message.
    let raw = error.to_string();
    let message = raw.split(" at line ").next().unwrap_or(&raw).to_string();

    let mut diagnostic = Diagnostic::error(code, message);
    if error.line() > 0 {
        diagnostic = diagnostic.with_location(Location::line_column(error.line(), error.column()));
    }
    Diagnostics::from(diagnostic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::parse as parse_scene;

    fn scene_with_fill(token: &str, profile: Option<&str>) -> crate::scene::Scene {
        let stroke = match profile {
            Some(id) => format!(r#","strokeProfileId":"{id}","strokeToken":"accent""#),
            None => r#","strokeProfileId":null"#.to_string(),
        };
        parse_scene(&format!(
            r#"{{"id":"s","projectId":"p","name":"S","formatVersion":"0.1","canvas":{{"width":1,"height":1,"background":"transparent"}},"elements":[{{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{{"width":1,"height":1}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"fillToken":"{token}"{stroke},"opacity":1,"visible":true}}]}}"#
        ))
        .unwrap()
    }

    fn palette() -> Palette {
        parse_palette(
            r##"{"id":"pal-1","projectId":"p","name":"P","tokens":[{"name":"accent","value":"#ff0000"}]}"##,
        )
        .unwrap()
    }

    fn profile() -> StrokeProfile {
        parse_stroke_profile(
            r#"{"id":"stroke-1","projectId":"p","name":"Outline","width":3,"cap":"butt","join":"miter"}"#,
        )
        .unwrap()
    }

    #[test]
    fn a_changed_token_value_restyles_the_referencing_element() {
        let element = &scene_with_fill("accent", None).elements[0];
        let before = resolve_fill(element, Some(&palette()), &mut Diagnostics::new());
        assert_eq!(before.as_deref(), Some("#ff0000"));

        let mut changed = palette();
        changed.tokens[0].value = "#0000ff".to_string();
        let after = resolve_fill(element, Some(&changed), &mut Diagnostics::new());
        assert_eq!(after.as_deref(), Some("#0000ff"));
    }

    #[test]
    fn an_undefined_token_is_an_error_naming_the_token() {
        let element = &scene_with_fill("missing", None).elements[0];
        let mut diagnostics = Diagnostics::new();
        let fill = resolve_fill(element, Some(&palette()), &mut diagnostics);
        assert!(fill.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_TOKEN);
        assert!(error.message.contains("missing"));
    }

    #[test]
    fn an_element_without_a_fill_resolves_to_nothing() {
        let scene = parse_scene(
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.1","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let element = &scene.elements[0];
        assert_eq!(element.fill_token, None);
        assert!(resolve_fill(element, Some(&palette()), &mut Diagnostics::new()).is_none());
    }

    #[test]
    fn a_stroke_profile_is_shared_by_the_elements_that_reference_it() {
        let profiles = [profile()];
        let first = &scene_with_fill("accent", Some("stroke-1")).elements[0];
        let stroke = resolve_stroke(first, &profiles, &mut Diagnostics::new()).expect("resolved");
        assert_eq!(stroke.width, 3.0);
        assert_eq!(stroke.cap, StrokeCap::Butt);
        assert_eq!(stroke.join, StrokeJoin::Miter);
    }

    #[test]
    fn an_undefined_stroke_profile_is_an_error_naming_the_id() {
        let element = &scene_with_fill("accent", Some("missing")).elements[0];
        let mut diagnostics = Diagnostics::new();
        assert!(resolve_stroke(element, &[profile()], &mut diagnostics).is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_STROKE);
        assert!(error.message.contains("missing"));
    }

    #[test]
    fn a_stroke_takes_its_colour_from_the_token_it_names() {
        let element = &scene_with_fill("accent", Some("stroke-1")).elements[0];
        let colour = resolve_stroke_color(element, Some(&palette()), &mut Diagnostics::new());
        assert_eq!(colour.as_deref(), Some("#ff0000"));
    }

    #[test]
    fn a_changed_token_value_restyles_the_stroke() {
        let element = &scene_with_fill("accent", Some("stroke-1")).elements[0];
        let mut changed = palette();
        changed.tokens[0].value = "#0000ff".to_string();
        let colour = resolve_stroke_color(element, Some(&changed), &mut Diagnostics::new());
        assert_eq!(colour.as_deref(), Some("#0000ff"));
    }

    #[test]
    fn an_undefined_stroke_token_is_an_error_naming_the_token() {
        let mut scene = scene_with_fill("accent", Some("stroke-1"));
        scene.elements[0].stroke_token = Some("missing".to_string());
        let mut diagnostics = Diagnostics::new();
        let colour = resolve_stroke_color(&scene.elements[0], Some(&palette()), &mut diagnostics);
        assert!(colour.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_TOKEN);
        assert!(error.message.contains("missing"));
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/strokeToken")
        );
    }

    #[test]
    fn an_element_without_a_stroke_token_resolves_to_no_colour() {
        let element = &scene_with_fill("accent", None).elements[0];
        assert_eq!(element.stroke_token, None);
        assert!(resolve_stroke_color(element, Some(&palette()), &mut Diagnostics::new()).is_none());
    }
}
