//! Acceptance tests for icon-set mode (FEAT-025, C-004).
//!
//! Drives the shipped `vectr icon-set export` command as a subprocess: every
//! icon in a set is written as its own named file under the set's shared canvas
//! and style, a palette change restyles every icon together in one recompile,
//! the mode is gated off by default, and a set that cannot render reports the
//! failure and leaves no partial output (FEAT-025, NFR-011).

mod common;

use common::*;
use serde_json::{json, Value};

/// The rollout flag that enables icon-set mode (FEAT-025). It is off by default.
const ENABLE: &str = "VECTR_ENABLE_ICON_SET_MODE";

/// A style-recipe document.
fn recipe(id: &str) -> String {
    json!({ "id": id, "projectId": "p", "name": id, "parameters": {} }).to_string()
}

/// A definition whose single rect is filled from the palette's `ink` token.
fn icon_definition(id: &str, width: f64) -> String {
    let mut body = def_rect(&format!("{id}-body"), id, 0, 0.0, 0.0, width, 10.0);
    body["fill"] = token_paint("ink");
    definition(id, json!([]), vec![body]).to_string()
}

/// An icon-set document placing the given `(name, definitionRef)` icons.
fn icon_set_document(icons: &[(&str, &str)], name_pattern: Option<&str>) -> String {
    let entries: Vec<Value> = icons
        .iter()
        .map(|(name, reference)| json!({ "name": name, "definitionRef": reference }))
        .collect();
    let mut value = json!({
        "id": "ui",
        "projectId": "p",
        "name": "UI",
        "canvas": { "width": 24.0, "height": 24.0, "background": "transparent" },
        "paletteId": "brand",
        "strokeProfileId": "line",
        "icons": entries
    });
    if let Some(pattern) = name_pattern {
        value["namePattern"] = json!(pattern);
    }
    value.to_string()
}

/// A project with a palette, a stroke profile, two icon definitions, and one
/// icon set placing both under a shared canvas (FEAT-025).
fn icon_set_project(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write(
        "vectr.project.json",
        r#"{"defaultPaletteId":"brand","defaultRecipeId":"flat"}"#,
    );
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("ink", "#111111")]),
    );
    dir.write(
        "strokes/line.json",
        &stroke_profile("line", 2.0, "butt", "miter"),
    );
    dir.write("recipes/flat.json", &recipe("flat"));
    dir.write("definitions/plus.json", &icon_definition("plus", 10.0));
    dir.write("definitions/minus.json", &icon_definition("minus", 8.0));
    dir.write(
        "icon-sets/ui.json",
        &icon_set_document(&[("plus", "plus"), ("minus", "minus")], Some("icon-{name}")),
    );
    dir
}

#[test]
fn icon_set_export_is_gated_off_by_default_and_writes_nothing() {
    let dir = icon_set_project("icon-set-gated");

    let output = run_vectr_env(
        dir.path(),
        &[
            "icon-set",
            "export",
            "ui",
            "--format",
            "svg",
            "--out-dir",
            "icons",
        ],
        &[ENABLE],
        &[],
    );
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("disabled"),
        "the gate is reported: {}",
        stderr(&output)
    );
    assert!(
        !dir.path().join("icons").exists(),
        "nothing is written while the mode is off"
    );
}

#[test]
fn every_icon_exports_to_its_own_named_file_under_the_shared_canvas_and_style() {
    let dir = icon_set_project("icon-set-export");

    let output = run_vectr_env(
        dir.path(),
        &[
            "icon-set",
            "export",
            "ui",
            "--format",
            "svg",
            "--out-dir",
            "icons",
        ],
        &[ENABLE],
        &[(ENABLE, "1")],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let plus = std::fs::read_to_string(dir.path().join("icons/icon-plus.svg"))
        .expect("the plus icon file");
    let minus = std::fs::read_to_string(dir.path().join("icons/icon-minus.svg"))
        .expect("the minus icon file");
    for svg in [&plus, &minus] {
        assert!(
            svg.contains("viewBox=\"0 0 24 24\""),
            "every icon shares the set's canvas: {svg}"
        );
        assert!(
            svg.contains("#111111"),
            "every icon shares the set's palette: {svg}"
        );
    }
    assert!(
        stdout(&output).contains("icon-plus.svg") && stdout(&output).contains("icon-minus.svg"),
        "each written file is named: {}",
        stdout(&output)
    );
}

#[test]
fn a_palette_change_restyles_every_icon_together() {
    let dir = icon_set_project("icon-set-restyle");

    let first = run_vectr_env(
        dir.path(),
        &[
            "icon-set",
            "export",
            "ui",
            "--format",
            "svg",
            "--out-dir",
            "icons-a",
        ],
        &[ENABLE],
        &[(ENABLE, "1")],
    );
    assert_eq!(code(&first), 0, "{}", stderr(&first));

    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("ink", "#ff0000")]),
    );

    let second = run_vectr_env(
        dir.path(),
        &[
            "icon-set",
            "export",
            "ui",
            "--format",
            "svg",
            "--out-dir",
            "icons-b",
        ],
        &[ENABLE],
        &[(ENABLE, "1")],
    );
    assert_eq!(code(&second), 0, "{}", stderr(&second));

    for name in ["icon-plus.svg", "icon-minus.svg"] {
        let before =
            std::fs::read_to_string(dir.path().join("icons-a").join(name)).expect("before");
        let after = std::fs::read_to_string(dir.path().join("icons-b").join(name)).expect("after");
        assert!(before.contains("#111111"), "{before}");
        assert!(
            after.contains("#ff0000"),
            "the restyle reaches every icon ({name}): {after}"
        );
    }
}

#[test]
fn a_duplicate_icon_name_is_refused_before_anything_is_written() {
    let dir = icon_set_project("icon-set-duplicate");
    dir.write(
        "icon-sets/ui.json",
        &icon_set_document(&[("plus", "plus"), ("plus", "minus")], None),
    );

    let output = run_vectr_env(
        dir.path(),
        &[
            "icon-set",
            "export",
            "ui",
            "--format",
            "svg",
            "--out-dir",
            "icons",
        ],
        &[ENABLE],
        &[(ENABLE, "1")],
    );
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).to_lowercase().contains("duplicate")
            || stderr(&output).contains("E_DUPLICATE_NAME"),
        "the duplicate name is reported: {}",
        stderr(&output)
    );
    assert!(
        !dir.path().join("icons").exists(),
        "no partial set is written"
    );
}

#[test]
fn an_unresolved_definition_is_reported_and_no_partial_set_is_written() {
    let dir = icon_set_project("icon-set-unresolved");
    dir.write(
        "icon-sets/ui.json",
        &icon_set_document(&[("plus", "plus"), ("ghost", "absent")], None),
    );

    let output = run_vectr_env(
        dir.path(),
        &[
            "icon-set",
            "export",
            "ui",
            "--format",
            "svg",
            "--out-dir",
            "icons",
        ],
        &[ENABLE],
        &[(ENABLE, "1")],
    );
    assert_ne!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("absent"),
        "the unresolved reference is named: {}",
        stderr(&output)
    );
    assert!(
        !dir.path().join("icons").exists(),
        "no partial set is written"
    );
}

/// An icon whose geometry reaches beyond the shared canvas is warned about
/// rather than silently clipped, and the rest of the set still exports
/// (FEAT-025).
#[test]
fn an_icon_that_overflows_the_shared_canvas_warns_that_detail_may_be_lost() {
    let dir = icon_set_project("icon-set-detail");
    dir.write("definitions/plus.json", &icon_definition("plus", 40.0));

    let output = run_vectr_env(
        dir.path(),
        &[
            "icon-set",
            "export",
            "ui",
            "--format",
            "svg",
            "--out-dir",
            "icons",
        ],
        &[ENABLE],
        &[(ENABLE, "1")],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("W_ICON_DETAIL"),
        "the icon that overflows the canvas is warned about: {}",
        stderr(&output)
    );
    assert!(
        dir.path().join("icons/icon-plus.svg").is_file(),
        "the overflowing icon is still exported"
    );
}
