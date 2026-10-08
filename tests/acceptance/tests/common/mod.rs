//! Shared helpers for the phase-gate acceptance suite.
//!
//! The suite drives the shipped library through its public API (C-002) and the
//! `vectr` binary as a subprocess (C-004). This module holds the scene builders,
//! the render-model helpers, and the process/temp-directory plumbing the
//! per-feature test files share.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};
use vectr_core::render::RenderModel;
use vectr_core::{compile, compile_with_style, parse, Diagnostics, Scene, Shape, StyleContext};

/// The format version the build writes and reads (C-001).
pub const VERSION: &str = "0.2";

/// The scene id every inline test scene uses.
pub const SCENE_ID: &str = "s";

/// A token paint reference as the scene language carries it.
pub fn token_paint(token: &str) -> Value {
    json!({ "kind": "token", "ref": token })
}

/// A gradient paint reference as the scene language carries it.
pub fn gradient_paint(id: &str) -> Value {
    json!({ "kind": "gradient", "ref": id })
}

/// A stroke pairing a profile with a token paint.
pub fn stroke(profile: &str, token: &str) -> Value {
    json!({ "profileId": profile, "paint": token_paint(token) })
}

/// The colour a resolved paint carries, or `None` for a gradient.
pub fn color(paint: &vectr_core::render::Paint) -> Option<&str> {
    match paint {
        vectr_core::render::Paint::Color { value } => Some(value),
        vectr_core::render::Paint::Gradient(_) => None,
    }
}

/// The colour of a node's resolved fill, or `None` for no fill or a gradient.
pub fn fill_color(node: &vectr_core::render::ResolvedNode) -> Option<&str> {
    node.paint.fill.as_ref().and_then(color)
}

/// Builds one element with the required fields and an identity transform.
///
/// Optional fields (`parentId`, `name`, `fill`, `stroke`, `fontId`) are added
/// by the caller mutating the returned JSON object.
pub fn element(id: &str, order: i64, kind: &str, geometry: Value) -> Value {
    json!({
        "id": id,
        "sceneId": SCENE_ID,
        "order": order,
        "kind": kind,
        "geometry": geometry,
        "transform": {
            "translateX": 0.0,
            "translateY": 0.0,
            "rotate": 0.0,
            "scaleX": 1.0,
            "scaleY": 1.0
        },
        "opacity": 1.0,
        "visible": true
    })
}

/// A rect element at `(x, y)` sized `width` by `height`.
pub fn rect(id: &str, order: i64, x: f64, y: f64, width: f64, height: f64) -> Value {
    element(
        id,
        order,
        "rect",
        json!({ "x": x, "y": y, "width": width, "height": height }),
    )
}

/// A line element through the given points.
pub fn line(id: &str, order: i64, points: Value) -> Value {
    element(id, order, "line", json!({ "points": points }))
}

/// A text element at its anchor with the given run fields.
pub fn text(id: &str, order: i64, x: f64, y: f64, value: &str, size: f64) -> Value {
    element(
        id,
        order,
        "text",
        json!({ "x": x, "y": y, "text": value, "fontSize": size }),
    )
}

/// A group element, optionally named.
pub fn group(id: &str, order: i64, name: Option<&str>) -> Value {
    let mut value = element(id, order, "group", json!({}));
    if let Some(name) = name {
        value["name"] = json!(name);
    }
    value
}

/// A complete scene document from its elements.
pub fn scene(elements: Vec<Value>) -> Value {
    scene_with(elements, None, None)
}

/// A scene document with optional constraints and palette reference.
pub fn scene_with(
    elements: Vec<Value>,
    constraints: Option<Value>,
    palette_id: Option<&str>,
) -> Value {
    let mut document = json!({
        "id": SCENE_ID,
        "projectId": "p",
        "name": "S",
        "formatVersion": VERSION,
        "canvas": { "width": 400.0, "height": 400.0, "background": "#ffffff" },
        "elements": elements
    });
    if let Some(constraints) = constraints {
        document["constraints"] = constraints;
    }
    if let Some(palette_id) = palette_id {
        document["paletteId"] = json!(palette_id);
    }
    document
}

/// Parses a scene document, panicking with the diagnostics on failure.
pub fn parse_scene(document: &Value) -> Scene {
    parse(&document.to_string()).unwrap_or_else(|diagnostics| {
        panic!("expected a valid scene, got: {diagnostics}");
    })
}

/// Parses and compiles a scene document, panicking on failure.
pub fn compile_doc(document: &Value) -> RenderModel {
    let scene = parse_scene(document);
    compile(&scene).unwrap_or_else(|diagnostics| {
        panic!("expected the scene to compile, got: {diagnostics}");
    })
}

/// Compiles a scene document against a style context, returning the result.
pub fn compile_with(doc: &Value, style: &StyleContext) -> Result<RenderModel, Diagnostics> {
    compile_with_style(&parse_scene(doc), style)
}

/// Whether two coordinates agree within the acceptance tolerance.
pub fn close(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-6
}

/// The net signed area of a resolved shape, summed over its flattened contours.
///
/// Used to check boolean and offset results by the area they leave visible
/// rather than by their exact vertex list (FEAT-003).
pub fn shape_area(shape: &Shape) -> f64 {
    vectr_core::flatten_shape(shape)
        .iter()
        .map(|contour| polygon_area(contour))
        .sum::<f64>()
        .abs()
}

fn polygon_area(points: &[[f64; 2]]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        sum += current[0] * next[1] - next[0] * current[1];
    }
    sum / 2.0
}

/// The workspace root: the parent of this crate's manifest directory.
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the tests crate lives two levels below the workspace root")
        .to_path_buf()
}

/// The path to the built `vectr` binary.
///
/// The acceptance crate is a separate workspace, so `CARGO_BIN_EXE_vectr` is
/// unavailable; the binary is resolved from the product workspace's target
/// directory, honouring `CARGO_TARGET_DIR`. Build the workspace first.
pub fn vectr_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("VECTR_BIN") {
        return PathBuf::from(path);
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"));
    let name = format!("vectr{}", std::env::consts::EXE_SUFFIX);
    target.join("debug").join(name)
}

/// Runs the `vectr` binary in `cwd` with `args`.
pub fn run_vectr(cwd: &Path, args: &[&str]) -> Output {
    let binary = vectr_bin();
    assert!(
        binary.is_file(),
        "build the workspace before the acceptance suite: `{}` is missing",
        binary.display()
    );
    Command::new(&binary)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("could not run `{}`: {error}", binary.display()))
}

/// The captured standard output as text.
pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The captured standard error as text.
pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The process exit code, or -1 when it was killed by a signal.
pub fn code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

/// A unique temporary directory, removed when it drops.
pub struct TempDir {
    path: PathBuf,
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

impl TempDir {
    /// Creates a fresh temporary directory tagged for the calling test.
    pub fn new(tag: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "vectr-acceptance-{tag}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("creates the temporary directory");
        Self { path }
    }

    /// The directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes a file at `relative`, creating parent directories as needed.
    pub fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("creates the parent directory");
        }
        std::fs::write(&path, text).expect("writes the file");
        path
    }

    /// Writes raw bytes at `relative`, creating parent directories as needed.
    pub fn write_bytes(&self, relative: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("creates the parent directory");
        }
        std::fs::write(&path, bytes).expect("writes the file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A palette document with the given token name/value pairs.
pub fn palette(id: &str, tokens: &[(&str, &str)]) -> String {
    let tokens: Vec<Value> = tokens
        .iter()
        .map(|(name, value)| json!({ "name": name, "value": value }))
        .collect();
    json!({
        "id": id,
        "projectId": "p",
        "name": id,
        "tokens": tokens
    })
    .to_string()
}

/// A stroke profile document.
pub fn stroke_profile(id: &str, width: f64, cap: &str, join: &str) -> String {
    json!({
        "id": id,
        "projectId": "p",
        "name": id,
        "width": width,
        "cap": cap,
        "join": join
    })
    .to_string()
}

/// The bytes of a bundled font asset.
pub fn font_bytes(file: &str) -> Vec<u8> {
    let path = workspace_root().join("assets/fonts").join(file);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
}

/// The path to the built `vectr-mcp` binary.
///
/// Mirrors [`vectr_bin`]: the acceptance crate is a separate workspace, so the
/// binary is resolved from the product workspace's target directory, honouring
/// `CARGO_TARGET_DIR` and the `VECTR_MCP_BIN` override. Build the workspace
/// first.
pub fn vectr_mcp_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("VECTR_MCP_BIN") {
        return PathBuf::from(path);
    }
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"));
    let name = format!("vectr-mcp{}", std::env::consts::EXE_SUFFIX);
    target.join("debug").join(name)
}

/// The captured result of one `vectr-mcp` stdio session.
pub struct McpRun {
    /// Every response line, decoded as JSON, in the order the server emitted them.
    pub responses: Vec<Value>,
    /// The process exit code, or -1 when it was killed by a signal.
    pub code: i32,
    /// The captured standard error.
    pub stderr: String,
}

/// Runs `vectr-mcp` over stdio, sending each request as one NDJSON line.
///
/// The requests are written while a reader thread drains standard output, so a
/// session carrying a large response (the full schema, a render model) cannot
/// deadlock on a full pipe.
pub fn run_mcp_session(cwd: &Path, extra_args: &[&str], requests: &[Value]) -> McpRun {
    let binary = vectr_mcp_bin();
    assert!(
        binary.is_file(),
        "build the workspace before the acceptance suite: `{}` is missing",
        binary.display()
    );
    let mut child = Command::new(&binary)
        .args(extra_args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("could not run `{}`: {error}", binary.display()));

    let mut stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        std::io::BufReader::new(stdout)
            .read_to_string(&mut text)
            .expect("reads the server's stdout");
        text
    });

    for request in requests {
        writeln!(stdin, "{request}").expect("writes the request");
    }
    drop(stdin);

    let text = reader.join().expect("the stdout reader joins");
    let mut stderr = String::new();
    if let Some(mut stream) = child.stderr.take() {
        let _ = stream.read_to_string(&mut stderr);
    }
    let status = child.wait().expect("waits for vectr-mcp");

    let responses = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("a server response was not JSON: {line}: {error}"))
        })
        .collect();

    McpRun {
        responses,
        code: status.code().unwrap_or(-1),
        stderr,
    }
}

/// A JSON-RPC request value for an MCP stdio session.
pub fn mcp_request(id: i64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// A JSON-RPC `tools/call` request for an MCP stdio session.
pub fn mcp_tool_call(id: i64, name: &str, arguments: Value) -> Value {
    mcp_request(id, "tools/call", json!({ "name": name, "arguments": arguments }))
}
