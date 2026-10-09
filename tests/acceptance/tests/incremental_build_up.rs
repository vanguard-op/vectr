//! Acceptance tests for the incremental composition build-up method (FEAT-029).
//!
//! FEAT-029 delivers a method, not a new engine surface: a complex graphic is
//! decomposed into named parts; each part is authored once as a reusable
//! definition ([FEAT-030]); the part is verified in isolation before it is
//! composed; verified parts are composed one at a time, with each increment
//! verified; and the whole is exported only once it passes. The method is
//! carried by the agent skill and authoring guide ([FEAT-020]), and it depends
//! on reusable definitions ([FEAT-030]) and part-scoped rendering ([FEAT-031]).
//!
//! These checks pin the deterministic half: the shipped skill and guide direct
//! the method rather than a single author-and-refine pass, and the shipped
//! `vectr` binary runs each step end to end — a definition is verified on its
//! own, composed, reused without being re-authored, and the whole compiles
//! completely and deterministically with no part dropped. The judged half — a
//! model following the method across providers — is measured by the evaluation
//! harness against the model-quality bar (NFR-030).

mod common;

use std::fs;

use common::*;
use serde_json::{json, Value};

/// `text` with every run of whitespace collapsed to one space, so a phrase that
/// wraps across lines reads as one string.
fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The shipped agent skill.
fn skill_text() -> String {
    fs::read_to_string(workspace_root().join("skills/vectr/SKILL.md")).expect("SKILL.md")
}

/// The shipped authoring guide — the skill's on-demand reference (FEAT-020).
fn guide_text() -> String {
    fs::read_to_string(workspace_root().join("skills/vectr/references/authoring-guide.md"))
        .expect("the authoring guide")
}

/// A minimal style-recipe document, as the project layout carries it (D-009).
///
/// A recipe's `name` selects one of the four shipped looks; `line-art` is used
/// because it is not the project default, so a test can tell the two apart.
fn recipe(id: &str, name: &str) -> String {
    json!({ "id": id, "projectId": "p", "name": name, "parameters": {} }).to_string()
}

/// An instance element placing `reference`, translated by `tx`.
fn instance(id: &str, order: i64, reference: &str, tx: f64) -> Value {
    json!({
        "id": id,
        "sceneId": SCENE_ID,
        "order": order,
        "kind": "instance",
        "geometry": {},
        "definitionRef": reference,
        "transform": {
            "translateX": tx, "translateY": 0.0, "rotate": 0.0, "scaleX": 1.0, "scaleY": 1.0
        },
        "opacity": 1.0,
        "visible": true
    })
}

/// A scene document holding `instances`, named `id`, selecting an optional
/// palette and recipe.
fn part_scene(
    id: &str,
    palette_id: Option<&str>,
    recipe_id: Option<&str>,
    instances: Vec<Value>,
) -> Value {
    let mut document = scene_with(instances, None, palette_id);
    document["id"] = json!(id);
    for element in document["elements"].as_array_mut().expect("elements") {
        element["sceneId"] = json!(id);
    }
    if let Some(recipe_id) = recipe_id {
        document["recipeId"] = json!(recipe_id);
    }
    document
}

/// A project with a `brand` palette, the flat recipe, and the default scene it
/// names, so a part can be authored, verified, and composed (D-009, D-032).
fn method_project(tag: &str, default_scene: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write(
        "vectr.project.json",
        &format!(
            r#"{{"defaultPaletteId":"brand","defaultRecipeId":"flat","defaultSceneId":"{default_scene}"}}"#
        ),
    );
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "#ff0000")]),
    );
    dir.write("recipes/flat.json", &recipe("flat", "flat"));
    dir
}

/// A definition whose single rect fills from the `accent` token.
fn accent_definition(id: &str, element: &str) -> Value {
    let mut body = def_rect(element, id, 0, 0.0, 0.0, 30.0, 40.0);
    body["fill"] = token_paint("accent");
    definition(id, json!([]), vec![body])
}

/// The render model a `compile --out` wrote.
fn compiled_model(path: &std::path::Path) -> Value {
    let text = fs::read_to_string(path).expect("the render model was written");
    serde_json::from_str(&text).expect("the render model is JSON")
}

/// The node ids a render model carries, in paint order.
fn node_ids(model: &Value) -> Vec<String> {
    model["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .filter_map(|node| node["id"].as_str().map(str::to_string))
        .collect()
}

// ---------------------------------------------------------------------------
// The method is directed, not a single author-and-refine pass
// ---------------------------------------------------------------------------

/// The skill directs a complex request to the build-up method rather than a
/// single pass: decompose into named parts, author each as a definition, verify
/// it alone, then compose the verified parts one at a time (FEAT-029, FEAT-020).
#[test]
fn the_skill_directs_a_complex_request_to_the_build_up_method() {
    let skill = flatten(&skill_text());

    assert!(
        skill.contains("built up in verified parts"),
        "the skill directs the build-up method: {skill}"
    );
    assert!(
        skill.contains("build-up method"),
        "the skill names the method: {skill}"
    );
    assert!(
        skill.contains("decompose the request into named parts"),
        "the skill directs decomposition into named parts: {skill}"
    );
    assert!(
        skill.contains("author each part as a reusable definition"),
        "the skill directs each part to be authored as a definition: {skill}"
    );
    assert!(
        skill.contains("verify it alone"),
        "the skill directs the part to be verified on its own: {skill}"
    );
    assert!(
        skill.contains("compose the verified parts"),
        "the skill directs composition of verified parts: {skill}"
    );
    // The method is the path for a complex request, while a simple mark stays a
    // single pass — the skill does not make one pass the only procedure.
    assert!(
        skill.contains("A simple mark is authored in one pass"),
        "the skill scopes the method to a complex request: {skill}"
    );
}

/// The guide carries the build-up method step by step, and names the part-scoped
/// verification the method depends on (FEAT-029, FEAT-031).
#[test]
fn the_guide_carries_the_build_up_method_step_by_step() {
    let guide = flatten(&guide_text());

    for step in [
        "Build a complex graphic up in verified parts",
        "Decompose.",
        "Author one part as a definition.",
        "Verify the part in isolation.",
        "Compose the verified part.",
        "Repeat for each part.",
        "Verify the whole and export.",
    ] {
        assert!(
            guide.contains(step),
            "the guide carries the step `{step}`: {guide}"
        );
    }

    // The verification the method names is part-scoped rendering, on both the
    // CLI and the MCP surface.
    assert!(
        guide.contains("vectr render <part>"),
        "the guide names CLI part rendering: {guide}"
    );
    assert!(
        guide.contains("render-part"),
        "the guide names the MCP part tool: {guide}"
    );
    // The method never composes an unverified part.
    assert!(
        guide.contains("never compose a part that has not been verified"),
        "the guide forbids composing an unverified part: {guide}"
    );
}

/// The guide documents a default decomposition, the dependency ordering, and
/// where a composition failure is located, so a request that does not decompose
/// cleanly still proceeds and a composition failure is not blamed on the parts
/// (FEAT-029 edge cases).
#[test]
fn the_guide_documents_the_default_decomposition_and_ordering_rules() {
    let guide = flatten(&guide_text());

    assert!(
        guide.contains("documented default decomposition"),
        "the guide documents a default decomposition: {guide}"
    );
    for layer in [
        "background and sky",
        "midground masses",
        "repeating or reused objects",
        "foreground detail",
    ] {
        assert!(
            guide.contains(layer),
            "the default decomposition names `{layer}`: {guide}"
        );
    }
    assert!(
        guide.contains("A request that does not decompose cleanly still gets this decomposition"),
        "a request that does not decompose cleanly does not stall: {guide}"
    );
    // A dependent part is authored and verified after the part it sits on, and
    // the order is recorded.
    assert!(
        guide.contains("after the part it sits on"),
        "the guide orders a dependent part: {guide}"
    );
    assert!(
        guide.contains("record that order"),
        "the guide records the order: {guide}"
    );
    // A composition failure names the composition step, not the parts.
    assert!(
        guide.contains("composition failure names the composition step, not the parts"),
        "a composition failure is located at the composition step: {guide}"
    );
    // The whole is complete only when no part is dropped.
    assert!(
        guide.contains("no part dropped"),
        "the whole is complete with no part dropped: {guide}"
    );
}

// ---------------------------------------------------------------------------
// The method's steps run through the shipped tools
// ---------------------------------------------------------------------------

/// A part is authored as a definition, verified on its own before any scene
/// places it, then composed; the composition carries the verified part at its
/// placement (FEAT-029, FEAT-030, FEAT-031).
#[test]
fn a_part_is_verified_in_isolation_before_it_is_composed() {
    let dir = method_project("build-up-verify", "forest");
    dir.write(
        "definitions/pine.json",
        &accent_definition("pine", "pine-body").to_string(),
    );

    // Verify the part on its own, before any scene places it.
    let isolated = run_vectr(
        dir.path(),
        &[
            "render",
            "pine",
            "--format",
            "svg",
            "--out",
            "dist/pine.svg",
        ],
    );
    assert_eq!(code(&isolated), 0, "{}", stderr(&isolated));
    let preview = fs::read_to_string(dir.path().join("dist/pine.svg")).expect("the part preview");
    assert!(
        preview.contains("<rect"),
        "the part's structure is drawn: {preview}"
    );
    assert!(
        preview.contains("#ff0000"),
        "the isolated part resolves the project default palette: {preview}"
    );
    assert!(
        stdout(&isolated).contains("frame"),
        "the frame used is reported: {}",
        stdout(&isolated)
    );

    // Compose the verified part into the scene, then verify the composition.
    dir.write(
        "scenes/forest.json",
        &part_scene("forest", None, None, vec![instance("i1", 0, "pine", 100.0)]).to_string(),
    );
    let validate = run_vectr(dir.path(), &["validate", "forest"]);
    assert_eq!(code(&validate), 0, "{}", stderr(&validate));

    let compile = run_vectr(
        dir.path(),
        &["compile", "forest", "--out", "forest.model.json"],
    );
    assert_eq!(code(&compile), 0, "{}", stderr(&compile));
    let model = compiled_model(&dir.path().join("forest.model.json"));
    let node = model["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|node| node["id"] == "pine-body~i1")
        .expect("the placed part's node");
    assert_eq!(
        node["transform"]["e"], 100.0,
        "the composition places the part where the instance says"
    );
}

/// A part that fails verification is corrected and re-verified before it is
/// composed, and composition does not proceed with an unverified part
/// (FEAT-029).
#[test]
fn a_part_that_fails_verification_is_corrected_before_it_is_composed() {
    let dir = method_project("build-up-correct", "forest");

    // A definition whose element references a parameter it does not declare is
    // not a verified part.
    let mut broken = accent_definition("pine", "pine-body");
    broken["elements"][0]["geometry"]["width"] = json!({ "param": "ghost" });
    dir.write("definitions/pine.json", &broken.to_string());

    let failed = run_vectr(
        dir.path(),
        &[
            "render",
            "pine",
            "--format",
            "svg",
            "--out",
            "dist/pine.svg",
        ],
    );
    assert_ne!(code(&failed), 0, "an unverified part is refused");
    assert!(
        stderr(&failed).contains("ghost"),
        "the failure names the offending reference: {}",
        stderr(&failed)
    );
    assert!(
        !dir.path().join("dist/pine.svg").exists(),
        "no preview is written for an unverified part"
    );

    // Correct the definition and re-verify it.
    dir.write(
        "definitions/pine.json",
        &accent_definition("pine", "pine-body").to_string(),
    );
    let corrected = run_vectr(
        dir.path(),
        &[
            "render",
            "pine",
            "--format",
            "svg",
            "--out",
            "dist/pine.svg",
        ],
    );
    assert_eq!(code(&corrected), 0, "{}", stderr(&corrected));
    assert!(
        dir.path().join("dist/pine.svg").is_file(),
        "the corrected part renders"
    );

    // Only the re-verified part is composed, and the composition validates.
    dir.write(
        "scenes/forest.json",
        &part_scene("forest", None, None, vec![instance("i1", 0, "pine", 0.0)]).to_string(),
    );
    let validate = run_vectr(dir.path(), &["validate", "forest"]);
    assert_eq!(code(&validate), 0, "{}", stderr(&validate));
}

/// A verified part is placed again in the same scene and in another scene
/// without being re-authored: the definition document is untouched and every
/// placement renders it (FEAT-029, FEAT-030).
#[test]
fn a_verified_part_is_reused_without_being_re_authoried() {
    let dir = method_project("build-up-reuse", "forest");
    dir.write(
        "definitions/pine.json",
        &accent_definition("pine", "pine-body").to_string(),
    );
    let authored =
        fs::read_to_string(dir.path().join("definitions/pine.json")).expect("the definition");

    // Two placements in one scene, and one in another scene, all the same part.
    dir.write(
        "scenes/forest.json",
        &part_scene(
            "forest",
            None,
            None,
            vec![
                instance("i1", 0, "pine", 0.0),
                instance("i2", 1, "pine", 50.0),
            ],
        )
        .to_string(),
    );
    dir.write(
        "scenes/grove.json",
        &part_scene("grove", None, None, vec![instance("i3", 0, "pine", 10.0)]).to_string(),
    );

    for scene_id in ["forest", "grove"] {
        let output = format!("{scene_id}.model.json");
        let compile = run_vectr(dir.path(), &["compile", scene_id, "--out", &output]);
        assert_eq!(code(&compile), 0, "{}", stderr(&compile));
    }

    let forest = node_ids(&compiled_model(&dir.path().join("forest.model.json")));
    assert!(forest.iter().any(|id| id == "pine-body~i1"), "{forest:?}");
    assert!(forest.iter().any(|id| id == "pine-body~i2"), "{forest:?}");
    let grove = node_ids(&compiled_model(&dir.path().join("grove.model.json")));
    assert!(grove.iter().any(|id| id == "pine-body~i3"), "{grove:?}");

    assert_eq!(
        fs::read_to_string(dir.path().join("definitions/pine.json")).expect("the definition"),
        authored,
        "reuse does not re-author the definition"
    );
}

/// A definition may place another definition, so parts compose into deeper
/// wholes; the composed whole compiles completely and deterministically with
/// no part dropped and no unresolved reference surviving (FEAT-029, FEAT-030).
#[test]
fn the_composed_whole_is_complete_and_deterministic_with_no_part_dropped() {
    let dir = method_project("build-up-whole", "forest");

    // A nested part: `frame` places `pine`, so parts compose into deeper wholes.
    dir.write(
        "definitions/pine.json",
        &accent_definition("pine", "pine-body").to_string(),
    );
    dir.write(
        "definitions/frame.json",
        &definition(
            "frame",
            json!([]),
            vec![def_instance("frame-inner", "frame", 0, "pine")],
        )
        .to_string(),
    );
    let mut stone = def_rect("stone-body", "stone", 0, 0.0, 0.0, 8.0, 8.0);
    stone["fill"] = token_paint("accent");
    dir.write(
        "definitions/stone.json",
        &definition("stone", json!([]), vec![stone]).to_string(),
    );

    dir.write(
        "scenes/forest.json",
        &part_scene(
            "forest",
            None,
            None,
            vec![
                instance("f1", 0, "frame", 0.0),
                instance("s1", 1, "stone", 40.0),
                instance("p2", 2, "pine", 80.0),
            ],
        )
        .to_string(),
    );

    let first = run_vectr(dir.path(), &["compile", "forest", "--out", "a.model.json"]);
    assert_eq!(code(&first), 0, "{}", stderr(&first));
    let second = run_vectr(dir.path(), &["compile", "forest", "--out", "b.model.json"]);
    assert_eq!(code(&second), 0, "{}", stderr(&second));

    let a = fs::read(dir.path().join("a.model.json")).expect("the first model");
    let b = fs::read(dir.path().join("b.model.json")).expect("the second model");
    assert_eq!(
        a, b,
        "the whole compiles byte-identically across runs (NFR-010, NFR-013)"
    );

    let model = compiled_model(&dir.path().join("a.model.json"));
    let ids = node_ids(&model);
    // Every part is present: the nested pine under the frame, the stone, the pine.
    assert!(
        ids.iter().any(|id| id == "pine-body~frame-inner~f1"),
        "{ids:?}"
    );
    assert!(ids.iter().any(|id| id == "stone-body~s1"), "{ids:?}");
    assert!(ids.iter().any(|id| id == "pine-body~p2"), "{ids:?}");
    assert_eq!(ids.len(), 3, "no part dropped: {ids:?}");

    // No unresolved reference survives into the render model (C-003).
    assert!(
        model["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .all(|node| node["kind"] != "instance"),
        "an instance contributes no node of its own: {ids:?}"
    );
    let text = String::from_utf8(a).expect("the model is UTF-8");
    assert!(
        !text.contains("definitionRef"),
        "no definition reference survives compilation"
    );
}

/// A definition carries no palette of its own: rendered in isolation it resolves
/// the project's default palette, and placed by a scene it resolves that scene's
/// palette (FEAT-029, FEAT-030, FEAT-031).
#[test]
fn a_definition_resolves_the_scene_palette_when_placed_and_the_project_default_when_isolated() {
    let dir = method_project("build-up-scene-palette", "forest");
    dir.write(
        "palettes/alt.json",
        &palette("alt", &[("accent", "#0000ff")]),
    );
    dir.write(
        "definitions/chip.json",
        &accent_definition("chip", "chip-body").to_string(),
    );

    // Isolated: the definition has no placing scene, so it resolves the
    // project's default palette.
    let isolated = run_vectr(
        dir.path(),
        &[
            "render",
            "chip",
            "--format",
            "svg",
            "--out",
            "dist/chip.svg",
        ],
    );
    assert_eq!(code(&isolated), 0, "{}", stderr(&isolated));
    let preview = fs::read_to_string(dir.path().join("dist/chip.svg")).expect("the preview");
    assert!(
        preview.contains("#ff0000"),
        "the isolated definition resolves the project default palette: {preview}"
    );
    assert!(
        !preview.contains("#0000ff"),
        "the isolated definition does not resolve a scene's palette: {preview}"
    );

    // Placed by a scene that selects a different palette, it resolves that
    // scene's palette.
    dir.write(
        "scenes/forest.json",
        &part_scene(
            "forest",
            Some("alt"),
            None,
            vec![instance("i1", 0, "chip", 0.0)],
        )
        .to_string(),
    );
    let compile = run_vectr(
        dir.path(),
        &["compile", "forest", "--out", "forest.model.json"],
    );
    assert_eq!(code(&compile), 0, "{}", stderr(&compile));
    let model = compiled_model(&dir.path().join("forest.model.json"));
    let node = model["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|node| node["id"] == "chip-body~i1")
        .expect("the placed part's node");
    assert_eq!(
        node["paint"]["fill"]["value"], "#0000ff",
        "the placing scene's palette applies to the part"
    );
}

/// A scene placing a definition renders under that scene's recipe (FEAT-029,
/// FEAT-030).
#[test]
fn a_scene_placing_a_definition_renders_under_the_scenes_recipe() {
    let dir = method_project("build-up-scene-recipe", "forest");
    dir.write("recipes/line-art.json", &recipe("line-art", "line-art"));
    dir.write(
        "definitions/chip.json",
        &accent_definition("chip", "chip-body").to_string(),
    );
    dir.write(
        "scenes/forest.json",
        &part_scene(
            "forest",
            None,
            Some("line-art"),
            vec![instance("i1", 0, "chip", 0.0)],
        )
        .to_string(),
    );

    let compile = run_vectr(
        dir.path(),
        &["compile", "forest", "--out", "forest.model.json"],
    );
    assert_eq!(code(&compile), 0, "{}", stderr(&compile));
    let model = compiled_model(&dir.path().join("forest.model.json"));
    assert_eq!(
        model["meta"]["recipe"], "line-art",
        "the placing scene's recipe applies to the definition it places"
    );
}
