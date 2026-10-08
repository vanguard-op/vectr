//! The four MCP tools: validate, compile, render and schema (C-005, FEAT-019).
//!
//! Each tool is a thin, stateless wrapper over the same engine calls the command
//! line makes, so an agent gets identical results to `vectr validate`, `vectr
//! compile`, `vectr export` and `vectr schema` (C-005: the MCP tools consume the
//! CLI's capabilities). A tool call carries no server state: it resolves the
//! input under the server's [`Scope`], runs the engine, and returns a JSON
//! value. That keeps concurrent calls independent (FEAT-021, FEAT-019).
//!
//! A tool addresses a scene exactly as the command line does: by the scene's
//! identifier resolved among a project's scenes, or, when none is named, the
//! project's default. The project is the one the caller names, or the server's
//! project context. A tool also accepts a scene document sent inline as a
//! draft, which is not a project scene: it never becomes or reads the default,
//! and its assets resolve against the same project a project scene uses. A call
//! naming both a scene identifier and a draft is malformed (C-005, FEAT-019).

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use vectr_core::scene::{is_color, INVALID_COLOR};
use vectr_core::{
    compile_with_style, export_png_reporting, export_svg_reporting, parse as parse_scene, schema,
    schema_for, validate as validate_scene, validate_gradient_usage, validate_palette_usage,
    Diagnostics, Location, RasterOptions, RenderModel, Scene, SchemaForm, SvgOptions,
};

use vectr_project::{resolve_scene, ProjectAssets};

use crate::output::write_atomic;
use crate::scope::{Scope, ScopeError, SCOPE};

/// The stable code a call that names both a scene identifier and an inline
/// draft is reported under (C-005): the two are mutually exclusive.
pub const MALFORMED: &str = "E_MALFORMED";

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
        "schema" => schema_tool(arguments),
        other => Err(CallError::InvalidParams(format!("Unknown tool: {other}"))),
    }
}

/// The four tool definitions, in the order `tools/list` publishes them.
pub fn definitions() -> Value {
    json!([
        validate_definition(),
        compile_definition(),
        render_definition(),
        schema_definition(),
    ])
}

fn validate_tool(scope: &Scope, arguments: &Map<String, Value>) -> Result<Value, CallError> {
    let args = Args::new(arguments);
    args.allowed(&["scene", "draft", "project"])?;
    let input = load_scene(scope, arguments)?;

    let assets = ProjectAssets::load(&input.root, &input.scene).map_err(exec_diagnostics)?;
    let mut findings = validate_scene(&input.scene);
    if let Some(palette) = assets.palette() {
        findings.extend(validate_palette_usage(
            &input.scene,
            palette,
            assets.gradients(),
        ));
    }
    findings.extend(validate_gradient_usage(&input.scene, assets.gradients()));
    findings.extend(assets.check_references(&input.scene));

    if findings.has_errors() {
        return Err(exec(ToolError::from_diagnostics(findings)));
    }
    Ok(json!({
        "valid": true,
        "diagnostics": serde_json::to_value(&findings).unwrap_or(Value::Null),
    }))
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
        None => default_output(&input, &format),
    };
    let target = scope.write_path(&target).map_err(scope_error)?;
    write_atomic(&target, &bytes).map_err(|error| {
        exec(ToolError::new(
            "E_OUTPUT",
            format!("cannot write output `{}`: {error}", target.display()),
        ))
    })?;

    Ok(json!({
        "path": target.display().to_string(),
        "format": format,
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
        "project": {
            "type": "string",
            "description": "Project root holding scenes/, palettes/, strokes/, gradients/, recipes/ and assets/. Defaults to the server's project context."
        }
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
        json!({ "type": "string", "enum": ["svg", "png"], "description": "The output format." }),
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
        "description": "Compile a scene and write it as SVG or PNG. Address a project scene by its identifier or send an inline draft; with neither, the project's default scene is used. Returns the path it wrote; a failure writes nothing.",
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
                "format": { "type": "string", "enum": ["svg", "png"] },
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
    fn definitions_name_four_tools_with_input_and_output_schemas() {
        let tools = definitions();
        let list = tools.as_array().expect("an array");
        let names: Vec<&str> = list
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();
        assert_eq!(names, vec!["validate", "compile", "render", "schema"]);
        for tool in list {
            assert!(tool.get("inputSchema").is_some(), "input schema: {tool}");
            assert!(tool.get("outputSchema").is_some(), "output schema: {tool}");
        }

        // The scene-addressing arguments are discoverable: every scene tool
        // publishes `scene`, `draft` and `project`, and neither `scene` nor
        // `draft` is required, so omitting both applies the default rule.
        for tool in list.iter().filter(|tool| tool["name"] != "schema") {
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
    fn render_refuses_an_unsupported_format_without_writing() {
        let dir = tempdir("render-unsupported");
        let out = dir.join("out.pdf");
        let error = assert_exec(call(
            &scope(&dir),
            "render",
            &args(json!({ "draft": RECT_SCENE, "format": "pdf", "out": out })),
        ));
        assert_eq!(error.code, "E_UNSUPPORTED");
        assert!(!dir.join("out.pdf").exists());
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
