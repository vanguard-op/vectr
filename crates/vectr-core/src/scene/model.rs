//! The scene document model (C-001).
//!
//! Shapes mirror docs/Vectr/schema.md, with the scene's elements and
//! constraints carried inline as the contract requires. Every struct is
//! `deny_unknown_fields`, so a document carrying a property the language does
//! not declare is rejected rather than silently ignored.

use serde::{Deserialize, Serialize};

use super::diagnostic::{Diagnostic, DiagnosticCode, Diagnostics};

/// A single authored graphic: a canvas, a tree of elements, and relationships.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene {
    /// Stable identifier for the scene.
    pub id: String,
    /// References the project this scene belongs to.
    pub project_id: String,
    /// Human-readable scene name, at most 120 characters.
    pub name: String,
    /// Declared format version, `major.minor`.
    pub format_version: String,
    /// The drawing surface.
    pub canvas: Canvas,
    /// References the palette whose tokens the elements use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette_id: Option<String>,
    /// References the style recipe applied to the scene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
    /// Accessible title carried into exported output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Accessible description carried into exported output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The scene's elements; an empty list is a valid, empty scene.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<Element>,
    /// Relationships resolved into concrete geometry at compile time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Vec<Constraint>>,
}

impl Scene {
    /// Finds an element by its stable identifier.
    pub fn element(&self, id: &str) -> Option<&Element> {
        self.elements.iter().find(|element| element.id == id)
    }

    /// Serializes the scene to compact JSON.
    pub fn to_json_string(&self) -> Result<String, Diagnostics> {
        serialize(self)
    }

    /// Serializes the scene to pretty, line-oriented JSON.
    ///
    /// One element per block keeps a text diff readable: a change lands on the
    /// named element it affects rather than inside an opaque blob (FEAT-001).
    pub fn to_json_pretty(&self) -> Result<String, Diagnostics> {
        serde_json::to_string_pretty(self).map_err(serialization_failure)
    }
}

/// The drawing surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Canvas {
    /// Canvas width in scene units.
    pub width: f64,
    /// Canvas height in scene units.
    pub height: f64,
    /// Canvas background color, or `transparent`.
    pub background: String,
}

/// One drawing entity: a primitive, a group, or a composition operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Element {
    /// Stable identifier for the element.
    pub id: String,
    /// References the scene this element belongs to.
    pub scene_id: String,
    /// References the parent element; absent or `null` marks a root element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Paint order among siblings; lower values paint first.
    pub order: i64,
    /// Element name, preserved in exported output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The element's role.
    pub kind: ElementKind,
    /// The element's geometry; which fields apply depends on `kind`.
    pub geometry: Geometry,
    /// Affine transform applied to the element and its children.
    pub transform: Transform,
    /// Palette token used as fill; absent or `null` for no fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_token: Option<String>,
    /// Stroke profile used for the stroke; absent or `null` for no stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_profile_id: Option<String>,
    /// Palette token used as stroke colour; absent or `null` for no stroke.
    ///
    /// Required when `stroke_profile_id` is set and absent otherwise: a stroke
    /// needs both a profile for its geometry and a token for its colour, so
    /// every colour lives in the palette (D-015).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_token: Option<String>,
    /// Element opacity, from 0 to 1.
    pub opacity: f64,
    /// Whether the element is rendered.
    pub visible: bool,
}

/// The element's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementKind {
    /// Axis-aligned rectangle.
    Rect,
    /// Ellipse.
    Ellipse,
    /// Closed point list.
    Polygon,
    /// Open point list.
    Line,
    /// Path data.
    Path,
    /// Container for child elements.
    Group,
    /// Repeat operation.
    Repeat,
    /// Boolean composition of two or more elements.
    Boolean,
    /// Element repeated along a path.
    AlongPath,
    /// Offset of another element.
    Offset,
    /// Projection of another element.
    Projection,
    /// Raster image.
    Raster,
}

/// The element's geometry; which fields apply depends on the element's kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Geometry {
    /// X origin in scene units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    /// Y origin in scene units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    /// Width for rect-like elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    /// Height for rect-like elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
    /// Corner radius in X.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rx: Option<f64>,
    /// Corner radius in Y.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ry: Option<f64>,
    /// Point list for polygon and line elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 2]>>,
    /// Path data for path elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_data: Option<String>,
    /// Number of copies for repeat elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Spacing between copies for repeat elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<f64>,
    /// Boolean operation for boolean elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<BooleanOperation>,
    /// Outline offset distance in scene units for offset elements; positive
    /// offsets outward and negative inward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<f64>,
    /// The axes a projection element maps its children onto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<ProjectionAxis>,
}

/// The axes a `projection` element maps its children onto (FEAT-003).
///
/// Distinct from [`Axis`], the constraint entity's `x`/`y`/`both`: a projection
/// names `x`, `y` or `isometric`, never `both`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectionAxis {
    /// Onto the local x-axis.
    X,
    /// Onto the local y-axis.
    Y,
    /// Onto the scene's isometric axes.
    Isometric,
}

impl ProjectionAxis {
    /// The axis name as it appears in the scene language.
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectionAxis::X => "x",
            ProjectionAxis::Y => "y",
            ProjectionAxis::Isometric => "isometric",
        }
    }

    /// Parses an axis name, or `None` when it is not one of the three.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "x" => Some(ProjectionAxis::X),
            "y" => Some(ProjectionAxis::Y),
            "isometric" => Some(ProjectionAxis::Isometric),
            _ => None,
        }
    }
}

/// A boolean composition operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BooleanOperation {
    /// Union of the operands.
    Union,
    /// Subtract later operands from the first.
    Subtract,
    /// Intersection of the operands.
    Intersect,
}

/// An affine transform applied to an element and its children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transform {
    /// Horizontal translation in scene units.
    pub translate_x: f64,
    /// Vertical translation in scene units.
    pub translate_y: f64,
    /// Rotation in degrees.
    pub rotate: f64,
    /// Horizontal scale factor.
    pub scale_x: f64,
    /// Vertical scale factor.
    pub scale_y: f64,
    /// Horizontal skew in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skew_x: Option<f64>,
    /// Vertical skew in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skew_y: Option<f64>,
}

/// A stated relationship resolved into concrete geometry at compile time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Constraint {
    /// Stable identifier for the constraint.
    pub id: String,
    /// References the scene this constraint belongs to.
    pub scene_id: String,
    /// The type of relationship.
    pub kind: ConstraintKind,
    /// References the elements the constraint relates; at least two.
    pub element_ids: Vec<String>,
    /// The axis the constraint applies to, when axis-specific.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<Axis>,
    /// The numeric value the constraint targets, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
}

/// The type of relationship a constraint expresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintKind {
    /// Even spacing between elements.
    EqualSpacing,
    /// Alignment along an axis.
    Align,
    /// Attachment to another element.
    Attach,
    /// Containment within another element.
    Contain,
    /// Snapping to the recipe grid.
    SnapToGrid,
}

/// The axis a constraint applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Axis {
    /// The horizontal axis.
    X,
    /// The vertical axis.
    Y,
    /// Both axes.
    Both,
}

fn serialize(scene: &Scene) -> Result<String, Diagnostics> {
    serde_json::to_string(scene).map_err(serialization_failure)
}

fn serialization_failure(error: serde_json::Error) -> Diagnostics {
    Diagnostics::from(Diagnostic::error(
        DiagnosticCode::SCHEMA,
        format!("could not serialize the scene: {error}"),
    ))
}
