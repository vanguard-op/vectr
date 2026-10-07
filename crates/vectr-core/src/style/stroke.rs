//! Named stroke configurations that elements reference (FEAT-005).
//!
//! A stroke profile fixes a width, cap and join. Several primitives that
//! reference the same profile therefore share those settings, and editing the
//! profile restyles all of them at once (FEAT-005).

use serde::{Deserialize, Serialize};

use super::{parse_document, to_json};
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics};

/// A named stroke configuration that elements reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StrokeProfile {
    /// Stable identifier for the profile.
    pub id: String,
    /// References the project this profile belongs to.
    pub project_id: String,
    /// Human-readable profile name.
    pub name: String,
    /// Stroke width in scene units.
    pub width: f64,
    /// Stroke line cap.
    pub cap: StrokeCap,
    /// Stroke line join.
    pub join: StrokeJoin,
}

/// How a stroke ends an open subpath.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StrokeCap {
    /// Flat end at the path end.
    Butt,
    /// Semicircular end.
    Round,
    /// Square end extending past the path end.
    Square,
}

/// How a stroke joins two segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StrokeJoin {
    /// Sharp corner.
    Miter,
    /// Rounded corner.
    Round,
    /// Flattened corner.
    Bevel,
}

/// The concrete settings a stroke profile contributes to a drawn element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResolvedStroke {
    /// Stroke width in scene units.
    pub width: f64,
    /// Stroke line cap.
    pub cap: StrokeCap,
    /// Stroke line join.
    pub join: StrokeJoin,
}

impl StrokeProfile {
    /// The settings this profile contributes to a stroke.
    pub fn resolved(&self) -> ResolvedStroke {
        ResolvedStroke {
            width: self.width,
            cap: self.cap,
            join: self.join,
        }
    }

    /// Serializes the profile to compact JSON.
    pub fn to_json_string(&self) -> Result<String, Diagnostics> {
        to_json(self)
    }

    /// Serializes the profile to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String, Diagnostics> {
        serde_json::to_string_pretty(self).map_err(|error| {
            Diagnostics::from(Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!("could not serialize the stroke profile: {error}"),
            ))
        })
    }
}

/// Reads a stroke profile from a JSON document.
pub fn parse(source: &str) -> Result<StrokeProfile, Diagnostics> {
    parse_document(source, "stroke profile")
}

/// Checks a stroke profile's values: the width must be finite and non-negative.
pub fn validate(profile: &StrokeProfile) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    if !profile.width.is_finite() || profile.width < 0.0 {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SCHEMA,
                "`width` must be a finite number that is zero or greater",
            )
            .at_path("/width"),
        );
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> &'static str {
        r#"{
  "id": "stroke-1",
  "projectId": "project-1",
  "name": "Outline",
  "width": 2.5,
  "cap": "round",
  "join": "bevel"
}"#
    }

    #[test]
    fn parses_and_round_trips() {
        let profile = parse(profile()).expect("valid profile");
        assert_eq!(profile.cap, StrokeCap::Round);
        assert_eq!(profile.join, StrokeJoin::Bevel);
        assert_eq!(profile.width, 2.5);
        let reparsed = parse(&profile.to_json_string().unwrap()).expect("round-trips");
        assert_eq!(profile, reparsed);
    }

    #[test]
    fn a_profile_contributes_the_same_settings_to_every_referencing_element() {
        let profile = parse(profile()).unwrap();
        let stroke = profile.resolved();
        assert_eq!(stroke, profile.resolved());
        assert_eq!(stroke.width, 2.5);
        assert_eq!(stroke.cap, StrokeCap::Round);
        assert_eq!(stroke.join, StrokeJoin::Bevel);
    }

    #[test]
    fn a_negative_width_is_rejected() {
        let mut profile = parse(profile()).unwrap();
        profile.width = -1.0;
        let diagnostics = validate(&profile);
        assert_eq!(
            diagnostics.errors().next().map(|error| error.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }
}
