//! Style recipes: the named looks a scene can be rendered with (FEAT-005).
//!
//! A recipe carries the parameters that govern composition and shading, kept
//! separate from the scene's structure so a graphic can be restyled without
//! editing its elements. Version one ships `flat`, `line-art`, `geometric` and
//! `isometric` (docs/Vectr/schema.md, "StyleRecipe").

use serde::{Deserialize, Serialize};

use super::{parse_document, to_json};
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics};

/// A named rendering recipe that governs how a scene is composed and shaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StyleRecipe {
    /// Stable identifier for the recipe.
    pub id: String,
    /// References the project this recipe belongs to.
    pub project_id: String,
    /// The recipe selected.
    pub name: RecipeName,
    /// Recipe-specific settings.
    pub parameters: RecipeParameters,
}

/// The recipe selected, one of the four version-one recipes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecipeName {
    /// Flat color, no line work.
    Flat,
    /// Line art.
    #[serde(rename = "line-art")]
    LineArt,
    /// Geometric construction.
    Geometric,
    /// Isometric projection.
    Isometric,
}

impl RecipeName {
    /// The four version-one recipes.
    pub const ALL: [RecipeName; 4] = [
        RecipeName::Flat,
        RecipeName::LineArt,
        RecipeName::Geometric,
        RecipeName::Isometric,
    ];

    /// The recipe name as it appears in the scene language.
    pub fn as_str(self) -> &'static str {
        match self {
            RecipeName::Flat => "flat",
            RecipeName::LineArt => "line-art",
            RecipeName::Geometric => "geometric",
            RecipeName::Isometric => "isometric",
        }
    }
}

/// Recipe-specific settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeParameters {
    /// Grid spacing for snapping, where the recipe uses a grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_size: Option<f64>,
    /// Whether elements snap to the grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<bool>,
    /// Shading model the recipe applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shading: Option<Shading>,
    /// Default stroke weight for stroke-based recipes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_weight: Option<f64>,
}

/// The shading model a recipe applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shading {
    /// No shading.
    None,
    /// A single shading layer.
    #[serde(rename = "single-layer")]
    SingleLayer,
    /// Raster-assisted shading.
    Raster,
}

/// The flat recipe renders even fills and no texture (FEAT-007).
pub const TEXTURE_UNSUPPORTED: DiagnosticCode = DiagnosticCode::new("W_TEXTURE_UNSUPPORTED");

/// The smallest stroke weight a line-art scene renders: a thinner line does not
/// survive rasterization, so it is clamped up and reported rather than drawn
/// invisibly (FEAT-008).
///
/// The docs name no figure for the minimum renderable unit; this is the
/// product's, in scene units.
pub const MIN_STROKE_WEIGHT: f64 = 0.05;

/// A line-art stroke weight below the minimum renderable unit was clamped up
/// (FEAT-008).
pub const STROKE_WEIGHT_CLAMPED: DiagnosticCode = DiagnosticCode::new("W_STROKE_WEIGHT_CLAMPED");

/// A line-art scene draws no strokes, so its line-art output is empty (FEAT-008).
pub const LINE_ART_EMPTY: DiagnosticCode = DiagnosticCode::new("W_LINE_ART_EMPTY");

impl StyleRecipe {
    /// The recipe name as it appears in the scene language.
    pub fn name_str(&self) -> &'static str {
        self.name.as_str()
    }

    /// Whether this is the flat recipe.
    pub fn is_flat(&self) -> bool {
        self.name == RecipeName::Flat
    }

    /// Whether this is the line-art recipe.
    pub fn is_line_art(&self) -> bool {
        self.name == RecipeName::LineArt
    }

    /// The recipe's default stroke weight, when it declares one (FEAT-008).
    pub fn stroke_weight(&self) -> Option<f64> {
        self.parameters.stroke_weight
    }

    /// The effective stroke weight under this recipe, and whether it was
    /// clamped up.
    ///
    /// The line-art recipe fixes a consistent weight (FEAT-008): its declared
    /// `strokeWeight` fills a stroke whose profile leaves the weight unset (a
    /// width of zero), while a profile that states a positive width is an
    /// explicit request for a varying weight and is honored. Either way a
    /// weight below the minimum renderable unit is clamped up, since a thinner
    /// line does not survive rasterization. Every other recipe, and the
    /// no-recipe path, leaves the profile's width untouched.
    pub fn resolve_stroke_weight(&self, profile_width: f64) -> (f64, bool) {
        if !self.is_line_art() {
            return (profile_width, false);
        }
        let width = match self.stroke_weight() {
            Some(default) if profile_width == 0.0 => default,
            _ => profile_width,
        };
        if width < MIN_STROKE_WEIGHT {
            (MIN_STROKE_WEIGHT, true)
        } else {
            (width, false)
        }
    }

    /// Serializes the recipe to compact JSON.
    pub fn to_json_string(&self) -> Result<String, Diagnostics> {
        to_json(self)
    }

    /// Serializes the recipe to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String, Diagnostics> {
        serde_json::to_string_pretty(self).map_err(|error| {
            Diagnostics::from(Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!("could not serialize the style recipe: {error}"),
            ))
        })
    }
}

/// Reads a style recipe from a JSON document.
pub fn parse(source: &str) -> Result<StyleRecipe, Diagnostics> {
    parse_document(source, "style recipe")
}

/// Checks a recipe's values: the grid size and stroke weight must be finite and
/// non-negative where present.
pub fn validate(recipe: &StyleRecipe) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    validate_non_negative(
        &mut diagnostics,
        recipe.parameters.grid_size,
        "/parameters/gridSize",
        "gridSize",
    );
    validate_non_negative(
        &mut diagnostics,
        recipe.parameters.stroke_weight,
        "/parameters/strokeWeight",
        "strokeWeight",
    );
    diagnostics
}

/// Reports where a recipe asks for a look the language cannot express.
///
/// The flat recipe draws even, solid fills and no texture (FEAT-007): a flat
/// recipe that declares raster shading is asking for a texture the language has
/// no way to draw, so the request is reported and the output stays flat rather
/// than silently textured.
pub fn check_expressible(recipe: &StyleRecipe) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    if recipe.is_flat() && recipe.parameters.shading == Some(Shading::Raster) {
        diagnostics.push(
            Diagnostic::warning(
                TEXTURE_UNSUPPORTED,
                format!(
                    "recipe `{}` declares raster shading, which the flat recipe cannot express; the output stays flat and no texture is drawn",
                    recipe.id
                ),
            )
            .at_path("/parameters/shading"),
        );
    }
    diagnostics
}

/// Reports a line-art scene that draws no stroke.
///
/// The recipe is stroke-based, so a scene whose compiled output carries no
/// stroke has no line work in it; the emptiness is reported rather than passed
/// off as a successful line-art render (FEAT-008).
pub fn check_line_art(recipe: &StyleRecipe, has_strokes: bool) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    if recipe.is_line_art() && !has_strokes {
        diagnostics.push(Diagnostic::warning(
            LINE_ART_EMPTY,
            format!(
                "line-art recipe `{}` draws no strokes: the line-art output is empty",
                recipe.id
            ),
        ));
    }
    diagnostics
}

fn validate_non_negative(
    diagnostics: &mut Diagnostics,
    value: Option<f64>,
    path: &str,
    field: &str,
) {
    if let Some(value) = value {
        if !value.is_finite() || value < 0.0 {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::SCHEMA,
                    format!("`{field}` must be a finite number that is zero or greater"),
                )
                .at_path(path),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> &'static str {
        r#"{
  "id": "recipe-1",
  "projectId": "project-1",
  "name": "line-art",
  "parameters": { "gridSize": 8, "snap": true, "shading": "single-layer", "strokeWeight": 1.5 }
}"#
    }

    #[test]
    fn parses_and_round_trips() {
        let recipe = parse(recipe()).expect("valid recipe");
        assert_eq!(recipe.name, RecipeName::LineArt);
        assert_eq!(recipe.parameters.shading, Some(Shading::SingleLayer));
        let reparsed = parse(&recipe.to_json_string().unwrap()).expect("round-trips");
        assert_eq!(recipe, reparsed);
    }

    #[test]
    fn version_one_ships_four_recipes() {
        assert_eq!(RecipeName::ALL.len(), 4);
        for name in RecipeName::ALL {
            assert!(!name.as_str().is_empty());
        }
        assert_eq!(RecipeName::LineArt.as_str(), "line-art");
    }

    #[test]
    fn a_negative_grid_size_is_rejected() {
        let mut recipe = parse(recipe()).unwrap();
        recipe.parameters.grid_size = Some(-1.0);
        let diagnostics = validate(&recipe);
        assert_eq!(
            diagnostics.errors().next().map(|error| error.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }

    #[test]
    fn a_flat_recipe_reports_texture_it_cannot_express() {
        let mut recipe =
            parse(r#"{"id":"r","projectId":"p","name":"flat","parameters":{"shading":"raster"}}"#)
                .unwrap();
        assert!(recipe.is_flat(), "the recipe names the flat look");
        assert_eq!(recipe.name_str(), "flat");

        let diagnostics = check_expressible(&recipe);
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, TEXTURE_UNSUPPORTED);
        assert!(warning.message.contains("flat"), "{}", warning.message);
        assert!(!diagnostics.has_errors(), "texture is reported, not fatal");

        // A flat recipe without raster shading asks for nothing it cannot draw.
        recipe.parameters.shading = Some(Shading::SingleLayer);
        assert!(!check_expressible(&recipe).has_errors());
        assert!(check_expressible(&recipe).warnings().next().is_none());
    }

    #[test]
    fn a_non_flat_recipe_is_not_checked_as_flat() {
        // Raster shading is only unexpressible for the flat look; the check is
        // the flat recipe's, and other recipes settle their own look.
        let recipe = parse(
            r#"{"id":"r","projectId":"p","name":"geometric","parameters":{"shading":"raster"}}"#,
        )
        .unwrap();
        assert!(!recipe.is_flat());
        assert!(!check_expressible(&recipe).has_errors());
        assert!(check_expressible(&recipe).warnings().next().is_none());
    }

    fn line_art(stroke_weight: Option<f64>) -> StyleRecipe {
        let parameters = match stroke_weight {
            Some(weight) => format!(r#"{{"strokeWeight":{weight}}}"#),
            None => "{}".to_string(),
        };
        parse(&format!(
            r#"{{"id":"r","projectId":"p","name":"line-art","parameters":{parameters}}}"#
        ))
        .expect("a line-art recipe")
    }

    #[test]
    fn a_line_art_recipe_fills_a_stroke_weight_left_unset() {
        let recipe = line_art(Some(2.5));
        assert!(recipe.is_line_art());
        assert_eq!(recipe.stroke_weight(), Some(2.5));

        // A profile of width zero leaves the weight to the recipe.
        assert_eq!(recipe.resolve_stroke_weight(0.0), (2.5, false));
    }

    #[test]
    fn a_line_art_recipe_honors_an_explicit_varying_weight() {
        let recipe = line_art(Some(2.5));
        // A positive profile width is an explicit request and wins.
        assert_eq!(recipe.resolve_stroke_weight(6.0), (6.0, false));
    }

    #[test]
    fn a_line_art_recipe_clamps_a_sub_minimum_weight() {
        let recipe = line_art(Some(2.5));
        let (width, clamped) = recipe.resolve_stroke_weight(0.001);
        assert!(clamped, "a weight below the minimum is reported");
        assert_eq!(width, MIN_STROKE_WEIGHT);

        // The recipe's own default is clamped the same way.
        let thin = line_art(Some(0.001));
        assert_eq!(thin.resolve_stroke_weight(0.0), (MIN_STROKE_WEIGHT, true));
    }

    #[test]
    fn a_non_line_art_recipe_leaves_the_weight_untouched() {
        let flat =
            parse(r#"{"id":"r","projectId":"p","name":"flat","parameters":{"strokeWeight":2.5}}"#)
                .unwrap();
        assert_eq!(flat.resolve_stroke_weight(0.0), (0.0, false));
        assert_eq!(flat.resolve_stroke_weight(7.0), (7.0, false));
    }

    #[test]
    fn a_line_art_scene_with_no_strokes_is_reported_empty() {
        let recipe = line_art(Some(2.0));
        let diagnostics = check_line_art(&recipe, false);
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, LINE_ART_EMPTY);
        assert!(warning.message.contains("empty"), "{}", warning.message);
        assert!(
            !diagnostics.has_errors(),
            "emptiness is reported, not fatal"
        );

        assert!(check_line_art(&recipe, true).warnings().next().is_none());
        // Another recipe is not a line-art scene and is not checked.
        let flat = parse(r#"{"id":"r","projectId":"p","name":"flat","parameters":{}}"#).unwrap();
        assert!(check_line_art(&flat, false).warnings().next().is_none());
    }
}
