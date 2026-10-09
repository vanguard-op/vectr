//! Acceptance tests for the incremental composition build-up method (FEAT-029).
//!
//! FEAT-029 delivers one method for every graphic, not a new engine surface:
//! the whole is sketched at low fidelity first; then the work is divided into
//! sections — a group, an instance, or a scene — and each section is focused and
//! refined in turn, verified in isolation and integrated before the next; and the
//! scene is exported only when the whole passes. A reusable definition
//! ([FEAT-030]) is one kind of section, not the required unit, and a section is
//! verified by structural validation and by rendering it on its own
//! ([FEAT-031]). The method is carried by the agent skill and its on-demand
//! references ([FEAT-020]).
//!
//! These checks pin the deterministic half: the shipped skill and references
//! direct the one universal method rather than a single author-and-refine pass
//! and rather than a simple-versus-complex fork, and the shipped `vectr` binary
//! runs each step end to end — a section is verified on its own, composed, reused
//! without being re-authored, and the whole compiles completely and
//! deterministically with no part dropped. The judged half — a model following
//! the method across providers — is measured by the evaluation harness against
//! the model-quality bar (NFR-030).

mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use serde_json::{json, Value};

/// `text` with every run of whitespace collapsed to one space, so a phrase that
/// wraps across lines reads as one string.
fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The shipped skill directory.
fn skill_dir() -> PathBuf {
    workspace_root().join("skills/vectr")
}

/// The shipped agent skill's always-read entry point.
fn skill_text() -> String {
    fs::read_to_string(skill_dir().join("SKILL.md")).expect("SKILL.md")
}

/// Every on-demand reference, concatenated in file-name order (FEAT-020).
fn references_text() -> String {
    let mut paths: Vec<PathBuf> = fs::read_dir(skill_dir().join("references"))
        .expect("the references directory")
        .map(|entry| entry.expect("a reference entry").path())
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|path| fs::read_to_string(path).expect("a reference is readable"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// One on-demand reference by file name.
fn reference(name: &str) -> String {
    fs::read_to_string(skill_dir().join("references").join(name))
        .unwrap_or_else(|error| panic!("the reference `{name}` is readable: {error}"))
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

/// Two named sections, each a group holding a rect, optionally integrated with
/// two constraints that cannot both hold (FEAT-029 edge case).
fn section_scene(with_conflict: bool) -> Value {
    let mut document = scene_with(
        vec![
            group("left", 0, Some("Left")),
            {
                let mut body = rect("left-body", 0, 0.0, 0.0, 10.0, 10.0);
                body["parentId"] = json!("left");
                body
            },
            group("right", 1, Some("Right")),
            {
                let mut body = rect("right-body", 0, 0.0, 0.0, 10.0, 10.0);
                body["parentId"] = json!("right");
                body
            },
        ],
        None,
        None,
    );
    if with_conflict {
        document["constraints"] = json!([
            { "id": "c1", "sceneId": "forest", "kind": "align", "elementIds": ["left", "right"], "axis": "x", "value": 0.0 },
            { "id": "c2", "sceneId": "forest", "kind": "align", "elementIds": ["left", "right"], "axis": "x", "value": 50.0 }
        ]);
    }
    document
}

// ---------------------------------------------------------------------------
// The one universal method is directed, not a single pass or a complexity fork
// ---------------------------------------------------------------------------

/// The skill directs every request through the same sketch-then-section method:
/// the whole is sketched at low fidelity, then its sections — a group, an
/// instance, or a scene — are refined one at a time, verified in isolation and
/// integrated before the next; complexity changes the number of turns, not the
/// method (FEAT-029, FEAT-020).
#[test]
fn the_skill_directs_every_request_to_the_one_universal_method() {
    let skill = flatten(&skill_text());

    assert!(
        skill.contains("Every request runs the same method, whatever its complexity"),
        "the method is universal: {skill}"
    );
    assert!(
        skill.contains("sketch the whole at low fidelity"),
        "the skill directs sketching the whole at low fidelity: {skill}"
    );
    assert!(
        skill.contains("refine its sections one at a time"),
        "the skill directs refining sections one at a time: {skill}"
    );
    assert!(
        skill.contains("integrating and verifying each before the next"),
        "the skill directs integrating and verifying each section before the next: {skill}"
    );
    assert!(
        skill.contains("a group, an instance, or a scene"),
        "the skill names the three kinds of section: {skill}"
    );
    assert!(
        skill.contains("A reusable definition is one kind of section, not the required unit"),
        "a reusable definition is one section kind, not the required unit: {skill}"
    );
    assert!(
        skill.contains("Complexity changes the number of turns, never the method"),
        "complexity changes the number of turns, not the method: {skill}"
    );
    assert!(
        skill.contains(
            "deduced from the prompt, which describes the picture and prescribes no structure"
        ),
        "the sections, recipe, palette, and depth are deduced from the prompt: {skill}"
    );

    // The method does not fork a simple request from a complex one: there is no
    // simple-versus-complex branch and no parts-only build-up path.
    for fork in [
        "A simple mark is authored in one pass",
        "built up in verified parts",
        "build-up method",
    ] {
        assert!(
            !skill.contains(fork),
            "the method must not fork on complexity: `{fork}`"
        );
    }
}

/// The skill's workflow carries the method step by step, and the references
/// name the part-scoped verification the method depends on and forbid composing
/// an unverified part (FEAT-029, FEAT-031).
#[test]
fn the_skill_and_references_carry_the_method_step_by_step() {
    let skill = flatten(&skill_text());
    let references = flatten(&references_text());

    for step in [
        "Sketch the whole at low fidelity",
        "Take up one section, refine it, and validate it",
        "Render the section on its own and correct it until it matches",
        "Integrate the verified section, render the whole so far, and verify it before the next section",
        "export the final SVG and PNG",
    ] {
        assert!(
            skill.contains(step),
            "the workflow carries the step `{step}`: {skill}"
        );
    }
    assert!(
        skill.contains("Repeat 5–7 for each section"),
        "the loop repeats per section, recording the order: {skill}"
    );

    // The verification the method names is part-scoped rendering, on both the
    // CLI and the MCP surface.
    assert!(
        skill.contains("vectr render <part>"),
        "the skill names CLI part rendering: {skill}"
    );
    assert!(
        skill.contains("render-part"),
        "the skill names the MCP part tool: {skill}"
    );
    // The method never composes an unverified part.
    assert!(
        references.contains("an unverified part is never composed"),
        "the references forbid composing an unverified part: {references}"
    );
    // A reusable part is verified on its own before it is placed.
    assert!(
        references.contains("Verify a definition on its own before it is placed"),
        "a reusable part is verified before it is placed: {references}"
    );
}

/// The references document a default decomposition, the dependency ordering,
/// and where a composition failure is located, so a request that does not
/// decompose cleanly still proceeds (FEAT-029 edge cases).
#[test]
fn the_references_document_the_default_decomposition_and_dependency_order() {
    let skill = flatten(&skill_text());
    let defaults = flatten(&reference("defaults.md"));

    assert!(
        defaults.contains("documented default decomposition"),
        "the reference documents a default decomposition: {defaults}"
    );
    for layer in [
        "background and sky",
        "midground masses",
        "repeating or reused objects",
        "foreground detail",
    ] {
        assert!(
            defaults.contains(layer),
            "the default decomposition names `{layer}`: {defaults}"
        );
    }
    assert!(
        defaults.contains("rather than a stall"),
        "a request that does not decompose cleanly does not stall: {defaults}"
    );

    // A dependent section is refined and verified after the part it sits on, and
    // the whole is verified after each integration before the next section.
    assert!(
        skill.contains("render the whole so far, and verify it before the next section"),
        "the whole is verified after each integration: {skill}"
    );
    // The whole is complete only when every section is integrated and verified.
    assert!(
        skill.contains("nothing is dropped"),
        "the whole is complete with no section dropped: {skill}"
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

/// The sections pass but the whole fails: the failure names the integration
/// step — the constraint that cannot hold — not a section (FEAT-029 edge case).
#[test]
fn a_composition_failure_names_the_integration_step_not_the_parts() {
    let dir = method_project("build-up-integration", "forest");

    // The sections verify in isolation: each named subtree renders on its own.
    write_scene_as(&dir, "forest", section_scene(false));
    for part in ["left", "right"] {
        let out = format!("dist/{part}.svg");
        let render = run_vectr(
            dir.path(),
            &["render", part, "--format", "svg", "--out", &out],
        );
        assert_eq!(
            code(&render),
            0,
            "the section `{part}` verifies on its own: {}",
            stderr(&render)
        );
    }

    // Integrating them with two constraints that cannot both hold fails the
    // whole, naming the conflicting constraints rather than a section.
    write_scene_as(&dir, "forest", section_scene(true));
    let compile = run_vectr(dir.path(), &["compile", "forest", "--check"]);
    assert_eq!(
        code(&compile),
        3,
        "a whole-scene conflict is a compilation failure: {}",
        stderr(&compile)
    );
    let message = stderr(&compile);
    assert!(
        message.contains("c1") && message.contains("c2"),
        "the failure names the integration step (the conflicting constraints): {message}"
    );
    assert!(
        !message.contains("left-body") && !message.contains("right-body"),
        "the failure is not blamed on a section: {message}"
    );
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
