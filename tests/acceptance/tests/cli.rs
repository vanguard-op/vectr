//! Acceptance tests for the command-line interface (FEAT-016, C-004).
//!
//! Drives the built `vectr` binary as a subprocess: exit codes distinguish
//! success from each class of failure, `--check` writes nothing, a missing input
//! or unwritable output is a clear error, `validate --json` is machine-readable,
//! and `init` scaffolds an idempotent project.

mod common;

use common::*;

const VALID_SCENE: &str = r##"{
  "id": "s",
  "projectId": "project",
  "name": "S",
  "formatVersion": "0.1",
  "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
  "elements": [
    {
      "id": "e1", "sceneId": "s", "order": 0, "kind": "rect",
      "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1, "visible": true
    }
  ]
}"##;

const INVALID_SCENE: &str = r##"{
  "id": "s",
  "projectId": "project",
  "name": "S",
  "formatVersion": "0.1",
  "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
  "elements": [
    {
      "id": "e1", "sceneId": "s", "order": 0, "kind": "rect",
      "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 2, "visible": true
    }
  ]
}"##;

const CYCLIC_SCENE: &str = r##"{
  "id": "s",
  "projectId": "project",
  "name": "S",
  "formatVersion": "0.1",
  "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
  "elements": [
    {
      "id": "a", "sceneId": "s", "parentId": "b", "order": 0, "kind": "rect",
      "geometry": { "width": 10, "height": 10 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1, "visible": true
    },
    {
      "id": "b", "sceneId": "s", "parentId": "a", "order": 1, "kind": "rect",
      "geometry": { "width": 10, "height": 10 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1, "visible": true
    }
  ]
}"##;

const TEXT_MISSING_FONT: &str = r##"{
  "id": "s",
  "projectId": "project",
  "name": "S",
  "formatVersion": "0.1",
  "canvas": { "width": 200, "height": 100, "background": "#ffffff" },
  "elements": [
    {
      "id": "t1", "sceneId": "s", "order": 0, "kind": "text", "fontId": "absent",
      "geometry": { "x": 10, "y": 60, "text": "Hi", "fontSize": 32 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1, "visible": true
    }
  ]
}"##;

fn project_with(tag: &str, scene: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", "{}");
    dir.write("scenes/scene.json", scene);
    dir
}

#[test]
fn no_arguments_prints_usage_and_exits_zero() {
    let dir = TempDir::new("cli-no-args");
    let output = run_vectr(dir.path(), &[]);
    assert_eq!(code(&output), 0);
    let text = stdout(&output);
    assert!(text.contains("Usage:"), "{text}");
    assert!(text.contains("validate"), "{text}");
}

#[test]
fn a_valid_scene_compiles_and_exits_zero_writing_the_model() {
    let dir = project_with("cli-compile", VALID_SCENE);
    let output = run_vectr(
        dir.path(),
        &["compile", "scenes/scene.json", "--out", "model.json"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = std::fs::read_to_string(dir.path().join("model.json")).expect("the model");
    vectr_core::render::parse(&text).expect("the output is a render model");
}

#[test]
fn an_invalid_scene_exits_one_with_diagnostics() {
    let dir = project_with("cli-invalid", INVALID_SCENE);
    let output = run_vectr(dir.path(), &["validate", "scenes/scene.json"]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("E_SCHEMA"), "{}", stderr(&output));
}

#[test]
fn check_only_writes_nothing_and_reflects_validity() {
    let dir = project_with("cli-check", VALID_SCENE);
    let output = run_vectr(dir.path(), &["compile", "scenes/scene.json", "--check"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        !dir.path().join("dist/scene.json").exists(),
        "no output under --check"
    );
}

#[test]
fn a_missing_input_file_is_a_clear_error_with_a_non_zero_exit() {
    let dir = TempDir::new("cli-missing");
    let output = run_vectr(dir.path(), &["validate", "absent.json"]);
    assert_ne!(code(&output), 0);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("cannot read"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_unwritable_output_path_is_a_clear_error_with_a_non_zero_exit() {
    let dir = project_with("cli-unwritable", VALID_SCENE);
    dir.write("blocker", "not a directory");
    let output = run_vectr(
        dir.path(),
        &[
            "compile",
            "scenes/scene.json",
            "--out",
            "blocker/model.json",
        ],
    );
    assert_eq!(code(&output), 5);
    assert!(
        stderr(&output).contains("cannot write"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_failed_compile_leaves_an_existing_output_untouched() {
    let dir = project_with("cli-untouched", INVALID_SCENE);
    dir.write("model.json", "previous");
    let output = run_vectr(
        dir.path(),
        &["compile", "scenes/scene.json", "--out", "model.json"],
    );
    assert_ne!(code(&output), 0);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("model.json")).expect("reads"),
        "previous",
        "no partial output (NFR-011)"
    );
}

#[test]
fn validate_json_emits_machine_readable_findings() {
    let valid = project_with("cli-json-valid", VALID_SCENE);
    let output = run_vectr(valid.path(), &["validate", "--json", "scenes/scene.json"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output).trim(), "[]");

    let invalid = project_with("cli-json-invalid", INVALID_SCENE);
    let output = run_vectr(invalid.path(), &["validate", "--json", "scenes/scene.json"]);
    assert_eq!(code(&output), 1);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout(&output).trim()).expect("valid JSON");
    assert!(parsed.as_array().is_some_and(|items| !items.is_empty()));
}

#[test]
fn the_exit_codes_distinguish_each_failure_class() {
    // 1: invalid scene.
    let invalid = project_with("cli-exit-1", INVALID_SCENE);
    assert_eq!(
        code(&run_vectr(
            invalid.path(),
            &["validate", "scenes/scene.json"]
        )),
        1
    );

    // 2: usage error or missing input.
    let usage = TempDir::new("cli-exit-2");
    assert_eq!(
        code(&run_vectr(usage.path(), &["validate", "absent.json"])),
        2
    );

    // 3: compilation failure (a cycle).
    let cyclic = project_with("cli-exit-3", CYCLIC_SCENE);
    assert_eq!(
        code(&run_vectr(
            cyclic.path(),
            &["compile", "scenes/scene.json", "--check"]
        )),
        3
    );

    // 4: export dependency missing (a required font).
    let missing_font = project_with("cli-exit-4", TEXT_MISSING_FONT);
    assert_eq!(
        code(&run_vectr(
            missing_font.path(),
            &["compile", "scenes/scene.json", "--check"]
        )),
        4
    );

    // 5: output I/O failure.
    let unwritable = project_with("cli-exit-5", VALID_SCENE);
    unwritable.write("blocker", "not a directory");
    assert_eq!(
        code(&run_vectr(
            unwritable.path(),
            &[
                "compile",
                "scenes/scene.json",
                "--out",
                "blocker/model.json"
            ]
        )),
        5
    );
}

#[test]
fn export_svg_writes_a_valid_document_and_exits_zero() {
    let dir = project_with("cli-svg", VALID_SCENE);
    let output = run_vectr(
        dir.path(),
        &[
            "export",
            "scenes/scene.json",
            "--format",
            "svg",
            "--out",
            "out.svg",
        ],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let svg = std::fs::read_to_string(dir.path().join("out.svg")).expect("the svg");
    assert!(svg.contains("<svg") && svg.contains("</svg>"), "{svg}");
}

#[test]
fn export_png_writes_a_png_and_exits_zero() {
    let dir = project_with("cli-png", VALID_SCENE);
    let output = run_vectr(
        dir.path(),
        &[
            "export",
            "scenes/scene.json",
            "--format",
            "png",
            "--out",
            "out.png",
        ],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let bytes = std::fs::read(dir.path().join("out.png")).expect("the png");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
}

#[test]
fn init_scaffolds_a_project_and_is_idempotent() {
    let dir = TempDir::new("cli-init");
    let output = run_vectr(dir.path(), &["init", "habit"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let config = dir.path().join("habit/vectr.project.json");
    assert!(config.is_file(), "the project config is written");
    let scene = dir.path().join("habit/scenes/example.json");
    assert!(scene.is_file(), "the starter scene is written");

    std::fs::write(&scene, "hand-edited").expect("the author edits the scene");
    let output = run_vectr(dir.path(), &["init", "habit"]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("already initialized"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&scene).expect("reads"),
        "hand-edited",
        "an existing file is not overwritten"
    );
}

#[test]
fn the_documented_key_commands_work_against_the_fixture() {
    let root = workspace_root();
    let dir = TempDir::new("cli-fixture");
    let out = dir.path().join("scene.svg");
    let out_arg = out.to_str().expect("utf-8 path");

    let validate = run_vectr(&root, &["validate", "fixtures/scene.json"]);
    assert_eq!(code(&validate), 0, "{}", stderr(&validate));

    let export = run_vectr(
        &root,
        &[
            "export",
            "fixtures/scene.json",
            "--format",
            "svg",
            "--out",
            out_arg,
        ],
    );
    assert_eq!(code(&export), 0, "{}", stderr(&export));
    let svg = std::fs::read_to_string(&out).expect("the svg");
    assert!(svg.contains("<svg") && svg.contains("</svg>"), "{svg}");
    assert!(
        svg.contains("data-name=\"Mark\""),
        "named groups survive: {svg}"
    );
}

#[test]
fn the_default_output_path_is_under_dist() {
    let dir = project_with("cli-default-out", VALID_SCENE);
    let output = run_vectr(dir.path(), &["compile", "scenes/scene.json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        dir.path().join("dist/scene.json").is_file(),
        "{}",
        stdout(&output)
    );
    let text = std::fs::read_to_string(dir.path().join("dist/scene.json")).expect("the model");
    vectr_core::render::parse(&text).expect("a render model");
}

#[test]
fn a_density_flag_on_svg_is_a_usage_error() {
    let dir = project_with("cli-density", VALID_SCENE);
    let output = run_vectr(
        dir.path(),
        &[
            "export",
            "scenes/scene.json",
            "--format",
            "svg",
            "--density",
            "2",
        ],
    );
    assert_eq!(code(&output), 2);
}
