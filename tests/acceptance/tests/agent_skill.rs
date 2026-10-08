//! Acceptance tests for the agent skill and authoring guide (FEAT-020, C-004).
//!
//! The skill's promise is that a model with only the shipped artifacts — the
//! skill, its reference guide, and the published schema — can author a scene
//! that validates, compiles, and renders. The judged, cross-model half of that
//! promise is measured by the evaluation harness; these checks pin the
//! deterministic half: the package is well formed, it targets the installed
//! tool, the scaffold's guide teaches the full workflow, and every example the
//! skill and the scaffold ship actually runs through the real toolchain.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use serde_json::Value;

/// The installed `vectr` version, as `vectr --version` prints it.
fn cli_version(dir: &TempDir) -> String {
    let output = run_vectr(dir.path(), &["--version"]);
    assert_eq!(code(&output), 0);
    stdout(&output)
        .trim()
        .rsplit(' ')
        .next()
        .expect("a version token")
        .to_string()
}

/// The format version the published contract declares.
fn declared_format_version(dir: &TempDir) -> String {
    let output = run_vectr(dir.path(), &["schema"]);
    assert_eq!(code(&output), 0);
    let contract: Value = serde_json::from_str(&stdout(&output)).expect("the contract is JSON");
    contract["x-vectr-formatVersion"]
        .as_str()
        .expect("a declared version")
        .to_string()
}

/// Every fenced `json` block in a markdown document that parses as JSON.
fn fenced_json_blocks(text: &str) -> Vec<Value> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```json") {
        let after = &rest[start + "```json".len()..];
        let Some(end) = after.find("```") else {
            break;
        };
        if let Ok(value) = serde_json::from_str::<Value>(after[..end].trim()) {
            blocks.push(value);
        }
        rest = &after[end + 3..];
    }
    blocks
}

/// The palette example: the block carrying tokens.
fn palette_block(blocks: &[Value]) -> &Value {
    blocks
        .iter()
        .find(|block| block.get("tokens").and_then(Value::as_array).is_some())
        .expect("a palette example")
}

/// The stroke-profile example: the block carrying cap, join, and width.
fn stroke_block(blocks: &[Value]) -> &Value {
    blocks
        .iter()
        .find(|block| {
            block.get("cap").is_some() && block.get("join").is_some() && block.get("width").is_some()
        })
        .expect("a stroke profile example")
}

/// The largest scene example: the block carrying the most elements.
fn scene_block(blocks: &[Value]) -> &Value {
    blocks
        .iter()
        .filter(|block| block.get("elements").and_then(Value::as_array).is_some())
        .max_by_key(|block| {
            block["elements"]
                .as_array()
                .map(Vec::len)
                .unwrap_or_default()
        })
        .expect("a scene example")
}

/// The skill's frontmatter fields, as `key: value` pairs.
///
/// A folded or literal block scalar (`>` / `|`) joins its indented continuation
/// lines into one value.
fn frontmatter(text: &str) -> Vec<(String, String)> {
    let body = text.strip_prefix("---").expect("frontmatter opens");
    let end = body.find("\n---").expect("frontmatter closes");

    let mut fields: Vec<(String, String)> = Vec::new();
    let mut current: Option<usize> = None;
    for line in body[..end].lines() {
        if line.starts_with([' ', '\t']) {
            if let Some(index) = current {
                let continuation = line.trim();
                if !continuation.is_empty() {
                    let value = &mut fields[index].1;
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(continuation);
                }
            }
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let value = if value == ">" || value == "|" {
            String::new()
        } else {
            value.to_string()
        };
        fields.push((key.trim().to_string(), value));
        current = Some(fields.len() - 1);
    }
    fields
}

fn field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Writes a project with a scene, palette, and stroke, then runs the full
/// author-validate-compile-render loop the guide teaches.
fn run_the_worked_example(tag: &str, palette: &Value, stroke: Option<&Value>, scene: &Value) {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", "{}");
    dir.write("palettes/brand.json", &palette.to_string());
    if let Some(stroke) = stroke {
        dir.write("strokes/hairline.json", &stroke.to_string());
    }
    dir.write("scenes/logo.json", &scene.to_string());

    let validate = run_vectr(dir.path(), &["validate", "scenes/logo.json"]);
    assert_eq!(
        code(&validate),
        0,
        "the example must validate:\n{}",
        stderr(&validate)
    );

    let compile = run_vectr(dir.path(), &["compile", "scenes/logo.json", "--check"]);
    assert_eq!(
        code(&compile),
        0,
        "the example must compile:\n{}",
        stderr(&compile)
    );

    let svg_out = dir.path().join("dist/logo.svg");
    let export_svg = run_vectr(
        dir.path(),
        &[
            "export",
            "scenes/logo.json",
            "--format",
            "svg",
            "--out",
            export_svg_arg(&svg_out),
        ],
    );
    assert_eq!(code(&export_svg), 0, "{}", stderr(&export_svg));
    let svg = fs::read_to_string(&svg_out).expect("the SVG was written");
    assert!(svg.contains("<svg") && svg.contains("</svg>"), "{svg}");
    assert!(!svg.contains("<script"), "the output is inert (NFR-023)");

    let png_out = dir.path().join("dist/logo.png");
    let export_png = run_vectr(
        dir.path(),
        &[
            "export",
            "scenes/logo.json",
            "--format",
            "png",
            "--width",
            "256",
            "--height",
            "256",
            "--out",
            export_png_arg(&png_out),
        ],
    );
    assert_eq!(code(&export_png), 0, "{}", stderr(&export_png));
    let png = fs::read(&png_out).expect("the PNG was written");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "a real PNG");
}

fn export_svg_arg(path: &Path) -> &str {
    path.to_str().expect("a utf-8 path")
}

fn export_png_arg(path: &Path) -> &str {
    path.to_str().expect("a utf-8 path")
}

#[test]
fn the_skill_package_is_well_formed_and_targets_the_installed_tool() {
    let root = workspace_root();
    let skill_dir = root.join("skills/vectr");
    let skill = fs::read_to_string(skill_dir.join("SKILL.md")).expect("SKILL.md");

    let fields = frontmatter(&skill);
    assert_eq!(field(&fields, "name"), Some("vectr"));
    let description = field(&fields, "description").expect("a description");
    assert!(description.len() > 40, "the description tells the model when to load it");
    let version = field(&fields, "version").expect("a version");
    let parts: Vec<&str> = version.split('.').collect();
    assert_eq!(parts.len(), 3, "`{version}` is semver");

    // The packaged files the skill promises are present.
    assert!(skill_dir.join("references/authoring-guide.md").is_file());
    assert!(skill_dir.join("assets/scene.template.json").is_file());
    assert!(skill.contains("references/authoring-guide.md"), "{skill}");
    assert!(skill.contains("assets/scene.template.json"), "{skill}");

    // The skill targets the installed build and the served contract; if these
    // diverge the skill's own instructions are to report the mismatch.
    let dir = TempDir::new("skill-version");
    assert_eq!(version, cli_version(&dir));
    assert_eq!(declared_format_version(&dir), VERSION);
    assert!(
        skill.contains("Version compatibility") && skill.contains("report the mismatch"),
        "the skill instructs a version-mismatch report"
    );
    assert!(skill.contains(VERSION), "the skill names the format version");
}

#[test]
fn the_shipped_scene_template_validates_and_compiles() {
    let root = workspace_root();
    let template =
        fs::read_to_string(root.join("skills/vectr/assets/scene.template.json")).expect("template");
    let template: Value = serde_json::from_str(&template).expect("the template is JSON");

    let dir = TempDir::new("skill-template");
    dir.write("vectr.project.json", "{}");
    dir.write("scenes/template.json", &template.to_string());

    let validate = run_vectr(dir.path(), &["validate", "scenes/template.json"]);
    assert_eq!(code(&validate), 0, "{}", stderr(&validate));
    let compile = run_vectr(dir.path(), &["compile", "scenes/template.json", "--check"]);
    assert_eq!(code(&compile), 0, "{}", stderr(&compile));
}

#[test]
fn the_guides_worked_example_validates_compiles_and_exports() {
    let guide = fs::read_to_string(workspace_root().join("skills/vectr/references/authoring-guide.md"))
        .expect("the authoring guide");
    let blocks = fenced_json_blocks(&guide);
    assert!(blocks.len() >= 3, "the guide carries several examples");

    run_the_worked_example(
        "skill-example",
        palette_block(&blocks),
        Some(stroke_block(&blocks)),
        scene_block(&blocks),
    );
}

#[test]
fn the_scaffolded_authoring_guide_teaches_the_workflow_and_its_example_works() {
    let dir = TempDir::new("scaffold-guide");
    let init = run_vectr(dir.path(), &["init", "habit"]);
    assert_eq!(code(&init), 0, "{}", stderr(&init));
    let project = dir.path().join("habit");
    let guide = fs::read_to_string(project.join("AGENTS.md")).expect("the scaffolded guide");

    for step in ["vectr schema", "vectr validate", "vectr compile", "vectr export"] {
        assert!(guide.contains(step), "the guide teaches `{step}`");
    }
    // The workflow's inspect step, its bounded retry, and the ambiguous-request
    // defaults are what FEAT-020's acceptance criteria turn on.
    assert!(guide.contains("Inspect and correct"), "the guide teaches inspection");
    assert!(guide.contains("Retry once"), "the guide bounds the retry");
    assert!(guide.contains("Defaults for an ambiguous request"));
    assert!(guide.contains("Licensing"));

    // The guide names the versions of the tool that wrote it.
    let cli_version = cli_version(&dir);
    assert!(guide.contains(&cli_version), "the guide names the tool version");
    assert!(guide.contains(VERSION), "the guide names the format version");

    // The scaffold's own worked example runs end to end.
    let blocks = fenced_json_blocks(&guide);
    run_the_worked_example(
        "scaffold-example",
        palette_block(&blocks),
        None,
        scene_block(&blocks),
    );
}

#[test]
fn the_guide_directs_recovery_from_an_invalid_scene() {
    let root = workspace_root();
    let skill = fs::read_to_string(root.join("skills/vectr/SKILL.md")).expect("SKILL.md");
    let guide = fs::read_to_string(root.join("skills/vectr/references/authoring-guide.md"))
        .expect("the authoring guide");

    // The guide directs validation before rendering and a bounded retry, and it
    // names the diagnostics it points a model at.
    assert!(
        skill.contains("Retry authoring once"),
        "the skill bounds the retry"
    );
    assert!(guide.contains("Retry once"), "the guide bounds the retry");
    assert!(guide.contains("Never export"), "no export from a failed scene");
    for code in [
        "E_SCHEMA",
        "E_PARSE",
        "E_FORMAT_VERSION",
        "E_INVALID_COLOR",
    ] {
        assert!(guide.contains(code), "the guide names `{code}`");
    }

    // The diagnostic the guide points at is one the tool actually reports, with
    // the location the guide promises.
    let dir = TempDir::new("guide-diagnostics");
    dir.write("vectr.project.json", "{}");
    let mut document = scene(vec![rect("r1", 0, 0.0, 0.0, 10.0, 10.0)]);
    document["elements"][0]["opacity"] = serde_json::json!(2);
    dir.write("scenes/scene.json", &document.to_string());

    let output = run_vectr(dir.path(), &["validate", "--json", "scenes/scene.json"]);
    assert_eq!(code(&output), 1);
    let findings: Value = serde_json::from_str(stdout(&output).trim()).expect("JSON findings");
    let finding = findings
        .as_array()
        .and_then(|findings| findings.iter().find(|f| f["code"] == "E_SCHEMA"))
        .expect("an E_SCHEMA finding");
    assert!(
        finding["location"].is_object(),
        "the diagnostic carries the location the guide describes: {finding}"
    );
}

/// The workspace's skill directory, exposed for tests that need more than the
/// shared helpers.
#[allow(dead_code)]
fn skill_dir() -> PathBuf {
    workspace_root().join("skills/vectr")
}
