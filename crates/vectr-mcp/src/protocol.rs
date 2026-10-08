//! JSON-RPC 2.0 framing and dispatch for the MCP tools (C-005, FEAT-019).
//!
//! The server speaks the initialization-based MCP revision `2025-06-18` over
//! JSON-RPC 2.0. [`Server::handle`] decodes one message and returns the reply —
//! `None` for a notification, which by JSON-RPC takes no response. The handler
//! holds nothing but the filesystem [`Scope`], so concurrent calls share no
//! mutable state (FEAT-021, FEAT-019).

use serde_json::{json, Value};

use crate::scope::Scope;
use crate::tools::{self, CallError};

/// The MCP revision this server implements.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Revisions accepted during version negotiation; an unrecognized request falls
/// back to [`PROTOCOL_VERSION`].
const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// Whether this build speaks a given MCP revision.
pub fn supports_version(version: &str) -> bool {
    SUPPORTED_VERSIONS.contains(&version)
}

/// The server's advertised name.
pub const SERVER_NAME: &str = "vectr-mcp";

/// The server's advertised version.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// The metadata a client reads after `initialize` to learn to use Vectr.
const INSTRUCTIONS: &str = "Vectr compiles authored scenes into SVG and PNG. Call `schema` to learn the scene language, `validate` to check a scene, `compile` for the render model, and `render` to write SVG or PNG.";

/// The tool server over one filesystem scope.
pub struct Server<'a> {
    scope: &'a Scope,
}

impl<'a> Server<'a> {
    /// Builds a server that resolves every tool call under `scope`.
    pub fn new(scope: &'a Scope) -> Self {
        Self { scope }
    }

    /// Handles one decoded JSON-RPC message.
    ///
    /// Returns `None` when no reply is owed: a notification (no `id`) or a
    /// response to a request the server never sent.
    pub fn handle(&self, message: &Value) -> Option<Value> {
        let object = match message.as_object() {
            Some(object) => object,
            None => {
                return Some(error_response(
                    Value::Null,
                    INVALID_REQUEST,
                    "invalid request",
                    None,
                ))
            }
        };

        // A message without a method is a response; the server sends no
        // requests, so there is nothing to correlate and nothing to reply.
        let method = object.get("method").and_then(Value::as_str)?;
        // A notification carries no `id` and takes no response.
        if !object.contains_key("id") {
            return None;
        }
        let id = object.get("id").cloned().unwrap_or(Value::Null);
        let params = object.get("params").cloned().unwrap_or(Value::Null);

        match self.dispatch(method, &params) {
            Ok(result) => Some(success(id, result)),
            Err(failure) => Some(error_response(
                id,
                failure.code,
                &failure.message,
                failure.data,
            )),
        }
    }

    /// Handles one line of the stdio transport, mapping a parse failure to a
    /// `-32700` response (the transport is newline-delimited JSON).
    pub fn handle_line(&self, line: &str) -> Option<Value> {
        match serde_json::from_str::<Value>(line) {
            Ok(message) => self.handle(&message),
            Err(failure) => Some(error_response(
                Value::Null,
                PARSE_ERROR,
                &format!("parse error: {failure}"),
                None,
            )),
        }
    }

    fn dispatch(&self, method: &str, params: &Value) -> Result<Value, RpcError> {
        match method {
            "initialize" => Ok(self.initialize(params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools::definitions() })),
            "tools/call" => self.call_tool(params),
            "server/discover" => Err(RpcError::method_not_found(method)),
            _ => Err(RpcError::method_not_found(method)),
        }
    }

    /// Negotiates the protocol version and advertises the tools capability.
    fn initialize(&self, params: &Value) -> Value {
        let requested = params.get("protocolVersion").and_then(Value::as_str);
        let negotiated = match requested {
            Some(version) if SUPPORTED_VERSIONS.contains(&version) => version,
            _ => PROTOCOL_VERSION,
        };
        json!({
            "protocolVersion": negotiated,
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": SERVER_NAME,
                "title": "Vectr",
                "version": SERVER_VERSION
            },
            "instructions": INSTRUCTIONS
        })
    }

    /// Invokes one tool. A malformed request is a protocol error; a tool that
    /// ran but failed is a result with `isError: true` (C-005).
    fn call_tool(&self, params: &Value) -> Result<Value, RpcError> {
        let object = params.as_object().ok_or_else(|| {
            RpcError::invalid_params("`tools/call` requires an object of parameters")
        })?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid_params("`tools/call` requires a string `name`"))?;
        let arguments = object
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let arguments = arguments.as_object().ok_or_else(|| {
            RpcError::invalid_params("`tools/call` `arguments` must be an object")
        })?;

        match tools::call(self.scope, name, arguments) {
            Ok(structured) => Ok(tool_result(structured, false)),
            Err(CallError::InvalidParams(message)) => Err(RpcError::invalid_params(message)),
            Err(CallError::Exec(error)) => Ok(tool_result(error.to_value(), true)),
        }
    }
}

/// A protocol-level error reply.
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl RpcError {
    fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
            data: None,
        }
    }

    fn method_not_found(method: &str) -> Self {
        Self {
            code: METHOD_NOT_FOUND,
            message: format!("method not found: {method}"),
            data: None,
        }
    }
}

/// A tool result: the structured body, its serialized text and the error flag.
fn tool_result(structured: Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(&structured).unwrap_or_default();
    json!({
        "content": [ { "type": "text", "text": text } ],
        "structuredContent": structured,
        "isError": is_error
    })
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut body = json!({ "code": code, "message": message });
    if let (Some(data), Some(object)) = (data, body.as_object_mut()) {
        object.insert("data".to_string(), data);
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::Scope;

    const RECT_SCENE: &str = r##"{
      "id": "s", "projectId": "p", "name": "Rect", "formatVersion": "0.2",
      "canvas": { "width": 10, "height": 10, "background": "#ffffff" },
      "elements": [{
        "id": "r1", "sceneId": "s", "order": 0, "kind": "rect",
        "geometry": { "x": 0, "y": 0, "width": 5, "height": 5 },
        "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
        "opacity": 1, "visible": true
      }]
    }"##;

    fn tempdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vectr-mcp-rpc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creates the temp dir");
        dir
    }

    #[test]
    fn initialize_advertises_the_tools_capability() {
        let dir = tempdir("initialize");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        let response = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": PROTOCOL_VERSION } }))
            .expect("a response");
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert_eq!(response["result"]["serverInfo"]["name"], SERVER_NAME);
    }

    #[test]
    fn supported_versions_are_recognised() {
        assert!(supports_version(PROTOCOL_VERSION));
        assert!(supports_version("2024-11-05"));
        assert!(!supports_version("1999-01-01"));
    }

    #[test]
    fn initialize_falls_back_for_an_unknown_protocol_version() {
        let dir = tempdir("initialize-fallback");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        let response = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } }))
            .expect("a response");
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn a_notification_receives_no_response() {
        let dir = tempdir("notification");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        assert!(server
            .handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .is_none());
    }

    #[test]
    fn tools_list_returns_the_four_tools() {
        let dir = tempdir("list");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        let response = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .expect("a response");
        let tools = response["result"]["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), 4);
        assert!(tools.iter().all(|tool| tool["inputSchema"].is_object()));
    }

    #[test]
    fn an_unknown_method_is_method_not_found() {
        let dir = tempdir("unknown-method");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        let response = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/frobnicate" }))
            .expect("a response");
        assert_eq!(response["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn an_unknown_tool_is_invalid_params() {
        let dir = tempdir("unknown-tool");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        let response = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "nope", "arguments": {} } }))
            .expect("a response");
        assert_eq!(response["error"]["code"], INVALID_PARAMS);
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Unknown tool"));
    }

    #[test]
    fn a_valid_tool_call_returns_a_result() {
        let dir = tempdir("call-valid");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        let response = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "compile", "arguments": { "scene": RECT_SCENE } } }))
            .expect("a response");
        assert_eq!(response["result"]["isError"], false);
        assert!(response["result"]["structuredContent"]["model"]["nodes"]
            .as_array()
            .is_some_and(|nodes| nodes.len() == 1));
    }

    #[test]
    fn an_invalid_scene_is_a_structured_tool_error() {
        let dir = tempdir("call-invalid");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        let invalid = RECT_SCENE.replace(r#""opacity": 1"#, r#""opacity": 2"#);
        let response = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": { "name": "compile", "arguments": { "scene": invalid } } }))
            .expect("a response");
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(response["result"]["structuredContent"]["code"], "E_SCHEMA");
        assert!(response["result"]["structuredContent"]["diagnostics"]
            .as_array()
            .is_some_and(|findings| !findings.is_empty()));
    }

    #[test]
    fn a_malformed_line_is_a_parse_error() {
        let dir = tempdir("parse-error");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        let response = server.handle_line("this is not json").expect("a response");
        assert_eq!(response["error"]["code"], PARSE_ERROR);
        assert_eq!(response["id"], Value::Null);
    }

    #[test]
    fn a_response_to_a_request_we_never_sent_is_ignored() {
        let dir = tempdir("late-response");
        let scope = Scope::new(vec![dir]);
        let server = Server::new(&scope);
        assert!(server
            .handle(&json!({ "jsonrpc": "2.0", "id": 9, "result": {} }))
            .is_none());
    }
}
