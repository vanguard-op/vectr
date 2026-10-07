//! Named color tokens that elements reference by name (FEAT-005).
//!
//! A palette holds color tokens; an element's `fillToken` names one. Resolving
//! the name yields the token's value, so editing one token restyles every
//! element that references it on the next recompile (FEAT-005).
//!
//! Two tokens may share a name: the later definition wins and a warning is
//! raised. A referenced name that no token defines is an error naming the
//! token, never a silent fallback color (FEAT-005).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{parse_document, to_json, REDEFINED_TOKEN, UNDEFINED_TOKEN, UNUSED_TOKEN};
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, Location, Scene};

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

/// Checks a palette on its own: every redefinition is a warning.
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
    diagnostics
}

/// Checks a scene's references against a palette.
///
/// A referenced token no definition provides is an error naming the token and
/// the element; a token nothing references is a warning. Redefinitions are
/// reported as by [`validate`] (FEAT-005).
pub fn validate_usage(scene: &Scene, palette: &Palette) -> Diagnostics {
    let mut diagnostics = validate(palette);

    let used: HashSet<&str> = scene
        .elements
        .iter()
        .filter_map(|element| element.fill_token.as_deref())
        .collect();

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
        let Some(token) = element.fill_token.as_deref() else {
            continue;
        };
        if palette.resolve(token).is_none() {
            diagnostics.push(
                Diagnostic::error(
                    UNDEFINED_TOKEN,
                    format!(
                        "element `{}` references undefined palette token `{token}`",
                        element.id
                    ),
                )
                .with_location(Location::element_at(
                    element.id.clone(),
                    format!("/elements/{index}/fillToken"),
                )),
            );
        }
    }

    diagnostics
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
    fn an_undefined_token_is_an_error_naming_the_token() {
        let scene = crate::scene::parse(
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.1","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"fillToken":"missing","opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let palette = parse(&palette()).unwrap();

        let diagnostics = validate_usage(&scene, &palette);
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
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.1","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"fillToken":"accent","opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let palette = parse(&palette()).unwrap();

        let diagnostics = validate_usage(&scene, &palette);
        assert!(!diagnostics.has_errors());
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, UNUSED_TOKEN);
        assert!(warning.message.contains("ink"));
    }
}
