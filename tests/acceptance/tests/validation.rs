//! Acceptance tests for scene validation (FEAT-018, C-004).
//!
//! Drives `vectr validate` as a subprocess with the scene addressed by its
//! identifier (FEAT-016): an unknown property and a broken reference are
//! reported with their location, a valid scene passes with no errors, a colour
//! value the target cannot render is reported before anything is written, every
//! problem in a scene is reported rather than only the first, warnings are
//! distinguished from errors, and a file that is not a scene is a clear type
//! error rather than a crash.

mod common;

use common::*;
use serde_json::{json, Value};

/// A project directory that a scene can resolve its palette from.
fn project(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", "{}");
    dir
}

/// Runs `vectr validate --json` and returns the exit code and decoded findings.
fn validate_json(dir: &TempDir, scene: &str) -> (i32, Vec<Value>, String) {
    let output = run_vectr(dir.path(), &["validate", "--json", scene]);
    let findings: Value = serde_json::from_str(stdout(&output).trim()).unwrap_or_else(|error| {
        panic!("validate --json is not JSON: {error}\n{}", stdout(&output))
    });
    let findings = findings.as_array().expect("--json prints an array").clone();
    (code(&output), findings, stderr(&output))
}

/// The diagnostic codes a finding list carries.
fn codes(findings: &[Value]) -> Vec<&str> {
    findings
        .iter()
        .filter_map(|finding| finding["code"].as_str())
        .collect()
}

/// The `jsonPath` a finding is located at, when it carries one.
fn json_path(finding: &Value) -> Option<&str> {
    finding["location"]["jsonPath"].as_str()
}

#[test]
fn a_valid_scene_validates_with_no_errors() {
    let dir = project("validate-valid");
    let document = scene(vec![rect("r1", 0, 0.0, 0.0, 10.0, 10.0)]);
    let id = write_scene_as(&dir, "scene", document);

    let (exit, findings, err) = validate_json(&dir, &id);
    assert_eq!(exit, 0, "{err}");
    assert!(findings.is_empty(), "{findings:?}");

    // Text mode agrees: a valid scene prints nothing and exits zero.
    let output = run_vectr(dir.path(), &["validate", id.as_str()]);
    assert_eq!(code(&output), 0);
    assert!(stderr(&output).is_empty(), "{}", stderr(&output));
}

#[test]
fn an_unknown_property_is_reported_with_its_location() {
    let dir = project("validate-unknown");
    let mut document = scene(vec![rect("r1", 0, 0.0, 0.0, 10.0, 10.0)]);
    document["bogus"] = json!(1);
    let id = write_scene_as(&dir, "unknown", document);

    let (exit, findings, _) = validate_json(&dir, &id);
    assert_eq!(exit, 1);
    assert!(codes(&findings).contains(&"E_SCHEMA"), "{findings:?}");
    let finding = findings
        .iter()
        .find(|finding| finding["code"] == "E_SCHEMA")
        .expect("an E_SCHEMA finding");
    assert!(finding["message"]
        .as_str()
        .is_some_and(|m| m.contains("bogus")));
    // The location is present: a line/column for a parse-level field error.
    let location = finding["location"].as_object().expect("a location");
    assert!(!location.is_empty(), "{finding}");
}

#[test]
fn a_broken_reference_is_reported_against_its_element() {
    let dir = project("validate-reference");
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "#ff0000")]),
    );

    let mut with_token = rect("r1", 0, 0.0, 0.0, 10.0, 10.0);
    with_token["fill"] = token_paint("nope");
    let mut with_profile = rect("r2", 1, 20.0, 0.0, 10.0, 10.0);
    with_profile["stroke"] = stroke("ghost", "accent");
    let document = scene_with(vec![with_token, with_profile], None, Some("brand"));
    let id = write_scene_as(&dir, "reference", document);

    let (exit, findings, _) = validate_json(&dir, &id);
    assert_eq!(exit, 1);
    let all = codes(&findings);
    assert!(all.contains(&"E_UNDEFINED_TOKEN"), "{findings:?}");
    assert!(all.contains(&"E_UNDEFINED_STROKE"), "{findings:?}");

    let token = findings
        .iter()
        .find(|finding| finding["code"] == "E_UNDEFINED_TOKEN")
        .expect("an undefined-token finding");
    assert_eq!(token["location"]["elementId"], "r1");
    assert!(json_path(token).is_some_and(|path| path.contains("/fill")));

    let profile = findings
        .iter()
        .find(|finding| finding["code"] == "E_UNDEFINED_STROKE")
        .expect("an undefined-stroke finding");
    assert_eq!(profile["location"]["elementId"], "r2");
}

#[test]
fn a_palette_token_that_is_not_a_colour_is_reported_and_nothing_is_written() {
    let dir = project("validate-palette-colour");
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "not-a-colour")]),
    );
    let document = scene_with(
        vec![{
            let mut element = rect("r1", 0, 0.0, 0.0, 10.0, 10.0);
            element["fill"] = token_paint("accent");
            element
        }],
        None,
        Some("brand"),
    );
    let id = write_scene_as(&dir, "palette-colour", document);

    let (exit, findings, _) = validate_json(&dir, &id);
    assert_ne!(exit, 0, "an invalid colour never passes silently");
    assert!(
        codes(&findings).contains(&"E_INVALID_COLOR"),
        "{findings:?}"
    );
    let finding = findings
        .iter()
        .find(|finding| finding["code"] == "E_INVALID_COLOR")
        .expect("an invalid-colour finding");
    assert!(
        json_path(finding).is_some_and(|path| path.contains("/tokens/0/value")),
        "the token value is located: {finding}"
    );

    // No rendering happens for the broken project.
    let output = run_vectr(
        dir.path(),
        &[
            "export",
            id.as_str(),
            "--format",
            "svg",
            "--out",
            "dist/out.svg",
        ],
    );
    assert_ne!(code(&output), 0);
    assert!(
        !dir.path().join("dist/out.svg").exists(),
        "no output for an invalid palette"
    );
}

#[test]
fn a_canvas_background_that_is_not_a_colour_is_reported_with_its_location() {
    let dir = project("validate-canvas-colour");
    let mut document = scene(vec![rect("r1", 0, 0.0, 0.0, 10.0, 10.0)]);
    document["canvas"]["background"] = json!("nope");
    let id = write_scene_as(&dir, "canvas-colour", document);

    let (exit, findings, _) = validate_json(&dir, &id);
    assert_eq!(exit, 1);
    let finding = findings
        .iter()
        .find(|finding| finding["code"] == "E_INVALID_COLOR")
        .expect("an invalid-colour finding");
    assert_eq!(json_path(finding), Some("/canvas/background"));
}

#[test]
fn an_export_background_that_is_not_a_colour_is_reported_and_nothing_is_written() {
    let dir = project("validate-export-colour");
    let document = scene(vec![rect("r1", 0, 0.0, 0.0, 10.0, 10.0)]);
    let id = write_scene_as(&dir, "export-colour", document);

    let output = run_vectr(
        dir.path(),
        &[
            "export",
            id.as_str(),
            "--format",
            "svg",
            "--background",
            "not-a-colour",
            "--out",
            "dist/out.svg",
        ],
    );
    assert_ne!(code(&output), 0);
    let err = stderr(&output);
    assert!(err.contains("E_INVALID_COLOR"), "{err}");
    assert!(err.contains("not-a-colour"), "{err}");
    assert!(
        !dir.path().join("dist/out.svg").exists(),
        "no output is written for an invalid export colour"
    );
}

#[test]
fn every_problem_in_a_scene_is_reported_not_only_the_first() {
    let dir = project("validate-multiple");
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "#ff0000"), ("unused", "#00ff00")]),
    );

    let mut first = rect("r1", 0, 0.0, 0.0, 10.0, 10.0);
    first["fill"] = token_paint("missing-one");
    let mut second = rect("r2", 1, 20.0, 0.0, 10.0, 10.0);
    second["fill"] = token_paint("missing-two");
    second["stroke"] = stroke("ghost", "accent");
    let document = scene_with(vec![first, second], None, Some("brand"));
    let id = write_scene_as(&dir, "multiple", document);

    let (exit, findings, _) = validate_json(&dir, &id);
    assert_eq!(exit, 1);
    let all = codes(&findings);

    // Both undefined tokens, the undefined stroke, and the unused-token warning
    // are present: no finding masks another.
    assert_eq!(
        all.iter()
            .filter(|code| **code == "E_UNDEFINED_TOKEN")
            .count(),
        2,
        "{findings:?}"
    );
    assert!(all.contains(&"E_UNDEFINED_STROKE"), "{findings:?}");
    assert!(all.contains(&"W_UNUSED_TOKEN"), "{findings:?}");
    let elements: Vec<&str> = findings
        .iter()
        .filter_map(|finding| finding["location"]["elementId"].as_str())
        .collect();
    assert!(
        elements.contains(&"r1") && elements.contains(&"r2"),
        "{elements:?}"
    );
}

#[test]
fn warnings_are_distinguished_from_errors() {
    let dir = project("validate-severity");
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "#ff0000"), ("unused", "#00ff00")]),
    );
    let mut element = rect("r1", 0, 0.0, 0.0, 10.0, 10.0);
    element["fill"] = token_paint("accent");
    let document = scene_with(vec![element], None, Some("brand"));
    let id = write_scene_as(&dir, "warn", document);

    // A scene with only a warning validates successfully and exits zero.
    let (exit, findings, _) = validate_json(&dir, &id);
    assert_eq!(exit, 0, "a warning does not fail validation");
    assert!(!findings.is_empty(), "the warning is reported");
    assert!(
        findings
            .iter()
            .all(|finding| finding["severity"] == "warning"),
        "{findings:?}"
    );
    assert!(codes(&findings).contains(&"W_UNUSED_TOKEN"));

    // An error is a distinct severity and a non-zero exit.
    let mut invalid = scene(vec![rect("r1", 0, 0.0, 0.0, 10.0, 10.0)]);
    invalid["elements"][0]["opacity"] = json!(2);
    let invalid_id = write_scene_as(&dir, "invalid", invalid);
    let (exit, findings, _) = validate_json(&dir, &invalid_id);
    assert_eq!(exit, 1);
    assert!(
        findings
            .iter()
            .any(|finding| finding["severity"] == "error"),
        "{findings:?}"
    );
}

#[test]
fn a_file_that_is_not_a_scene_is_a_clear_error_not_a_crash() {
    let dir = project("validate-not-a-scene");

    // A JSON array is not a scene document.
    dir.write("scenes/array.json", "[]");
    let (exit, findings, _) = validate_json(&dir, "array");
    assert_eq!(exit, 1);
    assert!(codes(&findings).contains(&"E_PARSE"), "{findings:?}");

    // A JSON object that is not a scene reports its unknown field.
    dir.write("scenes/object.json", r#"{"hello": 1}"#);
    let (exit, findings, _) = validate_json(&dir, "object");
    assert_eq!(exit, 1);
    assert!(codes(&findings).contains(&"E_SCHEMA"), "{findings:?}");

    // Neither path is a crash: the process exits with a code rather than a signal.
    let array = run_vectr(dir.path(), &["validate", "array"]);
    assert_ne!(code(&array), -1, "not killed by a signal");
    assert_ne!(code(&array), 0);
}

#[test]
fn validation_is_deterministic() {
    let dir = project("validate-determinism");
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "#ff0000")]),
    );
    let mut element = rect("r1", 0, 0.0, 0.0, 10.0, 10.0);
    element["fill"] = token_paint("missing");
    let document = scene_with(vec![element], None, Some("brand"));
    let id = write_scene_as(&dir, "determinism", document);

    let first = run_vectr(dir.path(), &["validate", "--json", id.as_str()]);
    let second = run_vectr(dir.path(), &["validate", "--json", id.as_str()]);
    assert_eq!(stdout(&first), stdout(&second), "NFR-010");
    assert_eq!(stderr(&first), stderr(&second));
}
