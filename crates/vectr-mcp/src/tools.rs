//! The four MCP tools: validate, compile, render and schema (C-005, FEAT-019).
//!
//! Each tool is a thin, stateless wrapper over the same engine calls the command
//! line makes, so an agent gets identical results to `vectr validate`, `vectr
//! compile`, `vectr export` and `vectr schema` (C-005: the MCP tools consume the
//! CLI's capabilities). A tool call carries no server state: it resolves the
//! input under the server's [`Scope`], runs the engine, and returns a JSON
//! value. That keeps concurrent calls independent (FEAT-021, FEAT-019).

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use vectr_core::scene::{is_color, INVALID_COLOR};
use vectr_core::{
    compile_with_style, export_png_reporting, export_svg_reporting, parse as parse_scene, schema,
    schema_for, validate as validate_scene, validate_gradient_usage, validate_palette_usage,
    Diagnostics, Location, RasterOptions, RenderModel, Scene, SchemaForm, SvgOptions,
};

use crate::assets::{self, ProjectAssets};
use crate::output::write_atomic;
use crate::scope::{Scope, ScopeError, SCOPE};

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
    args.allowed(&["scene", "project"])?;
    let input = load_scene(
        scope,
        &args.required_string("scene")?,
        args.string("project")?.as_deref(),
    )?;

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
    args.allowed(&["scene", "project"])?;
    let input = load_scene(
        scope,
        &args.required_string("scene")?,
        args.string("project")?.as_deref(),
    )?;

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
        "project",
        "format",
        "out",
        "width",
        "height",
        "density",
        "background",
    ])?;
    let input = load_scene(
        scope,
        &args.required_string("scene")?,
        args.string("project")?.as_deref(),
    )?;

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
    /// The scene file's stem, for a default output name; absent for inline text.
    stem: Option<String>,
}

/// Reads the `scene` argument as inline JSON text or as a path, then finds the
/// project root its assets live under.
fn load_scene(scope: &Scope, scene: &str, project: Option<&str>) -> Result<SceneInput, CallError> {
    if scene.trim_start().starts_with('{') {
        let parsed = parse_scene(scene).map_err(exec_diagnostics)?;
        let root = match project {
            Some(path) => scope.read_dir(Path::new(path)).map_err(scope_error)?,
            None => scope.base().to_path_buf(),
        };
        return Ok(SceneInput {
            scene: parsed,
            root,
            stem: None,
        });
    }

    let path = scope.read_path(Path::new(scene)).map_err(scope_error)?;
    let source = fs::read_to_string(&path).map_err(|error| {
        exec(ToolError::new(
            "E_INPUT",
            format!("cannot read scene `{}`: {error}", path.display()),
        ))
    })?;
    let parsed = parse_scene(&source).map_err(exec_diagnostics)?;
    let root = match project {
        Some(root) => scope.read_dir(Path::new(root)).map_err(scope_error)?,
        None => assets::project_root(&path),
    };
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_string);
    Ok(SceneInput {
        scene: parsed,
        root,
        stem,
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

/// The shared `scene` / `project` input properties.
fn scene_properties() -> Value {
    json!({
        "scene": {
            "type": "string",
            "description": "The scene document as JSON text, or the path to a scene document."
        },
        "project": {
            "type": "string",
            "description": "Project root holding palettes/, strokes/, gradients/, recipes/ and assets/. Defaults to the scene file's project, or the server's working directory for an inline scene."
        }
    })
}

fn validate_definition() -> Value {
    json!({
        "name": "validate",
        "title": "Validate a scene",
        "description": "Check a scene against the language contract and its project's style assets. Returns diagnostics with a location for each finding; nothing is written.",
        "inputSchema": {
            "type": "object",
            "properties": scene_properties(),
            "required": ["scene"],
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
        "description": "Resolve a scene and its project's palette, strokes, gradients and fonts into the render model every exporter reads (C-003). Returns the model and any warnings.",
        "inputSchema": {
            "type": "object",
            "properties": scene_properties(),
            "required": ["scene"],
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
        "description": "Compile a scene and write it as SVG or PNG. Returns the path it wrote; a failure writes nothing.",
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": ["scene", "format"],
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

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().expect("an object").clone()
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
    }

    #[test]
    fn validate_accepts_a_valid_scene() {
        let dir = tempdir("validate-valid");
        let result = call(
            &scope(&dir),
            "validate",
            &args(json!({ "scene": RECT_SCENE })),
        )
        .expect("valid");
        assert_eq!(result["valid"], Value::Bool(true));
        assert_eq!(result["diagnostics"], json!([]));
    }

    #[test]
    fn validate_reports_an_invalid_scene_as_a_structured_error() {
        let dir = tempdir("validate-invalid");
        let error = assert_exec(call(
            &scope(&dir),
            "validate",
            &args(json!({ "scene": INVALID_SCENE })),
        ));
        assert_eq!(error.code, "E_SCHEMA");
        assert!(!error.diagnostics.is_empty());
    }

    #[test]
    fn compile_returns_the_render_model() {
        let dir = tempdir("compile");
        let result = call(
            &scope(&dir),
            "compile",
            &args(json!({ "scene": RECT_SCENE })),
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
            &args(json!({ "scene": cyclic })),
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
            &args(json!({ "scene": RECT_SCENE, "format": "svg", "out": out })),
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
            &args(json!({ "scene": RECT_SCENE, "format": "png", "out": out })),
        )
        .expect("renders");
        let bytes = fs::read(result["path"].as_str().expect("a path")).expect("reads the png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn render_defaults_to_dist_under_the_project() {
        let dir = tempdir("render-default");
        let result = call(
            &scope(&dir),
            "render",
            &args(json!({ "scene": RECT_SCENE, "format": "svg" })),
        )
        .expect("renders");
        let path = result["path"].as_str().expect("a path");
        assert!(path.ends_with("dist/scene.svg"), "{path}");
    }

    #[test]
    fn render_refuses_an_unsupported_format_without_writing() {
        let dir = tempdir("render-unsupported");
        let out = dir.join("out.pdf");
        let error = assert_exec(call(
            &scope(&dir),
            "render",
            &args(json!({ "scene": RECT_SCENE, "format": "pdf", "out": out })),
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
            &args(json!({ "scene": RECT_SCENE, "format": "svg", "out": outside.join("out.svg") })),
        ));
        assert_eq!(error.code, SCOPE);
    }

    #[test]
    fn an_unknown_argument_is_invalid_params() {
        let dir = tempdir("unknown-arg");
        let result = call(
            &scope(&dir),
            "compile",
            &args(json!({ "scene": RECT_SCENE, "nope": 1 })),
        );
        assert!(matches!(result, Err(CallError::InvalidParams(_))));
    }

    #[test]
    fn a_missing_required_argument_is_invalid_params() {
        let dir = tempdir("missing-arg");
        let result = call(&scope(&dir), "compile", &args(json!({})));
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
                    let result = call(&scope, "compile", &args(json!({ "scene": RECT_SCENE })))
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
