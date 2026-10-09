//! Acceptance tests for deterministic procedural generation (FEAT-006, C-001,
//! C-004).
//!
//! A procedural element's generated geometry is governed by the scene's seed
//! and the generating element's stable identifier. Driven through the shipped
//! `vectr` binary as a subprocess (C-004), a seeded scene compiles to a
//! byte-identical render model on repeated runs (NFR-010), distinct seeds vary,
//! a scene that declares no seed records the default, an extreme seed stays
//! deterministic, and two instances of one definition each stay reproducible
//! while differing from one another (FEAT-006).

mod common;

use common::*;
use serde_json::{json, Value};

/// Compiles a project scene with the CLI and returns the raw render-model bytes.
///
/// Byte identity is the determinism evidence (NFR-010): the same scene and seed
/// must produce the same file, not merely an equal parse.
fn compiled_bytes(dir: &TempDir, scene: &str, out: &str) -> Vec<u8> {
    let output = run_vectr(dir.path(), &["compile", scene, "--out", out]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    std::fs::read(dir.path().join(out)).expect("the CLI wrote a render model")
}

/// Compiles a project scene with the CLI and returns the decoded render model.
fn compiled_model(dir: &TempDir, scene: &str, out: &str) -> Value {
    let bytes = compiled_bytes(dir, scene, out);
    serde_json::from_slice(&bytes).expect("the render model is JSON")
}

/// A project holding one scene with a stippling procedural element over a
/// rectangular region. `seed` is omitted from the scene when `None`.
fn procedural_project(tag: &str, seed: Option<i64>) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", r#"{"defaultSceneId":"s"}"#);

    let procedural = element(
        "p1",
        0,
        "procedural",
        json!({ "procedure": "stippling", "count": 12, "spacing": 3.0 }),
    );
    let mut region = rect("c1", 0, 60.0, 60.0, 80.0, 80.0);
    region["parentId"] = json!("p1");

    let mut document = scene_with(vec![procedural, region], None, None);
    document["canvas"] = json!({ "width": 200.0, "height": 200.0, "background": "#ffffff" });
    if let Some(seed) = seed {
        document["seed"] = json!(seed);
    }
    write_scene_as(&dir, "s", document);
    dir
}

#[test]
fn a_seeded_scene_compiles_identically_twice_through_the_cli() {
    let dir = procedural_project("procedural-seed", Some(7));

    let first = compiled_bytes(&dir, "s", "dist/a.json");
    let second = compiled_bytes(&dir, "s", "dist/b.json");
    assert_eq!(
        first, second,
        "identical input and seed yield byte-identical output (NFR-010)"
    );

    let model: Value = serde_json::from_slice(&first).expect("the render model is JSON");
    assert_eq!(model["meta"]["seed"], 7, "the declared seed is recorded");
    assert_eq!(
        model["nodes"].as_array().map(Vec::len),
        Some(12),
        "one node per generated stipple point"
    );
}

#[test]
fn two_seeds_generate_different_geometry_through_the_cli() {
    let first = compiled_model(
        &procedural_project("procedural-seed-a", Some(1)),
        "s",
        "dist/a.json",
    );
    let second = compiled_model(
        &procedural_project("procedural-seed-b", Some(2)),
        "s",
        "dist/b.json",
    );
    assert_ne!(first["nodes"], second["nodes"], "distinct seeds must vary");
}

#[test]
fn a_scene_without_a_seed_records_the_default_and_stays_reproducible() {
    let dir = procedural_project("procedural-default", None);

    let model = compiled_model(&dir, "s", "dist/a.json");
    assert_eq!(
        model["meta"]["seed"], 0,
        "a scene with no seed records the default"
    );

    let first = compiled_bytes(&dir, "s", "dist/a.json");
    let second = compiled_bytes(&dir, "s", "dist/b.json");
    assert_eq!(
        first, second,
        "a scene without a seed is still reproducible"
    );
}

#[test]
fn an_extreme_seed_remains_deterministic() {
    for seed in [i64::MIN, i64::MAX, -1] {
        let dir = procedural_project("procedural-extreme", Some(seed));
        let first = compiled_bytes(&dir, "s", "dist/a.json");
        let second = compiled_bytes(&dir, "s", "dist/b.json");
        assert_eq!(first, second, "seed {seed} must be reproducible");

        let model: Value = serde_json::from_slice(&first).expect("the render model is JSON");
        assert_eq!(model["meta"]["seed"], seed, "seed {seed} is recorded");
    }
}

/// The world offset of the generated dot belonging to one placed instance, read
/// from the render model's resolved transform.
fn instance_offset(model: &Value, instance: &str) -> (f64, f64) {
    let node = model["nodes"]
        .as_array()
        .expect("the model carries nodes")
        .iter()
        .find(|node| {
            node["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("dot") && id.contains(instance))
        })
        .unwrap_or_else(|| panic!("the dot placed by `{instance}`: {model}"));
    (
        node["transform"]["e"].as_f64().expect("a resolved x"),
        node["transform"]["f"].as_f64().expect("a resolved y"),
    )
}

#[test]
fn distinct_instances_of_one_definition_generate_differently_and_reproducibly() {
    let dir = TempDir::new("procedural-instances");
    dir.write("vectr.project.json", r#"{"defaultSceneId":"main"}"#);

    // A definition whose single rect is jittered by a procedural element, so
    // the instance's identifier (carried into the expanded element id) seeds the
    // generated displacement (FEAT-006, FEAT-030).
    let procedural = def_element(
        "p",
        "cloud",
        0,
        "procedural",
        json!({ "procedure": "jitter", "amount": 5.0 }),
    );
    let mut dot = def_rect("dot", "cloud", 1, 0.0, 0.0, 2.0, 2.0);
    dot["parentId"] = json!("p");
    dir.write(
        "definitions/cloud.json",
        &definition("cloud", json!([]), vec![procedural, dot]).to_string(),
    );

    let mut first = element("i1", 0, "instance", json!({}));
    first["definitionRef"] = json!("cloud");
    let mut second = element("i2", 1, "instance", json!({}));
    second["definitionRef"] = json!("cloud");
    let mut document = scene_with(vec![first, second], None, None);
    document["seed"] = json!(0);
    write_scene_as(&dir, "main", document);

    let first_run = compiled_model(&dir, "main", "dist/a.json");
    let again = compiled_model(&dir, "main", "dist/b.json");
    assert_eq!(
        first_run, again,
        "each instance's generated geometry stays reproducible (NFR-010)"
    );

    assert_ne!(
        instance_offset(&first_run, "i1"),
        instance_offset(&first_run, "i2"),
        "two instances of one definition must vary"
    );
}
