//! Acceptance tests for the command-line interface (FEAT-016, C-004).
//!
//! Drives the built `vectr` binary as a subprocess: a scene is addressed by its
//! identifier among the project's `scenes/` documents, an omitted scene uses the
//! project's default, exit codes distinguish success from each class of failure,
//! `--check` writes nothing, a missing scene or unwritable output is a clear
//! error, `validate --json` is machine-readable, and `init` scaffolds an
//! idempotent project whose starter scene is named as the default.

mod common;

use common::*;
use serde_json::{json, Value};

const VALID_SCENE: &str = r##"{
  "id": "scene",
  "projectId": "project",
  "name": "S",
  "formatVersion": "0.2",
  "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
  "elements": [
    {
      "id": "e1", "sceneId": "scene", "order": 0, "kind": "rect",
      "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1, "visible": true
    }
  ]
}"##;

const INVALID_SCENE: &str = r##"{
  "id": "scene",
  "projectId": "project",
  "name": "S",
  "formatVersion": "0.2",
  "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
  "elements": [
    {
      "id": "e1", "sceneId": "scene", "order": 0, "kind": "rect",
      "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 2, "visible": true
    }
  ]
}"##;

const CYCLIC_SCENE: &str = r##"{
  "id": "scene",
  "projectId": "project",
  "name": "S",
  "formatVersion": "0.2",
  "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
  "elements": [
    {
      "id": "a", "sceneId": "scene", "parentId": "b", "order": 0, "kind": "rect",
      "geometry": { "width": 10, "height": 10 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1, "visible": true
    },
    {
      "id": "b", "sceneId": "scene", "parentId": "a", "order": 1, "kind": "rect",
      "geometry": { "width": 10, "height": 10 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1, "visible": true
    }
  ]
}"##;

const TEXT_MISSING_FONT: &str = r##"{
  "id": "scene",
  "projectId": "project",
  "name": "S",
  "formatVersion": "0.2",
  "canvas": { "width": 200, "height": 100, "background": "#ffffff" },
  "elements": [
    {
      "id": "t1", "sceneId": "scene", "order": 0, "kind": "text", "fontId": "absent",
      "geometry": { "x": 10, "y": 60, "text": "Hi", "fontSize": 32 },
      "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
      "opacity": 1, "visible": true
    }
  ]
}"##;

/// A project holding one scene, `scenes/scene.json`, addressed as `scene`.
///
/// The project config names no default, so a command that omits the scene is a
/// usage error; tests that exercise the default build their own project.
fn project_with(tag: &str, scene: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", "{}");
    dir.write("scenes/scene.json", scene);
    dir
}

/// A scene at `scenes/<id>.json` whose one filled rect resolves palette `red` or
/// `blue`, so a test can tell which scene's assets a command resolved.
fn colored_scene(id: &str, palette_id: &str) -> Value {
    let mut element = rect("r1", 0, 0.0, 0.0, 10.0, 10.0);
    element["fill"] = token_paint("accent");
    let mut document = scene(vec![element]);
    document["id"] = json!(id);
    document["paletteId"] = json!(palette_id);
    document["elements"][0]["sceneId"] = json!(id);
    document
}

/// A project holding two scenes, `red` and `blue`, each resolving its own
/// palette, and optionally naming a default scene (FEAT-016).
fn two_scene_project(tag: &str, default_scene: Option<&str>) -> TempDir {
    let dir = TempDir::new(tag);
    let mut config = json!({});
    if let Some(id) = default_scene {
        config["defaultSceneId"] = json!(id);
    }
    dir.write("vectr.project.json", &config.to_string());
    dir.write(
        "palettes/red.json",
        &palette("red", &[("accent", "#ff0000")]),
    );
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

/// Reads a render model written by a command and returns its first node's fill.
fn first_fill(path: &std::path::Path) -> String {
    let text = std::fs::read_to_string(path).expect("the render model was written");
    let model: Value = serde_json::from_str(&text).expect("the render model is JSON");
    model["nodes"][0]["paint"]["fill"]["value"]
        .as_str()
        .expect("the first node carries a resolved fill")
        .to_string()
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
    let output = run_vectr(dir.path(), &["compile", "scene", "--out", "model.json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = std::fs::read_to_string(dir.path().join("model.json")).expect("the model");
    vectr_core::render::parse(&text).expect("the output is a render model");
}

#[test]
fn an_invalid_scene_exits_one_with_diagnostics() {
    let dir = project_with("cli-invalid", INVALID_SCENE);
    let output = run_vectr(dir.path(), &["validate", "scene"]);
    assert_eq!(code(&output), 1);
    assert!(stderr(&output).contains("E_SCHEMA"), "{}", stderr(&output));
}

#[test]
fn check_only_writes_nothing_and_reflects_validity() {
    let dir = project_with("cli-check", VALID_SCENE);
    let output = run_vectr(dir.path(), &["compile", "scene", "--check"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        !dir.path().join("dist/scene.json").exists(),
        "no output under --check"
    );
}

#[test]
fn a_named_scene_no_document_provides_is_a_clear_error() {
    let dir = project_with("cli-missing-scene", VALID_SCENE);
    let output = run_vectr(dir.path(), &["validate", "absent"]);
    assert_eq!(code(&output), 2);
    let err = stderr(&output);
    assert!(err.contains("absent"), "the missing scene is named: {err}");
    assert!(
        err.contains("was not found"),
        "the error says the scene was not found: {err}"
    );
}

#[test]
fn an_identifier_that_escapes_the_scene_directory_is_refused() {
    // An identifier is untrusted: it must not name a file outside `scenes/`
    // (NFR-021). The document exists, but only under a rejected identifier.
    let dir = project_with("cli-escape", VALID_SCENE);
    for id in ["../scene", "nested/scene", "."] {
        let output = run_vectr(dir.path(), &["validate", id]);
        assert_eq!(code(&output), 2, "`{id}` must be refused");
        assert!(
            stderr(&output).contains("not a valid scene identifier"),
            "{}",
            stderr(&output)
        );
    }
}

#[test]
fn an_unwritable_output_path_is_a_clear_error_with_a_non_zero_exit() {
    let dir = project_with("cli-unwritable", VALID_SCENE);
    dir.write("blocker", "not a directory");
    let output = run_vectr(
        dir.path(),
        &["compile", "scene", "--out", "blocker/model.json"],
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
    let output = run_vectr(dir.path(), &["compile", "scene", "--out", "model.json"]);
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
    let output = run_vectr(valid.path(), &["validate", "--json", "scene"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output).trim(), "[]");

    let invalid = project_with("cli-json-invalid", INVALID_SCENE);
    let output = run_vectr(invalid.path(), &["validate", "--json", "scene"]);
    assert_eq!(code(&output), 1);
    let parsed: serde_json::Value =
        serde_json::from_str(stdout(&output).trim()).expect("valid JSON");
    assert!(parsed.as_array().is_some_and(|items| !items.is_empty()));
}

#[test]
fn the_exit_codes_distinguish_each_failure_class() {
    // 1: invalid scene.
    let invalid = project_with("cli-exit-1", INVALID_SCENE);
    assert_eq!(code(&run_vectr(invalid.path(), &["validate", "scene"])), 1);

    // 2: usage error or missing scene.
    let usage = TempDir::new("cli-exit-2");
    assert_eq!(code(&run_vectr(usage.path(), &["validate", "absent"])), 2);

    // 3: compilation failure (a cycle).
    let cyclic = project_with("cli-exit-3", CYCLIC_SCENE);
    assert_eq!(
        code(&run_vectr(cyclic.path(), &["compile", "scene", "--check"])),
        3
    );

    // 4: export dependency missing (a required font).
    let missing_font = project_with("cli-exit-4", TEXT_MISSING_FONT);
    assert_eq!(
        code(&run_vectr(
            missing_font.path(),
            &["compile", "scene", "--check"]
        )),
        4
    );

    // 5: output I/O failure.
    let unwritable = project_with("cli-exit-5", VALID_SCENE);
    unwritable.write("blocker", "not a directory");
    assert_eq!(
        code(&run_vectr(
            unwritable.path(),
            &["compile", "scene", "--out", "blocker/model.json"]
        )),
        5
    );
}

#[test]
fn export_svg_writes_a_valid_document_and_exits_zero() {
    let dir = project_with("cli-svg", VALID_SCENE);
    let output = run_vectr(
        dir.path(),
        &["export", "scene", "--format", "svg", "--out", "out.svg"],
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
        &["export", "scene", "--format", "png", "--out", "out.png"],
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

    let project = dir.path().join("habit");
    let config = project.join("vectr.project.json");
    assert!(config.is_file(), "the project config is written");

    // The starter scene lives under `scenes/` and the project names it as the
    // default; there is no scene document at the project root (FEAT-016, D-032).
    let scene = project.join("scenes/example.json");
    assert!(
        scene.is_file(),
        "the starter scene is written under scenes/"
    );
    assert!(
        !project.join("scene.json").exists(),
        "no scene document at the project root"
    );
    let config_value: Value =
        serde_json::from_str(&std::fs::read_to_string(&config).expect("reads")).expect("JSON");
    assert_eq!(
        config_value["defaultSceneId"], "example",
        "the project names the starter scene as its default"
    );

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
fn an_omitted_scene_uses_the_project_default() {
    let dir = two_scene_project("cli-default", Some("blue"));
    let output = run_vectr(dir.path(), &["compile", "--out", "model.json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        first_fill(&dir.path().join("model.json")),
        "#0000ff",
        "the default scene's palette resolved"
    );
}

#[test]
fn a_project_that_names_no_default_reports_no_scene_selected() {
    let dir = two_scene_project("cli-no-default", None);
    let output = run_vectr(dir.path(), &["compile", "--out", "model.json"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    let err = stderr(&output);
    assert!(
        err.contains("no scene") && err.contains("default"),
        "the error reports that no scene was selected: {err}"
    );
    assert!(
        !dir.path().join("model.json").exists(),
        "nothing is chosen or written when no scene is selected"
    );
}

#[test]
fn a_default_scene_that_resolves_to_no_document_names_the_missing_scene() {
    let dir = two_scene_project("cli-default-missing", Some("absent"));
    let output = run_vectr(dir.path(), &["validate"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    let err = stderr(&output);
    assert!(
        err.contains("absent"),
        "the missing default scene is named: {err}"
    );
}

#[test]
fn a_named_scene_overrides_the_project_default() {
    let dir = two_scene_project("cli-named-override", Some("red"));
    let output = run_vectr(dir.path(), &["compile", "blue", "--out", "model.json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        first_fill(&dir.path().join("model.json")),
        "#0000ff",
        "the named scene overrides the default"
    );
}

#[test]
fn a_named_scene_in_a_multi_scene_project_resolves_only_its_own_assets() {
    let dir = two_scene_project("cli-multi-scene", Some("red"));

    let red = run_vectr(dir.path(), &["compile", "red", "--out", "red.json"]);
    assert_eq!(code(&red), 0, "{}", stderr(&red));
    let blue = run_vectr(dir.path(), &["compile", "blue", "--out", "blue.json"]);
    assert_eq!(code(&blue), 0, "{}", stderr(&blue));

    assert_eq!(first_fill(&dir.path().join("red.json")), "#ff0000");
    assert_eq!(first_fill(&dir.path().join("blue.json")), "#0000ff");
}

#[test]
fn the_documented_key_commands_work_against_the_fixture() {
    let fixtures = workspace_root().join("fixtures");
    let dir = TempDir::new("cli-fixture");
    let out = dir.path().join("scene.svg");
    let out_arg = out.to_str().expect("utf-8 path");

    let validate = run_vectr(&fixtures, &["validate", "example"]);
    assert_eq!(code(&validate), 0, "{}", stderr(&validate));

    let export = run_vectr(
        &fixtures,
        &["export", "example", "--format", "svg", "--out", out_arg],
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
fn the_fixture_project_names_its_default_scene_and_addresses_another() {
    // The fixture project is a two-scene project laid out per D-032: scenes live
    // under `scenes/` and the project names one as its default.
    let fixtures = workspace_root().join("fixtures");

    let default = run_vectr(&fixtures, &["validate"]);
    assert_eq!(code(&default), 0, "{}", stderr(&default));

    let named = run_vectr(&fixtures, &["validate", "mark"]);
    assert_eq!(code(&named), 0, "{}", stderr(&named));
}

#[test]
fn the_default_output_path_is_under_dist() {
    let dir = project_with("cli-default-out", VALID_SCENE);
    let output = run_vectr(dir.path(), &["compile", "scene"]);
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
        &["export", "scene", "--format", "svg", "--density", "2"],
    );
    assert_eq!(code(&output), 2);
}

// ---------------------------------------------------------------------------
// Part-scoped rendering from the command line (FEAT-031, C-004)
// ---------------------------------------------------------------------------

/// A definition addressed by its identifier renders on its own from the command
/// line: the preview is framed to the part's own bounds and carries its
/// project-resolved style, and the command reports the frame it used
/// (FEAT-031).
#[test]
fn a_definition_renders_on_its_own_from_the_cli() {
    let dir = part_project("cli-part-definition");
    let output = run_vectr(
        dir.path(),
        &["render", "badge", "--format", "svg", "--out", "badge.svg"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("frame 30x40"),
        "the render reports the frame it used: {}",
        stdout(&output)
    );

    let svg = std::fs::read_to_string(dir.path().join("badge.svg")).expect("the svg");
    assert!(svg.contains("<svg") && svg.contains("</svg>"), "{svg}");
    assert!(svg.contains("viewBox=\"0 0 30 40\""), "{svg}");
    assert!(svg.contains("badge-body"), "{svg}");
    assert!(
        svg.contains("fill=\"#ff0000\""),
        "the part carries the project's resolved style: {svg}"
    );
}

/// A named element subtree addressed by its identifier renders on its own from
/// the command line: its structure appears and the rest of the scene does not
/// (FEAT-031).
#[test]
fn an_element_subtree_renders_on_its_own_from_the_cli() {
    let dir = part_project("cli-part-subtree");
    let output = run_vectr(
        dir.path(),
        &["render", "mark", "--format", "svg", "--out", "mark.svg"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("frame 20x10"),
        "{}",
        stdout(&output)
    );

    let svg = std::fs::read_to_string(dir.path().join("mark.svg")).expect("the svg");
    assert!(svg.contains("mark-rect"), "{svg}");
    assert!(
        !svg.contains("other"),
        "the rest of the scene is absent: {svg}"
    );
}

/// A part that places another definition is included and resolved when rendered
/// in isolation from the command line (FEAT-031).
#[test]
fn a_part_that_places_another_definition_resolves_it_from_the_cli() {
    let dir = part_project("cli-part-nested");
    dir.write(
        "definitions/inner.json",
        &definition(
            "inner",
            json!([]),
            vec![def_rect("dot", "inner", 0, 0.0, 0.0, 5.0, 5.0)],
        )
        .to_string(),
    );
    dir.write(
        "definitions/outer.json",
        &definition(
            "outer",
            json!([]),
            vec![def_instance("place", "outer", 0, "inner")],
        )
        .to_string(),
    );

    let output = run_vectr(
        dir.path(),
        &["render", "outer", "--format", "svg", "--out", "outer.svg"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("frame 5x5"),
        "the referenced definition's geometry reaches the preview: {}",
        stdout(&output)
    );
}

/// A corrected part re-rendered in isolation reflects the correction from the
/// command line (FEAT-031).
#[test]
fn a_corrected_part_re_renders_from_the_cli() {
    let dir = part_project("cli-part-correction");
    let before = run_vectr(
        dir.path(),
        &["render", "badge", "--format", "svg", "--out", "badge.svg"],
    );
    assert_eq!(code(&before), 0, "{}", stderr(&before));
    assert!(
        stdout(&before).contains("frame 30x40"),
        "{}",
        stdout(&before)
    );

    dir.write(
        "definitions/badge.json",
        &definition(
            "badge",
            json!([]),
            vec![def_rect("badge-body", "badge", 0, 10.0, 20.0, 60.0, 80.0)],
        )
        .to_string(),
    );
    let after = run_vectr(
        dir.path(),
        &["render", "badge", "--format", "svg", "--out", "badge.svg"],
    );
    assert_eq!(code(&after), 0, "{}", stderr(&after));
    assert!(
        stdout(&after).contains("frame 60x80"),
        "the re-render reflects the corrected part: {}",
        stdout(&after)
    );
}

/// A part with no resolved geometry is reported with a defined fallback frame
/// rather than failing (FEAT-031).
#[test]
fn a_part_with_no_resolved_geometry_is_reported_with_a_fallback_frame_from_the_cli() {
    let dir = part_project("cli-part-empty");
    let output = run_vectr(
        dir.path(),
        &["render", "empty", "--format", "svg", "--out", "empty.svg"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("frame 100x100"),
        "the defined fallback frame is reported: {}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("W_EMPTY_PART_FRAME"),
        "the empty part is reported: {}",
        stderr(&output)
    );
    assert!(dir.path().join("empty.svg").is_file());
}

/// A part identifier that resolves to no definition or element is missing input
/// (exit 2), naming the part, and no preview is written (FEAT-031).
#[test]
fn a_part_identifier_that_resolves_to_nothing_is_missing_input() {
    let dir = part_project("cli-part-unknown");
    let output = run_vectr(dir.path(), &["render", "absent", "--format", "svg"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.contains("E_PART"), "{err}");
    assert!(
        err.contains("absent"),
        "the unresolved part is named: {err}"
    );
    assert!(
        !dir.path().join("dist/absent.svg").exists(),
        "no preview is written for an unresolved part"
    );
}

/// A definition rendered in isolation against a project that names no default
/// palette is a clear project-asset error naming the missing palette, and no
/// preview is written (FEAT-031, FEAT-016).
///
/// A definition carries no palette of its own and has no placing scene, so the
/// project's `defaultPaletteId` is the only source of its colours; a project
/// that names none cannot resolve them (FEAT-031, D-039).
#[test]
fn a_definition_with_no_default_palette_names_the_missing_palette() {
    let dir = part_project("cli-part-no-default-palette");
    // Drop the default palette the fixture otherwise names, leaving the
    // definition with no palette to resolve its token against.
    dir.write("vectr.project.json", r#"{"defaultSceneId":"main"}"#);

    let output = run_vectr(
        dir.path(),
        &["render", "badge", "--format", "svg", "--out", "badge.svg"],
    );
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    let err = stderr(&output);
    assert!(err.contains("E_PROJECT_ASSET"), "{err}");
    assert!(
        err.contains("no default palette"),
        "the missing default palette is named: {err}"
    );
    assert!(
        !dir.path().join("badge.svg").exists(),
        "no preview is written when the default palette is missing"
    );
}

/// A part larger than the requested preview size is rendered at the requested
/// size, and the command reports the frame it used (FEAT-031).
#[test]
fn a_part_too_large_for_the_requested_size_reports_the_frame_used_from_the_cli() {
    let dir = part_project("cli-part-size");
    let output = run_vectr(
        dir.path(),
        &["render", "badge", "--format", "svg", "--width", "15"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("frame 15x20"),
        "a single dimension scales the other to the part's aspect ratio: {}",
        stdout(&output)
    );
}

/// A part preview with no `--out` writes to `dist/<part>.<format>` (FEAT-031).
#[test]
fn a_part_preview_defaults_its_output_to_dist_part() {
    let dir = part_project("cli-part-default-out");
    let output = run_vectr(dir.path(), &["render", "badge", "--format", "svg"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        dir.path().join("dist/badge.svg").is_file(),
        "{}",
        stdout(&output)
    );
}

/// A part preview renders to PNG from the command line (FEAT-031).
#[test]
fn a_part_renders_to_png_from_the_cli() {
    let dir = part_project("cli-part-png");
    let output = run_vectr(
        dir.path(),
        &["render", "badge", "--format", "png", "--out", "badge.png"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let bytes = std::fs::read(dir.path().join("badge.png")).expect("the png");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
}

/// `vectr inspect` renders a whole-scene preview, reports where it was written
/// and the size it came out at, and notes that the visual comparison with the
/// request is the caller's (FEAT-022, C-004).
#[test]
fn inspect_writes_a_preview_and_notes_the_missing_capability() {
    let dir = project_with("cli-inspect", VALID_SCENE);
    let output = run_vectr(
        dir.path(),
        &["inspect", "scene", "--out", "dist/preview.png"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let bytes =
        std::fs::read(dir.path().join("dist/preview.png")).expect("the preview was written");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "a PNG preview");
    assert!(
        stdout(&output).contains("dist/preview.png"),
        "the written path is reported: {}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains("preview 100x100"),
        "the rendered size is reported: {}",
        stdout(&output)
    );
    assert!(
        stderr(&output).contains("W_INSPECTION_UNAVAILABLE"),
        "the command line has no inspection capability of its own: {}",
        stderr(&output)
    );
}

/// A preview too small to judge can be raised from the command line (FEAT-022).
#[test]
fn inspect_size_is_configurable_from_the_cli() {
    let dir = project_with("cli-inspect-size", VALID_SCENE);
    let output = run_vectr(
        dir.path(),
        &[
            "inspect",
            "scene",
            "--width",
            "64",
            "--height",
            "48",
            "--out",
            "dist/preview.png",
        ],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stdout(&output).contains("preview 64x48"),
        "the requested size is honoured: {}",
        stdout(&output)
    );
}

/// A preview with no `--out` defaults to `dist/<scene>.png` (FEAT-022).
#[test]
fn inspect_defaults_its_output_to_dist_scene_png() {
    let dir = project_with("cli-inspect-default-out", VALID_SCENE);
    let output = run_vectr(dir.path(), &["inspect", "scene"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        dir.path().join("dist/scene.png").is_file(),
        "{}",
        stdout(&output)
    );
}

/// A scene that fails the structural gate is reported before any preview is
/// rendered and nothing is written (FEAT-022, NFR-011).
#[test]
fn inspect_reports_a_structural_failure_before_rendering() {
    let dir = project_with("cli-inspect-invalid", INVALID_SCENE);
    let output = run_vectr(
        dir.path(),
        &["inspect", "scene", "--out", "dist/preview.png"],
    );
    assert_ne!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("E_SCHEMA"),
        "the structural finding is reported: {}",
        stderr(&output)
    );
    assert!(
        !dir.path().join("dist/preview.png").exists(),
        "nothing is rendered for an invalid scene"
    );
}
