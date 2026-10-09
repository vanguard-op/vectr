//! Acceptance tests for the MCP server (FEAT-019, C-005).
//!
//! Drives the built `vectr-mcp` binary as an agent host would. Over stdio: the
//! server publishes its four tools with input and output schemas; a scene is
//! addressed exactly as the command line addresses it — by its identifier among
//! a project's scenes, or the project's default when none is named — and an
//! inline draft is accepted instead of a project scene without ever becoming or
//! reading the default; a valid scene compiles and renders, an invalid scene or
//! an unsupported capability is a structured error, a call naming both a scene
//! and a draft is malformed, the server refuses to leave the filesystem scope,
//! and it leaves no partial or temporary output behind. Over its opt-in loopback
//! HTTP transport: concurrent calls stay independent.

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

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
        .map(|index| {
            rect(
                &format!("r{index}"),
                index as i64,
                index as f64 * 12.0,
                0.0,
                10.0,
                10.0,
            )
        })
        .collect();
    scene(elements).to_string()
}

/// A scene whose one filled rect resolves the named palette's `accent` token, so
/// a test can tell which scene's assets a tool resolved (FEAT-016, FEAT-019).
fn colored_scene(id: &str, palette_id: &str) -> Value {
    let mut card = rect("r1", 0, 0.0, 0.0, 10.0, 10.0);
    card["fill"] = token_paint("accent");
    let mut document = scene(vec![card]);
    document["id"] = json!(id);
    document["paletteId"] = json!(palette_id);
    document["elements"][0]["sceneId"] = json!(id);
    document
}

/// A project holding two scenes, `red` and `blue`, each resolving its own
/// palette, and optionally naming a default scene (FEAT-016, FEAT-019).
///
/// The two scenes are laid out per D-032: one document per scene under
/// `scenes/`, named for the scene's identifier.
fn two_scene_project(tag: &str, default_scene: Option<&str>) -> TempDir {
    let dir = TempDir::new(tag);
    let mut config = json!({});
    if let Some(id) = default_scene {
        config["defaultSceneId"] = json!(id);
    }
    dir.write("vectr.project.json", &config.to_string());
    dir.write("palettes/red.json", &palette("red", &[("accent", "#ff0000")]));
    dir.write(
        "palettes/blue.json",
        &palette("blue", &[("accent", "#0000ff")]),
    );
    dir.write("scenes/red.json", &colored_scene("red", "red").to_string());
    dir.write(
        "scenes/blue.json",
        &colored_scene("blue", "blue").to_string(),
    );
    dir
}

/// The resolved fill colour of the first node in a compile result.
fn first_fill(result: &Value) -> &str {
    result["structuredContent"]["model"]["nodes"][0]["paint"]["fill"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("the first node carries a resolved fill: {result}"))
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

    // The scene-addressing arguments are discoverable: every scene tool
    // publishes `scene`, `draft` and `project`, and neither `scene` nor `draft`
    // is required, so omitting both applies the default-scene rule (FEAT-019).
    for tool in tools.iter().filter(|tool| tool["name"] != "schema") {
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
        assert!(
            !required
                .iter()
                .any(|name| name == "scene" || name == "draft"),
            "{} does not require a scene or draft, so the default applies: {tool}",
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
            json!({ "draft": scene_with_rects(1) }),
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
fn a_valid_draft_sent_as_a_json_object_compiles_through_mcp() {
    // A draft may be a scene document as a JSON object or as JSON text; both go
    // through the same strict parser (C-005).
    let dir = TempDir::new("mcp-draft-object");
    let document: Value = serde_json::from_str(&scene_with_rects(2)).expect("a scene object");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({ "draft": document }))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    assert_eq!(
        result["structuredContent"]["model"]["nodes"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn a_project_scene_resolves_its_style_assets_through_mcp() {
    // The MCP server loads a scene's project assets through the shared project
    // loader, so a fill token and a stroke profile resolve exactly as they do
    // through the CLI (FEAT-016, FEAT-019). The inline-draft path is covered
    // above; this is the project path, where the palette and stroke documents
    // live beside the scene and the scene is addressed by its identifier.
    let dir = TempDir::new("mcp-project");
    dir.write("vectr.project.json", "{}");
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "#4f46e5")]),
    );
    dir.write(
        "strokes/hairline.json",
        &stroke_profile("hairline", 2.0, "round", "round"),
    );
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["fill"] = token_paint("accent");
    card["stroke"] = stroke("hairline", "accent");
    let document = scene_with(vec![card], None, Some("brand"));
    write_scene_as(&dir, "logo", document);

    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({ "scene": "logo" }))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    let model = &result["structuredContent"]["model"];
    assert_eq!(
        model["nodes"][0]["paint"]["fill"]["value"], "#4f46e5",
        "the project's palette resolves over MCP: {model}"
    );
    assert_eq!(
        model["nodes"][0]["paint"]["stroke"]["width"], 2.0,
        "the project's stroke profile resolves over MCP: {model}"
    );
}

#[test]
fn the_cli_and_the_mcp_server_resolve_a_project_identically() {
    // The project loader is shared between the two front ends, so the same
    // scene against the same project yields the same render model through both
    // (FEAT-016, FEAT-019, NFR-010).
    let dir = TempDir::new("mcp-consistency");
    dir.write("vectr.project.json", "{}");
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "#4f46e5")]),
    );
    dir.write(
        "strokes/hairline.json",
        &stroke_profile("hairline", 2.0, "round", "round"),
    );
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["fill"] = token_paint("accent");
    card["stroke"] = stroke("hairline", "accent");
    let document = scene_with(vec![card], None, Some("brand"));
    write_scene_as(&dir, "logo", document);

    let cli = run_vectr(dir.path(), &["compile", "logo", "--out", "dist/model.json"]);
    assert_eq!(code(&cli), 0, "{}", stderr(&cli));
    let cli_model: Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("dist/model.json"))
            .expect("the CLI wrote a render model"),
    )
    .expect("the CLI render model is JSON");

    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({ "scene": "logo" }))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    let mcp_model = &result["structuredContent"]["model"];

    assert_eq!(
        &cli_model, mcp_model,
        "both front ends resolve the project identically"
    );
}

#[test]
fn a_named_scene_addresses_one_scene_and_resolves_only_its_assets() {
    // A project holding more than one scene: naming one scene by its identifier
    // operates on that scene only, resolving that scene's project assets
    // (FEAT-019).
    let dir = two_scene_project("mcp-named-scene", Some("red"));

    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({ "scene": "blue" }))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    assert_eq!(
        first_fill(result),
        "#0000ff",
        "the named scene's own palette resolved"
    );
}

#[test]
fn an_omitted_scene_uses_the_project_default() {
    let dir = two_scene_project("mcp-default", Some("blue"));
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({}))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    assert_eq!(
        first_fill(result),
        "#0000ff",
        "the default scene's palette resolved"
    );
}

#[test]
fn a_named_scene_overrides_the_project_default() {
    let dir = two_scene_project("mcp-override", Some("red"));
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({ "scene": "blue" }))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    assert_eq!(
        first_fill(result),
        "#0000ff",
        "the named scene overrides the default"
    );
}

#[test]
fn a_project_that_names_no_default_reports_no_scene_selected() {
    // A project that names no default scene must not choose among its scenes;
    // it returns a structured error that no scene was selected (FEAT-019).
    let dir = two_scene_project("mcp-no-default", None);
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "validate", json!({}))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true, "{:?}", run.responses);
    let body = &result["structuredContent"];
    assert_eq!(body["code"], "E_SCENE");
    let message = body["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("no scene") && message.contains("default"),
        "the error reports that no scene was selected: {message}"
    );
}

#[test]
fn a_scene_identifier_no_document_provides_is_a_structured_error_naming_the_scene() {
    let dir = two_scene_project("mcp-missing-scene", Some("red"));
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({ "scene": "absent" }))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true, "{:?}", run.responses);
    let body = &result["structuredContent"];
    assert_eq!(body["code"], "E_SCENE");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("absent")),
        "the missing scene is named: {body}"
    );
}

#[test]
fn a_default_that_resolves_to_no_document_is_a_structured_error_naming_the_scene() {
    let dir = two_scene_project("mcp-missing-default", Some("ghost"));
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({}))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true, "{:?}", run.responses);
    let body = &result["structuredContent"];
    assert_eq!(body["code"], "E_SCENE");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("ghost")),
        "the missing default scene is named: {body}"
    );
}

#[test]
fn a_call_naming_both_a_scene_and_a_draft_is_malformed() {
    // A scene identifier and an inline document are mutually exclusive; naming
    // both is a malformed call, not a choice of one (FEAT-019).
    let dir = two_scene_project("mcp-both", Some("red"));
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(
            1,
            "compile",
            json!({ "scene": "red", "draft": scene_with_rects(1) }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true, "{:?}", run.responses);
    let body = &result["structuredContent"];
    assert_eq!(body["code"], "E_MALFORMED");
    assert!(
        body["message"].as_str().is_some_and(|m| !m.is_empty()),
        "the malformed call is explained: {body}"
    );
}

#[test]
fn an_inline_draft_is_used_as_a_draft_and_leaves_the_default_unchanged() {
    // A draft is not a scene of the project: it never becomes the default and
    // never reads or writes it (FEAT-019).
    let dir = two_scene_project("mcp-draft-default", Some("red"));

    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(
            1,
            "compile",
            json!({ "draft": scene_with_rects(3) }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    assert_eq!(
        result["structuredContent"]["model"]["nodes"]
            .as_array()
            .map(Vec::len),
        Some(3),
        "the inline document was compiled"
    );

    // The draft's own document is never written into the project.
    assert!(
        !dir.path().join("scenes/s.json").exists(),
        "a draft never becomes a project scene"
    );
    // The project configuration is untouched, and the default still resolves.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("vectr.project.json")).expect("reads the config"),
        r#"{"defaultSceneId":"red"}"#
    );
    let default = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({}))],
    );
    assert_eq!(default.code, 0, "{}", default.stderr);
    assert_eq!(
        first_fill(&response(&default, 1)["result"]),
        "#ff0000",
        "the default scene is unchanged by the draft"
    );
}

#[test]
fn an_inline_draft_resolves_its_assets_against_the_project() {
    let dir = two_scene_project("mcp-draft-assets", Some("red"));
    // The draft names the blue palette the project provides, though the project
    // default is red.
    let draft = colored_scene("d", "blue");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({ "draft": draft }))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    assert_eq!(
        first_fill(result),
        "#0000ff",
        "the draft resolves the project's palette: {result}"
    );
}

#[test]
fn an_inline_draft_whose_asset_reference_resolves_nowhere_names_the_reference() {
    // A draft naming a palette the project does not provide is a structured
    // error naming the reference (FEAT-019).
    let dir = two_scene_project("mcp-draft-missing-asset", Some("red"));
    let draft = colored_scene("d", "absent");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "validate", json!({ "draft": draft }))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true, "{:?}", run.responses);
    let body = &result["structuredContent"];
    assert_eq!(body["code"], "E_PROJECT_ASSET");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("absent")),
        "the unresolved reference is named: {body}"
    );
    assert!(
        body["diagnostics"]
            .as_array()
            .is_some_and(|findings| !findings.is_empty()),
        "the finding is reported: {body}"
    );
}

#[test]
fn an_inline_draft_whose_element_reference_resolves_nowhere_names_its_location() {
    // An element-level reference that resolves nowhere names both the reference
    // and the element that carries it (FEAT-019).
    let dir = two_scene_project("mcp-draft-undefined-ref", Some("red"));
    let mut draft = colored_scene("d", "blue");
    draft["elements"][0]["stroke"] = stroke("outline", "accent");
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(1, "compile", json!({ "draft": draft }))],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true, "{:?}", run.responses);
    let body = &result["structuredContent"];
    assert_eq!(body["code"], "E_UNDEFINED_STROKE");
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|message| message.contains("outline")),
        "the unresolved reference is named: {body}"
    );
    assert_eq!(
        body["location"]["elementId"], "r1",
        "the reference is located: {body}"
    );
    assert_eq!(body["location"]["jsonPath"], "/stroke/profileId");
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
            json!({ "draft": scene_with_rects(1), "format": "svg", "out": "dist/logo.svg" }),
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
fn a_project_scene_render_defaults_its_output_to_the_scene_identifier() {
    // A rendered project scene with no `out` writes `<project>/dist/<id>.<ext>`,
    // so the output is named for the addressed scene (FEAT-019).
    let dir = two_scene_project("mcp-render-default", Some("red"));
    let run = run_mcp_session(
        dir.path(),
        &[],
        &[mcp_tool_call(
            1,
            "render",
            json!({ "scene": "blue", "format": "svg" }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], false, "{:?}", run.responses);
    let path = result["structuredContent"]["path"]
        .as_str()
        .expect("the written path is returned");
    assert!(path.ends_with("dist/blue.svg"), "{path}");
    assert!(dir.path().join("dist/blue.svg").is_file());
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
            json!({ "draft": document.to_string() }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true);
    let body = &result["structuredContent"];
    assert_eq!(body["code"], "E_SCHEMA");
    assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
    assert!(
        body["diagnostics"]
            .as_array()
            .is_some_and(|f| !f.is_empty()),
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
                json!({ "draft": scene_with_rects(1), "format": "pdf", "out": "dist/out.pdf" }),
            ),
            mcp_request(2, "tools/list", json!({})),
        ],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);

    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "E_UNSUPPORTED");
    assert!(
        !dir.path().join("dist/out.pdf").exists(),
        "nothing is written"
    );

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
            json!({ "draft": scene_with_rects(1), "format": "svg", "out": target.to_string_lossy() }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);
    let result = &response(&run, 1)["result"];
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["code"], "E_SCOPE");
    assert!(
        !target.exists(),
        "a path outside the scope is never written"
    );
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
            json!({ "draft": scene_with_rects(1), "format": "svg", "out": target.to_string_lossy() }),
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
                json!({ "draft": scene_with_rects(1), "format": "pdf", "out": "dist/out.pdf" }),
            ),
            mcp_tool_call(
                2,
                "render",
                json!({ "draft": scene_with_rects(1), "format": "svg", "out": "dist/ok.svg" }),
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
    assert!(
        binary.is_file(),
        "build the workspace first: `{}`",
        binary.display()
    );
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
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .expect("writes the request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("reads the response");
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

/// Sends one JSON-RPC request and closes the connection without reading the
/// reply, as a client that disconnects mid-call does.
fn post_and_disconnect(addr: &str, body: &str) {
    let mut stream = TcpStream::connect(addr).expect("connects to the server");
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .expect("writes the request");
    // Dropping the stream closes it before the reply is read.
}

/// The `dist/` entries whose name contains `.tmp-`, the atomic write's
/// temporary file prefix.
fn temporary_leftovers(dist: &Path) -> Vec<String> {
    std::fs::read_dir(dist)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.contains(".tmp-"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_client_that_disconnects_mid_render_leaves_no_partial_output() {
    // A disconnect while a render is in flight must not leave a truncated or
    // partial file behind: the output is written atomically, so it is either
    // absent or a complete document (FEAT-019, NFR-011).
    let dir = TempDir::new("mcp-disconnect");
    let (_server, authority) = start_http_server(dir.path());
    let body = mcp_tool_call(
        1,
        "render",
        json!({ "draft": scene_with_rects(4), "format": "svg", "out": "dist/aborted.svg" }),
    )
    .to_string();

    post_and_disconnect(&authority, &body);

    // Wait for the server to finish the call it was sent.
    let target = dir.path().join("dist/aborted.svg");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !target.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }

    if let Ok(text) = std::fs::read_to_string(&target) {
        assert!(
            text.contains("<svg") && text.contains("</svg>"),
            "a disconnected client leaves a complete document, never a partial one: {text}"
        );
    }
    assert!(
        temporary_leftovers(&dir.path().join("dist")).is_empty(),
        "no temporary file is left behind"
    );
}

#[test]
fn concurrent_calls_are_independent_over_the_http_transport() {
    let dir = TempDir::new("mcp-concurrent");
    let (_server, authority) = start_http_server(dir.path());

    let workers: Vec<_> = (1..=8usize)
        .map(|count| {
            let authority = authority.clone();
            thread::spawn(move || {
                let body = mcp_tool_call(1, "compile", json!({ "draft": scene_with_rects(count) }))
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
