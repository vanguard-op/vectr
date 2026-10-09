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
    /// References the scene this element belongs to; `null` when the element
    /// belongs to a definition. Exactly one of `sceneId` and `definitionId` is
    /// set (C-001).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    /// References the definition this element belongs to; `null` when the
    /// element belongs to a scene (C-001).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_id: Option<String>,
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
    /// The element's fill paint, or absent for no fill (C-001).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// The element's stroke, or absent for no stroke (C-001).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Font asset a text element renders with; absent or `null` selects the
    /// default open-licensed font.
    ///
    /// Applies only to a text element: a font reference on any other kind is
    /// rejected (FEAT-024).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_id: Option<String>,
    /// Element opacity, from 0 to 1.
    pub opacity: f64,
    /// Whether the element is rendered.
    pub visible: bool,
    /// References the definition an `instance` element places; required when
    /// `kind` is `instance` and null otherwise (C-001, FEAT-030).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_ref: Option<String>,
    /// Values an instance binds to the placed definition's parameters; applies
    /// only to an instance (C-001, FEAT-030).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Vec<Binding>>,
    /// Appearance overrides an instance applies to named elements of the placed
    /// definition; applies only to an instance (C-001, FEAT-030).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<Override>>,
}

/// One value an instance binds to a placed definition's parameter (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    /// The name of the parameter being bound.
    pub name: String,
    /// The literal value bound, or a reference forwarding a parameter of the
    /// definition that contains the instance.
    pub value: BindingValue,
}

/// The value of a binding: a literal, or a parameter reference (C-001).
///
/// The untagged form matches the language's shape: a JSON number, string or
/// boolean is a literal, and the object `{ "param": "<name>" }` forwards a
/// parameter of the enclosing definition (D-035).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BindingValue {
    /// A numeric literal.
    Number(f64),
    /// A string literal.
    Text(String),
    /// A boolean literal.
    Boolean(bool),
    /// A reference forwarding an enclosing definition's parameter.
    Param(ParamRef),
}

/// A reference to a declared parameter, written `{ "param": "<name>" }` (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParamRef {
    /// The name of the parameter being referenced.
    pub param: String,
}

/// One appearance override an instance applies to a placed definition element
/// (C-001, FEAT-030).
///
/// A field left out is inherited from the definition; an explicit `null` fill or
/// stroke clears it at that use, so the two states are distinguished.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Override {
    /// Identifier of the placed definition's element whose appearance changes.
    pub target: String,
    /// The target's fill at this use; `null` clears it, absent inherits it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub fill: Option<Option<Paint>>,
    /// The target's stroke at this use; `null` clears it, absent inherits it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_double_option"
    )]
    pub stroke: Option<Option<Stroke>>,
    /// The target's opacity at this use; absent inherits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
}

/// Reads an optional field that also distinguishes an explicit `null`.
fn deserialize_double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A paint an element's fill or stroke names (C-001).
///
/// A paint names either a Palette token (a solid colour) or a Gradient (a
/// gradient); the reference is a name, not resolved at parse time, so the
/// compiler resolves it against the caller's palette and gradients (D-021).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Paint {
    /// Whether the reference names a palette token or a gradient.
    pub kind: PaintKind,
    /// The palette token name when `kind` is `token`, or the gradient id when
    /// `kind` is `gradient`.
    #[serde(rename = "ref")]
    pub reference: String,
}

/// The kind of paint a reference names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaintKind {
    /// The reference names a palette token (a solid paint).
    Token,
    /// The reference names a gradient (a gradient paint).
    Gradient,
}

/// An element's stroke: a profile for its geometry and a paint for its colour
/// (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stroke {
    /// References the stroke profile supplying width, cap and join.
    pub profile_id: String,
    /// The stroke's paint.
    pub paint: Paint,
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
    /// A run of text.
    Text,
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
    /// A placement of a reusable definition (FEAT-030).
    Instance,
}

impl ElementKind {
    /// The kind name as it appears in the scene language.
    pub fn as_str(self) -> &'static str {
        match self {
            ElementKind::Rect => "rect",
            ElementKind::Ellipse => "ellipse",
            ElementKind::Polygon => "polygon",
            ElementKind::Line => "line",
            ElementKind::Path => "path",
            ElementKind::Text => "text",
            ElementKind::Group => "group",
            ElementKind::Repeat => "repeat",
            ElementKind::Boolean => "boolean",
            ElementKind::AlongPath => "alongPath",
            ElementKind::Offset => "offset",
            ElementKind::Projection => "projection",
            ElementKind::Raster => "raster",
            ElementKind::Instance => "instance",
        }
    }
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
    /// The string a text element renders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Em size in scene units for a text element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f64>,
    /// Horizontal alignment of a text element's lines about its anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<TextAlign>,
    /// Baseline-to-baseline distance in scene units for a text element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<f64>,
    /// Additional advance between glyphs of a text element, in scene units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<f64>,
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

/// Horizontal alignment of a text element's lines about its anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    /// Lines begin at the anchor.
    Start,
    /// Lines are centred on the anchor.
    Center,
    /// Lines end at the anchor.
    End,
}

impl TextAlign {
    /// The alignment name as it appears in the scene language.
    pub fn as_str(self) -> &'static str {
        match self {
            TextAlign::Start => "start",
            TextAlign::Center => "center",
            TextAlign::End => "end",
        }
    }

    /// Parses an alignment name, or `None` when it is not one of the three.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "start" => Some(TextAlign::Start),
            "center" => Some(TextAlign::Center),
            "end" => Some(TextAlign::End),
            _ => None,
        }
    }
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

/// A named, project-scoped reusable part any scene in the project may place by
/// reference (C-001, FEAT-030).
///
/// A definition is not rendered unless an instance element references it. Its
/// `elements` are the definition's own element tree; each carries this
/// definition's id as its `definitionId`. Because it is project-scoped, editing
/// it changes what every scene that places it renders on the next compile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Definition {
    /// Stable identifier for the definition, unique within its project.
    pub id: String,
    /// References the project this definition belongs to.
    pub project_id: String,
    /// Human-readable definition name.
    pub name: String,
    /// The named inputs the definition accepts; an instance binds each one, or
    /// the parameter's default applies.
    pub parameters: Vec<Parameter>,
    /// The definition's local coordinate frame; an instance places the
    /// definition by transforming this origin.
    pub origin: Origin,
    /// The definition's element tree.
    pub elements: Vec<TemplateElement>,
}

impl Definition {
    /// Finds a declared parameter by name.
    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters
            .iter()
            .find(|parameter| parameter.name == name)
    }

    /// Serializes the definition to compact JSON.
    pub fn to_json_string(&self) -> Result<String, Diagnostics> {
        serde_json::to_string(self).map_err(serialization_failure)
    }

    /// Serializes the definition to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String, Diagnostics> {
        serde_json::to_string_pretty(self).map_err(serialization_failure)
    }
}

/// One named input a definition accepts (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Parameter {
    /// Parameter name an instance binds.
    pub name: String,
    /// The kind of value the parameter accepts.
    #[serde(rename = "type")]
    pub value_type: ParameterType,
    /// Value used when an instance does not bind the parameter; absent or `null`
    /// means the parameter has no default and a binding is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<ParameterValue>,
}

/// The kind of value a parameter accepts (C-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParameterType {
    /// A number, usable in any numeric field.
    Number,
    /// A string, usable in a text or path-data field.
    String,
    /// A boolean, usable in `visible`.
    Boolean,
    /// A palette token name, usable in place of a whole paint.
    Token,
}

impl ParameterType {
    /// The type name as it appears in the language.
    pub fn as_str(self) -> &'static str {
        match self {
            ParameterType::Number => "number",
            ParameterType::String => "string",
            ParameterType::Boolean => "boolean",
            ParameterType::Token => "token",
        }
    }
}

/// A literal parameter default (C-001). A token default is a string naming a
/// palette token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParameterValue {
    /// A numeric default.
    Number(f64),
    /// A string default.
    Text(String),
    /// A boolean default.
    Boolean(bool),
}

/// The definition's local coordinate frame (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Origin {
    /// X of the definition's origin in scene units.
    pub x: f64,
    /// Y of the definition's origin in scene units.
    pub y: f64,
}

/// One element of a definition: a template whose fields may hold parameter
/// references (C-001, FEAT-030).
///
/// Mirrors [`Element`] field for field, except that the fields a parameter may
/// occupy carry a value that is either a literal or a `{ "param": ... }`
/// reference. Expansion resolves each reference against an instance's bindings
/// into a concrete [`Element`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateElement {
    /// Stable identifier for the element, unique within its definition.
    pub id: String,
    /// References the definition this element belongs to.
    pub definition_id: String,
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
    pub geometry: TemplateGeometry,
    /// Affine transform applied to the element and its children.
    pub transform: TemplateTransform,
    /// The element's fill paint, or absent for no fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<PaintValue>,
    /// The element's stroke, or absent for no stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<TemplateStroke>,
    /// Font asset a text element renders with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_id: Option<String>,
    /// Element opacity, from 0 to 1, or a number parameter.
    pub opacity: NumberValue,
    /// Whether the element is rendered, or a boolean parameter.
    pub visible: BoolValue,
    /// References the definition an `instance` element places.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_ref: Option<String>,
    /// Values an instance binds to the placed definition's parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Vec<Binding>>,
    /// Appearance overrides an instance applies to the placed definition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<Override>>,
}

/// A definition element's geometry; numeric and string fields may hold a
/// parameter reference (C-001, FEAT-030).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateGeometry {
    /// X origin in scene units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<NumberValue>,
    /// Y origin in scene units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<NumberValue>,
    /// Width for rect-like elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<NumberValue>,
    /// Height for rect-like elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<NumberValue>,
    /// Corner radius in X.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rx: Option<NumberValue>,
    /// Corner radius in Y.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ry: Option<NumberValue>,
    /// Point list for polygon and line elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f64; 2]>>,
    /// Path data for path elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_data: Option<StringValue>,
    /// The string a text element renders.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<StringValue>,
    /// Em size in scene units for a text element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<NumberValue>,
    /// Horizontal alignment of a text element's lines about its anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<TextAlign>,
    /// Baseline-to-baseline distance in scene units for a text element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_height: Option<NumberValue>,
    /// Additional advance between glyphs of a text element, in scene units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<NumberValue>,
    /// Number of copies for repeat elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<NumberValue>,
    /// Spacing between copies for repeat elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<NumberValue>,
    /// Boolean operation for boolean elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<BooleanOperation>,
    /// Outline offset distance in scene units for offset elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<NumberValue>,
    /// The axes a projection element maps its children onto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<ProjectionAxis>,
}

/// A definition element's transform; its numeric fields may hold a parameter
/// reference (C-001, FEAT-030).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateTransform {
    /// Horizontal translation in scene units.
    pub translate_x: NumberValue,
    /// Vertical translation in scene units.
    pub translate_y: NumberValue,
    /// Rotation in degrees.
    pub rotate: NumberValue,
    /// Horizontal scale factor.
    pub scale_x: NumberValue,
    /// Vertical scale factor.
    pub scale_y: NumberValue,
    /// Horizontal skew in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skew_x: Option<NumberValue>,
    /// Vertical skew in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skew_y: Option<NumberValue>,
}

/// A definition element's stroke: a profile and a paint that may be a token
/// parameter reference (C-001, FEAT-030).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TemplateStroke {
    /// References the stroke profile supplying width, cap and join.
    pub profile_id: String,
    /// The stroke's paint, or a token parameter reference.
    pub paint: PaintValue,
}

/// A number that is either a literal or a parameter reference (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NumberValue {
    /// A literal number.
    Literal(f64),
    /// A reference to a declared number parameter.
    Param(ParamRef),
}

impl NumberValue {
    /// The literal value, or `None` when this is a parameter reference.
    pub fn literal(&self) -> Option<f64> {
        match self {
            NumberValue::Literal(value) => Some(*value),
            NumberValue::Param(_) => None,
        }
    }

    /// The referenced parameter name, or `None` when this is a literal.
    pub fn param(&self) -> Option<&str> {
        match self {
            NumberValue::Param(reference) => Some(&reference.param),
            NumberValue::Literal(_) => None,
        }
    }
}

/// A string that is either a literal or a parameter reference (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringValue {
    /// A literal string.
    Literal(String),
    /// A reference to a declared string parameter.
    Param(ParamRef),
}

impl StringValue {
    /// The literal value, or `None` when this is a parameter reference.
    pub fn literal(&self) -> Option<&str> {
        match self {
            StringValue::Literal(value) => Some(value),
            StringValue::Param(_) => None,
        }
    }

    /// The referenced parameter name, or `None` when this is a literal.
    pub fn param(&self) -> Option<&str> {
        match self {
            StringValue::Param(reference) => Some(&reference.param),
            StringValue::Literal(_) => None,
        }
    }
}

/// A boolean that is either a literal or a parameter reference (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BoolValue {
    /// A literal boolean.
    Literal(bool),
    /// A reference to a declared boolean parameter.
    Param(ParamRef),
}

impl BoolValue {
    /// The literal value, or `None` when this is a parameter reference.
    pub fn literal(&self) -> Option<bool> {
        match self {
            BoolValue::Literal(value) => Some(*value),
            BoolValue::Param(_) => None,
        }
    }

    /// The referenced parameter name, or `None` when this is a literal.
    pub fn param(&self) -> Option<&str> {
        match self {
            BoolValue::Param(reference) => Some(&reference.param),
            BoolValue::Literal(_) => None,
        }
    }
}

/// A paint that is either a literal or a token parameter reference (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PaintValue {
    /// A literal paint.
    Paint(Paint),
    /// A reference to a declared token parameter.
    Param(ParamRef),
}

impl PaintValue {
    /// The literal paint, or `None` when this is a parameter reference.
    pub fn literal(&self) -> Option<&Paint> {
        match self {
            PaintValue::Paint(paint) => Some(paint),
            PaintValue::Param(_) => None,
        }
    }

    /// The referenced parameter name, or `None` when this is a literal.
    pub fn param(&self) -> Option<&str> {
        match self {
            PaintValue::Param(reference) => Some(&reference.param),
            PaintValue::Paint(_) => None,
        }
    }
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
