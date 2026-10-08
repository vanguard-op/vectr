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
//! identifier derived from the element it came from. A node's `groups` carry its
//! ancestor group chain, outermost first, so an exporter can rebuild the
//! named-group nesting the scene declared (FEAT-011, FEAT-012).

use serde::{Deserialize, Serialize};

use crate::composition::Affine;
use crate::primitives::Shape;
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, Location, TextAlign};
use crate::style::{GradientType, Spread, StrokeCap, StrokeJoin};

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
    /// The resolved fonts the model's text nodes name, each carrying its file
    /// data so an exporter can finalize glyph geometry with no external state
    /// (C-003, FEAT-011, FEAT-024).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<ResolvedFont>,
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
    /// The style recipe the scene was compiled with, when one was supplied
    /// (C-003, FEAT-007).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
}

/// One ancestor group in a node's chain (C-003).
///
/// A group element is a container: it produces no node of its own, but its
/// identity and name travel on every descendant node so an exporter can rebuild
/// the named-group nesting (FEAT-011, FEAT-012). An unnamed group is still
/// carried, with no name, so an exporter may collapse it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeGroup {
    /// The group element's stable identifier.
    pub id: String,
    /// The group's name, when it declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// The node's ancestor group chain, outermost first; empty for a root
    /// element. Reading a model without the field yields an empty chain.
    #[serde(default)]
    pub groups: Vec<NodeGroup>,
    /// The node's concrete geometry, in its own local coordinates. Absent for a
    /// text node, whose glyph geometry the exporter finalizes from [`text`]
    /// (C-003).
    ///
    /// [`text`]: ResolvedNode::text
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geometry: Option<Shape>,
    /// The text run a text node carries; absent for every other node. A text
    /// node carries `text` and no `geometry` (C-003).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<TextRun>,
    /// The resolved world transform applied to the geometry or text.
    pub transform: Affine,
    /// The node's fill and stroke.
    pub paint: NodePaint,
    /// The node's effective opacity, from 0 to 1, composed down the element tree.
    pub opacity: f64,
    /// Whether the node is visible, composed down the element tree.
    pub visible: bool,
}

/// The paint applied to a node (C-003).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodePaint {
    /// The resolved fill, or absent for no fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Paint>,
    /// The resolved stroke, or absent for no stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<NodeStroke>,
}

/// A fully resolved paint: a concrete colour, or a gradient whose stop colours
/// and geometry are concrete (C-003).
///
/// No unresolved colour or gradient reference survives compilation: a palette
/// token becomes a [`Paint::Color`], and a gradient becomes a
/// [`Paint::Gradient`] carrying concrete stops.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Paint {
    /// A concrete colour.
    Color {
        /// The colour value, such as a hex colour.
        value: String,
    },
    /// A gradient paint.
    Gradient(GradientPaint),
}

/// A resolved gradient: concrete stop colours and concrete geometry, in object
/// bounding-box units (C-003).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradientPaint {
    /// Whether the gradient is linear or radial.
    #[serde(rename = "type")]
    pub gradient_type: GradientType,
    /// The concrete colour stops, in ascending offset order; two or more.
    pub stops: Vec<ResolvedStop>,
    /// How the gradient extends beyond its ends.
    pub spread: Spread,
    /// Start x of a linear gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x1: Option<f64>,
    /// Start y of a linear gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y1: Option<f64>,
    /// End x of a linear gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x2: Option<f64>,
    /// End y of a linear gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y2: Option<f64>,
    /// Center x of a radial gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cx: Option<f64>,
    /// Center y of a radial gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cy: Option<f64>,
    /// Radius of a radial gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r: Option<f64>,
    /// Focal x of a radial gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx: Option<f64>,
    /// Focal y of a radial gradient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fy: Option<f64>,
}

/// One resolved gradient colour stop (C-003).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedStop {
    /// Position along the gradient vector, from 0 to 1.
    pub offset: f64,
    /// The concrete stop colour.
    pub color: String,
    /// Stop opacity, from 0 to 1.
    pub opacity: f64,
}

/// A resolved stroke.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStroke {
    /// The stroke's resolved paint.
    pub paint: Paint,
    /// Stroke width in scene units.
    pub width: f64,
    /// Stroke line cap.
    pub cap: StrokeCap,
    /// Stroke line join.
    pub join: StrokeJoin,
}

/// A text node's run: its string and resolved layout, with no glyph geometry
/// (C-003, FEAT-011).
///
/// The compiler resolves the element's declared font and layout into these
/// concrete values and bakes the anchor into the node's transform, so an
/// exporter only has to turn the string into glyph outlines with the font named
/// by `font_id` (FEAT-024).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    /// The string the text element renders.
    pub value: String,
    /// The resolved font's identifier, naming an entry in [`RenderModel::fonts`].
    pub font_id: String,
    /// Em size in scene units.
    pub font_size: f64,
    /// Horizontal alignment of the run's lines about the anchor.
    pub align: TextAlign,
    /// Baseline-to-baseline distance in scene units.
    pub line_height: f64,
    /// Additional advance between glyphs, in scene units.
    pub letter_spacing: f64,
    /// The wrap width in scene units, when the element declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
}

/// A font the model's text nodes render with (C-003).
///
/// The font file travels with the model so an exporter finalizes glyph geometry
/// with no external state; in JSON `data` is base64-encoded, keeping the render
/// model plain text (D-014).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedFont {
    /// The font's stable identifier.
    pub id: String,
    /// The font's human-readable name.
    pub name: String,
    /// The font file's bytes, base64-encoded in JSON.
    #[serde(with = "font_data")]
    pub data: Vec<u8>,
}

impl ResolvedFont {
    /// The font's file size in bytes.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the font file is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Serde adapter that carries font bytes as base64 text (C-003).
mod font_data {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&super::base64::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        super::base64::decode(&text).map_err(serde::de::Error::custom)
    }
}

/// A small, dependency-free base64 codec (standard alphabet, padded), used only
/// to carry font bytes through the render model's JSON (C-003).
mod base64 {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b0 = u32::from(chunk[0]);
            let b1 = chunk.get(1).copied().map(u32::from).unwrap_or(0);
            let b2 = chunk.get(2).copied().map(u32::from).unwrap_or(0);
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
            out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
            out.push(if chunk.len() > 1 {
                ALPHABET[((triple >> 6) & 0x3f) as usize] as char
            } else {
                '='
            });
            out.push(if chunk.len() > 2 {
                ALPHABET[(triple & 0x3f) as usize] as char
            } else {
                '='
            });
        }
        out
    }

    pub fn decode(text: &str) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(text.len() / 4 * 3);
        let mut buffer = 0u32;
        let mut bits = 0u32;
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'=' {
                break;
            }
            let value = value_of(byte)
                .ok_or_else(|| format!("invalid base64 character at offset {index}"))?;
            buffer = (buffer << 6) | u32::from(value);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push(((buffer >> bits) & 0xff) as u8);
            }
        }
        Ok(out)
    }

    fn value_of(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
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
      "formatVersion": "0.2",
      "canvas": { "width": 400, "height": 400, "background": "#ffffff" },
      "elements": [
        {
          "id": "e1", "sceneId": "s", "order": 0, "kind": "rect",
          "geometry": { "x": 0, "y": 0, "width": 30, "height": 40 },
          "transform": { "translateX": 5, "translateY": 6, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "fill": { "kind": "token", "ref": "accent" },
          "stroke": { "profileId": "stroke-1", "paint": { "kind": "token", "ref": "accent" } },
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
    fn a_node_serializes_its_ancestor_group_chain() {
        const GROUPED: &str = r##"{
          "id": "s",
          "projectId": "p",
          "name": "S",
          "formatVersion": "0.2",
          "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
          "elements": [
            {
              "id": "g1", "sceneId": "s", "order": 0, "kind": "group", "name": "Outer",
              "geometry": {},
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            },
            {
              "id": "e1", "sceneId": "s", "order": 0, "kind": "rect", "parentId": "g1",
              "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            }
          ]
        }"##;
        let compiled = compile(&parse_scene(GROUPED).expect("a valid scene")).expect("compiles");
        let text = compiled.to_json_string().expect("serializable");
        assert!(
            text.contains("\"groups\":[{\"id\":\"g1\",\"name\":\"Outer\"}]"),
            "{text}"
        );
        let reparsed = parse(&text).expect("deserializable");
        assert_eq!(
            reparsed.nodes[0].groups,
            vec![NodeGroup {
                id: "g1".to_string(),
                name: Some("Outer".to_string()),
            }]
        );
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
        assert_eq!(node["paint"]["fill"]["kind"], "color");
        assert_eq!(node["paint"]["fill"]["value"], "accent");
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

    #[test]
    fn font_bytes_serialize_as_base64_and_round_trip() {
        let font = ResolvedFont {
            id: "body".to_string(),
            name: "Body".to_string(),
            data: vec![0, 1, 2, 253, 254, 255],
        };
        let text = serde_json::to_string(&font).unwrap();
        assert!(text.contains("\"data\":\"AAEC/f7/\""), "{text}");
        let reparsed: ResolvedFont = serde_json::from_str(&text).unwrap();
        assert_eq!(font, reparsed);
    }

    #[test]
    fn a_text_node_carries_text_and_no_geometry_in_json() {
        let model = RenderModel {
            canvas: RenderCanvas {
                width: 100.0,
                height: 50.0,
                background: "#ffffff".to_string(),
            },
            nodes: vec![ResolvedNode {
                id: "t1".to_string(),
                name: None,
                order: 0,
                kind: "text".to_string(),
                groups: Vec::new(),
                geometry: None,
                text: Some(TextRun {
                    value: "Hi".to_string(),
                    font_id: "body".to_string(),
                    font_size: 12.0,
                    align: TextAlign::Start,
                    line_height: 12.0,
                    letter_spacing: 0.0,
                    width: None,
                }),
                transform: crate::composition::Affine::IDENTITY,
                paint: NodePaint::default(),
                opacity: 1.0,
                visible: true,
            }],
            meta: RenderMeta::default(),
            diagnostics: Diagnostics::new(),
            fonts: vec![ResolvedFont {
                id: "body".to_string(),
                name: "Body".to_string(),
                data: vec![1, 2, 3],
            }],
        };

        let text = model.to_json_string().expect("serializable");
        assert!(text.contains("\"text\":{\"value\":\"Hi\""), "{text}");
        assert!(!text.contains("\"geometry\""), "{text}");
        assert!(text.contains("\"fontId\":\"body\""), "{text}");
        assert_eq!(parse(&text).expect("deserializable"), model);
    }
}
