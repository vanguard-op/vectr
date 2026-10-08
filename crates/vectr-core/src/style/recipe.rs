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
    /// Direction the scene's light comes from when shading is single-layer; the
    /// shading falls on the opposite side. Defaults to top-left when unset
    /// (FEAT-028).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light_direction: Option<LightDirection>,
    /// Name of the palette token supplying the single shading layer's colour;
    /// carried with the request and required when shading is single-layer
    /// (FEAT-028).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shade_token: Option<String>,
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

/// The direction a scene's light comes from when shading is single-layer: the
/// shading falls on the side facing away from it (FEAT-028).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LightDirection {
    /// Light from directly above.
    Top,
    /// Light from above and to the right.
    TopRight,
    /// Light from the right.
    Right,
    /// Light from below and to the right.
    BottomRight,
    /// Light from directly below.
    Bottom,
    /// Light from below and to the left.
    BottomLeft,
    /// Light from the left.
    Left,
    /// Light from above and to the left.
    TopLeft,
}

/// The flat recipe renders even fills and no texture (FEAT-007).
pub const TEXTURE_UNSUPPORTED: DiagnosticCode = DiagnosticCode::new("W_TEXTURE_UNSUPPORTED");

/// The flat recipe requests single-layer shading, a look it does not apply
/// (FEAT-007, FEAT-028).
pub const SHADING_UNSUPPORTED: DiagnosticCode = DiagnosticCode::new("W_SHADING_UNSUPPORTED");

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

/// An off-grid element was snapped onto the geometric recipe's grid (FEAT-009).
pub const GRID_SNAPPED: DiagnosticCode = DiagnosticCode::new("W_GRID_SNAPPED");

/// The geometric recipe's grid is finer than the renderable resolution
/// (FEAT-009).
pub const GRID_TOO_FINE: DiagnosticCode = DiagnosticCode::new("W_GRID_TOO_FINE");

/// A freeform curve was kept in a geometric scene as an explicit exception
/// (FEAT-009).
pub const FREEFORM_CURVE: DiagnosticCode = DiagnosticCode::new("W_FREEFORM_CURVE");

/// The smallest grid the geometric recipe renders on: snapping below this
/// resolves to positions the output cannot show, so a finer grid is reported as
/// a performance concern (FEAT-009).
///
/// The docs name no figure for the renderable resolution; this is the
/// product's, in scene units, matching the minimum renderable stroke weight.
pub const MIN_GRID_SIZE: f64 = 0.05;

/// A coordinate this close to a grid intersection, as a fraction of the grid,
/// already counts as on it (FEAT-009).
const GRID_TOLERANCE_FRACTION: f64 = 1e-6;

/// An element sat off the isometric grid and was snapped onto the axes
/// (FEAT-010).
pub const ISOMETRIC_OFF_AXIS: DiagnosticCode = DiagnosticCode::new("W_ISOMETRIC_OFF_AXIS");

/// Cosine and sine of the isometric axes' 30° elevation (FEAT-010).
///
/// The isometric grid is spanned by two unit axes rising 30° from the
/// horizontal — the positive x-axis to the upper right and the y-axis to the
/// upper left — the same axes the isometric projection helper maps onto
/// (FEAT-003). `ISOMETRIC_AXIS_COS` is `sqrt(3)/2` and `ISOMETRIC_AXIS_SIN`
/// is `1/2`.
const ISOMETRIC_AXIS_COS: f64 = 0.866_025_403_784_438_6;
const ISOMETRIC_AXIS_SIN: f64 = 0.5;

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

    /// Whether this is the geometric recipe.
    pub fn is_geometric(&self) -> bool {
        self.name == RecipeName::Geometric
    }

    /// Whether this is the isometric recipe.
    pub fn is_isometric(&self) -> bool {
        self.name == RecipeName::Isometric
    }

    /// The recipe's grid spacing, when it declares one (FEAT-009, FEAT-010).
    pub fn grid_size(&self) -> Option<f64> {
        self.parameters.grid_size
    }

    /// Whether the geometric recipe aligns its elements to its grid.
    ///
    /// Aligning to a grid is the geometric look's construction rule (FEAT-009),
    /// so it is on whenever the recipe names a usable grid, and off when `snap`
    /// is explicitly false or the grid is absent or unusable. The isometric
    /// recipe snaps to its own grid instead ([`StyleRecipe::snaps_to_isometric_grid`]);
    /// the flat and line-art looks leave positions untouched.
    pub fn snaps_to_grid(&self) -> bool {
        self.is_geometric()
            && self.parameters.snap.unwrap_or(true)
            && self
                .grid_size()
                .is_some_and(|size| size.is_finite() && size > 0.0)
    }

    /// Whether the isometric recipe aligns its elements to the isometric grid
    /// (FEAT-010).
    ///
    /// Aligning elements to the isometric grid is what makes them sit on the
    /// axes, so it is on whenever the isometric recipe names a usable grid, and
    /// off when `snap` is explicitly false or the grid is absent or unusable.
    /// The other looks, including the geometric one, snap to their own grid or
    /// not at all.
    pub fn snaps_to_isometric_grid(&self) -> bool {
        self.is_isometric()
            && self.parameters.snap.unwrap_or(true)
            && self
                .grid_size()
                .is_some_and(|size| size.is_finite() && size > 0.0)
    }

    /// Snaps a coordinate to the nearest grid intersection.
    ///
    /// Returns the grid intersection and whether the original coordinate sat
    /// off the grid beyond the grid tolerance. A coordinate already on the grid
    /// comes back at the intersection unchanged, and a recipe that does not
    /// construct on a grid — any other look, or a geometric recipe without a
    /// usable grid — returns the coordinate untouched, so a scene compiled
    /// without a grid is byte-identical to one compiled with no recipe at all
    /// (NFR-010).
    pub fn snap_coordinate(&self, value: f64) -> (f64, bool) {
        if !self.is_geometric() {
            return (value, false);
        }
        let Some(size) = self.grid_size() else {
            return (value, false);
        };
        if !size.is_finite() || size <= 0.0 || !value.is_finite() {
            return (value, false);
        }
        let target = (value / size).round() * size;
        let tolerance = size * GRID_TOLERANCE_FRACTION;
        (target, (value - target).abs() > tolerance)
    }

    /// Snaps a point onto the isometric grid (FEAT-010).
    ///
    /// The isometric grid is the lattice spanned by the two 30° axes at the
    /// recipe's grid spacing, so snapping a point expresses it in the axes'
    /// coordinates, rounds both, and maps it back — the same move as snapping a
    /// square grid in the plane before the isometric projection. Returns the
    /// nearest lattice point and whether the original sat off the grid beyond
    /// tolerance. A point already on the lattice comes back unchanged, and a
    /// recipe that does not construct on the isometric grid returns the point
    /// untouched, so its positions are exactly those the scene declared.
    pub fn snap_isometric(&self, point: [f64; 2]) -> ([f64; 2], bool) {
        if !self.snaps_to_isometric_grid() {
            return (point, false);
        }
        let Some(size) = self.grid_size() else {
            return (point, false);
        };
        if !point[0].is_finite() || !point[1].is_finite() {
            return (point, false);
        }

        // Express the point in the axes' coordinates, round each, and map back:
        // a lattice point `(i, j)` lands at `size * (i·u + j·v)`.
        let along_x = point[0] / (2.0 * size * ISOMETRIC_AXIS_COS);
        let along_y = point[1] / size;
        let i = (along_x + along_y).round();
        let j = (-along_x + along_y).round();
        let snapped = [
            size * ISOMETRIC_AXIS_COS * (i - j),
            size * ISOMETRIC_AXIS_SIN * (i + j),
        ];

        let tolerance = size * GRID_TOLERANCE_FRACTION;
        let moved =
            (point[0] - snapped[0]).abs() > tolerance || (point[1] - snapped[1]).abs() > tolerance;
        (snapped, moved)
    }

    /// Snaps a placement point onto whichever grid the recipe constructs on
    /// (FEAT-009, FEAT-010).
    ///
    /// The geometric recipe snaps each coordinate to its axis-aligned grid; the
    /// isometric recipe snaps the point onto the lattice its two 30° axes span.
    /// Every other look, a gridless recipe, and a point already on the grid come
    /// back untouched with `moved` false. This is the grid a shape's own
    /// placement snaps to — a rect or an ellipse's bounding-box origin, a
    /// polygon or line's vertices (schema.md, "Element") — as distinct from the
    /// element's transform translation.
    pub fn snap_point(&self, point: [f64; 2]) -> ([f64; 2], bool) {
        if self.snaps_to_grid() {
            let (x, moved_x) = self.snap_coordinate(point[0]);
            let (y, moved_y) = self.snap_coordinate(point[1]);
            ([x, y], moved_x || moved_y)
        } else if self.snaps_to_isometric_grid() {
            self.snap_isometric(point)
        } else {
            (point, false)
        }
    }

    /// The isometric depth of a point: larger values are nearer the viewer
    /// (FEAT-010).
    ///
    /// Both isometric axes descend the screen, so a point's depth is its screen
    /// y, which is the grid row `i + j` scaled by the spacing. With a usable
    /// grid the depth is quantized to whole rows, so elements that share a row
    /// share a depth; without one it falls back to the raw y, and the ordering
    /// stays deterministic either way (NFR-010).
    pub fn isometric_depth(&self, point: [f64; 2]) -> f64 {
        let Some(size) = self.grid_size() else {
            return point[1];
        };
        if !size.is_finite() || size <= 0.0 {
            return point[1];
        }
        (point[1] / (size * ISOMETRIC_AXIS_SIN)).round()
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

/// Reports where a recipe asks for a shading look it does not apply.
///
/// The flat recipe draws even, solid fills (FEAT-007). It does not apply
/// single-layer shading — that look ships as its own feature (FEAT-028) — and it
/// cannot draw a texture, so a request for either is reported and the shape
/// renders with its base fill rather than being left silently unshaded.
pub fn check_expressible(recipe: &StyleRecipe) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    if !recipe.is_flat() {
        return diagnostics;
    }
    match recipe.parameters.shading {
        Some(Shading::SingleLayer) => diagnostics.push(
            Diagnostic::warning(
                SHADING_UNSUPPORTED,
                format!(
                    "recipe `{}` requests single-layer shading, which the flat recipe does not apply; the shape renders with its base fill",
                    recipe.id
                ),
            )
            .at_path("/parameters/shading"),
        ),
        Some(Shading::Raster) => diagnostics.push(
            Diagnostic::warning(
                TEXTURE_UNSUPPORTED,
                format!(
                    "recipe `{}` declares raster shading, which the flat recipe cannot express; the output stays flat and no texture is drawn",
                    recipe.id
                ),
            )
            .at_path("/parameters/shading"),
        ),
        Some(Shading::None) | None => {}
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

/// Reports a geometric recipe whose grid is finer than the renderable
/// resolution (FEAT-009).
///
/// A grid below [`MIN_GRID_SIZE`] snaps elements to positions the output cannot
/// resolve. The grid is still applied, and the fine grid is reported as a
/// performance concern rather than silently accepted.
pub fn check_grid(recipe: &StyleRecipe) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();
    if !recipe.is_geometric() {
        return diagnostics;
    }
    if let Some(size) = recipe.grid_size() {
        if size.is_finite() && size > 0.0 && size < MIN_GRID_SIZE {
            diagnostics.push(
                Diagnostic::warning(
                    GRID_TOO_FINE,
                    format!(
                        "geometric recipe `{}` uses a grid of {size}, below the renderable resolution {MIN_GRID_SIZE}; snapping to it costs precision the output cannot show",
                        recipe.id
                    ),
                )
                .at_path("/parameters/gridSize"),
            );
        }
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
        let recipe =
            parse(r#"{"id":"r","projectId":"p","name":"flat","parameters":{"shading":"raster"}}"#)
                .unwrap();
        assert!(recipe.is_flat(), "the recipe names the flat look");
        assert_eq!(recipe.name_str(), "flat");

        let diagnostics = check_expressible(&recipe);
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, TEXTURE_UNSUPPORTED);
        assert!(warning.message.contains("flat"), "{}", warning.message);
        assert!(!diagnostics.has_errors(), "texture is reported, not fatal");

        // A flat recipe that draws no shading asks for nothing it cannot draw.
        let unshaded =
            parse(r#"{"id":"r","projectId":"p","name":"flat","parameters":{"shading":"none"}}"#)
                .unwrap();
        assert!(!check_expressible(&unshaded).has_errors());
        assert!(check_expressible(&unshaded).warnings().next().is_none());
    }

    #[test]
    fn a_flat_recipe_reports_single_layer_shading_it_does_not_apply() {
        let mut recipe = parse(
            r#"{"id":"r","projectId":"p","name":"flat","parameters":{"shading":"single-layer"}}"#,
        )
        .unwrap();
        // The request parses with the light direction and shade tone the schema
        // describes, so it reaches the check rather than being refused as an
        // unknown property.
        recipe.parameters.light_direction = Some(LightDirection::TopRight);
        recipe.parameters.shade_token = Some("shadow".to_string());

        let diagnostics = check_expressible(&recipe);
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, SHADING_UNSUPPORTED);
        assert!(
            warning.message.contains("single-layer"),
            "the warning names the request: {}",
            warning.message
        );
        assert!(!diagnostics.has_errors(), "shading is reported, not fatal");
        assert_eq!(
            warning
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/parameters/shading")
        );
    }

    #[test]
    fn a_single_layer_request_carries_its_light_direction_and_shade_token() {
        let recipe = parse(
            r#"{"id":"r","projectId":"p","name":"flat","parameters":{"shading":"single-layer","lightDirection":"bottomLeft","shadeToken":"shadow"}}"#,
        )
        .expect("a valid single-layer request");
        assert_eq!(recipe.parameters.shading, Some(Shading::SingleLayer));
        assert_eq!(
            recipe.parameters.light_direction,
            Some(LightDirection::BottomLeft)
        );
        assert_eq!(recipe.parameters.shade_token.as_deref(), Some("shadow"));

        // The request survives its camelCase JSON round trip (C-002).
        let reparsed = parse(&recipe.to_json_string().unwrap()).expect("round-trips");
        assert_eq!(recipe, reparsed);
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

    fn geometric(grid: f64, snap: Option<bool>) -> StyleRecipe {
        let snap = match snap {
            Some(value) => format!(r#","snap":{value}"#),
            None => String::new(),
        };
        parse(&format!(
            r#"{{"id":"r","projectId":"p","name":"geometric","parameters":{{"gridSize":{grid}{snap}}}}}"#
        ))
        .expect("a geometric recipe")
    }

    #[test]
    fn a_geometric_recipe_snaps_a_coordinate_to_its_grid() {
        let recipe = geometric(10.0, None);
        assert!(recipe.is_geometric());

        // An off-grid coordinate snaps to the nearest intersection and is
        // reported; a coordinate already on the grid does not move or report.
        assert_eq!(recipe.snap_coordinate(13.0), (10.0, true));
        assert_eq!(recipe.snap_coordinate(-4.0), (0.0, true));
        assert_eq!(recipe.snap_coordinate(20.0), (20.0, false));
        // A coordinate within tolerance counts as already on the grid.
        let (snapped, moved) = recipe.snap_coordinate(10.0 + 1e-9);
        assert_eq!(snapped, 10.0);
        assert!(!moved, "a hair off the grid is not an off-grid element");
    }

    #[test]
    fn the_geometric_recipe_snaps_unless_explicitly_disabled() {
        // A grid is the recipe's construction rule, so snapping is on by
        // default and can be switched off.
        assert!(geometric(10.0, None).snaps_to_grid());
        assert!(geometric(10.0, Some(true)).snaps_to_grid());
        assert!(!geometric(10.0, Some(false)).snaps_to_grid());

        // Without a usable grid there is nothing to snap to.
        let no_grid =
            parse(r#"{"id":"r","projectId":"p","name":"geometric","parameters":{}}"#).unwrap();
        assert!(!no_grid.snaps_to_grid());
        assert_eq!(no_grid.snap_coordinate(13.0), (13.0, false));
    }

    #[test]
    fn another_recipe_never_snaps() {
        let flat =
            parse(r#"{"id":"r","projectId":"p","name":"flat","parameters":{"gridSize":10}}"#)
                .unwrap();
        assert!(!flat.is_geometric());
        assert!(!flat.snaps_to_grid());
        assert_eq!(flat.snap_coordinate(13.0), (13.0, false));
    }

    #[test]
    fn a_placement_point_snaps_through_the_recipe_that_owns_the_grid() {
        // The geometric look snaps each coordinate to its axis-aligned grid.
        assert_eq!(
            geometric(10.0, None).snap_point([13.0, -4.0]),
            ([10.0, 0.0], true)
        );
        assert_eq!(
            geometric(10.0, None).snap_point([20.0, 30.0]),
            ([20.0, 30.0], false)
        );

        // The isometric look snaps the point onto its 30° lattice.
        let (snapped, moved) = isometric(10.0, None).snap_point([1.0, 1.0]);
        assert!(moved);
        assert!(close(snapped, [0.0, 0.0]));

        // Another look leaves the point where it is.
        let flat =
            parse(r#"{"id":"r","projectId":"p","name":"flat","parameters":{"gridSize":10}}"#)
                .unwrap();
        assert_eq!(flat.snap_point([3.0, 4.0]), ([3.0, 4.0], false));
    }

    #[test]
    fn a_grid_finer_than_the_renderable_resolution_is_reported() {
        let diagnostics = check_grid(&geometric(MIN_GRID_SIZE / 10.0, None));
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, GRID_TOO_FINE);
        assert!(warning.message.contains("grid"), "{}", warning.message);
        assert!(
            !diagnostics.has_errors(),
            "a fine grid is a performance concern, not fatal"
        );

        // A grid at or above the renderable resolution is fine, and no other
        // recipe is checked.
        assert!(check_grid(&geometric(MIN_GRID_SIZE, None))
            .warnings()
            .next()
            .is_none());
        let flat =
            parse(r#"{"id":"r","projectId":"p","name":"flat","parameters":{"gridSize":0.0001}}"#)
                .unwrap();
        assert!(check_grid(&flat).warnings().next().is_none());
    }

    fn isometric(grid: f64, snap: Option<bool>) -> StyleRecipe {
        let snap = match snap {
            Some(value) => format!(r#","snap":{value}"#),
            None => String::new(),
        };
        parse(&format!(
            r#"{{"id":"r","projectId":"p","name":"isometric","parameters":{{"gridSize":{grid}{snap}}}}}"#
        ))
        .expect("an isometric recipe")
    }

    fn close(left: [f64; 2], right: [f64; 2]) -> bool {
        (left[0] - right[0]).abs() < 1e-9 && (left[1] - right[1]).abs() < 1e-9
    }

    #[test]
    fn the_isometric_recipe_aligns_a_point_to_its_axes() {
        let recipe = isometric(10.0, None);
        assert!(recipe.is_isometric());
        let axis = 3.0_f64.sqrt() / 2.0;

        // Points already on the lattice — the origin and the two axes and their
        // sum — are left where they are.
        for point in [
            [0.0, 0.0],
            [10.0 * axis, 5.0],
            [-10.0 * axis, 5.0],
            [0.0, 10.0],
        ] {
            assert_eq!(recipe.snap_isometric(point), (point, false), "{point:?}");
        }

        // An off-axis point snaps to the nearest lattice point and is reported.
        assert_eq!(recipe.snap_isometric([1.0, 1.0]), ([0.0, 0.0], true));
        let (snapped, moved) = recipe.snap_isometric([5.0, 5.0]);
        assert!(moved, "an off-axis point is reported");
        assert!(
            close(snapped, [10.0 * axis, 5.0]),
            "snapped to a grid intersection: {snapped:?}"
        );
    }

    #[test]
    fn the_isometric_recipe_snaps_unless_explicitly_disabled() {
        assert!(isometric(10.0, None).snaps_to_isometric_grid());
        assert!(isometric(10.0, Some(true)).snaps_to_isometric_grid());
        assert!(!isometric(10.0, Some(false)).snaps_to_isometric_grid());

        // Without a usable grid there is nothing to snap to.
        let no_grid =
            parse(r#"{"id":"r","projectId":"p","name":"isometric","parameters":{}}"#).unwrap();
        assert!(!no_grid.snaps_to_isometric_grid());
        assert_eq!(no_grid.snap_isometric([1.0, 1.0]), ([1.0, 1.0], false));
    }

    #[test]
    fn another_recipe_never_snaps_isometrically() {
        let flat =
            parse(r#"{"id":"r","projectId":"p","name":"flat","parameters":{"gridSize":10}}"#)
                .unwrap();
        assert!(!flat.snaps_to_isometric_grid());
        assert_eq!(flat.snap_isometric([1.0, 1.0]), ([1.0, 1.0], false));

        // The geometric recipe owns its own axis-aligned grid, not this one.
        let geometric = geometric(10.0, None);
        assert!(!geometric.snaps_to_isometric_grid());
        assert_eq!(geometric.snap_isometric([1.0, 1.0]), ([1.0, 1.0], false));
    }

    #[test]
    fn the_isometric_depth_follows_the_grid_row() {
        let recipe = isometric(10.0, None);
        // Depth is the grid row `i + j`: the origin is back-most and lower
        // points are nearer the viewer, so a stack paints back to front.
        assert_eq!(recipe.isometric_depth([0.0, 0.0]), 0.0);
        assert_eq!(recipe.isometric_depth([0.0, 5.0]), 1.0);
        assert_eq!(recipe.isometric_depth([0.0, 10.0]), 2.0);
        assert_eq!(recipe.isometric_depth([0.0, -10.0]), -2.0);

        // Two points on the same row share a depth, so their order is
        // ambiguous and left to the deterministic tie-break.
        assert_eq!(
            recipe.isometric_depth([8.0, 5.0]),
            recipe.isometric_depth([-8.0, 5.0])
        );

        // Without a grid the raw screen position is the depth.
        let no_grid =
            parse(r#"{"id":"r","projectId":"p","name":"isometric","parameters":{}}"#).unwrap();
        assert_eq!(no_grid.isometric_depth([7.0, 3.0]), 3.0);
    }
}
