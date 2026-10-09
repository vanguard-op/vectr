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

pub mod gradient;
pub mod palette;
pub mod recipe;
pub mod stroke;

pub use gradient::{
    parse as parse_gradient, resolve as resolve_gradient, validate as validate_gradient,
    validate_usage as validate_gradient_usage, Gradient, GradientStop, GradientType, Spread,
    GRADIENT, UNDEFINED_GRADIENT, UNUSED_GRADIENT,
};
pub use palette::{Palette, PaletteToken};
pub use recipe::{
    LightDirection, RecipeName, RecipeParameters, Shading, StyleRecipe, SHADING_UNSUPPORTED,
    TEXTURE_UNSUPPORTED,
};
pub use recipe::{FREEFORM_CURVE, GRID_SNAPPED, GRID_TOO_FINE, ISOMETRIC_OFF_AXIS, MIN_GRID_SIZE};
pub use recipe::{LINE_ART_EMPTY, MIN_STROKE_WEIGHT, STROKE_WEIGHT_CLAMPED};
pub use stroke::{ResolvedStroke, StrokeCap, StrokeJoin, StrokeProfile};

pub use palette::validate_usage as validate_palette_usage;
pub use palette::{parse as parse_palette, validate as validate_palette};
pub use recipe::{
    check_expressible as check_recipe_expressible, check_grid as check_recipe_grid,
    check_line_art as check_recipe_line_art, parse as parse_style_recipe,
    validate as validate_style_recipe,
};
pub use stroke::{parse as parse_stroke_profile, validate as validate_stroke_profile};

use crate::render::Paint;
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, Element, Location, PaintKind};

/// An element references a palette token no definition provides.
pub const UNDEFINED_TOKEN: DiagnosticCode = DiagnosticCode::new("E_UNDEFINED_TOKEN");

/// An element references a stroke profile that does not exist.
pub const UNDEFINED_STROKE: DiagnosticCode = DiagnosticCode::new("E_UNDEFINED_STROKE");

/// A palette defines the same token name more than once.
pub const REDEFINED_TOKEN: DiagnosticCode = DiagnosticCode::new("W_REDEFINED_TOKEN");

/// A palette token nothing references.
pub const UNUSED_TOKEN: DiagnosticCode = DiagnosticCode::new("W_UNUSED_TOKEN");

/// Resolves an element's fill against a palette and gradients.
///
/// Returns the resolved paint, or `None` when the element declares no fill. A
/// token paint resolves to a concrete colour; a gradient paint resolves to a
/// gradient whose stops are concrete colours. A reference that does not resolve
/// — an undefined token, or a gradient that is not present — is a located error
/// and resolves to nothing, never a fallback (FEAT-005, FEAT-027).
///
/// Without a palette a token paint carries its declared name through unchanged,
/// mirroring the no-style compilation path; a gradient cannot be resolved
/// without its definition and is dropped with a warning (C-002).
pub fn resolve_fill(
    element: &Element,
    palette: Option<&Palette>,
    gradients: &[Gradient],
    diagnostics: &mut Diagnostics,
) -> Option<Paint> {
    resolve_paint(
        element,
        element.fill.as_ref()?.literal()?,
        palette,
        gradients,
        diagnostics,
        "/fill",
    )
}

/// Resolves an element's stroke geometry against the available profiles.
///
/// Returns the profile's settings, or `None` when the element declares no
/// stroke. When the element names a profile that is not present, an
/// [`UNDEFINED_STROKE`] error naming the id is recorded and `None` is returned.
pub fn resolve_stroke(
    element: &Element,
    profiles: &[StrokeProfile],
    diagnostics: &mut Diagnostics,
) -> Option<ResolvedStroke> {
    let id = element.stroke.as_ref()?.profile_id.as_str();
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
                .with_location(Location::element_at(
                    element.id.clone(),
                    "/stroke/profileId",
                )),
            );
            None
        }
    }
}

/// Resolves an element's stroke paint against a palette and gradients.
///
/// Returns the resolved paint, or `None` when the element declares no stroke or
/// its paint does not resolve. A missing reference is a located error naming it,
/// never a fallback (FEAT-005, FEAT-027).
pub fn resolve_stroke_paint(
    element: &Element,
    palette: Option<&Palette>,
    gradients: &[Gradient],
    diagnostics: &mut Diagnostics,
) -> Option<Paint> {
    let stroke = element.stroke.as_ref()?;
    resolve_paint(
        element,
        stroke.paint.literal()?,
        palette,
        gradients,
        diagnostics,
        "/stroke/paint",
    )
}

/// Resolves one paint against a palette and gradients.
fn resolve_paint(
    element: &Element,
    paint: &crate::scene::Paint,
    palette: Option<&Palette>,
    gradients: &[Gradient],
    diagnostics: &mut Diagnostics,
    path: &str,
) -> Option<Paint> {
    match paint.kind {
        PaintKind::Token => match palette {
            Some(palette) => match palette.resolve(&paint.reference) {
                Some(value) => Some(Paint::Color {
                    value: value.to_string(),
                }),
                None => {
                    diagnostics.push(
                        Diagnostic::error(
                            UNDEFINED_TOKEN,
                            format!(
                                "element `{}` references undefined palette token `{}`",
                                element.id, paint.reference
                            ),
                        )
                        .with_location(Location::element_at(element.id.clone(), path.to_string())),
                    );
                    None
                }
            },
            // Without a palette the declared token name is carried through.
            None => Some(Paint::Color {
                value: paint.reference.clone(),
            }),
        },
        PaintKind::Gradient => {
            let Some(gradient) = gradients
                .iter()
                .find(|gradient| gradient.id == paint.reference)
            else {
                if gradients.is_empty() {
                    diagnostics.push(
                        Diagnostic::warning(
                            UNRESOLVED_GRADIENT,
                            format!(
                                "gradient `{}` for element `{}` was not resolved; compile with a style context to apply it",
                                paint.reference, element.id
                            ),
                        )
                        .with_location(Location::element_at(element.id.clone(), path.to_string())),
                    );
                } else {
                    diagnostics.push(
                        Diagnostic::error(
                            UNDEFINED_GRADIENT,
                            format!(
                                "element `{}` references undefined gradient `{}`",
                                element.id, paint.reference
                            ),
                        )
                        .with_location(Location::element_at(element.id.clone(), path.to_string())),
                    );
                }
                return None;
            };
            gradient::resolve(gradient, palette, diagnostics).map(Paint::Gradient)
        }
    }
}

/// A gradient reference could not be resolved for lack of style assets.
pub const UNRESOLVED_GRADIENT: DiagnosticCode = DiagnosticCode::new("W_UNRESOLVED_GRADIENT");

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
    use crate::render::Paint as ResolvedPaint;
    use crate::scene::parse as parse_scene;

    fn scene_with_paints(fill: &str, stroke: Option<&str>) -> crate::scene::Scene {
        let stroke = match stroke {
            Some(id) => format!(
                r#","stroke":{{"profileId":"{id}","paint":{{"kind":"token","ref":"accent"}}}}"#
            ),
            None => r#","stroke":null"#.to_string(),
        };
        parse_scene(&format!(
            r#"{{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{{"width":1,"height":1,"background":"transparent"}},"elements":[{{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{{"width":1,"height":1}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"fill":{{"kind":"token","ref":"{fill}"}}{stroke},"opacity":1,"visible":true}}]}}"#
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

    fn color(paint: &ResolvedPaint) -> Option<&str> {
        match paint {
            ResolvedPaint::Color { value } => Some(value),
            ResolvedPaint::Gradient(_) => None,
        }
    }

    #[test]
    fn a_changed_token_value_restyles_the_referencing_element() {
        let element = &scene_with_paints("accent", None).elements[0];
        let before = resolve_fill(element, Some(&palette()), &[], &mut Diagnostics::new());
        assert_eq!(before.as_ref().and_then(color), Some("#ff0000"));

        let mut changed = palette();
        changed.tokens[0].value = "#0000ff".to_string();
        let after = resolve_fill(element, Some(&changed), &[], &mut Diagnostics::new());
        assert_eq!(after.as_ref().and_then(color), Some("#0000ff"));
    }

    #[test]
    fn an_undefined_token_is_an_error_naming_the_token() {
        let element = &scene_with_paints("missing", None).elements[0];
        let mut diagnostics = Diagnostics::new();
        let fill = resolve_fill(element, Some(&palette()), &[], &mut diagnostics);
        assert!(fill.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_TOKEN);
        assert!(error.message.contains("missing"));
    }

    #[test]
    fn an_element_without_a_fill_resolves_to_nothing() {
        let scene = parse_scene(
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let element = &scene.elements[0];
        assert_eq!(element.fill, None);
        assert!(resolve_fill(element, Some(&palette()), &[], &mut Diagnostics::new()).is_none());
    }

    #[test]
    fn a_stroke_profile_is_shared_by_the_elements_that_reference_it() {
        let profiles = [profile()];
        let first = &scene_with_paints("accent", Some("stroke-1")).elements[0];
        let stroke = resolve_stroke(first, &profiles, &mut Diagnostics::new()).expect("resolved");
        assert_eq!(stroke.width, 3.0);
        assert_eq!(stroke.cap, StrokeCap::Butt);
        assert_eq!(stroke.join, StrokeJoin::Miter);
    }

    #[test]
    fn an_undefined_stroke_profile_is_an_error_naming_the_id() {
        let element = &scene_with_paints("accent", Some("missing")).elements[0];
        let mut diagnostics = Diagnostics::new();
        assert!(resolve_stroke(element, &[profile()], &mut diagnostics).is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_STROKE);
        assert!(error.message.contains("missing"));
    }

    #[test]
    fn a_stroke_takes_its_colour_from_the_token_it_names() {
        let element = &scene_with_paints("accent", Some("stroke-1")).elements[0];
        let paint = resolve_stroke_paint(element, Some(&palette()), &[], &mut Diagnostics::new());
        assert_eq!(paint.as_ref().and_then(color), Some("#ff0000"));
    }

    #[test]
    fn a_changed_token_value_restyles_the_stroke() {
        let element = &scene_with_paints("accent", Some("stroke-1")).elements[0];
        let mut changed = palette();
        changed.tokens[0].value = "#0000ff".to_string();
        let paint = resolve_stroke_paint(element, Some(&changed), &[], &mut Diagnostics::new());
        assert_eq!(paint.as_ref().and_then(color), Some("#0000ff"));
    }

    #[test]
    fn an_undefined_stroke_token_is_an_error_naming_the_token() {
        let mut scene = scene_with_paints("accent", Some("stroke-1"));
        scene.elements[0]
            .stroke
            .as_mut()
            .unwrap()
            .paint
            .literal_mut()
            .unwrap()
            .reference = "missing".to_string();
        let mut diagnostics = Diagnostics::new();
        let paint =
            resolve_stroke_paint(&scene.elements[0], Some(&palette()), &[], &mut diagnostics);
        assert!(paint.is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_TOKEN);
        assert!(error.message.contains("missing"));
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/stroke/paint")
        );
    }

    #[test]
    fn a_gradient_fill_resolves_to_concrete_stops() {
        let mut scene = scene_with_paints("accent", None);
        scene.elements[0].fill = Some(crate::scene::PaintValue::Paint(crate::scene::Paint {
            kind: PaintKind::Gradient,
            reference: "fade".to_string(),
        }));
        let gradient = gradient::parse(
            r##"{"id":"fade","projectId":"p","name":"F","type":"linear","stops":[{"offset":0,"token":"accent"},{"offset":1,"token":"accent"}]}"##,
        )
        .unwrap();
        let paint = resolve_fill(
            &scene.elements[0],
            Some(&palette()),
            std::slice::from_ref(&gradient),
            &mut Diagnostics::new(),
        )
        .expect("resolves");
        match paint {
            ResolvedPaint::Gradient(gradient) => {
                assert_eq!(gradient.stops.len(), 2);
                assert_eq!(gradient.stops[0].color, "#ff0000");
            }
            other => panic!("expected a gradient, got {other:?}"),
        }
    }
}
