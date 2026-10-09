//! Acceptance tests for the embedded library (FEAT-021, C-002).
//!
//! The library is the same engine the command line drives, so an embedder that
//! links it gets the command line's result: the render model the CLI writes and
//! the SVG the CLI exports are what the library returns in process, and an
//! invalid scene is the same structured, located error the CLI reports. The
//! library's determinism and concurrency guarantees are pinned by the engine's
//! own embedded tests; these checks cover the cross-surface equivalence
//! acceptance criterion that neither front end alone can prove (FEAT-021).

mod common;

use common::*;
use serde_json::{json, Value};
use vectr_core::{compile_with_style, export_svg, parse, SvgOptions};
use vectr_project::ProjectAssets;

/// A project whose scene paints a palette token and strokes a shared profile,
/// so both front ends must resolve the same assets to match.
fn library_project(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
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
    write_scene_as(&dir, "logo", scene_with(vec![card], None, Some("brand")));
    dir
}

/// Parses the scene document a project holds, as the library receives it.
fn project_scene(dir: &TempDir, id: &str) -> vectr_core::Scene {
    let path = dir.path().join(format!("scenes/{id}.json"));
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    parse(&source).unwrap_or_else(|diagnostics| panic!("expected a valid scene: {diagnostics}"))
}

#[test]
fn the_library_compiles_the_same_project_as_the_command_line() {
    let dir = library_project("embedded-model");

    let cli = run_vectr(dir.path(), &["compile", "logo", "--out", "dist/model.json"]);
    assert_eq!(code(&cli), 0, "{}", stderr(&cli));
    let cli_model = std::fs::read_to_string(dir.path().join("dist/model.json"))
        .expect("the CLI wrote a render model");

    let scene = project_scene(&dir, "logo");
    let assets = ProjectAssets::load(dir.path(), &scene).expect("the project assets load");
    assert!(
        !assets.check_references(&scene).has_errors(),
        "the scene resolves against its project"
    );
    let model = compile_with_style(&scene, &assets.style_context()).expect("the scene compiles");
    let library_model = model.to_json_pretty().expect("the model serializes");

    assert_eq!(
        library_model, cli_model,
        "the library returns the render model the CLI writes (FEAT-021)"
    );
}

#[test]
fn the_library_exports_the_same_svg_as_the_command_line() {
    let dir = library_project("embedded-svg");

    let cli = run_vectr(
        dir.path(),
        &[
            "export",
            "logo",
            "--format",
            "svg",
            "--out",
            "dist/logo.svg",
        ],
    );
    assert_eq!(code(&cli), 0, "{}", stderr(&cli));
    let cli_svg =
        std::fs::read_to_string(dir.path().join("dist/logo.svg")).expect("the CLI wrote an SVG");

    let scene = project_scene(&dir, "logo");
    let assets = ProjectAssets::load(dir.path(), &scene).expect("the project assets load");
    let model = compile_with_style(&scene, &assets.style_context()).expect("the scene compiles");
    let library_svg = export_svg(&model, &SvgOptions::default()).expect("the model exports");

    assert_eq!(
        library_svg, cli_svg,
        "the library exports the SVG the CLI writes (FEAT-021)"
    );
}

#[test]
fn the_library_reports_the_same_structured_error_as_the_command_line() {
    let dir = TempDir::new("embedded-error");
    dir.write("vectr.project.json", "{}");
    let mut broken = rect("e1", 0, 0.0, 0.0, 10.0, 10.0);
    broken["opacity"] = json!(2.0);
    write_scene_as(&dir, "broken", scene(vec![broken]));

    let cli = run_vectr(dir.path(), &["validate", "broken", "--json"]);
    assert_eq!(code(&cli), 1, "{}", stderr(&cli));
    let findings: Value =
        serde_json::from_str(stdout(&cli).trim()).expect("validate --json prints an array");
    let cli_error = findings
        .as_array()
        .expect("an array of findings")
        .iter()
        .find(|finding| finding["code"] == "E_SCHEMA")
        .unwrap_or_else(|| panic!("the CLI reports the schema error: {findings}"));

    let source =
        std::fs::read_to_string(dir.path().join("scenes/broken.json")).expect("the scene document");
    let diagnostics = parse(&source).expect_err("the library refuses the same scene");
    let library_error = diagnostics.errors().next().expect("an error");
    let library_value = serde_json::to_value(library_error).expect("the error serializes");

    assert_eq!(
        &library_value, cli_error,
        "the library returns the same structured, located error as the CLI (FEAT-021)"
    );
}

#[test]
fn the_library_is_safe_for_concurrent_use() {
    // FEAT-021's edge case: concurrent use from multiple threads is safe, with
    // no shared-state corruption. The library's entry points take a `&Scene`
    // and hold no shared mutable state, so many threads compiling and
    // exporting the same scene must agree byte for byte.
    let dir = library_project("embedded-concurrent");
    let scene = project_scene(&dir, "logo");
    let assets = ProjectAssets::load(dir.path(), &scene).expect("the project assets load");
    let style = assets.style_context();
    let expected = {
        let model = compile_with_style(&scene, &style).expect("the scene compiles");
        export_svg(&model, &SvgOptions::default()).expect("the model exports")
    };

    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..4 {
                    let model = compile_with_style(&scene, &style)
                        .expect("the scene compiles concurrently");
                    let svg = export_svg(&model, &SvgOptions::default())
                        .expect("the model exports concurrently");
                    assert_eq!(svg, expected, "a concurrent export diverged");
                }
            });
        }
    });
}
