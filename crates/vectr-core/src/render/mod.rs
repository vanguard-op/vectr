//! The render model: the compiler's output and the single input every exporter
//! reads (C-003).
//!
//! A [`RenderModel`] is a flat, fully resolved list of drawing nodes — concrete
//! geometry, a resolved world transform, paint, and paint order — with no
//! reference left for an exporter to chase (FEAT-011). Compiling the same scene
//! twice yields an identical model, because every value is concrete and the node
//! order is fixed (NFR-010).
//!
//! Geometry reuses [`Shape`], the concrete type the primitive and composition
//! layers already produce, and [`Affine`] carries the resolved world transform.
//! A node's `kind` names the scene element that produced it; where a composition
//! expands one element into several nodes (a repeat), each node carries a stable
//! identifier derived from the element it came from.

use serde::{Deserialize, Serialize};

use crate::composition::Affine;
use crate::primitives::Shape;
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, Location};
use crate::style::{StrokeCap, StrokeJoin};

/// The compiled result of a scene, ready for any exporter (C-003).
///
/// The model serializes to camelCase JSON and back (D-014), so `vectr compile
/// --out` and the MCP compile tool can hand it across a process or tool
/// boundary as data while every exporter still reads the same in-process type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderModel {
    /// The drawing surface.
    pub canvas: RenderCanvas,
    /// The drawing nodes, in paint order.
    pub nodes: Vec<ResolvedNode>,
    /// Accessible metadata carried from the scene.
    #[serde(default)]
    pub meta: RenderMeta,
    /// Findings recorded while compiling: warnings only on success.
    #[serde(default)]
    pub diagnostics: Diagnostics,
}

impl RenderModel {
    /// Finds a node by its resolved identifier.
    pub fn node(&self, id: &str) -> Option<&ResolvedNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// Serializes the model to compact camelCase JSON.
    pub fn to_json_string(&self) -> Result<String, Diagnostics> {
        serde_json::to_string(self).map_err(serialization_failure)
    }

    /// Serializes the model to pretty camelCase JSON.
    pub fn to_json_pretty(&self) -> Result<String, Diagnostics> {
        serde_json::to_string_pretty(self).map_err(serialization_failure)
    }
}

/// Reads a render model from its camelCase JSON (D-014).
///
/// The model is the compiler's output, not untrusted scene input, so this is a
/// plain structural read: malformed JSON is an [`DiagnosticCode::PARSE`] error
/// and a document of the wrong shape is a [`DiagnosticCode::SCHEMA`] error, each
/// carrying its line and column.
pub fn parse(source: &str) -> Result<RenderModel, Diagnostics> {
    let value: serde_json::Value = serde_json::from_str(source)
        .map_err(|error| diagnostics_from_serde(DiagnosticCode::PARSE, &error))?;
    if !value.is_object() {
        return Err(Diagnostics::from(Diagnostic::error(
            DiagnosticCode::PARSE,
            "not a render model: the top level must be a JSON object",
        )));
    }
    serde_json::from_str(source)
        .map_err(|error| diagnostics_from_serde(DiagnosticCode::SCHEMA, &error))
}

fn serialization_failure(error: serde_json::Error) -> Diagnostics {
    Diagnostics::from(Diagnostic::error(
        DiagnosticCode::SCHEMA,
        format!("could not serialize the render model: {error}"),
    ))
}

fn diagnostics_from_serde(code: DiagnosticCode, error: &serde_json::Error) -> Diagnostics {
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

/// The drawing surface carried into output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderCanvas {
    /// Canvas width in scene units.
    pub width: f64,
    /// Canvas height in scene units.
    pub height: f64,
    /// Canvas background color, or `transparent`.
    pub background: String,
}

/// Accessible metadata carried into exported output.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderMeta {
    /// Accessible title, when the scene declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Accessible description, when the scene declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One concrete drawing node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedNode {
    /// Stable identifier; unique across the model.
    pub id: String,
    /// Element name, preserved for the exporter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Position in the model's paint order; lower values paint first.
    pub order: usize,
    /// The scene-language kind that produced the node.
    pub kind: String,
    /// The node's concrete geometry, in its own local coordinates.
    pub geometry: Shape,
    /// The resolved world transform applied to the geometry.
    pub transform: Affine,
    /// The node's fill and stroke.
    pub paint: Paint,
    /// The node's effective opacity, from 0 to 1, composed down the element tree.
    pub opacity: f64,
    /// Whether the node is visible, composed down the element tree.
    pub visible: bool,
}

/// The paint applied to a node.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paint {
    /// The resolved fill color, or the scene's declared token name when the
    /// model was compiled without a palette; absent for no fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// The resolved stroke, or absent for no stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<NodeStroke>,
}

/// A resolved stroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStroke {
    /// The stroke paint value.
    pub value: String,
    /// Stroke width in scene units.
    pub width: f64,
    /// Stroke line cap.
    pub cap: StrokeCap,
    /// Stroke line join.
    pub join: StrokeJoin,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::compile;
    use crate::scene::parse as parse_scene;

    /// A scene with a styled rect and an arc path, so the model carries a
    /// resolved transform, concrete geometry, paint, and a warning finding.
    const SCENE: &str = r##"{
      "id": "s",
      "projectId": "p",
      "name": "S",
      "formatVersion": "0.1",
      "canvas": { "width": 400, "height": 400, "background": "#ffffff" },
      "elements": [
        {
          "id": "e1", "sceneId": "s", "order": 0, "kind": "rect",
          "geometry": { "x": 0, "y": 0, "width": 30, "height": 40 },
          "transform": { "translateX": 5, "translateY": 6, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "fillToken": "accent", "strokeProfileId": "stroke-1", "strokeToken": "accent",
          "opacity": 1, "visible": true
        },
        {
          "id": "p1", "sceneId": "s", "order": 1, "kind": "path",
          "geometry": { "pathData": "M0 0 A5 5 0 0 1 10 0" },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    fn model() -> RenderModel {
        compile(&parse_scene(SCENE).expect("a valid scene")).expect("compiles")
    }

    #[test]
    fn a_compiled_model_round_trips_through_json() {
        let original = model();
        let text = original.to_json_string().expect("serializable");
        let reparsed = parse(&text).expect("deserializable");
        assert_eq!(original, reparsed);
    }

    #[test]
    fn the_model_serializes_to_camel_case() {
        let text = model().to_json_string().expect("serializable");
        assert!(text.contains("\"kind\":\"rect\""), "{text}");
        assert!(text.contains("\"kind\":\"arc\""), "{text}");
        assert!(text.contains("\"xRotation\""), "{text}");
        assert!(text.contains("\"elementId\":\"e1\""), "{text}");
    }

    #[test]
    fn a_node_carries_its_concrete_geometry_and_resolved_transform() {
        let text = model().to_json_string().expect("serializable");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let node = &value["nodes"][0];
        assert_eq!(node["geometry"]["width"], 30.0);
        assert_eq!(node["geometry"]["height"], 40.0);
        assert_eq!(node["transform"]["e"], 5.0);
        assert_eq!(node["transform"]["f"], 6.0);
        assert_eq!(node["paint"]["fill"], "accent");
    }

    #[test]
    fn serialization_is_deterministic() {
        let first = model().to_json_string().unwrap();
        let second = model().to_json_string().unwrap();
        assert_eq!(first, second, "serialization must be stable (NFR-010)");
    }

    #[test]
    fn malformed_json_is_a_parse_error_with_a_location() {
        let diagnostics = parse("{ not json").expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::PARSE);
        assert!(error.location.as_ref().and_then(|l| l.line).is_some());
    }

    #[test]
    fn a_non_object_document_is_a_parse_error() {
        let diagnostics = parse("[1, 2, 3]").expect_err("refused");
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::PARSE)
        );
    }

    #[test]
    fn a_document_of_the_wrong_shape_is_a_schema_error() {
        let text = model().to_json_string().unwrap().replace(
            "\"geometry\":{\"kind\":\"rect\"",
            "\"geometry\":{\"kind\":\"nope\"",
        );
        let diagnostics = parse(&text).expect_err("refused");
        assert_eq!(
            diagnostics.errors().next().map(|d| d.code.clone()),
            Some(DiagnosticCode::SCHEMA)
        );
    }
}
