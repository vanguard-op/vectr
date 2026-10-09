//! The six MCP tools: validate, compile, render, inspect, render-part and schema
//! (C-005, FEAT-019, FEAT-022, FEAT-031).
//!
//! Each tool is a thin, stateless wrapper over the same engine calls the command
//! line makes, so an agent gets identical results to `vectr validate`, `vectr
//! compile`, `vectr export`, `vectr inspect`, `vectr render` and `vectr schema`
//! (C-005: the MCP tools consume the CLI's capabilities). A tool call carries no
//! server state: it resolves the input under the server's [`Scope`], runs the
//! engine, and returns a JSON value. That keeps concurrent calls independent
//! (FEAT-021, FEAT-019).
//!
//! A scene tool addresses a scene exactly as the command line does: by the
//! scene's identifier resolved among a project's scenes, or, when none is named,
//! the project's default. The project is the one the caller names, or the
//! server's project context. A tool also accepts a scene document sent inline as
//! a draft, which is not a project scene: it never becomes or reads the default,
//! and its assets resolve against the same project a project scene uses. A call
//! naming both a scene identifier and a draft is malformed (C-005, FEAT-019).
//!
//! The render-part tool renders one part on its own — a reusable definition or
//! an element subtree, each addressed by its identifier — so a model can verify
//! it before it is composed (FEAT-031).
//!
//! The inspect tool closes the render-in-the-loop: it compiles a whole scene,
//! writes a preview image (SVG or PNG) at a configurable size, and reports the
//! path and the size it rendered at, so a person or a model can compare the
//! result against the request (FEAT-022). The MCP server has no inspection
//! capability of its own, so it records that limitation alongside the preview
//! rather than hiding it.
//!
//! The render tool writes SVG or PNG, and also PDF when the `enable_pdf_export`
//! rollout flag is set; the flag is off by default, so PDF is reported as an
//! unsupported capability until it is enabled (FEAT-014, C-005).

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use vectr_core::scene::{is_color, INVALID_COLOR};
use vectr_core::{
    compile_definition as compile_part_definition, compile_subtree, compile_with_style,
    export_pdf_reporting, export_png_reporting, export_svg_reporting, parse as parse_scene, schema,
    schema_for, validate as validate_scene, validate_gradient_usage, validate_palette_usage,
    Diagnostic, DiagnosticCode, Diagnostics, Location, PdfOptions, RasterOptions, RenderModel,
    Scene, SchemaForm, SvgOptions,
};

use vectr_project::{resolve_part, resolve_scene, ProjectAssets, ResolvedPart};

use crate::output::write_atomic;
use crate::scope::{Scope, ScopeError, SCOPE};

/// The stable code a call that names both a scene identifier and an inline
/// draft is reported under (C-005): the two are mutually exclusive.
pub const MALFORMED: &str = "E_MALFORMED";

/// The environment variable that enables PDF export, a rollout flag that is off
/// by default (FEAT-014). The same variable the command line reads, so the two
/// front ends gate the capability identically.
pub const ENABLE_PDF_ENV: &str = "VECTR_ENABLE_PDF_EXPORT";

/// The finding recorded when the server can render a preview but has no
/// in-process capability to compare it with the request, so the caller performs
/// the inspection (FEAT-022).
const INSPECTION_UNAVAILABLE: DiagnosticCode = DiagnosticCode::new("W_INSPECTION_UNAVAILABLE");

/// A tool call the client asked for could not be carried out.
#[derive(Debug)]
pub enum CallError {
    /// The request is malformed: a missing, mistyped or unknown argument, or an
    /// unknown tool. Reported as a JSON-RPC `-32602` protocol error.
    InvalidParams(String),
    /// The tool ran but could not complete: the scene is invalid, a reference is
    /// unresolvable, or the requested capability is unsupported. Reported as a
    /// tool execution error with `isError: true` and a structured body.
    Exec(Box<ToolError>),
}

/// A structured tool execution error: the code, message and location C-005
/// requires, with the full diagnostics for a model to correct the scene.
#[derive(Debug, Clone)]
pub struct ToolError {
    /// Stable machine-readable code, e.g. `E_SCHEMA`.
    pub code: String,
    /// Human-readable explanation.
    pub message: String,
    /// Where the failure applies, when known.
    pub location: Option<Location>,
    /// Every finding recorded before the failure.
    pub diagnostics: Diagnostics,
}

impl ToolError {
    /// Builds an error from a code and message.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            location: None,
            diagnostics: Diagnostics::new(),
        }
    }

    /// Builds an error from the first error in a diagnostics collection.
    pub fn from_diagnostics(diagnostics: Diagnostics) -> Self {
        let first = diagnostics
            .errors()
            .next()
            .or_else(|| diagnostics.iter().next());
        let (code, message, location) = match first {
            Some(finding) => (
                finding.code.to_string(),
                finding.message.clone(),
                finding.location.clone(),
            ),
            None => (
                "E_UNKNOWN".to_string(),
                "the tool call could not be completed".to_string(),
                None,
            ),
        };
        Self {
            code,
            message,
            location,
            diagnostics,
        }
    }

    /// The structured body returned in a tool execution error (C-005).
    pub fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("code".to_string(), Value::String(self.code.clone()));
        object.insert("message".to_string(), Value::String(self.message.clone()));
        if let Some(location) = &self.location {
            object.insert(
                "location".to_string(),
                serde_json::to_value(location).unwrap_or(Value::Null),
            );
        }
        object.insert(
            "diagnostics".to_string(),
            serde_json::to_value(&self.diagnostics).unwrap_or(Value::Null),
        );
        Value::Object(object)
    }
}

/// Calls a tool by name with already-decoded arguments.
pub fn call(scope: &Scope, name: &str, arguments: &Map<String, Value>) -> Result<Value, CallError> {
    match name {
        "validate" => validate_tool(scope, arguments),
        "compile" => compile_tool(scope, arguments),
        "render" => render_tool(scope, arguments),
        "inspect" => inspect_tool(scope, arguments),
        "render-part" => render_part_tool(scope, arguments),
        "schema" => schema_tool(arguments),
        other => Err(CallError::InvalidParams(format!("Unknown tool: {other}"))),
    }
}

/// The six tool definitions, in the order `tools/list` publishes them.
pub fn definitions() -> Value {
    json!([
        validate_definition(),
        compile_definition(),
        render_definition(),
        inspect_definition(),
        render_part_definition(),
        schema_definition(),
    ])
}

fn validate_tool(scope: &Scope, arguments: &Map<String, Value>) -> Result<Value, CallError> {
    let args = Args::new(arguments);
    args.allowed(&["scene", "draft", "project"])?;
    let input = load_scene(scope, arguments)?;

    let assets = ProjectAssets::load(&input.root, &input.scene).map_err(exec_diagnostics)?;
    let findings = structural_findings(&input.scene, &assets);

    if findings.has_errors() {
        return Err(exec(ToolError::from_diagnostics(findings)));
    }
    Ok(json!({
        "valid": true,
        "diagnostics": serde_json::to_value(&findings).unwrap_or(Value::Null),
    }))
}

/// The structural checks a scene and its project assets must pass (FEAT-018).
///
/// A palette token the palette no longer defines, a stroke profile, a gradient,
/// or a font that does not resolve is an error naming it, so a scene that will
/// not render is caught before anything is compiled or written (FEAT-005,
/// FEAT-024, FEAT-027). Shared by `validate` and `inspect`, so both gate on the
/// same findings (FEAT-022).
fn structural_findings(scene: &Scene, assets: &ProjectAssets) -> Diagnostics {
    let mut findings = validate_scene(scene);
    if let Some(palette) = assets.palette() {
        findings.extend(validate_palette_usage(scene, palette, assets.gradients()));
    }
    findings.extend(validate_gradient_usage(scene, assets.gradients()));
    findings.extend(assets.check_references(scene));
    findings
}

fn compile_tool(scope: &Scope, arguments: &Map<String, Value>) -> Result<Value, CallError> {
    let args = Args::new(arguments);
    args.allowed(&["scene", "draft", "project"])?;
    let input = load_scene(scope, arguments)?;

    let (model, diagnostics) = compile_scene(&input)?;
    Ok(json!({
        "model": serde_json::to_value(&model).unwrap_or(Value::Null),
        "diagnostics": serde_json::to_value(&diagnostics).unwrap_or(Value::Null),
    }))
}

fn render_tool(scope: &Scope, arguments: &Map<String, Value>) -> Result<Value, CallError> {
    let args = Args::new(arguments);
    args.allowed(&[
        "scene",
        "draft",
        "project",
        "format",
        "out",
        "width",
        "height",
        "density",
        "background",
    ])?;
    let input = load_scene(scope, arguments)?;

    let format = args.required_string("format")?;
    if format != "svg" && format != "png" && format != "pdf" {
        return Err(unsupported(format!(
            "rendering `{format}` is not supported; use `svg`, `png` or `pdf`"
        )));
    }
    // PDF export is off by default and enabled by the rollout flag, so a PDF
    // request while it is off is an unsupported capability, refused before
    // anything is compiled or written (FEAT-014, C-005).
    if format == "pdf" && !pdf_enabled() {
        return Err(unsupported(format!(
            "PDF export is disabled; set {ENABLE_PDF_ENV}=1 to enable it"
        )));
    }
    let width = args.number("width")?;
    let height = args.number("height")?;
    let density = args.number("density")?;
    let background = args.string("background")?;

    if format != "png" && density.is_some() {
        return Err(unsupported("`density` applies only to PNG output"));
    }
    if let Some(colour) = &background {
        if !is_color(colour) {
            return Err(exec(ToolError::new(
                INVALID_COLOR.as_str(),
                format!("`background` is not a colour SVG supports: `{colour}`"),
            )));
        }
    }

    let (model, mut diagnostics) = compile_scene(&input)?;

    let bytes = match format.as_str() {
        "svg" => {
            let options = SvgOptions {
                width,
                height,
                background: background.clone(),
            };
            let export = export_svg_reporting(&model, &options).map_err(exec_diagnostics)?;
            diagnostics.extend(export.diagnostics);
            export.svg.into_bytes()
        }
        "png" => {
            let options = RasterOptions {
                width,
                height,
                density,
                background: background.clone(),
            };
            let export = export_png_reporting(&model, &options).map_err(exec_diagnostics)?;
            diagnostics.extend(export.diagnostics);
            export.png
        }
        _ => {
            // The MCP surface carries no print-profile argument (C-005), so a
            // PDF render uses the exporter's default sRGB profile.
            let options = PdfOptions {
                page_width: width,
                page_height: height,
                profile: None,
                background: background.clone(),
            };
            let export = export_pdf_reporting(&model, &options).map_err(exec_diagnostics)?;
            diagnostics.extend(export.diagnostics);
            export.pdf
        }
    };

    let target = match args.string("out")? {
        Some(out) => PathBuf::from(out),
        None => default_output(&input, &format),
    };
    let target = scope.write_path(&target).map_err(scope_error)?;
    write_atomic(&target, &bytes).map_err(|error| {
        exec(ToolError::new(
            "E_OUTPUT",
            format!("cannot write output `{}`: {error}", report_path(&target)),
        ))
    })?;

    Ok(json!({
        "path": report_path(&target),
        "format": format,
        "diagnostics": serde_json::to_value(&diagnostics).unwrap_or(Value::Null),
    }))
}

/// Renders a whole-scene preview for inspection (C-005, FEAT-022).
///
/// The scene passes the same structural gate `validate` applies and is compiled
/// as `render` compiles it, so a broken scene is reported before a preview is
/// attempted and a render failure is reported before any inspection (NFR-011).
/// The preview is written as SVG or PNG at the requested size — the size is
/// configurable so a preview too small to judge can be raised — and the tool
/// reports its path and the size it rendered at. The server has no inspection
/// capability of its own, so it records that limitation alongside the preview
/// for the caller to compare against the request rather than hiding it
/// (FEAT-022).
fn inspect_tool(scope: &Scope, arguments: &Map<String, Value>) -> Result<Value, CallError> {
    let args = Args::new(arguments);
    args.allowed(&[
        "scene",
        "draft",
        "project",
        "format",
        "out",
        "width",
        "height",
        "density",
        "background",
    ])?;
    let input = load_scene(scope, arguments)?;

    let format = args.required_string("format")?;
    if format != "svg" && format != "png" {
        return Err(unsupported(format!(
            "inspecting as `{format}` is not supported; use `svg` or `png`"
        )));
    }
    let width = args.number("width")?;
    let height = args.number("height")?;
    let density = args.number("density")?;
    let background = args.string("background")?;

    if format != "png" && density.is_some() {
        return Err(unsupported("`density` applies only to PNG output"));
    }
    if let Some(colour) = &background {
        if !is_color(colour) {
            return Err(exec(ToolError::new(
                INVALID_COLOR.as_str(),
                format!("`background` is not a colour SVG supports: `{colour}`"),
            )));
        }
    }

    let assets = ProjectAssets::load(&input.root, &input.scene).map_err(exec_diagnostics)?;
    // Structural checks run first and gate the preview: a scene that fails them
    // is reported and nothing is rendered, so the findings still stand even when
    // no inspection can follow (FEAT-018, FEAT-022).
    let mut diagnostics = structural_findings(&input.scene, &assets);
    if diagnostics.has_errors() {
        return Err(exec_diagnostics(diagnostics));
    }

    let model =
        compile_with_style(&input.scene, &assets.style_context()).map_err(exec_diagnostics)?;
    diagnostics.extend(model.diagnostics.clone());
    let frame = output_frame(&model, &format, width, height, density);

    let bytes = match format.as_str() {
        "svg" => {
            let options = SvgOptions {
                width,
                height,
                background: background.clone(),
            };
            let export = export_svg_reporting(&model, &options).map_err(exec_diagnostics)?;
            diagnostics.extend(export.diagnostics);
            export.svg.into_bytes()
        }
        _ => {
            let options = RasterOptions {
                width,
                height,
                density,
                background: background.clone(),
            };
            let export = export_png_reporting(&model, &options).map_err(exec_diagnostics)?;
            diagnostics.extend(export.diagnostics);
            export.png
        }
    };

    // The preview is complete: note that the visual comparison with the request
    // is the caller's, so the limitation is reported rather than hidden.
    diagnostics.push(Diagnostic::warning(
        INSPECTION_UNAVAILABLE,
        "no inspection capability is available; the preview is rendered for a person or a model to compare against the request",
    ));

    let target = match args.string("out")? {
        Some(out) => PathBuf::from(out),
        None => default_output(&input, &format),
    };
    let target = scope.write_path(&target).map_err(scope_error)?;
    write_atomic(&target, &bytes).map_err(|error| {
        exec(ToolError::new(
            "E_OUTPUT",
            format!("cannot write output `{}`: {error}", report_path(&target)),
        ))
    })?;

    Ok(json!({
        "path": report_path(&target),
        "format": format,
        "size": { "width": frame.0, "height": frame.1 },
        "diagnostics": serde_json::to_value(&diagnostics).unwrap_or(Value::Null),
    }))
}

/// Renders one part on its own: a reusable definition or an element subtree,
/// each addressed by its identifier (C-005, FEAT-031).
///
/// The part is resolved within the project — the caller's `project`, or the
/// server's project context — in the project's shared definition-and-element
/// namespace, a definition first and otherwise the element subtree, and compiled
/// in isolation, framed to its own bounds or to the requested size. A definition
/// has no placing scene, so it resolves its colours and style from the project's
/// default palette and default recipe; an element subtree resolves them from the
/// scene that owns it. The returned frame is the size the preview actually came
/// out at, so a caller can see when a part did not fit the requested size. A
/// part identifier that resolves to neither a definition nor an element, or that
/// is not unique across the namespace, is a structured error naming it; a part
/// with no drawable geometry is reported with a defined fallback frame rather
/// than failing (FEAT-031, D-039).
fn render_part_tool(scope: &Scope, arguments: &Map<String, Value>) -> Result<Value, CallError> {
    let args = Args::new(arguments);
    args.allowed(&[
        "part",
        "project",
        "format",
        "out",
        "width",
        "height",
        "density",
        "background",
    ])?;

    let part = args.required_string("part")?;
    let format = args.required_string("format")?;
    if format != "svg" && format != "png" {
        return Err(unsupported(format!(
            "rendering `{format}` is not supported; use `svg` or `png`"
        )));
    }
    let width = args.number("width")?;
    let height = args.number("height")?;
    let density = args.number("density")?;
    let background = args.string("background")?;

    if format == "svg" && density.is_some() {
        return Err(unsupported("`density` applies only to PNG output"));
    }
    if let Some(colour) = &background {
        if !is_color(colour) {
            return Err(exec(ToolError::new(
                INVALID_COLOR.as_str(),
                format!("`background` is not a colour SVG supports: `{colour}`"),
            )));
        }
    }

    let root = resolve_project_root(scope, args.string("project")?.as_deref())?;
    let resolved = resolve_part(&root, &part).map_err(exec_diagnostics)?;

    let model = match &resolved {
        ResolvedPart::Definition { definition, assets } => {
            compile_part_definition(definition, &assets.style_context())
        }
        ResolvedPart::Element { scene, assets } => {
            compile_subtree(scene, &part, &assets.style_context())
        }
    }
    .map_err(exec_diagnostics)?;
    let mut diagnostics = model.diagnostics.clone();

    let frame = output_frame(&model, &format, width, height, density);

    let bytes = match format.as_str() {
        "svg" => {
            let options = SvgOptions {
                width,
                height,
                background: background.clone(),
            };
            let export = export_svg_reporting(&model, &options).map_err(exec_diagnostics)?;
            diagnostics.extend(export.diagnostics);
            export.svg.into_bytes()
        }
        _ => {
            let options = RasterOptions {
                width,
                height,
                density,
                background: background.clone(),
            };
            let export = export_png_reporting(&model, &options).map_err(exec_diagnostics)?;
            diagnostics.extend(export.diagnostics);
            export.png
        }
    };

    let target = match args.string("out")? {
        Some(out) => PathBuf::from(out),
        None => default_part_output(&root, &part, &format),
    };
    let target = scope.write_path(&target).map_err(scope_error)?;
    write_atomic(&target, &bytes).map_err(|error| {
        exec(ToolError::new(
            "E_OUTPUT",
            format!("cannot write output `{}`: {error}", report_path(&target)),
        ))
    })?;

    Ok(json!({
        "path": report_path(&target),
        "format": format,
        "frame": { "width": frame.0, "height": frame.1 },
        "diagnostics": serde_json::to_value(&diagnostics).unwrap_or(Value::Null),
    }))
}

fn schema_tool(arguments: &Map<String, Value>) -> Result<Value, CallError> {
    let args = Args::new(arguments);
    args.allowed(&["form", "type"])?;
    let form = args.string("form")?.unwrap_or_else(|| "full".to_string());
    if form != "full" && form != "compact" {
        return Err(unsupported(format!(
            "schema form `{form}` is not supported; use `full` or `compact`"
        )));
    }
    let type_name = args.string("type")?;

    let rendered = match &type_name {
        Some(name) => schema_for(name),
        None => schema(if form == "compact" {
            SchemaForm::Compact
        } else {
            SchemaForm::Full
        }),
    }
    .map_err(exec_diagnostics)?;

    let value: Value = serde_json::from_str(&rendered).map_err(|error| {
        exec(ToolError::new(
            "E_SCHEMA",
            format!("the published schema is not valid JSON: {error}"),
        ))
    })?;

    Ok(json!({
        "form": form,
        "type": type_name,
        "schema": value,
        "json": rendered,
    }))
}

/// A scene argument resolved to a parsed scene and the project root its style
/// assets are read from.
struct SceneInput {
    scene: Scene,
    root: PathBuf,
    /// The scene's identifier, for a default output name; absent for a draft.
    stem: Option<String>,
}

/// Resolves the `scene`, `draft` and `project` arguments into a parsed scene
/// and the project root its assets resolve against (C-005, FEAT-019).
///
/// A call names at most one of `scene` (a project-scene identifier) and `draft`
/// (an inline document); naming both is malformed, since the two are mutually
/// exclusive. Omitting both applies the default-scene rule. The project is the
/// one the caller names, or the server's project context otherwise; an inline
/// draft resolves against the same project a project scene would.
fn load_scene(scope: &Scope, arguments: &Map<String, Value>) -> Result<SceneInput, CallError> {
    let args = Args::new(arguments);
    let scene_id = args.string("scene")?;
    let draft = args.value("draft")?;
    let project = args.string("project")?;

    if scene_id.is_some() && draft.is_some() {
        return Err(exec(ToolError::new(
            MALFORMED,
            "a call names either a scene identifier or an inline draft, not both",
        )));
    }

    let root = resolve_project_root(scope, project.as_deref())?;

    match (scene_id, draft) {
        (Some(id), None) => project_scene(&root, Some(&id)),
        (None, Some(document)) => draft_scene(&root, document),
        (None, None) => project_scene(&root, None),
        (Some(_), Some(_)) => unreachable!("naming both was refused above"),
    }
}

/// Resolves the project a call's scenes and assets read from: the caller's
/// `project`, or the server's project context (C-005, NFR-024).
///
/// The server's project context is its working directory, the root of its
/// filesystem scope, so a call that names no project stays inside the scope. A
/// caller naming another project names a directory inside the scope, or widens
/// the scope with `--allow`.
fn resolve_project_root(scope: &Scope, project: Option<&str>) -> Result<PathBuf, CallError> {
    match project {
        Some(path) => scope.read_dir(Path::new(path)).map_err(scope_error),
        None => Ok(scope.base().to_path_buf()),
    }
}

/// Resolves a project scene by identifier, or the project's default (FEAT-016).
fn project_scene(root: &Path, requested: Option<&str>) -> Result<SceneInput, CallError> {
    let resolved = resolve_scene(root, requested).map_err(exec_diagnostics)?;
    let source = resolved.source().map_err(exec_diagnostics)?;
    let scene = parse_scene(&source).map_err(exec_diagnostics)?;
    Ok(SceneInput {
        scene,
        root: resolved.root().to_path_buf(),
        stem: Some(resolved.id().to_string()),
    })
}

/// Parses an inline draft document (C-005).
///
/// The draft is a scene document as a JSON object, or the same document as JSON
/// text; either way it goes through the same strict parser a project scene does,
/// so its diagnostics carry the same locations. A draft is never resolved as a
/// project scene and never reads the project's default.
fn draft_scene(root: &Path, document: &Value) -> Result<SceneInput, CallError> {
    let source = match document {
        Value::Object(_) => serde_json::to_string(document).map_err(|error| {
            exec(ToolError::new(
                MALFORMED,
                format!("the inline draft could not be read: {error}"),
            ))
        })?,
        Value::String(text) => text.clone(),
        _ => {
            return Err(CallError::InvalidParams(
                "`draft` must be a scene document or JSON text".to_string(),
            ))
        }
    };
    let scene = parse_scene(&source).map_err(exec_diagnostics)?;
    Ok(SceneInput {
        scene,
        root: root.to_path_buf(),
        stem: None,
    })
}

/// Loads a scene's assets and compiles it, returning the model and its warnings.
fn compile_scene(input: &SceneInput) -> Result<(RenderModel, Diagnostics), CallError> {
    let assets = ProjectAssets::load(&input.root, &input.scene).map_err(exec_diagnostics)?;
    let references = assets.check_references(&input.scene);
    if references.has_errors() {
        return Err(exec_diagnostics(references));
    }
    let style = assets.style_context();
    let model = compile_with_style(&input.scene, &style).map_err(exec_diagnostics)?;
    let diagnostics = model.diagnostics.clone();
    Ok((model, diagnostics))
}

/// The default output path for a scene: `<project>/dist/<stem>.<extension>`.
fn default_output(input: &SceneInput, extension: &str) -> PathBuf {
    let stem = input.stem.clone().unwrap_or_else(|| "scene".to_string());
    input.root.join("dist").join(format!("{stem}.{extension}"))
}

/// The default output path for a part: `<project>/dist/<part>.<extension>`.
fn default_part_output(root: &Path, part: &str, extension: &str) -> PathBuf {
    root.join("dist").join(format!("{part}.{extension}"))
}

/// The output frame a part preview actually uses, after size and density
/// compose (FEAT-031).
///
/// A single requested dimension scales the other to keep the part's aspect
/// ratio, and PNG density multiplies the resolved size, mirroring the
/// exporters. Reporting the resolved frame tells a caller how large the preview
/// came out when the part did not fit the requested size.
fn output_frame(
    model: &RenderModel,
    format: &str,
    width: Option<f64>,
    height: Option<f64>,
    density: Option<f64>,
) -> (f64, f64) {
    let canvas = &model.canvas;
    let mut resolved_width = width.unwrap_or(canvas.width);
    let mut resolved_height = height.unwrap_or(canvas.height);
    let scalable = canvas.width > 0.0 && canvas.height > 0.0;
    match (width, height) {
        (Some(width), None) if scalable => {
            resolved_height = width * canvas.height / canvas.width;
        }
        (None, Some(height)) if scalable => {
            resolved_width = height * canvas.width / canvas.height;
        }
        _ => {}
    }
    if format == "png" {
        let density = density.unwrap_or(1.0);
        resolved_width *= density;
        resolved_height *= density;
    }
    (resolved_width, resolved_height)
}

/// The written output's path as it is reported to a client (C-005, FEAT-019).
///
/// The path is a JSON string that crosses the tool boundary, so it must read
/// the same on every platform. `fs::canonicalize` yields native separators and,
/// on Windows, a `\\?\` extended-length prefix (or `\\?\UNC\` for a network
/// path) — Windows-specific spellings a client would have to unwrap. The file
/// is still written at the canonical path; only the reported string is
/// normalized to forward slashes with the verbatim prefix removed, and every
/// platform accepts that form for reading.
fn report_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let normalized = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text.into_owned()
    };
    normalized.replace('\\', "/")
}

fn exec(error: ToolError) -> CallError {
    CallError::Exec(Box::new(error))
}

fn exec_diagnostics(diagnostics: Diagnostics) -> CallError {
    exec(ToolError::from_diagnostics(diagnostics))
}

fn scope_error(error: ScopeError) -> CallError {
    exec(ToolError::new(SCOPE, error.to_string()))
}

fn unsupported(message: impl Into<String>) -> CallError {
    exec(ToolError::new("E_UNSUPPORTED", message))
}

/// Whether the PDF rollout flag is set (FEAT-014).
///
/// The flag is read from the environment so the capability can be enabled
/// without a rebuild, matching the command line's gate. It is off by default.
fn pdf_enabled() -> bool {
    enabled_value(std::env::var(ENABLE_PDF_ENV).ok().as_deref())
}

/// Whether a flag value enables a gated capability.
fn enabled_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("on")
    )
}

/// The `project` input property shared by the tools that resolve a project.
fn project_property() -> Value {
    json!({
        "type": "string",
        "description": "Project root holding scenes/, definitions/, palettes/, strokes/, gradients/, recipes/ and assets/. Defaults to the server's project context."
    })
}

/// The shared `scene` / `draft` / `project` input properties (C-005).
fn scene_properties() -> Value {
    json!({
        "scene": {
            "type": "string",
            "description": "A scene identifier resolved among the project's scenes (`scenes/<id>.json`). Omit it to use the project's default scene. Mutually exclusive with `draft`."
        },
        "draft": {
            "type": ["object", "string"],
            "description": "An inline scene document, as a JSON object or JSON text, used as a draft instead of a project scene. It is never the project's default; its assets resolve against `project` or the server's project context. Mutually exclusive with `scene`."
        },
        "project": project_property()
    })
}

fn validate_definition() -> Value {
    json!({
        "name": "validate",
        "title": "Validate a scene",
        "description": "Check a scene against the language contract and its project's style assets. Address a project scene by its identifier or send an inline draft; with neither, the project's default scene is used. Returns diagnostics with a location for each finding; nothing is written.",
        "inputSchema": {
            "type": "object",
            "properties": scene_properties(),
            "additionalProperties": false
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "valid": { "type": "boolean", "description": "True when the scene has no errors." },
                "diagnostics": {
                    "type": "array",
                    "description": "Findings, each with a severity, code, message and location.",
                    "items": { "type": "object" }
                }
            },
            "required": ["valid", "diagnostics"]
        },
        "annotations": { "readOnlyHint": true }
    })
}

fn compile_definition() -> Value {
    json!({
        "name": "compile",
        "title": "Compile a scene",
        "description": "Resolve a scene and its project's palette, strokes, gradients and fonts into the render model every exporter reads (C-003). Address a project scene by its identifier or send an inline draft; with neither, the project's default scene is used. Returns the model and any warnings.",
        "inputSchema": {
            "type": "object",
            "properties": scene_properties(),
            "additionalProperties": false
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "model": { "type": "object", "description": "The compiled render model (C-003)." },
                "diagnostics": {
                    "type": "array",
                    "description": "Warnings recorded while compiling.",
                    "items": { "type": "object" }
                }
            },
            "required": ["model", "diagnostics"]
        },
        "annotations": { "readOnlyHint": true }
    })
}

fn render_definition() -> Value {
    let mut properties = scene_properties();
    let object = properties
        .as_object_mut()
        .expect("the scene properties are an object");
    object.insert(
        "format".to_string(),
        json!({ "type": "string", "enum": ["svg", "png", "pdf"], "description": "The output format. `pdf` is available only when the `enable_pdf_export` rollout flag is enabled." }),
    );
    object.insert(
        "out".to_string(),
        json!({ "type": "string", "description": "Where to write the output. Defaults to <project>/dist/<scene>.<format>." }),
    );
    object.insert(
        "width".to_string(),
        json!({ "type": "number", "description": "Output width in scene units; the canvas width when absent." }),
    );
    object.insert(
        "height".to_string(),
        json!({ "type": "number", "description": "Output height in scene units; the canvas height when absent." }),
    );
    object.insert(
        "density".to_string(),
        json!({ "type": "number", "description": "Pixel density multiplier; PNG only." }),
    );
    object.insert(
        "background".to_string(),
        json!({ "type": "string", "description": "Background override, or `transparent`." }),
    );

    json!({
        "name": "render",
        "title": "Render a scene",
        "description": "Compile a scene and write it as SVG or PNG, or as vector PDF when the `enable_pdf_export` rollout flag is enabled. Address a project scene by its identifier or send an inline draft; with neither, the project's default scene is used. Returns the path it wrote; a failure writes nothing.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["format"],
            "additionalProperties": false
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The path the output was written to." },
                "format": { "type": "string", "enum": ["svg", "png", "pdf"] },
                "diagnostics": {
                    "type": "array",
                    "description": "Warnings recorded while compiling and exporting.",
                    "items": { "type": "object" }
                }
            },
            "required": ["path", "format", "diagnostics"]
        },
        "annotations": { "readOnlyHint": false }
    })
}

fn inspect_definition() -> Value {
    let mut properties = scene_properties();
    let object = properties
        .as_object_mut()
        .expect("the scene properties are an object");
    object.insert(
        "format".to_string(),
        json!({ "type": "string", "enum": ["svg", "png"], "description": "The preview format; SVG or PNG." }),
    );
    object.insert(
        "out".to_string(),
        json!({ "type": "string", "description": "Where to write the preview. Defaults to <project>/dist/<scene>.<format>." }),
    );
    object.insert(
        "width".to_string(),
        json!({ "type": "number", "description": "Preview width in scene units; the canvas width when absent." }),
    );
    object.insert(
        "height".to_string(),
        json!({ "type": "number", "description": "Preview height in scene units; the canvas height when absent." }),
    );
    object.insert(
        "density".to_string(),
        json!({ "type": "number", "description": "Pixel density multiplier; PNG only." }),
    );
    object.insert(
        "background".to_string(),
        json!({ "type": "string", "description": "Background override, or `transparent`." }),
    );

    json!({
        "name": "inspect",
        "title": "Render a scene preview for inspection",
        "description": "Compile a scene and write a preview image for a person or a model to compare against the request (FEAT-022). Structural checks run first and gate the preview, and the size is configurable so a preview too small to judge can be raised. Returns the path it wrote and the size it rendered at; a failure writes nothing.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["format"],
            "additionalProperties": false
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The path the preview was written to." },
                "format": { "type": "string", "enum": ["svg", "png"] },
                "size": {
                    "type": "object",
                    "description": "The preview's resolved size after the requested size and density compose.",
                    "properties": {
                        "width": { "type": "number" },
                        "height": { "type": "number" }
                    },
                    "required": ["width", "height"]
                },
                "diagnostics": {
                    "type": "array",
                    "description": "Findings recorded while compiling and exporting, including the note that the visual comparison with the request is the caller's.",
                    "items": { "type": "object" }
                }
            },
            "required": ["path", "format", "size", "diagnostics"]
        },
        "annotations": { "readOnlyHint": false }
    })
}

fn render_part_definition() -> Value {
    let properties = json!({
        "part": {
            "type": "string",
            "description": "The identifier of a reusable definition or of a named element subtree, resolved within the project."
        },
        "project": project_property(),
        "format": { "type": "string", "enum": ["svg", "png"], "description": "The output format." },
        "out": { "type": "string", "description": "Where to write the preview. Defaults to <project>/dist/<part>.<format>." },
        "width": { "type": "number", "description": "Preview width in scene units; the part's own width when absent." },
        "height": { "type": "number", "description": "Preview height in scene units; the part's own height when absent." },
        "density": { "type": "number", "description": "Pixel density multiplier; PNG only." },
        "background": { "type": "string", "description": "Background override, or `transparent`." }
    });

    json!({
        "name": "render-part",
        "title": "Render a part",
        "description": "Compile one part on its own — a reusable definition or a named element subtree, each addressed by its identifier — and write it as SVG or PNG, framed to the part's own bounds or to a requested size. Returns the path it wrote and the frame the preview used; a failure writes nothing.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["part", "format"],
            "additionalProperties": false
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The path the preview was written to." },
                "format": { "type": "string", "enum": ["svg", "png"] },
                "frame": {
                    "type": "object",
                    "description": "The preview's resolved size after the requested size and density compose.",
                    "properties": {
                        "width": { "type": "number" },
                        "height": { "type": "number" }
                    },
                    "required": ["width", "height"]
                },
                "diagnostics": {
                    "type": "array",
                    "description": "Warnings recorded while compiling and exporting, including an empty part's fallback frame.",
                    "items": { "type": "object" }
                }
            },
            "required": ["path", "format", "frame", "diagnostics"]
        },
        "annotations": { "readOnlyHint": false }
    })
}

fn schema_definition() -> Value {
    json!({
        "name": "schema",
        "title": "Read the language contract",
        "description": "Return the published scene-language contract, or one named type's properties and allowed values, so a model can author a valid scene without prior training.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "form": {
                    "type": "string",
                    "enum": ["full", "compact"],
                    "description": "The whole contract, indented (`full`, the default) or minified (`compact`)."
                },
                "type": {
                    "type": "string",
                    "description": "Return one type's schema instead of the whole contract; matched case-insensitively. An unknown type lists the closest names."
                }
            },
            "additionalProperties": false
        },
        "outputSchema": {
            "type": "object",
            "properties": {
                "form": { "type": "string", "enum": ["full", "compact"] },
                "type": { "type": ["string", "null"] },
                "schema": { "type": "object", "description": "The schema document." },
                "json": { "type": "string", "description": "The schema document as serialized JSON text." }
            },
            "required": ["form", "schema", "json"]
        },
        "annotations": { "readOnlyHint": true }
    })
}

/// A typed view over a tool call's `arguments`, refusing anything the tool does
/// not declare.
struct Args<'a> {
    map: &'a Map<String, Value>,
}

impl<'a> Args<'a> {
    fn new(map: &'a Map<String, Value>) -> Self {
        Self { map }
    }

    fn allowed(&self, names: &[&str]) -> Result<(), CallError> {
        for key in self.map.keys() {
            if !names.contains(&key.as_str()) {
                return Err(CallError::InvalidParams(format!(
                    "unknown argument `{key}`"
                )));
            }
        }
        Ok(())
    }

    fn string(&self, name: &str) -> Result<Option<String>, CallError> {
        match self.map.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(CallError::InvalidParams(format!(
                "argument `{name}` must be a string"
            ))),
        }
    }

    fn required_string(&self, name: &str) -> Result<String, CallError> {
        self.string(name)?
            .ok_or_else(|| CallError::InvalidParams(format!("missing required argument `{name}`")))
    }

    /// The raw value of an argument, or `None` when it is absent or null. Used
    /// for `draft`, which is a scene document rather than a scalar.
    fn value(&self, name: &str) -> Result<Option<&Value>, CallError> {
        match self.map.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => Ok(Some(value)),
        }
    }

    fn number(&self, name: &str) -> Result<Option<f64>, CallError> {
        match self.map.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(number)) => number.as_f64().map(Some).ok_or_else(|| {
                CallError::InvalidParams(format!("argument `{name}` is not a finite number"))
            }),
            Some(_) => Err(CallError::InvalidParams(format!(
                "argument `{name}` must be a number"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// Serializes the tests that toggle the PDF rollout flag, which lives in the
    /// process environment and is shared by every test in this binary.
    static PDF_FLAG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Holds the PDF-flag lock for the duration of a test.
    fn pdf_flag() -> std::sync::MutexGuard<'static, ()> {
        PDF_FLAG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    const RECT_SCENE: &str = r##"{
      "id": "s",
      "projectId": "p",
      "name": "Rect",
      "formatVersion": "0.2",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "r1", "sceneId": "s", "order": 0, "kind": "rect",
          "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    const INVALID_SCENE: &str = r##"{
      "id": "s",
      "projectId": "p",
      "name": "Bad",
      "formatVersion": "0.2",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "r1", "sceneId": "s", "order": 0, "kind": "rect",
          "geometry": { "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 2, "visible": true
        }
      ]
    }"##;

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vectr-mcp-tools-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("creates the temp dir");
        dir
    }

    /// Writes a project whose configuration is `config` and whose scene
    /// documents are the given `(identifier, document)` pairs.
    fn write_project(dir: &Path, config: &str, scenes: &[(&str, &str)]) {
        fs::write(dir.join("vectr.project.json"), config).expect("writes the project config");
        for (id, text) in scenes {
            let path = dir.join("scenes").join(format!("{id}.json"));
            fs::create_dir_all(path.parent().expect("scenes/")).expect("creates scenes/");
            fs::write(path, text).expect("writes the scene document");
        }
    }

    /// Writes one project file, creating its parent directory.
    fn write_file(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a parent")).expect("creates the parent");
        fs::write(path, text).expect("writes the file");
    }

    /// A palette the part previews resolve their tokens against.
    const PART_PALETTE: &str = r##"{"id":"brand","projectId":"project","name":"Brand","tokens":[{"name":"accent","value":"#ff0000"}]}"##;

    /// A definition whose single rect sits off the origin, so a preview must
    /// frame it to its own bounds.
    const BADGE_DEFINITION: &str = r##"{
      "id": "badge",
      "projectId": "project",
      "name": "Badge",
      "parameters": [],
      "origin": { "x": 0, "y": 0 },
      "elements": [
        {
          "id": "r1", "definitionId": "badge", "order": 0, "kind": "rect",
          "geometry": { "x": 10, "y": 20, "width": 30, "height": 40 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "fill": { "kind": "token", "ref": "accent" },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    /// A definition with no drawable geometry.
    const EMPTY_DEFINITION: &str = r##"{
      "id": "empty",
      "projectId": "project",
      "name": "Empty",
      "parameters": [],
      "origin": { "x": 0, "y": 0 },
      "elements": []
    }"##;

    /// A scene holding a named group subtree beside an unrelated shape.
    const SUBTREE_SCENE: &str = r##"{
      "id": "main",
      "projectId": "project",
      "name": "Main",
      "formatVersion": "0.2",
      "paletteId": "brand",
      "canvas": { "width": 200, "height": 200, "background": "#ffffff" },
      "elements": [
        {
          "id": "mark", "sceneId": "main", "order": 0, "kind": "group", "name": "Mark",
          "geometry": {},
          "transform": { "translateX": 50, "translateY": 50, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        },
        {
          "id": "mark-rect", "sceneId": "main", "parentId": "mark", "order": 0, "kind": "rect",
          "geometry": { "x": 0, "y": 0, "width": 20, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "fill": { "kind": "token", "ref": "accent" },
          "opacity": 1, "visible": true
        },
        {
          "id": "other", "sceneId": "main", "order": 1, "kind": "rect",
          "geometry": { "x": 100, "y": 100, "width": 50, "height": 50 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    /// A palette the default scene names, distinct from the project's default,
    /// so a test can tell which palette a definition resolved against.
    const BLUE_PALETTE: &str = r##"{"id":"blue","projectId":"project","name":"Blue","tokens":[{"name":"accent","value":"#0000ff"}]}"##;

    /// A default scene that names the `blue` palette rather than the project's
    /// default `brand`, so an isolated definition must not resolve its tokens.
    const BLUE_SCENE: &str = r##"{
      "id": "main",
      "projectId": "project",
      "name": "Main",
      "formatVersion": "0.2",
      "paletteId": "blue",
      "canvas": { "width": 200, "height": 200, "background": "#ffffff" },
      "elements": [
        {
          "id": "main-rect", "sceneId": "main", "order": 0, "kind": "rect",
          "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "fill": { "kind": "token", "ref": "accent" },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    /// A definition whose identifier collides with the default scene's `mark`
    /// element, so the shared namespace is not unique (FEAT-031, D-039).
    const MARK_DEFINITION: &str = r##"{
      "id": "mark",
      "projectId": "project",
      "name": "Mark",
      "parameters": [],
      "origin": { "x": 0, "y": 0 },
      "elements": []
    }"##;

    /// Writes a project with a default scene, a palette, and two definitions.
    fn part_project(dir: &Path) {
        write_project(
            dir,
            r#"{"defaultSceneId":"main","defaultPaletteId":"brand"}"#,
            &[("main", SUBTREE_SCENE)],
        );
        write_file(dir, "palettes/brand.json", PART_PALETTE);
        write_file(dir, "definitions/badge.json", BADGE_DEFINITION);
        write_file(dir, "definitions/empty.json", EMPTY_DEFINITION);
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().expect("an object").clone()
    }

    /// A scene document parsed into a JSON object, for a `draft` argument.
    fn draft_object(text: &str) -> Value {
        serde_json::from_str(text).expect("a scene object")
    }

    fn scope(dir: &Path) -> Scope {
        Scope::new(vec![dir.to_path_buf()])
    }

    fn assert_exec(result: Result<Value, CallError>) -> ToolError {
        match result {
            Err(CallError::Exec(error)) => *error,
            other => panic!("expected a tool execution error, got {other:?}"),
        }
    }

    #[test]
    fn definitions_name_six_tools_with_input_and_output_schemas() {
        let tools = definitions();
        let list = tools.as_array().expect("an array");
        let names: Vec<&str> = list
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(
            names,
            vec![
                "validate",
                "compile",
                "render",
                "inspect",
                "render-part",
                "schema"
            ]
        );
        for tool in list {
            assert!(tool.get("inputSchema").is_some(), "input schema: {tool}");
            assert!(tool.get("outputSchema").is_some(), "output schema: {tool}");
        }

        // The scene-addressing arguments are discoverable: every scene tool
        // publishes `scene`, `draft` and `project`, and neither `scene` nor
        // `draft` is required, so omitting both applies the default rule.
        for tool in list.iter().filter(|tool| {
            matches!(
                tool["name"].as_str(),
                Some("validate" | "compile" | "render" | "inspect")
            )
        }) {
            let properties = &tool["inputSchema"]["properties"];
            for name in ["scene", "draft", "project"] {
                assert!(
                    properties.get(name).is_some(),
                    "{} publishes `{name}`: {tool}",
                    tool["name"]
                );
            }
            let required = tool["inputSchema"]["required"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            assert!(!required
                .iter()
                .any(|name| name == "scene" || name == "draft"));
        }

        // The inspect tool takes a preview format and a configurable size, and
        // reports the size it rendered at (FEAT-022).
        let inspect = list
            .iter()
            .find(|tool| tool["name"] == "inspect")
            .expect("the inspect tool is published");
        let properties = &inspect["inputSchema"]["properties"];
        for name in ["format", "out", "width", "height", "density", "background"] {
            assert!(
                properties.get(name).is_some(),
                "inspect publishes `{name}`: {inspect}"
            );
        }
        let required: Vec<&str> = inspect["inputSchema"]["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(required, vec!["format"]);

        // The part tool addresses a part within a project rather than a scene,
        // and requires the part and a format; an omitted size frames the part to
        // its own bounds (FEAT-031).
        let part = list
            .iter()
            .find(|tool| tool["name"] == "render-part")
            .expect("the part tool is published");
        let properties = &part["inputSchema"]["properties"];
        assert!(properties.get("part").is_some(), "{part}");
        assert!(properties.get("project").is_some(), "{part}");
        assert!(properties.get("scene").is_none(), "{part}");
        let required: Vec<&str> = part["inputSchema"]["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(required, vec!["part", "format"]);
    }

    #[test]
    fn validate_accepts_a_project_scene_by_identifier() {
        let dir = tempdir("validate-named");
        write_project(&dir, "{}", &[("logo", RECT_SCENE)]);
        let result =
            call(&scope(&dir), "validate", &args(json!({ "scene": "logo" }))).expect("valid");
        assert_eq!(result["valid"], Value::Bool(true));
        assert_eq!(result["diagnostics"], json!([]));
    }

    #[test]
    fn validate_accepts_an_inline_draft_as_an_object_and_as_text() {
        let dir = tempdir("validate-draft");
        let as_object = call(
            &scope(&dir),
            "validate",
            &args(json!({ "draft": draft_object(RECT_SCENE) })),
        )
        .expect("valid");
        assert_eq!(as_object["valid"], Value::Bool(true));

        let as_text = call(
            &scope(&dir),
            "validate",
            &args(json!({ "draft": RECT_SCENE })),
        )
        .expect("valid");
        assert_eq!(as_text["valid"], Value::Bool(true));
    }

    #[test]
    fn validate_reports_an_invalid_draft_as_a_structured_error() {
        let dir = tempdir("validate-invalid");
        let error = assert_exec(call(
            &scope(&dir),
            "validate",
            &args(json!({ "draft": INVALID_SCENE })),
        ));
        assert_eq!(error.code, "E_SCHEMA");
        assert!(!error.diagnostics.is_empty());
    }

    #[test]
    fn a_call_naming_both_a_scene_and_a_draft_is_malformed() {
        let dir = tempdir("both");
        write_project(&dir, "{}", &[("logo", RECT_SCENE)]);
        let error = assert_exec(call(
            &scope(&dir),
            "compile",
            &args(json!({ "scene": "logo", "draft": RECT_SCENE })),
        ));
        assert_eq!(error.code, MALFORMED);
        assert!(!error.message.is_empty());
    }

    #[test]
    fn an_omitted_scene_uses_the_project_default() {
        let dir = tempdir("default");
        write_project(
            &dir,
            r#"{"defaultSceneId":"logo"}"#,
            &[("logo", RECT_SCENE)],
        );
        let result = call(&scope(&dir), "compile", &Map::new()).expect("compiles");
        assert_eq!(result["model"]["nodes"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn a_project_that_names_no_default_reports_no_scene_selected() {
        let dir = tempdir("no-default");
        write_project(&dir, "{}", &[("one", RECT_SCENE), ("two", RECT_SCENE)]);
        let error = assert_exec(call(&scope(&dir), "validate", &Map::new()));
        assert_eq!(error.code, "E_SCENE");
        assert!(
            error.message.contains("no default scene"),
            "{}",
            error.message
        );
    }

    #[test]
    fn a_named_scene_overrides_the_project_default() {
        let dir = tempdir("override");
        // The default is invalid; naming the valid scene must win.
        write_project(
            &dir,
            r#"{"defaultSceneId":"bad"}"#,
            &[("bad", INVALID_SCENE), ("good", RECT_SCENE)],
        );
        let result =
            call(&scope(&dir), "validate", &args(json!({ "scene": "good" }))).expect("valid");
        assert_eq!(result["valid"], Value::Bool(true));
    }

    #[test]
    fn a_scene_identifier_no_document_provides_is_a_structured_error() {
        let dir = tempdir("missing-named");
        write_project(&dir, "{}", &[]);
        let error = assert_exec(call(
            &scope(&dir),
            "compile",
            &args(json!({ "scene": "absent" })),
        ));
        assert_eq!(error.code, "E_SCENE");
        assert!(error.message.contains("absent"), "{}", error.message);
    }

    #[test]
    fn a_default_that_resolves_to_no_document_names_it() {
        let dir = tempdir("missing-default");
        write_project(&dir, r#"{"defaultSceneId":"absent"}"#, &[]);
        let error = assert_exec(call(&scope(&dir), "compile", &Map::new()));
        assert_eq!(error.code, "E_SCENE");
        assert!(error.message.contains("absent"), "{}", error.message);
    }

    #[test]
    fn an_inline_draft_is_used_without_reading_or_writing_the_default() {
        let dir = tempdir("draft-default");
        write_project(
            &dir,
            r#"{"defaultSceneId":"logo"}"#,
            &[("logo", RECT_SCENE)],
        );

        // The draft compiles even though the project's default is a different
        // scene, and the draft's own document is never written into the project.
        let result = call(
            &scope(&dir),
            "compile",
            &args(json!({ "draft": RECT_SCENE })),
        )
        .expect("compiles");
        assert_eq!(result["model"]["nodes"].as_array().map(Vec::len), Some(1));
        assert!(
            !dir.join("scenes").join("s.json").exists(),
            "a draft never becomes a project scene"
        );

        // The project configuration is untouched, and the default still resolves.
        let config = fs::read_to_string(dir.join("vectr.project.json")).expect("reads the config");
        assert_eq!(config, r#"{"defaultSceneId":"logo"}"#);
        let default = call(&scope(&dir), "compile", &Map::new()).expect("compiles");
        assert_eq!(default["model"]["nodes"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn an_inline_draft_resolves_its_assets_against_the_project() {
        let dir = tempdir("draft-assets");
        write_project(&dir, "{}", &[]);
        fs::create_dir_all(dir.join("palettes")).expect("creates palettes/");
        fs::write(
            dir.join("palettes").join("brand.json"),
            r##"{"id":"brand","projectId":"p","name":"Brand","tokens":[{"name":"accent","value":"#4f46e5"}]}"##,
        )
        .expect("writes the palette");
        let draft = RECT_SCENE
            .replace(
                r##""elements": ["##,
                r##""paletteId": "brand", "elements": ["##,
            )
            .replace(
                r##""kind": "rect","##,
                r##""kind": "rect", "fill": {"kind": "token", "ref": "accent"},"##,
            );

        let result =
            call(&scope(&dir), "compile", &args(json!({ "draft": draft }))).expect("compiles");
        assert_eq!(
            result["model"]["nodes"][0]["paint"]["fill"]["value"], "#4f46e5",
            "the draft resolves the project's palette: {result}"
        );
    }

    #[test]
    fn an_inline_draft_whose_asset_reference_resolves_nowhere_is_a_structured_error() {
        let dir = tempdir("draft-missing-asset");
        write_project(&dir, "{}", &[]);
        // The draft names a palette the project does not provide.
        let draft = RECT_SCENE.replace(
            r##""elements": ["##,
            r##""paletteId": "brand", "elements": ["##,
        );
        let error = assert_exec(call(
            &scope(&dir),
            "validate",
            &args(json!({ "draft": draft })),
        ));
        assert_eq!(error.code, "E_PROJECT_ASSET");
        assert!(error.message.contains("brand"), "{}", error.message);
    }

    #[test]
    fn an_inline_draft_with_an_undefined_element_reference_names_its_location() {
        let dir = tempdir("draft-undefined-ref");
        write_project(&dir, "{}", &[]);
        let draft = RECT_SCENE.replace(
            r##""kind": "rect","##,
            r##""kind": "rect", "stroke": {"profileId": "outline", "paint": {"kind": "token", "ref": "accent"}},"##,
        );
        let error = assert_exec(call(
            &scope(&dir),
            "compile",
            &args(json!({ "draft": draft })),
        ));
        assert_eq!(error.code, "E_UNDEFINED_STROKE");
        let location = error.location.expect("the reference is located");
        assert_eq!(location.element_id.as_deref(), Some("r1"));
    }

    #[test]
    fn a_draft_of_the_wrong_type_is_invalid_params() {
        let dir = tempdir("draft-type");
        let result = call(&scope(&dir), "compile", &args(json!({ "draft": 5 })));
        assert!(matches!(result, Err(CallError::InvalidParams(_))));
    }

    #[test]
    fn compile_returns_the_render_model() {
        let dir = tempdir("compile");
        let result = call(
            &scope(&dir),
            "compile",
            &args(json!({ "draft": RECT_SCENE })),
        )
        .expect("compiles");
        assert!(result["model"]["nodes"]
            .as_array()
            .is_some_and(|n| n.len() == 1));
        assert_eq!(result["diagnostics"], json!([]));
    }

    #[test]
    fn compile_reports_a_cycle_as_a_structured_error() {
        let dir = tempdir("compile-cycle");
        let cyclic = RECT_SCENE.replace(
            r#""id": "r1", "sceneId": "s", "order": 0, "kind": "rect","#,
            r#""id": "r1", "sceneId": "s", "parentId": "r1", "order": 0, "kind": "rect","#,
        );
        let error = assert_exec(call(
            &scope(&dir),
            "compile",
            &args(json!({ "draft": cyclic })),
        ));
        assert!(!error.diagnostics.is_empty());
    }

    #[test]
    fn render_writes_svg_and_returns_its_path() {
        let dir = tempdir("render-svg");
        let out = dir.join("dist").join("rect.svg");
        let result = call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "svg", "out": out })),
        )
        .expect("renders");
        let path = result["path"].as_str().expect("a path");
        let svg = fs::read_to_string(path).expect("reads the svg");
        assert!(svg.contains("<svg"), "{svg}");
        assert!(svg.contains("<rect"), "{svg}");
        assert_eq!(result["format"], "svg");
    }

    #[test]
    fn render_writes_png_and_returns_its_path() {
        let dir = tempdir("render-png");
        let out = dir.join("rect.png");
        let result = call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "png", "out": out })),
        )
        .expect("renders");
        let bytes = fs::read(result["path"].as_str().expect("a path")).expect("reads the png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn a_draft_render_defaults_to_dist_scene() {
        let dir = tempdir("render-default");
        let result = call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "svg" })),
        )
        .expect("renders");
        let path = result["path"].as_str().expect("a path");
        assert!(path.ends_with("dist/scene.svg"), "{path}");
    }

    #[test]
    fn a_project_scene_render_defaults_to_its_identifier() {
        let dir = tempdir("render-project-default");
        write_project(&dir, "{}", &[("logo", RECT_SCENE)]);
        let result = call(
            &scope(&dir),
            "render",
            &args(json!({ "scene": "logo", "format": "svg" })),
        )
        .expect("renders");
        let path = result["path"].as_str().expect("a path");
        assert!(path.ends_with("dist/logo.svg"), "{path}");
    }

    #[test]
    fn a_reported_path_is_portable_across_platforms() {
        // The written path is a JSON string a client reads on any platform, so
        // a Windows canonical path is reported with forward slashes and without
        // the `\\?\` extended-length prefix `fs::canonicalize` yields there
        // (C-005, FEAT-019).
        assert_eq!(
            report_path(Path::new(r"\\?\C:\proj\dist\logo.svg")),
            "C:/proj/dist/logo.svg"
        );
        assert_eq!(
            report_path(Path::new(r"C:\proj\dist\logo.svg")),
            "C:/proj/dist/logo.svg"
        );
        assert_eq!(
            report_path(Path::new(r"\\?\UNC\server\share\dist\logo.svg")),
            "//server/share/dist/logo.svg"
        );
        assert_eq!(
            report_path(Path::new("/home/kenji/proj/dist/logo.svg")),
            "/home/kenji/proj/dist/logo.svg"
        );
    }

    #[test]
    fn render_refuses_an_unsupported_format_without_writing() {
        let dir = tempdir("render-unsupported");
        let out = dir.join("out.jpeg");
        let error = assert_exec(call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "jpeg", "out": out })),
        ));
        assert_eq!(error.code, "E_UNSUPPORTED");
        assert!(!dir.join("out.jpeg").exists());
    }

    #[test]
    fn pdf_render_is_gated_by_the_rollout_flag() {
        // Off by default: an unsupported capability and nothing written
        // (FEAT-014, C-005).
        let _guard = pdf_flag();
        std::env::remove_var(ENABLE_PDF_ENV);
        let dir = tempdir("render-pdf-off");
        let out = dir.join("out.pdf");
        let error = assert_exec(call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "pdf", "out": out.clone() })),
        ));
        assert_eq!(error.code, "E_UNSUPPORTED");
        assert!(!out.exists(), "nothing is written while the flag is off");

        // Enabled: a vector PDF is written.
        std::env::set_var(ENABLE_PDF_ENV, "1");
        let result = call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "pdf", "out": out.clone() })),
        )
        .expect("renders");
        std::env::remove_var(ENABLE_PDF_ENV);

        assert_eq!(result["format"], "pdf");
        let bytes = fs::read(out).expect("reads the pdf");
        assert!(bytes.starts_with(b"%PDF-"), "a PDF signature");
        assert!(bytes.ends_with(b"%%EOF\n"), "a complete document");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("/Subtype /Image"), "vector only: {text}");
    }

    #[test]
    fn pdf_render_defaults_to_dist_scene_pdf() {
        let _guard = pdf_flag();
        std::env::set_var(ENABLE_PDF_ENV, "1");
        let dir = tempdir("render-pdf-default");
        let result = call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "pdf" })),
        )
        .expect("renders");
        std::env::remove_var(ENABLE_PDF_ENV);
        let path = result["path"].as_str().expect("a path");
        assert!(path.ends_with("dist/scene.pdf"), "{path}");
    }

    #[test]
    fn a_pdf_render_refuses_density() {
        let _guard = pdf_flag();
        std::env::set_var(ENABLE_PDF_ENV, "1");
        let dir = tempdir("render-pdf-density");
        let error = assert_exec(call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "pdf", "density": 2 })),
        ));
        std::env::remove_var(ENABLE_PDF_ENV);
        assert_eq!(error.code, "E_UNSUPPORTED");
    }

    #[test]
    fn the_pdf_flag_recognizes_the_enabling_values() {
        assert!(enabled_value(Some("1")));
        assert!(enabled_value(Some("true")));
        assert!(enabled_value(Some("on")));
        assert!(!enabled_value(Some("0")));
        assert!(!enabled_value(None));
    }

    #[test]
    fn render_refuses_an_output_outside_the_scope() {
        let dir = tempdir("render-scope");
        let outside = tempdir("render-outside");
        let error = assert_exec(call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "svg", "out": outside.join("out.svg") })),
        ));
        assert_eq!(error.code, SCOPE);
    }

    // -----------------------------------------------------------------------
    // Render-in-the-loop inspection (FEAT-022)
    // -----------------------------------------------------------------------

    #[test]
    fn inspect_writes_a_png_preview_and_reports_its_size() {
        let dir = tempdir("inspect-png");
        let out = dir.join("preview.png");
        let result = call(
            &scope(&dir),
            "inspect",
            &args(json!({ "draft": RECT_SCENE, "format": "png", "out": out.clone() })),
        )
        .expect("inspects");
        assert_eq!(result["format"], "png");
        assert_eq!(result["size"]["width"], 100.0);
        assert_eq!(result["size"]["height"], 100.0);
        let bytes = fs::read(out).expect("reads the preview");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn inspect_writes_an_svg_preview() {
        let dir = tempdir("inspect-svg");
        let out = dir.join("preview.svg");
        let result = call(
            &scope(&dir),
            "inspect",
            &args(json!({ "draft": RECT_SCENE, "format": "svg", "out": out.clone() })),
        )
        .expect("inspects");
        assert_eq!(result["format"], "svg");
        let svg = fs::read_to_string(out).expect("reads the preview");
        assert!(svg.contains("<svg"), "{svg}");
        assert!(svg.contains("<rect"), "{svg}");
    }

    #[test]
    fn inspect_notes_that_the_visual_comparison_is_the_callers() {
        // The server has no inspection capability of its own, so it records the
        // limitation rather than hiding it (FEAT-022).
        let dir = tempdir("inspect-note");
        let result = call(
            &scope(&dir),
            "inspect",
            &args(json!({ "draft": RECT_SCENE, "format": "png" })),
        )
        .expect("inspects");
        let diagnostics = result["diagnostics"].as_array().expect("diagnostics");
        assert!(
            diagnostics
                .iter()
                .any(|finding| finding["code"] == "W_INSPECTION_UNAVAILABLE"),
            "the missing inspection capability is noted: {result}"
        );
    }

    #[test]
    fn inspect_size_is_configurable() {
        // A preview too small to judge can be raised (FEAT-022).
        let dir = tempdir("inspect-size");
        let result = call(
            &scope(&dir),
            "inspect",
            &args(json!({ "draft": RECT_SCENE, "format": "png", "width": 200 })),
        )
        .expect("inspects");
        assert_eq!(result["size"]["width"], 200.0);
        assert_eq!(result["size"]["height"], 200.0);
    }

    #[test]
    fn inspect_defaults_its_output_to_dist_scene() {
        let dir = tempdir("inspect-default-out");
        let result = call(
            &scope(&dir),
            "inspect",
            &args(json!({ "draft": RECT_SCENE, "format": "png" })),
        )
        .expect("inspects");
        let path = result["path"].as_str().expect("a path");
        assert!(path.ends_with("dist/scene.png"), "{path}");
    }

    #[test]
    fn inspect_runs_structural_checks_before_rendering() {
        // A structural error gates the preview: it is reported and nothing is
        // written, so the findings stand even though no inspection follows
        // (FEAT-018, FEAT-022).
        let dir = tempdir("inspect-structural");
        let draft = RECT_SCENE.replace(
            r##""kind": "rect","##,
            r##""kind": "rect", "stroke": {"profileId": "outline", "paint": {"kind": "token", "ref": "accent"}},"##,
        );
        let out = dir.join("preview.png");
        let error = assert_exec(call(
            &scope(&dir),
            "inspect",
            &args(json!({ "draft": draft, "format": "png", "out": out.clone() })),
        ));
        assert_eq!(error.code, "E_UNDEFINED_STROKE");
        assert!(!out.exists(), "nothing is written when the checks fail");
    }

    #[test]
    fn inspect_refuses_an_unsupported_format() {
        let dir = tempdir("inspect-format");
        let error = assert_exec(call(
            &scope(&dir),
            "inspect",
            &args(json!({ "draft": RECT_SCENE, "format": "pdf" })),
        ));
        assert_eq!(error.code, "E_UNSUPPORTED");
    }

    #[test]
    fn inspect_refuses_a_density_on_svg() {
        let dir = tempdir("inspect-density");
        let error = assert_exec(call(
            &scope(&dir),
            "inspect",
            &args(json!({ "draft": RECT_SCENE, "format": "svg", "density": 2 })),
        ));
        assert_eq!(error.code, "E_UNSUPPORTED");
    }

    #[test]
    fn inspect_requires_a_format() {
        let dir = tempdir("inspect-required");
        let result = call(
            &scope(&dir),
            "inspect",
            &args(json!({ "draft": RECT_SCENE })),
        );
        assert!(matches!(result, Err(CallError::InvalidParams(_))));
    }

    // -----------------------------------------------------------------------
    // Part-scoped rendering (FEAT-031)
    // -----------------------------------------------------------------------

    #[test]
    fn render_part_writes_a_definition_preview_framed_to_its_bounds() {
        let dir = tempdir("part-definition");
        part_project(&dir);
        let out = dir.join("badge.svg");
        let result = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "badge", "format": "svg", "out": out })),
        )
        .expect("renders");
        assert_eq!(result["format"], "svg");
        assert_eq!(result["frame"]["width"], 30.0);
        assert_eq!(result["frame"]["height"], 40.0);

        let svg = fs::read_to_string(result["path"].as_str().expect("a path")).expect("reads");
        assert!(svg.contains("viewBox=\"0 0 30 40\""), "{svg}");
        assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
    }

    #[test]
    fn render_part_writes_an_element_subtree_preview() {
        let dir = tempdir("part-subtree");
        part_project(&dir);
        let out = dir.join("mark.svg");
        let result = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "mark", "format": "svg", "out": out })),
        )
        .expect("renders");
        assert_eq!(result["frame"]["width"], 20.0);
        assert_eq!(result["frame"]["height"], 10.0);

        let svg = fs::read_to_string(result["path"].as_str().expect("a path")).expect("reads");
        assert!(svg.contains("mark-rect"), "{svg}");
        assert!(
            !svg.contains("other"),
            "the rest of the scene is absent: {svg}"
        );
    }

    #[test]
    fn render_part_includes_a_referenced_definition() {
        // A part that places another definition renders with the referenced
        // part resolved, so its geometry reaches the preview (FEAT-031).
        let dir = tempdir("part-nested");
        part_project(&dir);
        write_file(
            &dir,
            "definitions/inner.json",
            r##"{
              "id": "inner", "projectId": "project", "name": "Inner",
              "parameters": [], "origin": { "x": 0, "y": 0 },
              "elements": [
                {
                  "id": "dot", "definitionId": "inner", "order": 0, "kind": "rect",
                  "geometry": { "x": 0, "y": 0, "width": 5, "height": 5 },
                  "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
                  "opacity": 1, "visible": true
                }
              ]
            }"##,
        );
        write_file(
            &dir,
            "definitions/outer.json",
            r##"{
              "id": "outer", "projectId": "project", "name": "Outer",
              "parameters": [], "origin": { "x": 0, "y": 0 },
              "elements": [
                {
                  "id": "place", "definitionId": "outer", "order": 0, "kind": "instance",
                  "geometry": {},
                  "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
                  "definitionRef": "inner", "opacity": 1, "visible": true
                }
              ]
            }"##,
        );

        let out = dir.join("outer.svg");
        let result = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "outer", "format": "svg", "out": out })),
        )
        .expect("renders");
        assert_eq!(
            result["frame"]["width"], 5.0,
            "the referenced definition's geometry reaches the preview: {result}"
        );
        assert_eq!(result["frame"]["height"], 5.0);
        assert_eq!(result["diagnostics"], json!([]));
    }

    #[test]
    fn render_part_reports_the_resolved_output_frame() {
        let dir = tempdir("part-frame");
        part_project(&dir);
        let result = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "mark", "format": "svg", "width": 100 })),
        )
        .expect("renders");
        assert_eq!(result["frame"]["width"], 100.0);
        assert_eq!(
            result["frame"]["height"], 50.0,
            "a single dimension scales the other"
        );
    }

    #[test]
    fn render_part_defaults_to_dist_part() {
        let dir = tempdir("part-default-out");
        part_project(&dir);
        let result = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "badge", "format": "svg" })),
        )
        .expect("renders");
        let path = result["path"].as_str().expect("a path");
        assert!(path.ends_with("dist/badge.svg"), "{path}");
    }

    #[test]
    fn render_part_reports_an_unknown_part_by_name() {
        let dir = tempdir("part-unknown");
        part_project(&dir);
        let out = dir.join("absent.svg");
        let error = assert_exec(call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "absent", "format": "svg", "out": out.clone() })),
        ));
        assert_eq!(error.code, "E_PART");
        assert!(error.message.contains("absent"), "{}", error.message);
        assert!(!out.exists(), "no preview is written for an unknown part");
    }

    #[test]
    fn render_part_reports_an_empty_part_with_the_fallback_frame() {
        let dir = tempdir("part-empty");
        part_project(&dir);
        let out = dir.join("empty.svg");
        let result = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "empty", "format": "svg", "out": out })),
        )
        .expect("renders");
        assert_eq!(result["frame"]["width"], 100.0);
        assert_eq!(result["frame"]["height"], 100.0);
        let diagnostics = result["diagnostics"].as_array().expect("diagnostics");
        assert!(
            diagnostics
                .iter()
                .any(|finding| finding["code"] == "W_EMPTY_PART_FRAME"),
            "{result}"
        );
        assert!(out.exists(), "an empty part still yields a preview");
    }

    #[test]
    fn render_part_resolves_a_definition_from_the_default_palette_not_the_default_scene() {
        // A definition has no placing scene, so it resolves its colours from the
        // project's default palette and default recipe, never from the default
        // scene's own palette (FEAT-031, D-039).
        let dir = tempdir("part-default-palette");
        write_project(
            &dir,
            r#"{"defaultSceneId":"main","defaultPaletteId":"brand"}"#,
            &[("main", BLUE_SCENE)],
        );
        write_file(&dir, "palettes/brand.json", PART_PALETTE);
        write_file(&dir, "palettes/blue.json", BLUE_PALETTE);
        write_file(&dir, "definitions/badge.json", BADGE_DEFINITION);

        let result = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "badge", "format": "svg" })),
        )
        .expect("renders");
        let svg = fs::read_to_string(result["path"].as_str().expect("a path")).expect("reads");
        assert!(
            svg.contains("fill=\"#ff0000\""),
            "the project default palette resolves the definition: {svg}"
        );
        assert!(
            !svg.contains("#0000ff"),
            "the default scene's palette does not leak into the definition: {svg}"
        );
    }

    #[test]
    fn render_part_names_a_missing_default_palette_for_a_definition() {
        // A definition carries no palette of its own and has no placing scene,
        // so a project that names no default palette cannot resolve its colours;
        // the missing palette is named before anything is written (FEAT-031).
        let dir = tempdir("part-no-default-palette");
        write_project(
            &dir,
            r#"{"defaultSceneId":"main"}"#,
            &[("main", SUBTREE_SCENE)],
        );
        write_file(&dir, "palettes/brand.json", PART_PALETTE);
        write_file(&dir, "definitions/badge.json", BADGE_DEFINITION);

        let out = dir.join("badge.svg");
        let error = assert_exec(call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "badge", "format": "svg", "out": out.clone() })),
        ));
        assert_eq!(error.code, "E_PROJECT_ASSET");
        assert!(
            error.message.contains("no default palette"),
            "{}",
            error.message
        );
        assert!(
            !out.exists(),
            "no preview is written when the default palette is missing"
        );
    }

    #[test]
    fn render_part_reports_a_duplicate_part_identifier() {
        // A definition and an element share one identifier, so the project's
        // namespace is not unique; the resolution is refused naming it before
        // any render (FEAT-031, D-039).
        let dir = tempdir("part-duplicate");
        write_project(
            &dir,
            r#"{"defaultSceneId":"main","defaultPaletteId":"brand"}"#,
            &[("main", SUBTREE_SCENE)],
        );
        write_file(&dir, "palettes/brand.json", PART_PALETTE);
        write_file(&dir, "definitions/mark.json", MARK_DEFINITION);

        let out = dir.join("mark.svg");
        let error = assert_exec(call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "mark", "format": "svg", "out": out.clone() })),
        ));
        assert_eq!(error.code, "E_DUPLICATE_ID");
        assert!(error.message.contains("mark"), "{}", error.message);
        assert!(
            !out.exists(),
            "no preview is written for a duplicate part identifier"
        );
    }

    #[test]
    fn render_part_refuses_a_density_on_svg() {
        let dir = tempdir("part-density");
        part_project(&dir);
        let error = assert_exec(call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "badge", "format": "svg", "density": 2 })),
        ));
        assert_eq!(error.code, "E_UNSUPPORTED");
    }

    #[test]
    fn render_part_refuses_an_unsupported_format_without_writing() {
        let dir = tempdir("part-format");
        part_project(&dir);
        let out = dir.join("out.pdf");
        let error = assert_exec(call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "badge", "format": "pdf", "out": out })),
        ));
        assert_eq!(error.code, "E_UNSUPPORTED");
        assert!(!dir.join("out.pdf").exists());
    }

    #[test]
    fn render_part_writes_png() {
        let dir = tempdir("part-png");
        part_project(&dir);
        let out = dir.join("badge.png");
        let result = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "badge", "format": "png", "out": out })),
        )
        .expect("renders");
        let bytes = fs::read(result["path"].as_str().expect("a path")).expect("reads the png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn render_part_requires_a_part_and_a_format() {
        let dir = tempdir("part-required");
        part_project(&dir);
        let missing_part = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "format": "svg" })),
        );
        assert!(matches!(missing_part, Err(CallError::InvalidParams(_))));
        let missing_format = call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "badge" })),
        );
        assert!(matches!(missing_format, Err(CallError::InvalidParams(_))));
    }

    #[test]
    fn render_part_refuses_an_output_outside_the_scope() {
        let dir = tempdir("part-scope");
        let outside = tempdir("part-outside");
        part_project(&dir);
        let error = assert_exec(call(
            &scope(&dir),
            "render-part",
            &args(json!({ "part": "badge", "format": "svg", "out": outside.join("out.svg") })),
        ));
        assert_eq!(error.code, SCOPE);
    }

    #[test]
    fn an_unknown_argument_is_invalid_params() {
        let dir = tempdir("unknown-arg");
        let result = call(
            &scope(&dir),
            "compile",
            &args(json!({ "draft": RECT_SCENE, "nope": 1 })),
        );
        assert!(matches!(result, Err(CallError::InvalidParams(_))));
    }

    #[test]
    fn a_missing_required_argument_is_invalid_params() {
        let dir = tempdir("missing-arg");
        // The project provides a default scene, so the missing `format` is the
        // first failure, not an unresolved scene.
        write_project(
            &dir,
            r#"{"defaultSceneId":"logo"}"#,
            &[("logo", RECT_SCENE)],
        );
        let result = call(&scope(&dir), "render", &args(json!({})));
        assert!(matches!(result, Err(CallError::InvalidParams(_))));
    }

    #[test]
    fn an_unknown_tool_is_invalid_params() {
        let dir = tempdir("unknown-tool");
        let result = call(&scope(&dir), "frobnicate", &Map::new());
        match result {
            Err(CallError::InvalidParams(message)) => assert!(message.contains("frobnicate")),
            other => panic!("expected invalid params, got {other:?}"),
        }
    }

    #[test]
    fn concurrent_calls_are_independent() {
        let dir = tempdir("concurrent");
        let scope = scope(&dir);
        std::thread::scope(|threads| {
            let mut handles = Vec::new();
            for _ in 0..8 {
                handles.push(threads.spawn(|| {
                    let result = call(&scope, "compile", &args(json!({ "draft": RECT_SCENE })))
                        .expect("compiles");
                    result["model"]["nodes"].as_array().map(Vec::len)
                }));
            }
            for handle in handles {
                assert_eq!(handle.join().expect("thread"), Some(1));
            }
        });
    }

    #[test]
    fn schema_returns_the_contract_and_one_type() {
        let dir = tempdir("schema");
        let full = call(&scope(&dir), "schema", &Map::new()).expect("full");
        assert_eq!(full["form"], "full");
        assert!(full["schema"]["$defs"].is_object());

        let compact =
            call(&scope(&dir), "schema", &args(json!({ "form": "compact" }))).expect("compact");
        assert!(!compact["json"].as_str().unwrap().contains('\n'));

        let element =
            call(&scope(&dir), "schema", &args(json!({ "type": "Element" }))).expect("element");
        assert_eq!(element["type"], "Element");
        assert!(element["schema"]["$defs"]["ElementKind"].is_object());
    }

    #[test]
    fn schema_reports_an_unknown_type_with_similar_names() {
        let dir = tempdir("schema-unknown");
        let error = assert_exec(call(
            &scope(&dir),
            "schema",
            &args(json!({ "type": "Palete" })),
        ));
        assert_eq!(error.code, "E_SCHEMA_TYPE");
        assert!(error.message.contains("Palette"), "{}", error.message);
    }
}
