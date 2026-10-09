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
    /// Seed governing all generated geometry; when absent a default of `0`
    /// applies and is recorded in the render model (FEAT-006).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_seed"
    )]
    pub seed: Option<i64>,
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

/// One drawing entity: a primitive, a group, a composition operation, a text
/// run, or a placement of a reusable definition (C-001, FEAT-030).
///
/// One shape serves both a scene and a definition. A parameter-capable field
/// holds either a literal of the field's type or a [`ParamRef`]; a reference is
/// valid only where the element is owned by a definition (D-038).
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
    /// Element name, the maintenance name preserved in exported output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Accessible name for assistive technology, carried into exported output
    /// and distinct from the maintenance name in `name` (FEAT-026).
    ///
    /// When absent the element emits no accessible name; a text element's
    /// string is carried as accessible text whether or not one is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessible_name: Option<String>,
    /// The element's role.
    pub kind: ElementKind,
    /// The element's geometry; which fields apply depends on `kind`.
    pub geometry: Geometry,
    /// Affine transform applied to the element and its children.
    pub transform: Transform,
    /// The element's fill paint, a token parameter reference, or absent for no
    /// fill (C-001).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<PaintValue>,
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
    /// Element opacity, a literal or a number parameter reference.
    pub opacity: NumberValue,
    /// Whether the element is rendered, a literal or a boolean parameter
    /// reference.
    pub visible: BoolValue,
    /// References the definition an `instance` element places; required when
    /// `kind` is `instance` and null otherwise (C-001, FEAT-030).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_ref: Option<String>,
    /// Values an instance binds to the placed definition's parameters; applies
    /// only to an instance (C-001, FEAT-030).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bindings: Option<Vec<Binding>>,
}

impl Element {
    /// The element's opacity as a literal, or `1.0` when it is a parameter
    /// reference (a definition element resolves before compilation).
    pub fn opacity(&self) -> f64 {
        self.opacity.literal().unwrap_or(1.0)
    }

    /// Whether the element is visible as a literal, or `true` when it is a
    /// parameter reference.
    pub fn visible(&self) -> bool {
        self.visible.literal().unwrap_or(true)
    }
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
    /// The stroke's paint, or a token parameter reference.
    pub paint: PaintValue,
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
    /// Seeded procedural generation over the element's children (FEAT-006).
    Procedural,
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
            ElementKind::Procedural => "procedural",
            ElementKind::Raster => "raster",
            ElementKind::Instance => "instance",
        }
    }
}

/// The element's geometry; which fields apply depends on the element's kind.
///
/// A numeric or string field holds either a literal of its type or a parameter
/// reference; `points`, `align`, `operation`, and `axis` hold their literal
/// value only (C-001, D-038).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Geometry {
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
    /// Outline offset distance in scene units for offset elements; positive
    /// offsets outward and negative inward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<NumberValue>,
    /// The axes a projection element maps its children onto.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<ProjectionAxis>,
    /// The generation a procedural element applies to its children (FEAT-006).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub procedure: Option<Procedure>,
    /// Maximum displacement or feature amplitude for a procedural element, in
    /// scene units; a literal number or a number parameter reference
    /// (FEAT-006).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<NumberValue>,
}

impl Geometry {
    /// The literal `x`, or `None` when it is a parameter reference.
    pub fn x(&self) -> Option<f64> {
        self.x.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `y`, or `None` when it is a parameter reference.
    pub fn y(&self) -> Option<f64> {
        self.y.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `width`, or `None` when it is a parameter reference.
    pub fn width(&self) -> Option<f64> {
        self.width.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `height`, or `None` when it is a parameter reference.
    pub fn height(&self) -> Option<f64> {
        self.height.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `rx`, or `None` when it is a parameter reference.
    pub fn rx(&self) -> Option<f64> {
        self.rx.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `ry`, or `None` when it is a parameter reference.
    pub fn ry(&self) -> Option<f64> {
        self.ry.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `pathData`, or `None` when it is a parameter reference.
    pub fn path_data(&self) -> Option<&str> {
        self.path_data.as_ref().and_then(StringValue::literal)
    }

    /// The literal `text`, or `None` when it is a parameter reference.
    pub fn text(&self) -> Option<&str> {
        self.text.as_ref().and_then(StringValue::literal)
    }

    /// The literal `fontSize`, or `None` when it is a parameter reference.
    pub fn font_size(&self) -> Option<f64> {
        self.font_size.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `lineHeight`, or `None` when it is a parameter reference.
    pub fn line_height(&self) -> Option<f64> {
        self.line_height.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `letterSpacing`, or `None` when it is a parameter reference.
    pub fn letter_spacing(&self) -> Option<f64> {
        self.letter_spacing.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `count` as a whole number, or `None` when it is a parameter
    /// reference.
    pub fn count(&self) -> Option<u32> {
        self.count
            .as_ref()
            .and_then(NumberValue::literal)
            .map(|value| value.max(0.0) as u32)
    }

    /// The literal `spacing`, or `None` when it is a parameter reference.
    pub fn spacing(&self) -> Option<f64> {
        self.spacing.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `distance`, or `None` when it is a parameter reference.
    pub fn distance(&self) -> Option<f64> {
        self.distance.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `amount`, or `None` when it is a parameter reference.
    pub fn amount(&self) -> Option<f64> {
        self.amount.as_ref().and_then(NumberValue::literal)
    }
}

/// The generation a procedural element applies to its children (FEAT-006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Procedure {
    /// Decomposes the region enclosed by the children's outlines into triangles.
    Triangulation,
    /// Distributes copies of the first child across the region the rest enclose.
    Scatter,
    /// Displaces each child's geometry.
    Jitter,
    /// Distributes points within the region the children enclose.
    Stippling,
    /// Generates ornamental features along the children's outlines.
    Ornament,
}

impl Procedure {
    /// The procedure name as it appears in the scene language.
    pub fn as_str(self) -> &'static str {
        match self {
            Procedure::Triangulation => "triangulation",
            Procedure::Scatter => "scatter",
            Procedure::Jitter => "jitter",
            Procedure::Stippling => "stippling",
            Procedure::Ornament => "ornament",
        }
    }

    /// Parses a procedure name, or `None` when it is not one of the five.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "triangulation" => Some(Procedure::Triangulation),
            "scatter" => Some(Procedure::Scatter),
            "jitter" => Some(Procedure::Jitter),
            "stippling" => Some(Procedure::Stippling),
            "ornament" => Some(Procedure::Ornament),
            _ => None,
        }
    }
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
///
/// Each field holds either a literal number or a number parameter reference
/// (C-001, D-038).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transform {
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

impl Transform {
    /// The literal `translateX`, or `0.0` when it is a parameter reference.
    pub fn translate_x(&self) -> f64 {
        self.translate_x.literal().unwrap_or(0.0)
    }

    /// The literal `translateY`, or `0.0` when it is a parameter reference.
    pub fn translate_y(&self) -> f64 {
        self.translate_y.literal().unwrap_or(0.0)
    }

    /// The literal `rotate`, or `0.0` when it is a parameter reference.
    pub fn rotate(&self) -> f64 {
        self.rotate.literal().unwrap_or(0.0)
    }

    /// The literal `scaleX`, or `1.0` when it is a parameter reference.
    pub fn scale_x(&self) -> f64 {
        self.scale_x.literal().unwrap_or(1.0)
    }

    /// The literal `scaleY`, or `1.0` when it is a parameter reference.
    pub fn scale_y(&self) -> f64 {
        self.scale_y.literal().unwrap_or(1.0)
    }

    /// The literal `skewX`, or `None` when it is a parameter reference.
    pub fn skew_x(&self) -> Option<f64> {
        self.skew_x.as_ref().and_then(NumberValue::literal)
    }

    /// The literal `skewY`, or `None` when it is a parameter reference.
    pub fn skew_y(&self) -> Option<f64> {
        self.skew_y.as_ref().and_then(NumberValue::literal)
    }

    /// Replaces the translation with literal values.
    pub fn set_translation(&mut self, x: f64, y: f64) {
        self.translate_x = NumberValue::Literal(x);
        self.translate_y = NumberValue::Literal(y);
    }
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
    pub elements: Vec<Element>,
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

/// A project-scoped document that generates a set of icons sharing one canvas,
/// palette, recipe, stroke profile, and naming scheme (C-001, FEAT-025).
///
/// Each icon is a reusable [`Definition`] the set places. The set renders every
/// icon on its own under the shared canvas and style rather than the project's
/// isolated-definition defaults, and exports each to a file named by
/// [`IconSet::file_name_for`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IconSet {
    /// Stable identifier for the icon set, unique within its project.
    pub id: String,
    /// References the project this icon set belongs to.
    pub project_id: String,
    /// Human-readable icon-set name, at most 120 characters.
    pub name: String,
    /// The drawing surface shared by every icon in the set.
    pub canvas: Canvas,
    /// References the palette the set's icons resolve against; when absent, the
    /// project's `defaultPaletteId` applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette_id: Option<String>,
    /// References the recipe the set's icons render under; when absent, the
    /// project's `defaultRecipeId` applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<String>,
    /// References the stroke profile every icon in the set draws with; an icon
    /// element's stroke must name this profile.
    pub stroke_profile_id: String,
    /// The naming scheme for the set's exported files: a pattern containing the
    /// placeholder `{name}`, replaced by each icon's name. Defaults to `{name}`.
    #[serde(default = "default_name_pattern")]
    pub name_pattern: String,
    /// The icons in the set; an empty list is invalid.
    pub icons: Vec<IconEntry>,
}

impl IconSet {
    /// The naming pattern, which defaults to `{name}` when none is declared.
    pub fn name_pattern(&self) -> &str {
        &self.name_pattern
    }

    /// The file name one icon exports to: the naming pattern with `{name}`
    /// replaced by the icon's name and the format extension appended.
    pub fn file_name_for(&self, icon: &IconEntry, extension: &str) -> String {
        format!(
            "{}.{}",
            self.name_pattern.replace("{name}", &icon.name),
            extension
        )
    }
}

/// The default icon naming pattern (C-001, FEAT-025).
pub fn default_name_pattern() -> String {
    "{name}".to_string()
}

/// One icon in a set: a named placement of a reusable definition (C-001).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IconEntry {
    /// The icon's name, unique within the set and substituted for `{name}` in
    /// the set's naming pattern to name the exported file.
    pub name: String,
    /// References the reusable definition that draws the icon.
    pub definition_ref: String,
    /// Accessible name carried into the icon's exported output, distinct from
    /// the icon's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessible_name: Option<String>,
}

/// A number that is either a literal or a parameter reference (C-001).
///
/// Deserialization is hand-written rather than an untagged enum so an invalid
/// value is reported by name — `invalid type: string "sideways", expected a
/// number or a parameter reference` — instead of a generic variant mismatch
/// (FEAT-018).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum NumberValue {
    /// A literal number.
    Literal(f64),
    /// A reference to a declared number parameter.
    Param(ParamRef),
}

impl<'de> Deserialize<'de> for NumberValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct NumberVisitor;

        impl<'de> serde::de::Visitor<'de> for NumberVisitor {
            type Value = NumberValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a number or a parameter reference")
            }

            fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<NumberValue, E> {
                Ok(NumberValue::Literal(value))
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<NumberValue, E> {
                Ok(NumberValue::Literal(value as f64))
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<NumberValue, E> {
                Ok(NumberValue::Literal(value as f64))
            }

            fn visit_map<A>(self, map: A) -> Result<NumberValue, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let reference =
                    ParamRef::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(NumberValue::Param(reference))
            }
        }

        deserializer.deserialize_any(NumberVisitor)
    }
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
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum StringValue {
    /// A literal string.
    Literal(String),
    /// A reference to a declared string parameter.
    Param(ParamRef),
}

impl<'de> Deserialize<'de> for StringValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct StringVisitor;

        impl<'de> serde::de::Visitor<'de> for StringVisitor {
            type Value = StringValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a string or a parameter reference")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<StringValue, E> {
                Ok(StringValue::Literal(value.to_string()))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<StringValue, E> {
                Ok(StringValue::Literal(value))
            }

            fn visit_map<A>(self, map: A) -> Result<StringValue, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let reference =
                    ParamRef::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(StringValue::Param(reference))
            }
        }

        deserializer.deserialize_any(StringVisitor)
    }
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
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum BoolValue {
    /// A literal boolean.
    Literal(bool),
    /// A reference to a declared boolean parameter.
    Param(ParamRef),
}

impl<'de> Deserialize<'de> for BoolValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct BoolVisitor;

        impl<'de> serde::de::Visitor<'de> for BoolVisitor {
            type Value = BoolValue;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a boolean or a parameter reference")
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<BoolValue, E> {
                Ok(BoolValue::Literal(value))
            }

            fn visit_map<A>(self, map: A) -> Result<BoolValue, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let reference =
                    ParamRef::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(BoolValue::Param(reference))
            }
        }

        deserializer.deserialize_any(BoolVisitor)
    }
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
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PaintValue {
    /// A literal paint.
    Paint(Paint),
    /// A reference to a declared token parameter.
    Param(ParamRef),
}

impl<'de> Deserialize<'de> for PaintValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// The union of a paint's fields and a parameter reference's field.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Fields {
            #[serde(default)]
            kind: Option<PaintKind>,
            #[serde(default, rename = "ref")]
            reference: Option<String>,
            #[serde(default)]
            param: Option<String>,
        }

        let fields = Fields::deserialize(deserializer)?;
        match (fields.kind, fields.reference, fields.param) {
            (Some(kind), Some(reference), None) => Ok(PaintValue::Paint(Paint { kind, reference })),
            (None, None, Some(param)) => Ok(PaintValue::Param(ParamRef { param })),
            _ => Err(serde::de::Error::custom(
                "a paint needs `kind` and `ref`, or a parameter reference needs `param`",
            )),
        }
    }
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

    /// The literal paint mutably, or `None` when this is a parameter reference.
    pub fn literal_mut(&mut self) -> Option<&mut Paint> {
        match self {
            PaintValue::Paint(paint) => Some(paint),
            PaintValue::Param(_) => None,
        }
    }
}

/// Reads a scene seed, reporting a value that is not an integer by name
/// (FEAT-006).
///
/// The seed is a 64-bit integer; a fractional value, a string or any other
/// type is a schema error whose message names the invalid value, rather than a
/// generic type mismatch. An integral number written with a fractional part
/// (`2.0`) is accepted as the integer it denotes; `null` is treated as absent.
fn deserialize_seed<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                return Ok(Some(integer));
            }
            if let Some(unsigned) = number.as_u64() {
                return i64::try_from(unsigned)
                    .map(Some)
                    .map_err(|_| Error::custom(format!("`seed` {number} is out of range")));
            }
            if let Some(float) = number.as_f64() {
                if float.fract() == 0.0 && float >= i64::MIN as f64 && float <= i64::MAX as f64 {
                    return Ok(Some(float as i64));
                }
            }
            Err(Error::custom(format!(
                "`seed` must be an integer, got {number}"
            )))
        }
        other => Err(Error::custom(format!(
            "`seed` must be an integer, got {other}"
        ))),
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
