//! Named color tokens that elements reference by name (FEAT-005).
//!
//! A palette holds color tokens; an element's `fill` names one by paint.
//! Resolving the name yields the token's value, so editing one token restyles
//! every element that references it on the next recompile (FEAT-005).
//!
//! Two tokens may share a name: the later definition wins and a warning is
//! raised. A referenced name that no token defines is an error naming the
//! token, never a silent fallback color (FEAT-005).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{parse_document, to_json, Gradient, REDEFINED_TOKEN, UNDEFINED_TOKEN, UNUSED_TOKEN};
use crate::scene::{
    validate_color, Diagnostic, DiagnosticCode, Diagnostics, Element, Location, Paint, PaintKind,
    PaintValue, Scene,
};

/// A named set of color tokens that scenes reference by token name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Palette {
    /// Stable identifier for the palette.
    pub id: String,
    /// References the project this palette belongs to.
    pub project_id: String,
    /// Human-readable palette name.
    pub name: String,
    /// The color tokens in the palette.
    pub tokens: Vec<PaletteToken>,
}

/// A single named color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PaletteToken {
    /// Token name referenced by elements.
    pub name: String,
    /// Color value, such as a hex color.
    pub value: String,
    /// What the color is used for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Palette {
    /// Resolves a token name to its value.
    ///
    /// When a name is defined more than once the later definition wins, so the
    /// most recent value is the one returned.
    pub fn resolve(&self, name: &str) -> Option<&str> {
        self.tokens
            .iter()
            .rev()
            .find(|token| token.name == name)
            .map(|token| token.value.as_str())
    }

    /// The first definition of each distinct token name, with its position.
    pub fn first_definitions(&self) -> Vec<(usize, &PaletteToken)> {
        let mut seen = HashSet::new();
        self.tokens
            .iter()
            .enumerate()
            .filter_map(|(index, token)| seen.insert(token.name.as_str()).then_some((index, token)))
            .collect()
    }

    /// Each redefinition of an already-seen token name, with its position.
    pub fn redefinitions(&self) -> Vec<(usize, &PaletteToken)> {
        let mut seen = HashSet::new();
        self.tokens
            .iter()
            .enumerate()
            .filter_map(|(index, token)| {
                (!seen.insert(token.name.as_str())).then_some((index, token))
            })
            .collect()
    }

    /// Serializes the palette to compact JSON.
    pub fn to_json_string(&self) -> Result<String, Diagnostics> {
        to_json(self)
    }

    /// Serializes the palette to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String, Diagnostics> {
        serde_json::to_string_pretty(self).map_err(|error| {
            Diagnostics::from(Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!("could not serialize the palette: {error}"),
            ))
        })
    }
}

/// Reads a palette from a JSON document.
pub fn parse(source: &str) -> Result<Palette, Diagnostics> {
    parse_document(source, "palette")
}

/// Checks a palette on its own: every token value is a colour, and every
/// redefinition is a warning.
pub fn validate(palette: &Palette) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    for (index, token) in palette.redefinitions() {
        diagnostics.push(
            Diagnostic::warning(
                REDEFINED_TOKEN,
                format!(
                    "palette token `{}` is defined more than once; the later definition wins",
                    token.name
                ),
            )
            .at_path(format!("/tokens/{index}")),
        );
    }
    for (index, token) in palette.tokens.iter().enumerate() {
        validate_color(
            &mut diagnostics,
            &token.value,
            &format!("palette token `{}` value", token.name),
            format!("/tokens/{index}/value"),
        );
    }
    diagnostics
}

/// Checks a scene's references against a palette.
///
/// A referenced token no definition provides is an error naming the token and
/// the element or gradient; a token nothing references is a warning. Tokens
/// named by a gradient's stops count as used. Redefinitions are reported as by
/// [`validate`] (FEAT-005, FEAT-027).
pub fn validate_usage(scene: &Scene, palette: &Palette, gradients: &[Gradient]) -> Diagnostics {
    let mut diagnostics = validate(palette);

    let mut used: HashSet<&str> = HashSet::new();
    for element in &scene.elements {
        for paint in element_paints(element) {
            if paint.kind == PaintKind::Token {
                used.insert(paint.reference.as_str());
            }
        }
    }
    for gradient in gradients {
        for stop in &gradient.stops {
            used.insert(stop.token.as_str());
        }
    }

    for (index, token) in palette.first_definitions() {
        if !used.contains(token.name.as_str()) {
            diagnostics.push(
                Diagnostic::warning(
                    UNUSED_TOKEN,
                    format!("palette token `{}` is never referenced", token.name),
                )
                .at_path(format!("/tokens/{index}")),
            );
        }
    }

    for (index, element) in scene.elements.iter().enumerate() {
        for (paint, field) in [
            (element.fill.as_ref().and_then(PaintValue::literal), "fill"),
            (
                element
                    .stroke
                    .as_ref()
                    .and_then(|stroke| stroke.paint.literal()),
                "stroke/paint",
            ),
        ] {
            let Some(paint) = paint else {
                continue;
            };
            if paint.kind != PaintKind::Token {
                continue;
            }
            if palette.resolve(&paint.reference).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        UNDEFINED_TOKEN,
                        format!(
                            "element `{}` references undefined palette token `{}`",
                            element.id, paint.reference
                        ),
                    )
                    .with_location(Location::element_at(
                        element.id.clone(),
                        format!("/elements/{index}/{field}"),
                    )),
                );
            }
        }
    }

    for (index, gradient) in gradients.iter().enumerate() {
        for (stop_index, stop) in gradient.stops.iter().enumerate() {
            if palette.resolve(&stop.token).is_none() {
                diagnostics.push(
                    Diagnostic::error(
                        UNDEFINED_TOKEN,
                        format!(
                            "gradient `{}` references undefined palette token `{}`",
                            gradient.id, stop.token
                        ),
                    )
                    .at_path(format!("/gradients/{index}/stops/{stop_index}")),
                );
            }
        }
    }

    diagnostics
}

/// The paints an element declares, its fill and its stroke's paint.
fn element_paints(element: &Element) -> Vec<&Paint> {
    let mut paints = Vec::new();
    if let Some(fill) = element.fill.as_ref().and_then(PaintValue::literal) {
        paints.push(fill);
    }
    if let Some(paint) = element
        .stroke
        .as_ref()
        .and_then(|stroke| stroke.paint.literal())
    {
        paints.push(paint);
    }
    paints
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette() -> String {
        r##"{
  "id": "palette-1",
  "projectId": "project-1",
  "name": "Brand",
  "tokens": [
    { "name": "accent", "value": "#ff6600", "description": "Primary accent" },
    { "name": "ink", "value": "#111111" }
  ]
}"##
        .to_string()
    }

    #[test]
    fn parses_and_round_trips() {
        let palette = parse(&palette()).expect("valid palette");
        assert_eq!(palette.tokens.len(), 2);
        let reparsed = parse(&palette.to_json_string().unwrap()).expect("round-trips");
        assert_eq!(palette, reparsed);
    }

    #[test]
    fn the_later_definition_of_a_token_wins() {
        let source = palette().replace(
            r##"{ "name": "ink", "value": "#111111" }"##,
            r##"{ "name": "ink", "value": "#222222" }"##,
        );
        let palette = parse(&source).unwrap();
        assert_eq!(palette.resolve("ink"), Some("#222222"));
    }

    #[test]
    fn a_redefinition_is_a_warning() {
        let source = palette().replace(
            r##"{ "name": "ink", "value": "#111111" }"##,
            r##"{ "name": "accent", "value": "#222222" }"##,
        );
        let palette = parse(&source).unwrap();
        let diagnostics = validate(&palette);
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, REDEFINED_TOKEN);
        assert!(warning.message.contains("accent"));
    }

    #[test]
    fn a_token_value_that_is_not_a_colour_is_an_error_naming_the_token() {
        let source = palette().replace(r##""#ff6600""##, r##""not-a-colour""##);
        let palette = parse(&source).expect("parses");
        let diagnostics = validate(&palette);
        let error = diagnostics
            .errors()
            .find(|diagnostic| diagnostic.code == crate::scene::INVALID_COLOR)
            .expect("an invalid-colour error");
        assert!(error.message.contains("accent"), "{}", error.message);
        assert!(error.message.contains("not-a-colour"), "{}", error.message);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/tokens/0/value")
        );
    }

    #[test]
    fn a_colour_token_that_carries_alpha_is_accepted() {
        for value in [
            "#ff660080",
            "rgba(255, 102, 0, 0.5)",
            "hsla(24, 100%, 50%, 0.25)",
        ] {
            let source = palette().replace(r##""#ff6600""##, &format!("\"{value}\""));
            let palette = parse(&source).expect("parses");
            let diagnostics = validate(&palette);
            assert!(
                !diagnostics
                    .errors()
                    .any(|diagnostic| diagnostic.code == crate::scene::INVALID_COLOR),
                "expected {value:?} to be accepted: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn an_undefined_token_is_an_error_naming_the_token() {
        let scene = crate::scene::parse(
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"fill":{"kind":"token","ref":"missing"},"opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let palette = parse(&palette()).unwrap();

        let diagnostics = validate_usage(&scene, &palette, &[]);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_TOKEN);
        assert!(error.message.contains("missing"));
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.element_id.as_deref()),
            Some("e1")
        );
    }

    #[test]
    fn an_unused_token_is_a_warning_not_an_error() {
        let scene = crate::scene::parse(
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"fill":{"kind":"token","ref":"accent"},"opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let palette = parse(&palette()).unwrap();

        let diagnostics = validate_usage(&scene, &palette, &[]);
        assert!(!diagnostics.has_errors());
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, UNUSED_TOKEN);
        assert!(warning.message.contains("ink"));
    }

    #[test]
    fn a_referenced_stroke_token_counts_as_used() {
        let scene = crate::scene::parse(
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"stroke":{"profileId":"stroke-1","paint":{"kind":"token","ref":"ink"}},"opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let palette = parse(&palette()).unwrap();

        let diagnostics = validate_usage(&scene, &palette, &[]);
        assert!(!diagnostics.has_errors());
        let unused: Vec<&str> = diagnostics
            .warnings()
            .filter(|warning| warning.code == UNUSED_TOKEN)
            .map(|warning| warning.message.as_str())
            .collect();
        assert!(
            !unused.iter().any(|message| message.contains("ink")),
            "a stroke token is not unused: {unused:?}"
        );
    }

    #[test]
    fn an_undefined_stroke_token_is_an_error_naming_the_token() {
        let scene = crate::scene::parse(
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"stroke":{"profileId":"stroke-1","paint":{"kind":"token","ref":"missing"}},"opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let palette = parse(&palette()).unwrap();

        let diagnostics = validate_usage(&scene, &palette, &[]);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_TOKEN);
        assert!(error.message.contains("missing"));
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/elements/0/stroke/paint")
        );
    }

    #[test]
    fn a_gradient_stop_token_counts_as_used() {
        let scene = crate::scene::parse(
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let palette = parse(&palette()).unwrap();
        let gradient = super::super::parse_gradient(
            r##"{"id":"fade","projectId":"p","name":"F","type":"linear","stops":[{"offset":0,"token":"accent"},{"offset":1,"token":"ink"}]}"##,
        )
        .unwrap();

        let diagnostics = validate_usage(&scene, &palette, std::slice::from_ref(&gradient));
        assert!(!diagnostics.has_errors());
        assert!(
            !diagnostics
                .warnings()
                .any(|warning| warning.code == UNUSED_TOKEN),
            "gradient stop tokens are used: {diagnostics:?}"
        );
    }
}
