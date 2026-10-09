//! Named gradient paints that elements reference by paint (FEAT-027).
//!
//! A gradient is a named project definition that an element's fill or stroke
//! references by paint, so changing the gradient restyles every element that
//! uses it on the next recompile (FEAT-027). A gradient is linear or radial;
//! conic and mesh gradients are not part of the language. Each colour stop draws
//! its colour from a palette token, so every colour still lives in the palette
//! (FEAT-005, D-021).
//!
//! A gradient with fewer than two stops is an error naming the gradient; a stop
//! naming a token the palette does not define is an error naming the token,
//! never a silent fallback; an unused gradient is a warning.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::{parse_document, to_json, UNDEFINED_TOKEN};
use crate::render::{GradientPaint, ResolvedStop};
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, PaintKind, PaintValue, Scene};

/// A gradient is malformed: fewer than two stops, or an out-of-range value.
pub const GRADIENT: DiagnosticCode = DiagnosticCode::new("E_GRADIENT");

/// An element references a gradient that does not exist.
pub const UNDEFINED_GRADIENT: DiagnosticCode = DiagnosticCode::new("E_UNDEFINED_GRADIENT");

/// A gradient nothing references.
pub const UNUSED_GRADIENT: DiagnosticCode = DiagnosticCode::new("W_UNUSED_GRADIENT");

/// A named gradient paint that an element's fill or stroke can reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Gradient {
    /// Stable identifier for the gradient.
    pub id: String,
    /// References the project this gradient belongs to.
    pub project_id: String,
    /// Human-readable gradient name.
    pub name: String,
    /// The gradient's geometry: linear or radial.
    #[serde(rename = "type")]
    pub gradient_type: GradientType,
    /// The gradient's colour stops, ordered along the gradient vector.
    pub stops: Vec<GradientStop>,
    /// Start x of a linear gradient in object bounding-box units; defaults to 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x1: Option<f64>,
    /// Start y of a linear gradient in object bounding-box units; defaults to 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y1: Option<f64>,
    /// End x of a linear gradient in object bounding-box units; defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x2: Option<f64>,
    /// End y of a linear gradient in object bounding-box units; defaults to 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y2: Option<f64>,
    /// Center x of a radial gradient in object bounding-box units; defaults to 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cx: Option<f64>,
    /// Center y of a radial gradient in object bounding-box units; defaults to 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cy: Option<f64>,
    /// Radius of a radial gradient in object bounding-box units; defaults to 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<f64>,
    /// Focal x of a radial gradient in object bounding-box units; defaults to `cx`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<f64>,
    /// Focal y of a radial gradient in object bounding-box units; defaults to `cy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fy: Option<f64>,
    /// How the gradient extends beyond its ends; defaults to pad.
    #[serde(default, skip_serializing_if = "Spread::is_pad")]
    pub spread: Spread,
}

/// The geometry of a gradient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GradientType {
    /// Transitions along a straight line.
    Linear,
    /// Radiates from a centre.
    Radial,
}

/// How a gradient extends beyond its ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Spread {
    /// The end colours extend.
    #[default]
    Pad,
    /// The gradient reflects back and forth.
    Reflect,
    /// The gradient repeats.
    Repeat,
}

impl Spread {
    /// Whether the spread is the default `pad`, so it may be omitted.
    pub fn is_pad(&self) -> bool {
        *self == Spread::Pad
    }
}

/// A single colour stop, its colour drawn from a palette token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GradientStop {
    /// Position along the gradient vector, from 0 to 1.
    pub offset: f64,
    /// Name of the palette token supplying the stop's colour.
    pub token: String,
    /// Stop opacity, from 0 to 1; defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
}

impl Gradient {
    /// Serializes the gradient to compact JSON.
    pub fn to_json_string(&self) -> Result<String, Diagnostics> {
        to_json(self)
    }

    /// Serializes the gradient to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String, Diagnostics> {
        serde_json::to_string_pretty(self).map_err(|error| {
            Diagnostics::from(Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!("could not serialize the gradient: {error}"),
            ))
        })
    }
}

/// Reads a gradient from a JSON document.
pub fn parse(source: &str) -> Result<Gradient, Diagnostics> {
    parse_document(source, "gradient")
}

/// Checks a gradient on its own: at least two stops, and every value in range.
///
/// The colour a stop names is resolved against a palette at compile time, so a
/// missing token is reported there, naming the token (FEAT-027).
pub fn validate(gradient: &Gradient) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();

    if gradient.stops.len() < 2 {
        diagnostics.push(
            Diagnostic::error(
                GRADIENT,
                format!(
                    "gradient `{}` must declare at least two stops, found {}",
                    gradient.id,
                    gradient.stops.len()
                ),
            )
            .at_path("/stops"),
        );
    }

    for (index, stop) in gradient.stops.iter().enumerate() {
        if !stop.offset.is_finite() || !(0.0..=1.0).contains(&stop.offset) {
            diagnostics.push(
                Diagnostic::error(
                    GRADIENT,
                    format!(
                        "gradient `{}` stop {index} has an offset outside 0 to 1",
                        gradient.id
                    ),
                )
                .at_path(format!("/stops/{index}/offset")),
            );
        }
        if let Some(opacity) = stop.opacity {
            if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                diagnostics.push(
                    Diagnostic::error(
                        GRADIENT,
                        format!(
                            "gradient `{}` stop {index} has an opacity outside 0 to 1",
                            gradient.id
                        ),
                    )
                    .at_path(format!("/stops/{index}/opacity")),
                );
            }
        }
    }

    if let Some(r) = gradient.r {
        if !r.is_finite() || r < 0.0 {
            diagnostics.push(
                Diagnostic::error(
                    GRADIENT,
                    format!(
                        "gradient `{}` has a radius that is not zero or greater",
                        gradient.id
                    ),
                )
                .at_path("/r"),
            );
        }
    }

    for (field, value) in [
        ("x1", gradient.x1),
        ("y1", gradient.y1),
        ("x2", gradient.x2),
        ("y2", gradient.y2),
        ("cx", gradient.cx),
        ("cy", gradient.cy),
        ("fx", gradient.fx),
        ("fy", gradient.fy),
    ] {
        if let Some(value) = value {
            if !value.is_finite() {
                diagnostics.push(
                    Diagnostic::error(
                        GRADIENT,
                        format!("gradient `{}` has a non-finite `{field}`", gradient.id),
                    )
                    .at_path(format!("/{field}")),
                );
            }
        }
    }

    diagnostics
}

/// Resolves a gradient against a palette into concrete stop colours (C-002).
///
/// Every stop's token must resolve; a token that does not — including when no
/// palette is supplied — is an [`UNDEFINED_TOKEN`] error naming the token and
/// the gradient, and nothing is returned, never a fallback colour (FEAT-027).
pub fn resolve(
    gradient: &Gradient,
    palette: Option<&super::Palette>,
    diagnostics: &mut Diagnostics,
) -> Option<GradientPaint> {
    if gradient.stops.len() < 2 {
        diagnostics.push(Diagnostic::error(
            GRADIENT,
            format!(
                "gradient `{}` must declare at least two stops, found {}",
                gradient.id,
                gradient.stops.len()
            ),
        ));
        return None;
    }

    let mut stops: Vec<ResolvedStop> = Vec::with_capacity(gradient.stops.len());
    for stop in &gradient.stops {
        let Some(value) = palette.and_then(|palette| palette.resolve(&stop.token)) else {
            diagnostics.push(
                Diagnostic::error(
                    UNDEFINED_TOKEN,
                    format!(
                        "gradient `{}` references undefined palette token `{}`",
                        gradient.id, stop.token
                    ),
                )
                .at_path(format!("/stops/{}", stops.len())),
            );
            return None;
        };
        stops.push(ResolvedStop {
            offset: stop.offset,
            color: value.to_string(),
            opacity: stop.opacity.unwrap_or(1.0),
        });
    }

    // The render model requires stops in ascending offset order.
    stops.sort_by(|left, right| {
        left.offset
            .partial_cmp(&right.offset)
            .unwrap_or(Ordering::Equal)
    });

    Some(match gradient.gradient_type {
        GradientType::Linear => GradientPaint {
            gradient_type: GradientType::Linear,
            stops,
            spread: gradient.spread,
            x1: Some(gradient.x1.unwrap_or(0.0)),
            y1: Some(gradient.y1.unwrap_or(0.0)),
            x2: Some(gradient.x2.unwrap_or(1.0)),
            y2: Some(gradient.y2.unwrap_or(0.0)),
            cx: None,
            cy: None,
            r: None,
            fx: None,
            fy: None,
        },
        GradientType::Radial => {
            let cx = gradient.cx.unwrap_or(0.5);
            let cy = gradient.cy.unwrap_or(0.5);
            GradientPaint {
                gradient_type: GradientType::Radial,
                stops,
                spread: gradient.spread,
                x1: None,
                y1: None,
                x2: None,
                y2: None,
                cx: Some(cx),
                cy: Some(cy),
                r: Some(gradient.r.unwrap_or(0.5)),
                fx: Some(gradient.fx.unwrap_or(cx)),
                fy: Some(gradient.fy.unwrap_or(cy)),
            }
        }
    })
}

/// Checks a scene's gradient references against the gradients the project
/// provides.
///
/// A gradient no element references is a warning; it is not an error
/// (FEAT-027). An element naming a gradient that is not present is reported by
/// the compiler when it resolves the paint.
pub fn validate_usage(scene: &Scene, gradients: &[Gradient]) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();

    let used: Vec<&str> = scene
        .elements
        .iter()
        .flat_map(|element| {
            [
                element.fill.as_ref().and_then(PaintValue::literal),
                element
                    .stroke
                    .as_ref()
                    .and_then(|stroke| stroke.paint.literal()),
            ]
        })
        .flatten()
        .filter(|paint| paint.kind == PaintKind::Gradient)
        .map(|paint| paint.reference.as_str())
        .collect();

    for (index, gradient) in gradients.iter().enumerate() {
        if !used.contains(&gradient.id.as_str()) {
            diagnostics.push(
                Diagnostic::warning(
                    UNUSED_GRADIENT,
                    format!("gradient `{}` is never referenced", gradient.id),
                )
                .at_path(format!("/gradients/{index}")),
            );
        }
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient() -> &'static str {
        r##"{
  "id": "brand-gradient",
  "projectId": "project-1",
  "name": "Brand",
  "type": "linear",
  "stops": [
    { "offset": 0, "token": "accent" },
    { "offset": 1, "token": "ink", "opacity": 0.5 }
  ]
}"##
    }

    fn palette() -> super::super::Palette {
        super::super::parse_palette(
            r##"{"id":"pal","projectId":"p","name":"P","tokens":[{"name":"accent","value":"#ff0000"},{"name":"ink","value":"#111111"}]}"##,
        )
        .expect("a palette")
    }

    #[test]
    fn parses_and_round_trips() {
        let gradient = parse(gradient()).expect("valid gradient");
        assert_eq!(gradient.gradient_type, GradientType::Linear);
        assert_eq!(gradient.stops.len(), 2);
        assert_eq!(gradient.spread, Spread::Pad);
        let reparsed = parse(&gradient.to_json_string().unwrap()).expect("round-trips");
        assert_eq!(gradient, reparsed);
    }

    #[test]
    fn a_gradient_with_fewer_than_two_stops_is_an_error_naming_it() {
        let source = r##"{
  "id": "brand-gradient",
  "projectId": "project-1",
  "name": "Brand",
  "type": "linear",
  "stops": [{ "offset": 0, "token": "accent" }]
}"##;
        let gradient = parse(source).expect("parses");
        let diagnostics = validate(&gradient);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, GRADIENT);
        assert!(
            error.message.contains("brand-gradient"),
            "{}",
            error.message
        );
    }

    #[test]
    fn stops_resolve_to_the_palette_tokens_they_name() {
        let gradient = parse(gradient()).unwrap();
        let resolved =
            resolve(&gradient, Some(&palette()), &mut Diagnostics::new()).expect("resolves");
        assert_eq!(resolved.stops[0].color, "#ff0000");
        assert_eq!(resolved.stops[1].color, "#111111");
        assert_eq!(resolved.stops[1].opacity, 0.5);
    }

    #[test]
    fn a_missing_stop_token_is_an_error_naming_the_token() {
        let source = gradient().replace("accent", "missing");
        let gradient = parse(&source).unwrap();
        let mut diagnostics = Diagnostics::new();
        assert!(resolve(&gradient, Some(&palette()), &mut diagnostics).is_none());
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_TOKEN);
        assert!(error.message.contains("missing"), "{}", error.message);
    }

    #[test]
    fn a_radial_gradient_defaults_its_geometry() {
        let source = gradient().replace("\"linear\"", "\"radial\"");
        let gradient = parse(&source).unwrap();
        let resolved = resolve(&gradient, Some(&palette()), &mut Diagnostics::new()).unwrap();
        assert_eq!(resolved.gradient_type, GradientType::Radial);
        assert_eq!(resolved.cx, Some(0.5));
        assert_eq!(resolved.cy, Some(0.5));
        assert_eq!(resolved.r, Some(0.5));
        assert_eq!(resolved.x1, None);
    }

    #[test]
    fn a_gradient_with_no_palette_cannot_resolve() {
        let gradient = parse(gradient()).unwrap();
        let mut diagnostics = Diagnostics::new();
        assert!(resolve(&gradient, None, &mut diagnostics).is_none());
        assert!(diagnostics.errors().next().is_some());
    }

    #[test]
    fn an_unused_gradient_is_a_warning_not_an_error() {
        let scene = crate::scene::parse(
            r#"{"id":"s","projectId":"p","name":"S","formatVersion":"0.2","canvas":{"width":1,"height":1,"background":"transparent"},"elements":[{"id":"e1","sceneId":"s","order":0,"kind":"rect","geometry":{"width":1,"height":1},"transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1},"opacity":1,"visible":true}]}"#,
        )
        .unwrap();
        let gradients = [parse(gradient()).unwrap()];
        let diagnostics = validate_usage(&scene, &gradients);
        assert!(!diagnostics.has_errors());
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, UNUSED_GRADIENT);
        assert!(warning.message.contains("brand-gradient"));
    }
}
