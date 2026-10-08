//! Acceptance tests for the MCP server (FEAT-019, C-005).
//!
//! Drives the built `vectr-mcp` binary as an agent host would. Over stdio: the
//! server publishes its four tools with input and output schemas, compiles and
//! renders a valid scene, returns a structured error for an invalid scene or an
//! unsupported capability, refuses to leave the filesystem scope, and leaves no
//! partial or temporary output behind. Over its opt-in loopback HTTP transport:
//! concurrent calls stay independent.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use common::*;
use serde_json::{json, Value};

/// The response carrying a given JSON-RPC id.
fn response(run: &McpRun, id: i64) -> &Value {
    run.responses
        .iter()
        .find(|response| response["id"] == id)
        .unwrap_or_else(|| panic!("no response with id {id}: {:?}", run.responses))
}

/// A scene with `count` sibling rects, as JSON text.
fn scene_with_rects(count: usize) -> String {
    let elements: Vec<Value> = (0..count)
        .map(|index| rect(&format!("r{index}"), index as i64, index as f64 * 12.0, 0.0, 10.0, 10.0))
        .collect();
    scene(elements).to_string()
}

#[test]
fn an_mcp_client_lists_four_tools_with_their_schemas_and_the_server_version() {
    let dir = TempDir::new("mcp-list");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[
            mcp_request(
                1,
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "acceptance", "version": "1" }
                }),
            ),
            mcp_request(2, "tools/list", json!({})),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);

    let init = response(&run, 1);
    let info = &init["result"]["serverInfo"];
    assert_eq!(info["name"], "vectr-mcp");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    // The advertised version matches the installed CLI, so a caller can compare
    // the two (FEAT-020 version compatibility).
    let cli = run_vectr(dir.path(), &["--version"]);
    let cli_version = stdout(&cli)
        .trim()
        .rsplit(' ')
        .next()
        .unwrap_or_default()
        .to_string();
    assert_eq!(info["version"], cli_version);

    let list = response(&run, 2);
    let tools = list["result"]["tools"].as_array().expect("tools");
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert_eq!(names, vec!["validate", "compile", "render", "schema"]);
    for tool in tools {
        assert!(
            tool["inputSchema"].is_object(),
            "{} publishes an input schema",
            tool["name"]
        );
        assert!(
            tool["outputSchema"].is_object(),
            "{} publishes an output schema",
            tool["name"]
        );
    }
}

#[test]
fn a_valid_scene_compiles_through_mcp_and_returns_the_render_model() {
    let dir = TempDir::new("mcp-compile");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(
            1,
            "compile",
            json!({ "scene": scene_with_rects(1) }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false);
    let model = &result["structuredContent"]["model"];
    assert_eq!(model["nodes"].as_array().map(Vec::len), Some(1));
    assert_eq!(result["structuredContent"]["diagnostics"], json!([]));
}

#[test]
fn a_valid_scene_renders_through_mcp_and_writes_the_file() {
    let dir = TempDir::new("mcp-render");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(
            1,
            "render",
            json!({ "scene": scene_with_rects(1), "format": "svg", "out": "dist/logo.svg" }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    let path = result["structuredContent"]["path"]
        .as_str()
        .expect("the written path is returned");
    let text = std::fs::read_to_string(path).expect("the render wrote a file");
    assert!(text.contains("<svg"), "{text}");
    assert!(text.contains("</svg>"), "{text}");
}

#[test]
fn an_invalid_scene_is_a_structured_tool_error() {
    let dir = TempDir::new("mcp-invalid");
    let mut document: Value = serde_json::from_str(&scene_with_rects(1)).expect("a scene");
    document["elements"][0]["opacity"] = json!(2);

    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(
            1,
            "compile",
            json!({ "scene": document.to_string() }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true);
    let body = &result["structuredContent"];
    assert_eq!(body["code"], "E_SCHEMA");
    assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert!(
        body["diagnostics"].as_array().is_some_and(|f| !f.is_empty()),
        "{body}"
    );
    assert!(body["location"].is_object(), "the error is located: {body}");
}

#[test]
fn an_unsupported_capability_is_a_structured_error_and_the_server_survives() {
    let dir = TempDir::new("mcp-unsupported");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[
            mcp_tool_call(
                1,
                "render",
                json!({ "scene": scene_with_rects(1), "format": "pdf", "out": "dist/out.pdf" }),
            ),
            mcp_request(2, "tools/list", json!({})),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);

    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "E_UNSUPPORTED");
    assert!(!dir.path().join("dist/out.pdf").exists(), "nothing is written");

    // The server answered the next request, so the failure did not crash it.
    let tools = response(&run, 2)["result"]["tools"]
        .as_array()
        .map(Vec::len);
    assert_eq!(tools, Some(4));
}

#[test]
fn an_unknown_tool_and_an_unknown_method_are_protocol_errors() {
    let dir = TempDir::new("mcp-unknown");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[
            mcp_tool_call(1, "frobnicate", json!({})),
            mcp_request(2, "tools/frobnicate", json!({})),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);

    let tool_error = &response(&run, 1)["error"];
    assert_eq!(tool_error["code"], -32602);
    assert!(
        tool_error["message"]
            .as_str()
            .is_some_and(|message| message.contains("frobnicate")),
        "{tool_error}"
    );

    let method_error = &response(&run, 2)["error"];
    assert_eq!(method_error["code"], -32601);
}

#[test]
fn an_output_outside_the_filesystem_scope_is_refused_without_writing() {
    let dir = TempDir::new("mcp-scope");
    let outside = TempDir::new("mcp-scope-outside");
    let target = outside.path().join("escape.svg");

    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(
            1,
            "render",
            json!({ "scene": scene_with_rects(1), "format": "svg", "out": target.to_string_lossy() }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "E_SCOPE");
    assert!(!target.exists(), "a path outside the scope is never written");
}

#[test]
fn an_allow_flag_widens_the_filesystem_scope() {
    let dir = TempDir::new("mcp-allow");
    let outside = TempDir::new("mcp-allow-outside");
    let target = outside.path().join("allowed.svg");

    let run = run_mcp_session(
        dir.path(),
        &["--allow", outside.path().to_str().expect("utf-8")],
        &[mcp_tool_call(
            1,
            "render",
            json!({ "scene": scene_with_rects(1), "format": "svg", "out": target.to_string_lossy() }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    assert!(target.is_file(), "an explicitly allowed root is written to");
}

#[test]
fn no_partial_or_temporary_output_is_left_behind() {
    let dir = TempDir::new("mcp-atomic");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[
            mcp_tool_call(
                1,
                "render",
                json!({ "scene": scene_with_rects(1), "format": "pdf", "out": "dist/out.pdf" }),
            ),
            mcp_tool_call(
                2,
                "render",
                json!({ "scene": scene_with_rects(1), "format": "svg", "out": "dist/ok.svg" }),
            ),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    assert_eq!(response(&run, 1)["result"]["isError"], true);
    assert_eq!(response(&run, 2)["result"]["isError"], false);
    assert!(!dir.path().join("dist/out.pdf").exists());
    assert!(dir.path().join("dist/ok.svg").is_file());

    // The atomic write's temporary file is renamed into place, never left.
    let leftovers: Vec<String> = std::fs::read_dir(dir.path().join("dist"))
        .expect("dist exists")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "no temp files remain: {leftovers:?}");
}

/// An HTTP server child that is killed when the test ends.
struct ServerProcess(Child);

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts `vectr-mcp --bind 127.0.0.1:0` in `cwd` and returns it with its
/// `host:port` authority, parsed from the address the server reports.
fn start_http_server(cwd: &Path) -> (ServerProcess, String) {
    let binary = vectr_mcp_bin();
    assert!(binary.is_file(), "build the workspace first: `{}`", binary.display());
    let mut child = Command::new(&binary)
        .args(["--bind", "127.0.0.1:0"])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawns vectr-mcp");

    let stderr = child.stderr.take().expect("stderr is piped");
    let (sender, receiver) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains("listening on") {
                let _ = sender.send(line);
            }
        }
    });
    let line = receiver
        .recv_timeout(Duration::from_secs(10))
        .expect("the server reports its address");
    let url = line
        .find("http://")
        .map(|start| line[start..].trim().to_string())
        .unwrap_or_else(|| panic!("no address in: {line}"));
    let authority = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .expect("an authority")
        .to_string();
    (ServerProcess(child), authority)
}

/// Posts one JSON-RPC request to the server's `/mcp` endpoint.
fn post(addr: &str, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).expect("connects to the server");
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.as_bytes().len()
    );
    stream.write_all(request.as_bytes()).expect("writes the request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("reads the response");
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let payload = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, payload)
}

#[test]
fn concurrent_calls_are_independent_over_the_http_transport() {
    let dir = TempDir::new("mcp-concurrent");
    let (_server, authority) = start_http_server(dir.path());

    let workers: Vec<_> = (1..=8usize)
        .map(|count| {
            let authority = authority.clone();
            thread::spawn(move || {
                let body = mcp_tool_call(
                    1,
                    "compile",
                    json!({ "scene": scene_with_rects(count) }),
                )
                .to_string();
                let (status, payload) = post(&authority, &body);
                assert_eq!(status, 200, "worker {count}: {payload}");
                let value: Value = serde_json::from_str(&payload).expect("valid JSON-RPC");
                assert_eq!(value["result"]["isError"], false, "worker {count}");
                assert_eq!(
                    value["result"]["structuredContent"]["model"]["nodes"]
                        .as_array()
                        .map(Vec::len),
                    Some(count),
                    "worker {count} got another call's model"
                );
            })
        })
        .collect();

    for worker in workers {
        worker.join().expect("a worker thread panicked");
    }
}
