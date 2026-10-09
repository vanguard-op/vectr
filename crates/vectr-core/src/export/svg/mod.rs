//! SVG export: the render model as a portable, inert SVG document (FEAT-012).
//!
//! [`export_svg`] is the library entry point (C-002). It walks the model's flat
//! node list in paint order, rebuilding the named-group nesting each node's
//! ancestor chain declares so every named group survives at its own level, and
//! emits one group per node carrying the node's stable identifier and, when the
//! scene named it, its human-readable name; the concrete geometry becomes a
//! native SVG shape and the resolved paint becomes fill and stroke attributes.
//! An unnamed group is collapsed, which FEAT-012 permits. The document carries
//! no script, event handler, or foreign content, so it is inert when opened
//! (NFR-023), and identical input yields byte-identical output (NFR-010).
//!
//! # Background
//!
//! The one background the document draws is validated before emission: the
//! override from [`SvgOptions`] when present, otherwise the canvas's own. A
//! value that is neither a colour SVG supports nor one of the transparent
//! sentinels the options document is a located error and no document, so an
//! unusable colour is refused at validation rather than handed to a rasterizer
//! to reject (FEAT-018).
//!
//! # Unsupported content
//!
//! A node the SVG target cannot represent — a raster layer, or geometry whose
//! coordinates are not finite — is omitted rather than emitted corrupted, and
//! the omission is reported as a warning naming the node (C-003). The frozen
//! [`export_svg`] signature has no channel for those warnings, so
//! [`export_svg_reporting`] returns them alongside the document; the CLI and
//! other callers that surface warnings use it, while [`export_svg`] remains the
//! frozen shape.

use crate::render::RenderModel;
use crate::scene::{validate_color, Diagnostic, DiagnosticCode, Diagnostics};

mod emit;

/// A node the SVG target cannot represent, omitted with a warning.
pub const UNSUPPORTED: DiagnosticCode = DiagnosticCode::new("W_UNSUPPORTED_SVG");

/// The requested output size is not a usable dimension.
pub const OPTIONS: DiagnosticCode = DiagnosticCode::new("E_SVG_OPTIONS");

/// The scene declares no accessible metadata (FEAT-026).
pub const MISSING_METADATA: DiagnosticCode = DiagnosticCode::new("W_MISSING_METADATA");

/// Options controlling SVG output.
///
/// The defaults reproduce the canvas exactly: no size override and the canvas's
/// own background. A width or height alone scales the other dimension to keep
/// the canvas aspect ratio, while the view box stays in canvas coordinates.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SvgOptions {
    /// Output width in user units; the canvas width when absent.
    pub width: Option<f64>,
    /// Output height in user units; the canvas height when absent.
    pub height: Option<f64>,
    /// Background override; the canvas background when absent. `transparent`
    /// (or `none`, or empty) omits the background entirely.
    pub background: Option<String>,
}

/// An exported SVG document and the findings recorded while writing it.
#[derive(Debug, Clone, PartialEq)]
pub struct SvgExport {
    /// The SVG text.
    pub svg: String,
    /// Warnings for content the target could not represent; errors never reach
    /// this type, because an export that cannot proceed returns `Err` instead.
    pub diagnostics: Diagnostics,
}

/// Exports a render model as SVG (C-002).
///
/// The document is returned as text; a request the exporter cannot satisfy is a
/// located diagnostic and no partial document. Content the SVG target cannot
/// represent is omitted; use [`export_svg_reporting`] to observe those warnings.
pub fn export_svg(model: &RenderModel, options: &SvgOptions) -> Result<String, Diagnostics> {
    export_svg_reporting(model, options).map(|export| export.svg)
}

/// Exports a render model as SVG, returning the warnings alongside the document.
///
/// This is the same emission as [`export_svg`]; it exists because the frozen
/// signature returns only the document and would otherwise drop the warnings an
/// exporter records when it omits content the target cannot represent (C-003).
pub fn export_svg_reporting(
    model: &RenderModel,
    options: &SvgOptions,
) -> Result<SvgExport, Diagnostics> {
    let size = resolve_size(model, options)?;
    let (background, from_options) = match options.background.as_deref() {
        Some(value) => (value, true),
        None => (model.canvas.background.as_str(), false),
    };

    let mut diagnostics = Diagnostics::new();
    validate_background(&mut diagnostics, background, from_options);
    if diagnostics.has_errors() {
        return Err(diagnostics);
    }

    let svg = emit::document(model, &size, background, &mut diagnostics);
    if diagnostics.has_errors() {
        return Err(diagnostics);
    }
    warn_missing_metadata(model, &mut diagnostics);

    Ok(SvgExport { svg, diagnostics })
}

/// Notes a document that carries no accessible metadata (FEAT-026).
///
/// A scene without a title and without a description exports successfully, but
/// the document then has nothing for assistive technology to announce; the gap
/// is reported rather than left silent. The warning follows any content the
/// target omitted, so the exporter's own findings stay first.
fn warn_missing_metadata(model: &RenderModel, diagnostics: &mut Diagnostics) {
    if model.meta.title.is_none() && model.meta.description.is_none() {
        diagnostics.push(Diagnostic::warning(
            MISSING_METADATA,
            "the scene declares no title or description, so the exported SVG carries no accessible metadata",
        ));
    }
}

/// Validates the one background the document is about to draw.
///
/// A value that is neither a colour SVG supports nor a transparent sentinel is
/// a located error, so no invalid colour reaches the output (FEAT-018).
/// `from_options` selects the name and location, distinguishing an export
/// override from the canvas's own background.
fn validate_background(diagnostics: &mut Diagnostics, value: &str, from_options: bool) {
    if emit::is_transparent(value) {
        return;
    }
    let (what, path) = if from_options {
        ("export background", "/background")
    } else {
        ("canvas background", "/canvas/background")
    };
    validate_color(diagnostics, value, what, path);
}

/// Resolves the rendered size, refusing a dimension that is not finite and
/// positive (FEAT-013's rule for an invalid size, applied to vector output too).
fn resolve_size(model: &RenderModel, options: &SvgOptions) -> Result<emit::Size, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    if let Some(width) = options.width {
        validate_dimension(&mut diagnostics, width, "width");
    }
    if let Some(height) = options.height {
        validate_dimension(&mut diagnostics, height, "height");
    }
    if diagnostics.has_errors() {
        return Err(diagnostics);
    }

    let canvas = &model.canvas;
    let width = options.width.unwrap_or(canvas.width);
    let height = options.height.unwrap_or(canvas.height);

    // A single dimension scales the other to preserve the canvas aspect ratio.
    let scalable = canvas.width > 0.0 && canvas.height > 0.0;
    let (width, height) = match (options.width, options.height) {
        (Some(width), None) if scalable => (width, width * canvas.height / canvas.width),
        (None, Some(height)) if scalable => (height * canvas.width / canvas.height, height),
        _ => (width, height),
    };

    Ok(emit::Size { width, height })
}

fn validate_dimension(diagnostics: &mut Diagnostics, value: f64, field: &str) {
    if !value.is_finite() || value <= 0.0 {
        diagnostics.push(Diagnostic::error(
            OPTIONS,
            format!("`{field}` must be a finite number greater than zero"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::Affine;
    use crate::primitives::{
        Ellipse, Line, Path as PathGeometry, Polygon, Rect, Segment, Shape, SubPath,
    };
    use crate::render::{
        NodePaint, NodeStroke, Paint, RenderCanvas, RenderMeta, ResolvedFont, ResolvedNode, TextRun,
    };
    use crate::scene::TextAlign;
    use crate::style::{StrokeCap, StrokeJoin};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Shape {
        Shape::Rect(Rect {
            x,
            y,
            width,
            height,
            rx: 0.0,
            ry: 0.0,
        })
    }

    fn node(id: &str, name: Option<&str>, geometry: Shape) -> ResolvedNode {
        ResolvedNode {
            id: id.to_string(),
            name: name.map(str::to_string),
            accessible_name: None,
            order: 0,
            kind: geometry.kind().to_string(),
            groups: Vec::new(),
            geometry: Some(geometry),
            text: None,
            transform: Affine::IDENTITY,
            paint: NodePaint::default(),
            opacity: 1.0,
            visible: true,
        }
    }

    fn model(nodes: Vec<ResolvedNode>) -> RenderModel {
        RenderModel {
            canvas: RenderCanvas {
                width: 100.0,
                height: 50.0,
                background: "#ffffff".to_string(),
            },
            nodes,
            meta: RenderMeta::default(),
            diagnostics: Diagnostics::new(),
            fonts: Vec::new(),
        }
    }

    fn export(model: &RenderModel) -> String {
        export_svg(model, &SvgOptions::default()).expect("exports")
    }

    /// A text node carrying a string and resolved layout, as the compiler emits.
    fn text_node(id: &str, name: Option<&str>, value: &str, font_id: &str) -> ResolvedNode {
        ResolvedNode {
            id: id.to_string(),
            name: name.map(str::to_string),
            accessible_name: None,
            order: 0,
            kind: "text".to_string(),
            groups: Vec::new(),
            geometry: None,
            text: Some(TextRun {
                value: value.to_string(),
                font_id: font_id.to_string(),
                font_size: 48.0,
                align: TextAlign::Start,
                line_height: 48.0,
                letter_spacing: 0.0,
                width: None,
            }),
            transform: Affine::IDENTITY,
            paint: NodePaint {
                fill: Some(Paint::Color {
                    value: "#000000".to_string(),
                }),
                stroke: None,
            },
            opacity: 1.0,
            visible: true,
        }
    }

    fn font_bytes(file: &str) -> Vec<u8> {
        let path = format!("{}/../../assets/fonts/{file}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(&path).unwrap_or_else(|error| panic!("could not read {path}: {error}"))
    }

    #[test]
    fn a_compiled_scene_exports_its_resolved_paint_and_geometry() {
        use crate::compiler::{compile_with_style, StyleContext};
        use crate::scene::parse as parse_scene;
        use crate::style::{parse_palette, parse_stroke_profile};
        use serde_json::json;

        let scene = parse_scene(
            &json!({
                "id": "s",
                "projectId": "p",
                "name": "S",
                "formatVersion": "0.2",
                "canvas": { "width": 200.0, "height": 200.0, "background": "#ffffff" },
                "elements": [
                    {
                        "id": "e1", "sceneId": "s", "order": 0, "kind": "rect", "name": "Box",
                        "geometry": { "x": 0.0, "y": 0.0, "width": 30.0, "height": 40.0 },
                        "transform": { "translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0 },
                        "fill": { "kind": "token", "ref": "accent" },
                        "stroke": { "profileId": "stroke-1", "paint": { "kind": "token", "ref": "accent" } },
                        "opacity": 1.0, "visible": true
                    },
                    {
                        "id": "p1", "sceneId": "s", "order": 1, "kind": "path",
                        "geometry": { "pathData": "M0 0 A5 5 0 0 1 10 0" },
                        "transform": { "translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0 },
                        "opacity": 1.0, "visible": true
                    }
                ]
            })
            .to_string(),
        )
        .expect("a valid scene");
        let palette = parse_palette(
            r##"{"id":"pal","projectId":"p","name":"P","tokens":[{"name":"accent","value":"#ff0000"}]}"##,
        )
        .expect("a palette");
        let profile = parse_stroke_profile(
            r#"{"id":"stroke-1","projectId":"p","name":"O","width":3,"cap":"butt","join":"miter"}"#,
        )
        .expect("a profile");
        let style = StyleContext {
            palette: Some(&palette),
            strokes: std::slice::from_ref(&profile),
            gradients: &[],
            fonts: &[],
            recipe: None,
            definitions: &[],
        };

        let compiled = compile_with_style(&scene, &style).expect("compiles");
        let svg = export(&compiled);
        assert!(svg.contains("viewBox=\"0 0 200 200\""), "{svg}");
        assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
        assert!(svg.contains("stroke=\"#ff0000\""), "{svg}");
        assert!(svg.contains("<g id=\"e1\" data-name=\"Box\">"), "{svg}");
        assert!(svg.contains("<path d=\"M 0 0 A 5 5 0 0 1 10 0\""), "{svg}");
        assert!(svg.contains("</svg>"));
    }

    #[test]
    fn an_exported_document_is_well_formed_with_the_canvas_view_box() {
        let svg = export(&model(Vec::new()));
        assert!(svg.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(svg.contains(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"50\" viewBox=\"0 0 100 50\">"
        ));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn a_filled_and_stroked_rect_matches_the_render_model() {
        let mut shape = node("e1", Some("Box"), rect(1.0, 2.0, 30.0, 40.0));
        shape.paint = NodePaint {
            fill: Some(Paint::Color {
                value: "#ff0000".to_string(),
            }),
            stroke: Some(NodeStroke {
                paint: Paint::Color {
                    value: "#0000ff".to_string(),
                },
                width: 2.5,
                cap: StrokeCap::Round,
                join: StrokeJoin::Bevel,
            }),
        };
        let svg = export(&model(vec![shape]));
        assert!(
            svg.contains("<rect x=\"1\" y=\"2\" width=\"30\" height=\"40\""),
            "{svg}"
        );
        assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
        assert!(svg.contains("stroke=\"#0000ff\""), "{svg}");
        assert!(svg.contains("stroke-width=\"2.5\""), "{svg}");
        assert!(svg.contains("stroke-linecap=\"round\""), "{svg}");
        assert!(svg.contains("stroke-linejoin=\"bevel\""), "{svg}");
        assert!(svg.contains("<g id=\"e1\" data-name=\"Box\">"), "{svg}");
        assert!(svg.contains("<title>Box</title>"), "{svg}");
    }

    #[test]
    fn a_compiled_elements_accessible_name_reaches_the_document() {
        const SCENE: &str = r##"{
          "id": "s", "projectId": "p", "name": "S", "formatVersion": "0.2",
          "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
          "elements": [
            {
              "id": "e1", "sceneId": "s", "order": 0, "kind": "rect",
              "name": "Box", "accessibleName": "Red box",
              "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            }
          ]
        }"##;
        let svg = export(&compile_scene(SCENE));
        assert!(svg.contains("<g id=\"e1\" data-name=\"Box\">"), "{svg}");
        assert!(svg.contains("<title>Red box</title>"), "{svg}");
    }

    #[test]
    fn an_elements_accessible_name_becomes_its_title_while_its_maintenance_name_stays_in_data_name()
    {
        let mut shape = node("e1", Some("Box"), rect(0.0, 0.0, 1.0, 1.0));
        shape.accessible_name = Some("Red box".to_string());
        let svg = export(&model(vec![shape]));
        assert!(svg.contains("<g id=\"e1\" data-name=\"Box\">"), "{svg}");
        assert!(svg.contains("<title>Red box</title>"), "{svg}");
        assert!(!svg.contains("<title>Box</title>"), "{svg}");
    }

    #[test]
    fn an_elements_accessible_name_alone_still_titles_the_group() {
        let mut shape = node("e1", None, rect(0.0, 0.0, 1.0, 1.0));
        shape.accessible_name = Some("Red box".to_string());
        let svg = export(&model(vec![shape]));
        assert!(!svg.contains("data-name="), "{svg}");
        assert!(svg.contains("<title>Red box</title>"), "{svg}");
    }

    #[test]
    fn a_groups_accessible_name_is_carried_as_its_title() {
        const SCENE: &str = r##"{
          "id": "s", "projectId": "p", "name": "S", "formatVersion": "0.2",
          "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
          "elements": [
            {
              "id": "outer", "sceneId": "s", "order": 0, "kind": "group",
              "name": "Outer", "accessibleName": "Frame",
              "geometry": {},
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            },
            {
              "id": "c1", "sceneId": "s", "order": 0, "kind": "rect", "parentId": "outer",
              "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            }
          ]
        }"##;
        let svg = export(&compile_scene(SCENE));
        assert!(
            svg.contains("<g id=\"outer\" data-name=\"Outer\">"),
            "{svg}"
        );
        assert!(svg.contains("<title>Frame</title>"), "{svg}");
    }

    #[test]
    fn a_group_with_only_an_accessible_name_is_kept_and_titled() {
        const SCENE: &str = r##"{
          "id": "s", "projectId": "p", "name": "S", "formatVersion": "0.2",
          "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
          "elements": [
            {
              "id": "outer", "sceneId": "s", "order": 0, "kind": "group",
              "accessibleName": "Frame",
              "geometry": {},
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            },
            {
              "id": "c1", "sceneId": "s", "order": 0, "kind": "rect", "parentId": "outer",
              "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            }
          ]
        }"##;
        let svg = export(&compile_scene(SCENE));
        assert!(svg.contains("<g id=\"outer\">"), "{svg}");
        assert!(!svg.contains("data-name=\"\""), "{svg}");
        assert!(svg.contains("<title>Frame</title>"), "{svg}");
    }

    #[test]
    fn a_scene_without_metadata_warns_but_still_exports() {
        let export =
            export_svg_reporting(&model(Vec::new()), &SvgOptions::default()).expect("exports");
        assert!(export.svg.contains("</svg>"), "{}", export.svg);
        assert_eq!(
            export
                .diagnostics
                .warnings()
                .next()
                .map(|warning| warning.code.clone()),
            Some(MISSING_METADATA)
        );
    }

    #[test]
    fn a_scene_with_metadata_does_not_warn_about_missing_metadata() {
        let mut document = model(Vec::new());
        document.meta.title = Some("Logo".to_string());
        let export = export_svg_reporting(&document, &SvgOptions::default()).expect("exports");
        assert!(
            !export
                .diagnostics
                .warnings()
                .any(|warning| warning.code == MISSING_METADATA),
            "{:?}",
            export.diagnostics
        );
    }

    #[test]
    fn a_shape_without_a_fill_is_not_filled() {
        let svg = export(&model(vec![node("e1", None, rect(0.0, 0.0, 1.0, 1.0))]));
        assert!(svg.contains("fill=\"none\""), "{svg}");
    }

    #[test]
    fn a_resolved_transform_becomes_a_matrix() {
        let mut shape = node("e1", None, rect(0.0, 0.0, 10.0, 10.0));
        shape.transform = Affine {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 5.0,
            f: 6.0,
        };
        let svg = export(&model(vec![shape]));
        assert!(svg.contains("transform=\"matrix(1 0 0 1 5 6)\""), "{svg}");
    }

    #[test]
    fn an_identity_transform_is_omitted() {
        let svg = export(&model(vec![node("e1", None, rect(0.0, 0.0, 1.0, 1.0))]));
        assert!(!svg.contains("transform="), "{svg}");
    }

    #[test]
    fn each_primitive_becomes_a_native_svg_shape() {
        let shapes = [
            (
                Shape::Ellipse(Ellipse {
                    cx: 5.0,
                    cy: 6.0,
                    rx: 7.0,
                    ry: 8.0,
                }),
                "<ellipse cx=\"5\" cy=\"6\" rx=\"7\" ry=\"8\"",
            ),
            (
                Shape::Polygon(Polygon {
                    points: vec![[0.0, 0.0], [10.0, 0.0], [5.0, 8.0]],
                }),
                "<polygon points=\"0,0 10,0 5,8\"",
            ),
            (
                Shape::Line(Line {
                    points: vec![[0.0, 0.0], [10.0, 10.0]],
                }),
                "<polyline points=\"0,0 10,10\"",
            ),
        ];
        for (shape, expected) in shapes {
            let svg = export(&model(vec![node("e1", None, shape)]));
            assert!(svg.contains(expected), "expected {expected} in {svg}");
        }
    }

    #[test]
    fn a_path_becomes_path_data_with_every_segment_kind() {
        let path = PathGeometry {
            subpaths: vec![SubPath {
                start: [0.0, 0.0],
                segments: vec![
                    Segment::Line { to: [10.0, 0.0] },
                    Segment::Cubic {
                        ctrl1: [1.0, 1.0],
                        ctrl2: [2.0, 2.0],
                        to: [3.0, 3.0],
                    },
                    Segment::Quadratic {
                        ctrl: [4.0, 4.0],
                        to: [5.0, 5.0],
                    },
                    Segment::Arc {
                        rx: 6.0,
                        ry: 7.0,
                        x_rotation: 0.0,
                        large_arc: false,
                        sweep: true,
                        to: [8.0, 8.0],
                    },
                ],
                closed: true,
            }],
        };
        let svg = export(&model(vec![node("e1", None, Shape::Path(path))]));
        assert!(
            svg.contains("d=\"M 0 0 L 10 0 C 1 1 2 2 3 3 Q 4 4 5 5 A 6 7 0 0 1 8 8 Z\""),
            "{svg}"
        );
    }

    #[test]
    fn a_path_with_no_segments_draws_nothing() {
        let path = PathGeometry {
            subpaths: vec![SubPath {
                start: [1.0, 2.0],
                segments: Vec::new(),
                closed: false,
            }],
        };
        let svg = export(&model(vec![node("e1", None, Shape::Path(path))]));
        assert!(!svg.contains("<path"), "{svg}");
    }

    #[test]
    fn a_negative_extent_is_normalised_rather_than_emitted_invalid() {
        let svg = export(&model(vec![node("e1", None, rect(10.0, 10.0, -4.0, -5.0))]));
        assert!(
            svg.contains("<rect x=\"6\" y=\"5\" width=\"4\" height=\"5\""),
            "{svg}"
        );
    }

    #[test]
    fn an_extreme_coordinate_still_produces_a_valid_document() {
        let svg = export(&model(vec![node(
            "e1",
            None,
            rect(1e300, -1e300, 1e300, 1e300),
        )]));
        assert!(svg.contains("1e300"), "{svg}");
        assert!(!svg.contains("inf") && !svg.contains("NaN"), "{svg}");
    }

    #[test]
    fn an_invisible_node_is_preserved_but_not_drawn() {
        let mut shape = node("e1", None, rect(0.0, 0.0, 1.0, 1.0));
        shape.visible = false;
        let svg = export(&model(vec![shape]));
        assert!(svg.contains("display=\"none\""), "{svg}");
    }

    #[test]
    fn node_opacity_is_carried_to_the_group() {
        let mut shape = node("e1", None, rect(0.0, 0.0, 1.0, 1.0));
        shape.opacity = 0.5;
        let svg = export(&model(vec![shape]));
        assert!(svg.contains("opacity=\"0.5\""), "{svg}");
    }

    #[test]
    fn repeated_exports_are_identical() {
        let document = model(vec![
            node("e1", Some("A"), rect(1.0, 2.0, 3.0, 4.0)),
            node("e2", None, rect(5.0, 6.0, 7.0, 8.0)),
        ]);
        assert_eq!(export(&document), export(&document));
    }

    fn compile_scene(scene: &str) -> RenderModel {
        crate::compiler::compile(&crate::scene::parse(scene).expect("a valid scene"))
            .expect("compiles")
    }

    /// A named group inside a named group, holding one named shape.
    const NESTED_GROUPS: &str = r##"{
      "id": "s", "projectId": "p", "name": "S", "formatVersion": "0.2",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "outer", "sceneId": "s", "order": 0, "kind": "group", "name": "Outer",
          "geometry": {},
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        },
        {
          "id": "inner", "sceneId": "s", "order": 0, "kind": "group", "name": "Inner", "parentId": "outer",
          "geometry": {},
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        },
        {
          "id": "c1", "sceneId": "s", "order": 0, "kind": "rect", "name": "Leaf", "parentId": "inner",
          "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    #[test]
    fn named_group_nesting_is_reconstructed() {
        let svg = export(&compile_scene(NESTED_GROUPS));
        let outer = svg
            .find("<g id=\"outer\" data-name=\"Outer\">")
            .expect("the outer group");
        let inner = svg
            .find("<g id=\"inner\" data-name=\"Inner\">")
            .expect("the inner group");
        let leaf = svg
            .find("<g id=\"c1\" data-name=\"Leaf\">")
            .expect("the named shape");
        assert!(outer < inner && inner < leaf, "{svg}");

        // Each group closes inside its parent, so the closes run leaf, inner,
        // outer.
        let closes: Vec<usize> = svg.match_indices("</g>").map(|(at, _)| at).collect();
        assert_eq!(closes.len(), 3, "{svg}");
        assert!(
            leaf < closes[0] && closes[0] < closes[1] && closes[1] < closes[2],
            "{svg}"
        );
    }

    #[test]
    fn an_unnamed_group_is_collapsed_but_keeps_its_named_child_in_place() {
        const SCENE: &str = r##"{
          "id": "s", "projectId": "p", "name": "S", "formatVersion": "0.2",
          "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
          "elements": [
            {
              "id": "outer", "sceneId": "s", "order": 0, "kind": "group", "name": "Outer",
              "geometry": {},
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            },
            {
              "id": "middle", "sceneId": "s", "order": 0, "kind": "group", "parentId": "outer",
              "geometry": {},
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            },
            {
              "id": "c1", "sceneId": "s", "order": 0, "kind": "rect", "name": "Leaf", "parentId": "middle",
              "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            }
          ]
        }"##;
        let svg = export(&compile_scene(SCENE));
        assert!(!svg.contains("id=\"middle\""), "{svg}");
        let outer = svg.find("<g id=\"outer\"").expect("the outer group");
        let leaf = svg.find("<g id=\"c1\"").expect("the named shape");
        assert!(outer < leaf, "{svg}");
        assert_eq!(svg.matches("<g id=").count(), 2, "{svg}");
    }

    #[test]
    fn a_named_group_spans_all_of_its_children() {
        const SCENE: &str = r##"{
          "id": "s", "projectId": "p", "name": "S", "formatVersion": "0.2",
          "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
          "elements": [
            {
              "id": "outer", "sceneId": "s", "order": 0, "kind": "group", "name": "Outer",
              "geometry": {},
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            },
            {
              "id": "c1", "sceneId": "s", "order": 0, "kind": "rect", "parentId": "outer",
              "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            },
            {
              "id": "c2", "sceneId": "s", "order": 1, "kind": "rect", "parentId": "outer",
              "geometry": { "x": 20, "y": 0, "width": 10, "height": 10 },
              "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
              "opacity": 1, "visible": true
            }
          ]
        }"##;
        let svg = export(&compile_scene(SCENE));
        assert_eq!(svg.matches("<g id=\"outer\"").count(), 1, "{svg}");
        assert_eq!(svg.matches("<g id=\"c1\"").count(), 1, "{svg}");
        assert_eq!(svg.matches("<g id=\"c2\"").count(), 1, "{svg}");
        assert_eq!(svg.matches("</g>").count(), 3, "{svg}");
    }

    #[test]
    fn grouped_exports_are_identical() {
        let document = compile_scene(NESTED_GROUPS);
        assert_eq!(export(&document), export(&document));
    }

    #[test]
    fn names_and_colours_are_escaped_so_the_output_stays_inert() {
        let mut shape = node(
            "e1",
            Some("<script>alert(1)</script>"),
            rect(0.0, 0.0, 1.0, 1.0),
        );
        shape.paint.fill = Some(Paint::Color {
            value: "\"><script>".to_string(),
        });
        let svg = export(&model(vec![shape]));
        assert!(!svg.contains("<script>"), "{svg}");
        assert!(svg.contains("&lt;script&gt;"), "{svg}");
        assert!(svg.contains("&quot;&gt;&lt;script&gt;"), "{svg}");
    }

    #[test]
    fn a_raster_node_is_omitted_with_a_warning() {
        let mut shape = node("r1", None, rect(0.0, 0.0, 1.0, 1.0));
        shape.kind = "raster".to_string();
        let export =
            export_svg_reporting(&model(vec![shape]), &SvgOptions::default()).expect("exports");
        assert!(!export.svg.contains("r1"), "{}", export.svg);
        assert_eq!(
            export.diagnostics.warnings().next().map(|w| w.code.clone()),
            Some(UNSUPPORTED)
        );
    }

    #[test]
    fn non_finite_geometry_is_omitted_with_a_warning() {
        let shape = node("e1", None, rect(f64::NAN, 0.0, 1.0, 1.0));
        let export =
            export_svg_reporting(&model(vec![shape]), &SvgOptions::default()).expect("exports");
        assert!(!export.svg.contains("<g id=\"e1\""), "{}", export.svg);
        assert!(export.diagnostics.warnings().any(|w| w.code == UNSUPPORTED));
    }

    #[test]
    fn the_frozen_entry_point_still_returns_a_document() {
        let mut shape = node("r1", None, rect(0.0, 0.0, 1.0, 1.0));
        shape.kind = "raster".to_string();
        let svg = export_svg(&model(vec![shape]), &SvgOptions::default()).expect("exports");
        assert!(svg.contains("</svg>"), "{svg}");
    }

    #[test]
    fn an_invalid_output_size_is_refused() {
        let document = model(Vec::new());
        for width in [0.0, -1.0, f64::NAN] {
            let diagnostics = export_svg(
                &document,
                &SvgOptions {
                    width: Some(width),
                    ..SvgOptions::default()
                },
            )
            .expect_err("refused");
            assert_eq!(
                diagnostics.errors().next().map(|e| e.code.clone()),
                Some(OPTIONS)
            );
        }
    }

    #[test]
    fn the_background_is_drawn_unless_transparent() {
        let document = model(Vec::new());
        assert!(export(&document)
            .contains("<rect x=\"0\" y=\"0\" width=\"100\" height=\"50\" fill=\"#ffffff\"/>"));

        let transparent = export_svg(
            &document,
            &SvgOptions {
                background: Some("transparent".to_string()),
                ..SvgOptions::default()
            },
        )
        .expect("exports");
        assert!(!transparent.contains("fill=\"#ffffff\""), "{transparent}");

        let overridden = export_svg(
            &document,
            &SvgOptions {
                background: Some("#000000".to_string()),
                ..SvgOptions::default()
            },
        )
        .expect("exports");
        assert!(overridden.contains("fill=\"#000000\""), "{overridden}");
    }

    #[test]
    fn an_invalid_export_background_is_refused_with_its_location() {
        let diagnostics = export_svg(
            &model(Vec::new()),
            &SvgOptions {
                background: Some("not-a-colour".to_string()),
                ..SvgOptions::default()
            },
        )
        .expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, crate::scene::INVALID_COLOR);
        assert!(
            error.message.contains("export background"),
            "{}",
            error.message
        );
        assert!(error.message.contains("not-a-colour"), "{}", error.message);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/background")
        );
    }

    #[test]
    fn a_canvas_background_that_is_not_a_colour_is_refused_by_the_exporter() {
        let mut document = model(Vec::new());
        document.canvas.background = "not-a-colour".to_string();
        let diagnostics = export_svg(&document, &SvgOptions::default()).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, crate::scene::INVALID_COLOR);
        assert!(
            error.message.contains("canvas background"),
            "{}",
            error.message
        );
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/canvas/background")
        );
    }

    #[test]
    fn the_transparent_background_sentinels_remain_accepted() {
        let document = model(Vec::new());
        for background in ["transparent", "none", ""] {
            let svg = export_svg(
                &document,
                &SvgOptions {
                    background: Some(background.to_string()),
                    ..SvgOptions::default()
                },
            )
            .unwrap_or_else(|diagnostics| panic!("{background:?} should export: {diagnostics}"));
            assert!(!svg.contains("fill=\"#ffffff\""), "{background:?}: {svg}");
        }
    }

    #[test]
    fn a_single_output_dimension_preserves_the_aspect_ratio() {
        let document = model(Vec::new());
        let svg = export_svg(
            &document,
            &SvgOptions {
                width: Some(200.0),
                ..SvgOptions::default()
            },
        )
        .expect("exports");
        assert!(
            svg.contains("width=\"200\" height=\"100\" viewBox=\"0 0 100 50\""),
            "{svg}"
        );
    }

    #[test]
    fn accessible_metadata_reaches_the_document() {
        let mut document = model(Vec::new());
        document.meta = RenderMeta {
            title: Some("Logo".to_string()),
            description: Some("A mark".to_string()),
            recipe: None,
            seed: 0,
        };
        let svg = export(&document);
        assert!(svg.contains("<title>Logo</title>"), "{svg}");
        assert!(svg.contains("<desc>A mark</desc>"), "{svg}");
    }

    #[test]
    fn identifiers_are_made_valid_and_unique() {
        let svg = export(&model(vec![
            node("1 bad", None, rect(0.0, 0.0, 1.0, 1.0)),
            node("1-bad", None, rect(0.0, 0.0, 1.0, 1.0)),
        ]));
        assert!(svg.contains("id=\"vectr-1-bad\""), "{svg}");
        assert!(svg.contains("id=\"vectr-1-bad-2\""), "{svg}");
    }

    #[test]
    fn a_text_element_exports_as_outlined_glyphs_with_its_name_and_text() {
        use crate::compiler::{compile_with_style, FontAsset, StyleContext, DEFAULT_FONT_ID};
        use crate::scene::parse as parse_scene;
        use serde_json::json;

        let scene = parse_scene(
            &json!({
                "id": "s", "projectId": "p", "name": "S", "formatVersion": "0.2",
                "canvas": { "width": 200.0, "height": 100.0, "background": "#ffffff" },
                "title": "Wordmark", "description": "A greeting",
                "elements": [{
                    "id": "t1", "sceneId": "s", "order": 0, "kind": "text", "name": "Wordmark",
                    "geometry": { "x": 10.0, "y": 60.0, "text": "Hi", "fontSize": 48.0 },
                    "transform": { "translateX": 0.0, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0 },
                    "fill": { "kind": "token", "ref": "ink" }, "opacity": 1.0, "visible": true
                }]
            })
            .to_string(),
        )
        .expect("a valid scene");
        let fonts = [FontAsset::new(
            DEFAULT_FONT_ID,
            "Inter",
            font_bytes("Inter.ttf"),
        )];
        let compiled = compile_with_style(
            &scene,
            &StyleContext {
                palette: None,
                strokes: &[],
                gradients: &[],
                fonts: &fonts,
                recipe: None,
                definitions: &[],
            },
        )
        .expect("compiles");

        let export = export_svg_reporting(&compiled, &SvgOptions::default()).expect("exports");
        assert!(export.diagnostics.is_empty(), "{:?}", export.diagnostics);
        assert!(export.svg.contains("<path"), "{}", export.svg);
        assert!(!export.svg.contains("<text"), "{}", export.svg);
        assert!(
            export.svg.contains("id=\"t1\" data-name=\"Wordmark\""),
            "{}",
            export.svg
        );
        assert!(
            export.svg.contains("<title>Wordmark</title>"),
            "{}",
            export.svg
        );
        assert!(export.svg.contains("<desc>Hi</desc>"), "{}", export.svg);
    }

    #[test]
    fn a_text_node_whose_font_is_missing_is_omitted_with_a_warning() {
        let document = model(vec![text_node("t1", None, "Hi", "ghost")]);
        let export = export_svg_reporting(&document, &SvgOptions::default()).expect("exports");
        assert!(!export.svg.contains("t1"), "{}", export.svg);
        assert_eq!(
            export.diagnostics.warnings().next().map(|w| w.code.clone()),
            Some(crate::fonts::FONT_MISSING)
        );
    }

    #[test]
    fn a_glyph_the_chosen_font_lacks_is_outlined_from_the_fallback() {
        let mut document = model(vec![text_node("t1", None, "\u{149}", "body")]);
        document.fonts = vec![
            ResolvedFont {
                id: "body".to_string(),
                name: "Inter".to_string(),
                data: font_bytes("Inter.ttf"),
            },
            ResolvedFont {
                id: crate::fonts::FALLBACK_FONT_ID.to_string(),
                name: "Noto Sans".to_string(),
                data: font_bytes("NotoSans.ttf"),
            },
        ];
        let export = export_svg_reporting(&document, &SvgOptions::default()).expect("exports");
        assert!(export.svg.contains("<path"), "{}", export.svg);
        assert!(
            !export
                .diagnostics
                .warnings()
                .any(|warning| warning.code == crate::fonts::MISSING_GLYPH),
            "{:?}",
            export.diagnostics
        );
    }
}
