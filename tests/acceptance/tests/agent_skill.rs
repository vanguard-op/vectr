//! Acceptance tests for the agent skill and authoring guide (FEAT-020, C-004).
//!
//! The skill's promise is that a model with only the shipped artifacts — the
//! always-read entry point, its on-demand references, its worked examples, and
//! the published schema — can author a scene that validates, compiles, and
//! renders. The judged, cross-model half of that promise is measured by the
//! evaluation harness; these checks pin the deterministic half: the package is
//! well formed, it targets the installed tool, the scaffolded guide stays
//! minimal and carries no procedure, and every example the skill ships under
//! `examples/` actually runs through the real toolchain.
//!
//! The authoring procedure is dissolved into the on-demand references, so there
//! is no monolithic guide: the entry point carries the workflow and the
//! references carry the depth for each step (FEAT-020, D-046, D-047).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use serde_json::Value;

/// The shipped skill directory.
fn skill_dir() -> PathBuf {
    workspace_root().join("skills/vectr")
}

/// The shipped skill entry point (`SKILL.md`).
fn skill_text() -> String {
    fs::read_to_string(skill_dir().join("SKILL.md")).expect("SKILL.md")
}

/// One on-demand reference by file name.
fn reference(name: &str) -> String {
    fs::read_to_string(skill_dir().join("references").join(name))
        .unwrap_or_else(|error| panic!("the reference `{name}` is readable: {error}"))
}

/// Every on-demand reference, concatenated in file-name order.
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

/// Every worked example, as (file name, the one JSON document it carries).
///
/// Each example is a markdown document under `examples/` carrying exactly one
/// fenced JSON scene or definition, kept apart from `assets/` so it is never
/// mistaken for a copyable asset (FEAT-020, D-047).
fn example_documents() -> Vec<(String, Value)> {
    let dir = skill_dir().join("examples");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("the examples directory")
        .map(|entry| entry.expect("an example entry").path())
        .collect();
    paths.sort();
    let mut documents = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .expect("an example file name")
            .to_string_lossy()
            .into_owned();
        let text = fs::read_to_string(&path).expect("an example is readable");
        let mut blocks = fenced_json_blocks(&text);
        assert_eq!(
            blocks.len(),
            1,
            "the example `{name}` carries exactly one JSON document"
        );
        documents.push((name, blocks.remove(0)));
    }
    documents
}

/// Whether a JSON document is a scene: it carries a canvas and elements.
fn is_scene(document: &Value) -> bool {
    document.get("canvas").is_some() && document.get("elements").and_then(Value::as_array).is_some()
}

/// Whether a JSON document is a reusable definition: parameters, an origin, and
/// elements, with no canvas (FEAT-030).
fn is_definition(document: &Value) -> bool {
    document.get("canvas").is_none()
        && document.get("parameters").is_some()
        && document.get("origin").is_some()
        && document.get("elements").and_then(Value::as_array).is_some()
}

/// The worked scenes the skill ships, as (file name, document).
fn example_scenes() -> Vec<(String, Value)> {
    example_documents()
        .into_iter()
        .filter(|(_, document)| is_scene(document))
        .collect()
}

/// The reusable definitions the skill ships, as (file name, document).
fn example_definitions() -> Vec<(String, Value)> {
    example_documents()
        .into_iter()
        .filter(|(_, document)| is_definition(document))
        .collect()
}

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
            block.get("cap").is_some()
                && block.get("join").is_some()
                && block.get("width").is_some()
        })
        .expect("a stroke profile example")
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

/// Writes a project with a scene, palette, stroke, and any reusable definitions
/// the scene places, then runs the full author-validate-compile-render loop the
/// skill teaches.
fn run_the_worked_example(
    tag: &str,
    palette: &Value,
    stroke: &Value,
    definitions: &[Value],
    scene: &Value,
) {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", "{}");
    dir.write("palettes/brand.json", &palette.to_string());
    dir.write("strokes/hairline.json", &stroke.to_string());
    for definition in definitions {
        let id = definition["id"].as_str().expect("a definition identifier");
        dir.write(&format!("definitions/{id}.json"), &definition.to_string());
    }
    let scene_id = write_scene(&dir, scene);

    let validate = run_vectr(dir.path(), &["validate", scene_id.as_str()]);
    assert_eq!(
        code(&validate),
        0,
        "the example must validate:\n{}",
        stderr(&validate)
    );

    let compile = run_vectr(dir.path(), &["compile", scene_id.as_str(), "--check"]);
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
            scene_id.as_str(),
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
            scene_id.as_str(),
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

/// Whether `version` is a semver string: `MAJOR.MINOR.PATCH`, optionally with a
/// `-prerelease` and/or `+build` suffix.
///
/// The skill tracks the release it was written for, and the release channel is
/// a pre-release (the shipped `0.1.0-pre.N`) until the first stable 0.1 ships
/// (release.md, "Rollout Phases & Feature Flags"), so a pre-release is a valid
/// skill version and a naive dot-count must not reject it (FEAT-020).
fn is_semver(version: &str) -> bool {
    let (without_build, build) = match version.split_once('+') {
        Some((head, build)) => (head, Some(build)),
        None => (version, None),
    };
    let (core, pre) = match without_build.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (without_build, None),
    };

    let numbers: Vec<&str> = core.split('.').collect();
    let numeric = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    let identifier = |part: &str| {
        !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };

    numbers.len() == 3
        && numbers.iter().all(|part| numeric(part))
        && pre.is_none_or(|pre| pre.split('.').all(identifier))
        && build.is_none_or(|build| build.split('.').all(identifier))
}

#[test]
fn the_skill_package_is_well_formed_and_targets_the_installed_tool() {
    let dir_skill = skill_dir();
    let skill = skill_text();

    let fields = frontmatter(&skill);
    assert_eq!(field(&fields, "name"), Some("vectr"));
    let description = field(&fields, "description").expect("a description");
    assert!(
        description.len() > 40,
        "the description tells the model when to load it"
    );
    let version = field(&fields, "version").expect("a version");
    assert!(
        is_semver(version),
        "`{version}` is semver, pre-release allowed"
    );

    // The packaged files the skill promises are present: the entry point, the
    // on-demand references, the worked examples, and the scene template.
    assert!(dir_skill.join("assets/scene.template.json").is_file());
    let references = fs::read_dir(dir_skill.join("references"))
        .expect("the references directory")
        .count();
    assert!(references > 0, "the skill ships on-demand references");
    let examples = fs::read_dir(dir_skill.join("examples"))
        .expect("the examples directory")
        .count();
    assert!(examples > 0, "the skill ships worked examples");

    // The procedure is dissolved into the references, not a monolithic guide
    // (FEAT-020, D-046).
    assert!(
        !dir_skill.join("references/authoring-guide.md").exists(),
        "the skill must not ship a monolithic authoring guide"
    );

    // The entry point routes to both the references and the examples directory.
    assert!(skill.contains("references/"), "{skill}");
    assert!(skill.contains("examples/"), "{skill}");

    // The skill targets the installed build and the served contract; if these
    // diverge the skill's own instructions are to report the mismatch.
    let dir = TempDir::new("skill-version");
    assert_eq!(version, cli_version(&dir));
    assert_eq!(declared_format_version(&dir), VERSION);
    assert!(
        skill.contains("Version compatibility") && skill.contains("report the mismatch"),
        "the skill instructs a version-mismatch report"
    );
    assert!(
        skill.contains(VERSION),
        "the skill names the format version"
    );
}

/// `text` with every run of whitespace collapsed to one space.
///
/// A phrase in a prose document wraps across lines, so a phrase check reads the
/// flattened text rather than the wrapped source.
fn flatten(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn a_skill_version_mismatch_is_reported_against_the_installed_tool() {
    // FEAT-020's edge case: a skill version that does not match the installed
    // tool is reported. The skill tells the agent to compare the installed
    // tool's version before authoring and, on a difference, to report the
    // mismatch naming both versions rather than author against a tool the skill
    // was not written for.
    let skill = skill_text();
    let references = references_text();

    // A pre-release channel version is a valid skill version, so the package is
    // accepted on the pre-release channel (release.md, "Rollout Phases &
    // Feature Flags"); a version that is not semver is not. The shipped skill's
    // own version is the illustration, so the example tracks the channel the
    // skill ships on rather than a stale release.
    let fields = frontmatter(&skill);
    let version = field(&fields, "version").expect("a version");
    assert!(
        is_semver(version),
        "the shipped pre-release skill version `{version}` is accepted"
    );
    assert!(
        is_semver("1.2.3-alpha.1+build.5"),
        "a pre-release with build metadata is accepted"
    );
    assert!(!is_semver("0.1"), "major.minor alone is not semver");
    assert!(!is_semver("0.1.0.1"), "four numeric parts is not semver");

    // The comparison the agent is told to run: the installed tool reports the
    // version the skill was written for, so a difference is the mismatch to
    // report and both versions are nameable.
    let dir = TempDir::new("skill-mismatch");
    assert_eq!(
        version,
        cli_version(&dir),
        "the skill targets the installed tool's version"
    );

    // The skill names the two version surfaces an agent compares: `vectr
    // --version` and the MCP `serverInfo.version`, and directs the mismatch
    // report naming both versions.
    let skill = flatten(&skill);
    assert!(
        skill.contains("vectr --version") && skill.contains("serverInfo.version"),
        "the skill names the surfaces to compare: {skill}"
    );
    assert!(
        skill.contains("report the mismatch") && skill.contains("name both versions"),
        "the skill directs the mismatch report naming both versions: {skill}"
    );
    assert!(
        skill.contains(version),
        "the skill names its own version: {skill}"
    );

    // The on-demand references carry the format-version statement the skill
    // compares through `vectr schema`.
    let references = flatten(&references);
    assert!(
        references.contains(VERSION),
        "the references name the format version: {references}"
    );
}

#[test]
fn the_shipped_scene_template_validates_and_compiles() {
    let template = fs::read_to_string(skill_dir().join("assets/scene.template.json"))
        .expect("the scene template");
    let template: Value = serde_json::from_str(&template).expect("the template is JSON");

    let dir = TempDir::new("skill-template");
    dir.write("vectr.project.json", "{}");
    let scene_id = write_scene(&dir, &template);

    let validate = run_vectr(dir.path(), &["validate", scene_id.as_str()]);
    assert_eq!(code(&validate), 0, "{}", stderr(&validate));
    let compile = run_vectr(dir.path(), &["compile", scene_id.as_str(), "--check"]);
    assert_eq!(code(&compile), 0, "{}", stderr(&compile));
}

#[test]
fn the_references_teach_authoring_from_the_schema_alone() {
    // FEAT-020: with only the authoring guide and the published schema, a model
    // produces a valid scene for a basic request. The references direct reading
    // the contract rather than authoring from memory, and ship a starter scene
    // skeleton that is itself a valid scene document.
    let rules = reference("rules.md");
    let flat = flatten(&rules);
    assert!(
        flat.contains("Do not author from memory"),
        "the reference directs reading the contract: {flat}"
    );
    assert!(
        flat.contains("vectr schema"),
        "the reference names the schema command: {flat}"
    );
    assert!(
        flat.contains("scenes/<id>.json"),
        "the reference states how a scene is addressed: {flat}"
    );

    let blocks = fenced_json_blocks(&rules);
    let skeleton = blocks
        .iter()
        .find(|block| is_scene(block))
        .expect("the starter scene skeleton");
    let scene = vectr_core::parse(&skeleton.to_string()).expect("the skeleton parses");
    assert!(
        !vectr_core::validate(&scene).has_errors(),
        "the starter skeleton is a valid scene"
    );
}

#[test]
fn the_worked_examples_validate_compile_and_export() {
    // FEAT-020 requires the worked examples to span the complexity range — a
    // simple mark and a compositionally complex illustration — plus the
    // reusable-parts example the method turns on. Every worked scene the skill
    // ships must validate, compile, and export through the real toolchain, not
    // only the largest one.
    let rules = reference("rules.md");
    let blocks = fenced_json_blocks(&rules);
    let palette = palette_block(&blocks);
    let stroke = stroke_block(&blocks);

    let scenes = example_scenes();
    assert!(
        scenes.len() >= 2,
        "the examples ship more than one worked scene: {}",
        scenes.len()
    );
    let definitions: Vec<Value> = example_definitions()
        .into_iter()
        .map(|(_, document)| document)
        .collect();
    assert!(
        !definitions.is_empty(),
        "the examples ship a reusable definition"
    );

    for (name, scene) in &scenes {
        run_the_worked_example(
            &format!("skill-example-{name}"),
            palette,
            stroke,
            &definitions,
            scene,
        );
    }
}

#[test]
fn the_examples_reusable_definitions_render_on_their_own() {
    // The method verifies a reusable part in isolation before it is composed
    // (FEAT-029, FEAT-031), and the examples ship worked definitions to show
    // it. Each definition the examples ship must render on its own through the
    // part-scoped command, resolving the project's default palette.
    let rules = reference("rules.md");
    let blocks = fenced_json_blocks(&rules);
    let palette = palette_block(&blocks);
    let stroke = stroke_block(&blocks);

    let scenes = example_scenes();
    let definitions = example_definitions();
    assert!(
        !definitions.is_empty(),
        "the examples ship a reusable definition"
    );
    let default = &scenes
        .first()
        .expect("a worked scene to name the default")
        .1;

    for (_, definition) in &definitions {
        let id = definition["id"].as_str().expect("a definition identifier");
        let dir = TempDir::new("skill-definition");
        dir.write(
            "vectr.project.json",
            &format!(
                r#"{{"defaultSceneId":"{}","defaultPaletteId":"brand"}}"#,
                default["id"].as_str().expect("a default scene identifier")
            ),
        );
        dir.write("palettes/brand.json", &palette.to_string());
        dir.write("strokes/hairline.json", &stroke.to_string());
        write_scene(&dir, default);
        // Every definition the examples ship, so the default scene resolves.
        for (_, other) in &definitions {
            let other_id = other["id"].as_str().expect("a definition identifier");
            dir.write(&format!("definitions/{other_id}.json"), &other.to_string());
        }

        let out = dir.path().join(format!("dist/{id}.svg"));
        let render = run_vectr(
            dir.path(),
            &[
                "render",
                id,
                "--format",
                "svg",
                "--out",
                export_svg_arg(&out),
            ],
        );
        assert_eq!(
            code(&render),
            0,
            "the definition `{id}` renders on its own:\n{}",
            stderr(&render)
        );
        let svg = fs::read_to_string(&out).expect("the part preview was written");
        assert!(
            svg.contains("<svg") && svg.contains("</svg>"),
            "the part preview is an SVG: {svg}"
        );
    }
}

/// The scaffolded project's `vectr.project.json`, read as JSON.
fn project_config(project: &Path) -> Value {
    let text = fs::read_to_string(project.join("vectr.project.json")).expect("the project config");
    serde_json::from_str(&text).expect("the project config is JSON")
}

#[test]
fn the_scaffolded_guide_is_minimal_and_states_the_projects_local_facts() {
    // FEAT-020's packaging amendment: the scaffold writes a small always-read
    // guide (`AGENTS.md`) that orients a coding agent and states the project's
    // local facts — its identifier, default scene, default palette, and default
    // recipe, and the scene-addressing rule — then points at the skill's
    // on-demand references. The full procedure is not embedded.
    let dir = TempDir::new("scaffold-guide");
    let init = run_vectr(dir.path(), &["init", "habit"]);
    assert_eq!(code(&init), 0, "{}", stderr(&init));
    let project = dir.path().join("habit");
    let guide = fs::read_to_string(project.join("AGENTS.md")).expect("the scaffolded guide");
    let flat = flatten(&guide);

    // Orientation: what a scene is and what the toolchain does with it.
    assert!(
        flat.contains("Vectr scenes"),
        "the guide orients the agent: {flat}"
    );
    assert!(
        flat.contains("published schema"),
        "the guide points at the contract: {flat}"
    );

    // The project's local facts are the values `vectr init` actually wrote, so
    // the guide cannot drift from the project it orients.
    let config = project_config(&project);
    for (field, label) in [
        ("id", "identifier"),
        ("defaultSceneId", "default scene"),
        ("defaultPaletteId", "default palette"),
        ("defaultRecipeId", "default recipe"),
    ] {
        let value = config[field]
            .as_str()
            .unwrap_or_else(|| panic!("the scaffolded project names a {label}"));
        assert!(
            flat.contains(&format!("`{value}`")),
            "the guide names the project's {label} `{value}`: {flat}"
        );
    }

    // The scene-addressing rule: a command names a scene by its identifier and
    // its document is `scenes/<id>.json` (FEAT-016, D-032).
    assert!(
        flat.contains("scenes/<id>.json"),
        "the guide states the scene-addressing rule: {flat}"
    );

    // The tool surface, and pointers to the schema, the skill's on-demand
    // references, and the examples directory.
    assert!(
        flat.contains("vectr validate"),
        "the guide names `vectr validate`: {flat}"
    );
    assert!(
        flat.contains("vectr schema"),
        "the guide points at the schema: {flat}"
    );
    assert!(
        flat.contains("on-demand references"),
        "the guide points at the skill's on-demand references: {flat}"
    );
    assert!(
        flat.contains("references/rules.md"),
        "the guide names the skill reference to start from: {flat}"
    );
    assert!(
        flat.contains("examples/"),
        "the guide points at the skill's worked examples: {flat}"
    );

    // The guide names the versions it targets, so a mismatch is caught before
    // authoring (FEAT-020).
    let version = cli_version(&dir);
    assert!(
        flat.contains(&version),
        "the guide names the tool version `{version}`: {flat}"
    );
    assert!(
        flat.contains(VERSION),
        "the guide names the format version: {flat}"
    );
}

#[test]
fn the_scaffolded_guide_carries_no_procedure_worked_examples_or_asset_summary() {
    // The guide is always-read material kept across the authoring loop, so it
    // stays minimal: the procedure, the worked examples, the error catalogue,
    // and any generated summary of the project's style assets live in the
    // skill's on-demand references and `examples/`, discovered from the
    // project's documents through the tools (FEAT-020, D-042, D-043).
    let dir = TempDir::new("scaffold-guide-minimal");
    let init = run_vectr(dir.path(), &["init", "habit"]);
    assert_eq!(code(&init), 0, "{}", stderr(&init));
    let project = dir.path().join("habit");
    let guide = fs::read_to_string(project.join("AGENTS.md")).expect("the scaffolded guide");
    let flat = flatten(&guide);

    // No worked example: the guide embeds no fenced JSON scene or definition.
    assert!(
        fenced_json_blocks(&guide).is_empty(),
        "the minimal guide embeds no worked example: {guide}"
    );

    // No embedded procedure: the steps, the bounded retry, the ambiguous-request
    // defaults, and the error catalogue are not carried here.
    for forbidden in [
        "Inspect and correct",
        "Retry once",
        "Defaults for an ambiguous request",
        "Licensing",
        "E_SCHEMA",
        "E_PARSE",
        "E_FORMAT_VERSION",
        "E_INVALID_COLOR",
    ] {
        assert!(
            !flat.contains(forbidden),
            "the minimal guide must not carry `{forbidden}`: {flat}"
        );
    }

    // No generated summary of the project's style assets: the scaffold's own
    // palette tokens are not written into the always-read guide; they are read
    // from the project's documents through the tools.
    let palette = fs::read_to_string(project.join("palettes/brand.json")).expect("the palette");
    let palette: Value = serde_json::from_str(&palette).expect("the palette is JSON");
    for token in palette["tokens"].as_array().expect("the palette tokens") {
        let name = token["name"].as_str().expect("a token name");
        assert!(
            !flat.contains(name),
            "the guide carries no summary of the palette token `{name}`: {flat}"
        );
    }
    assert!(
        flat.contains("through the tools"),
        "the guide directs the agent to discover assets through the tools: {flat}"
    );
}

#[test]
fn a_scaffolded_project_without_the_skill_orients_from_the_schema() {
    // FEAT-020's edge case: a project with no skill available still works. The
    // minimal agent guide orients the agent and points at the published schema,
    // the procedure is absent rather than embedded, and the guide states no
    // requirement that the skill be present.
    let dir = TempDir::new("scaffold-guide-no-skill");
    let init = run_vectr(dir.path(), &["init", "habit"]);
    assert_eq!(code(&init), 0, "{}", stderr(&init));
    let project = dir.path().join("habit");
    let guide = fs::read_to_string(project.join("AGENTS.md")).expect("the scaffolded guide");
    let flat = flatten(&guide);

    // The guide points at the schema as the contract to author against and
    // states the skill is not required.
    assert!(
        flat.contains("vectr schema"),
        "points at the schema: {flat}"
    );
    assert!(
        flat.contains("the skill is not required"),
        "states the skill is not required: {flat}"
    );

    // The procedure is absent, not condensed: no worked example and none of the
    // procedure's own step names.
    assert!(
        fenced_json_blocks(&guide).is_empty(),
        "no embedded worked example: {guide}"
    );
    for step in [
        "Inspect and correct",
        "Retry once",
        "Defaults for an ambiguous request",
    ] {
        assert!(
            !flat.contains(step),
            "the procedure is absent, not condensed: `{step}`"
        );
    }
}

#[test]
fn the_always_read_material_stays_within_its_documented_budget() {
    // FEAT-020: the always-read material — the skill's entry point and the
    // project's scaffolded agent guide — stays within the size budget nfr.md
    // states (NFR-008, NFR-009), so no depth is loaded before the step that
    // needs it. The crate constants encode those figures; pin them here so the
    // bar cannot be loosened in the build to make a run pass.
    assert_eq!(vectr_project::AGENT_GUIDE_BUDGET_BYTES, 3_072);
    assert_eq!(vectr_project::SKILL_ENTRY_BUDGET_BYTES, 8_192);

    let dir = TempDir::new("scaffold-guide-budget");
    let init = run_vectr(dir.path(), &["init", "habit"]);
    assert_eq!(code(&init), 0, "{}", stderr(&init));
    let guide = fs::read_to_string(dir.path().join("habit/AGENTS.md")).expect("the guide");
    assert!(
        guide.len() <= vectr_project::AGENT_GUIDE_BUDGET_BYTES,
        "the scaffolded agent guide is {} bytes, over the documented {}",
        guide.len(),
        vectr_project::AGENT_GUIDE_BUDGET_BYTES
    );

    let skill = skill_text();
    assert!(
        skill.len() <= vectr_project::SKILL_ENTRY_BUDGET_BYTES,
        "the skill entry point is {} bytes, over the documented {}",
        skill.len(),
        vectr_project::SKILL_ENTRY_BUDGET_BYTES
    );
}

#[test]
fn the_references_direct_recovery_from_an_invalid_scene() {
    // FEAT-020's edge case: a model that produces an invalid scene is directed
    // to validate and retry. The reference the entry point names for a finding
    // carries the bounded retry and the diagnostics it points a model at.
    let skill = flatten(&skill_text());
    let failures = flatten(&reference("failures.md"));

    assert!(
        skill.contains("Retry authoring once"),
        "the skill bounds the retry"
    );
    assert!(
        failures.contains("Retry once"),
        "the reference bounds the retry"
    );
    assert!(
        failures.contains("Never export"),
        "no export from a failed scene"
    );
    for code in ["E_SCHEMA", "E_PARSE", "E_FORMAT_VERSION", "E_INVALID_COLOR"] {
        assert!(failures.contains(code), "the reference names `{code}`");
    }

    // The diagnostic the reference points at is one the tool actually reports,
    // with the location it promises.
    let dir = TempDir::new("guide-diagnostics");
    dir.write("vectr.project.json", "{}");
    let mut document = scene(vec![rect("r1", 0, 0.0, 0.0, 10.0, 10.0)]);
    document["elements"][0]["opacity"] = serde_json::json!(2);
    let scene_id = write_scene_as(&dir, "scene", document);

    let output = run_vectr(dir.path(), &["validate", "--json", scene_id.as_str()]);
    assert_eq!(code(&output), 1);
    let findings: Value = serde_json::from_str(stdout(&output).trim()).expect("JSON findings");
    let finding = findings
        .as_array()
        .and_then(|findings| findings.iter().find(|f| f["code"] == "E_SCHEMA"))
        .expect("an E_SCHEMA finding");
    assert!(
        finding["location"].is_object(),
        "the diagnostic carries the location the reference describes: {finding}"
    );
}

#[test]
fn the_references_direct_a_detailed_request_to_be_composed_in_full() {
    // FEAT-020's edge case: a request for a very complex or detailed
    // illustration is answered by composing the required elements, never by a
    // refusal or a simplification to a simple mark.
    let skill = flatten(&skill_text());
    let defaults = flatten(&reference("defaults.md"));

    assert!(
        skill.contains("never simplified to a simple mark"),
        "the skill forbids simplifying a detailed request"
    );
    assert!(
        defaults.contains("authored in full"),
        "the reference directs a detailed request to be authored in full"
    );
    assert!(
        defaults.contains("never simplified to a simple mark"),
        "the reference forbids reducing a detailed request"
    );
}

#[test]
fn the_references_direct_a_documented_default_for_an_ambiguous_request() {
    // FEAT-020's edge case: an ambiguous request is answered by a documented
    // default rather than a stall. The defaults reference carries the choices
    // and directs stating one and proceeding.
    let defaults = flatten(&reference("defaults.md"));

    assert!(
        defaults.contains("Choose the documented default"),
        "the reference directs choosing a documented default: {defaults}"
    );
    assert!(
        defaults.contains("rather than stopping to ask"),
        "the reference proceeds rather than stalling: {defaults}"
    );
    for (label, value) in [
        ("canvas size", "512×512"),
        ("background", "transparent"),
        ("recipe", "defaultRecipeId"),
    ] {
        assert!(
            defaults.contains(value),
            "the default {label} is `{value}`: {defaults}"
        );
    }
}

/// The skill's frontmatter description as a lowercased string.
fn skill_description() -> String {
    frontmatter(&skill_text())
        .into_iter()
        .find(|(key, _)| key == "description")
        .map(|(_, value)| value.to_lowercase())
        .expect("a description")
}

#[test]
fn the_skill_description_names_the_full_complexity_range_and_every_task() {
    // The description is the surface an agent matches against, so it must name
    // both the graphic types and the tasks across the full range, with no
    // "simple" ceiling that would stop an agent loading it for a complex
    // request (FEAT-020).
    let description = skill_description();

    for graphic in [
        "logos",
        "wordmarks",
        "icons and icon sets",
        "badges",
        "diagrams",
        "illustrations",
    ] {
        assert!(
            description.contains(graphic),
            "the description names `{graphic}`: {description}"
        );
    }
    assert!(
        description.contains("simple") && description.contains("very complex"),
        "the description spans simple to very complex, with no simple ceiling: {description}"
    );

    for task in [
        "authoring",
        "validating",
        "rendering and exporting",
        "editing",
        "restyling through palette tokens",
        "debugging",
        "wire the vectr tools into an agent",
    ] {
        assert!(
            description.contains(task),
            "the description names the `{task}` task: {description}"
        );
    }
}

#[test]
fn the_complex_example_is_compositional_and_validates() {
    // FEAT-020 requires a shipped worked example that is compositionally
    // complex and validates, compiles, and renders. The end-to-end run is
    // exercised elsewhere; here the example's structure is pinned: the
    // compositionally complex illustration the examples ship must compose
    // nested elements with the composition primitives, not enumerate
    // primitives by hand.
    let examples = example_documents();
    let (_, complex) = examples
        .iter()
        .find(|(name, _)| name == "alpine-lake.md")
        .expect("the compositionally complex example");
    let elements = complex["elements"]
        .as_array()
        .expect("the example carries elements");

    let kinds: Vec<&str> = elements
        .iter()
        .filter_map(|element| element["kind"].as_str())
        .collect();
    for primitive in [
        "group",
        "repeat",
        "boolean",
        "alongPath",
        "offset",
        "projection",
    ] {
        assert!(
            kinds.contains(&primitive),
            "the complex example exercises `{primitive}`: {kinds:?}"
        );
    }
    assert!(
        complex["constraints"]
            .as_array()
            .is_some_and(|constraints| !constraints.is_empty()),
        "the complex example carries a constraint"
    );

    // Nesting several levels deep: at least one element sits at depth three or
    // more (a child of a composition inside another composition).
    let parent = |id: &str| -> Option<String> {
        elements
            .iter()
            .find(|element| element["id"] == id)
            .and_then(|element| element["parentId"].as_str())
            .map(str::to_string)
    };
    let depth = |mut id: String| -> usize {
        let mut depth = 0;
        while let Some(next) = parent(&id) {
            depth += 1;
            id = next;
        }
        depth
    };
    let deepest = elements
        .iter()
        .filter_map(|element| element["id"].as_str())
        .map(|id| depth(id.to_string()))
        .max()
        .unwrap_or(0);
    assert!(
        deepest >= 3,
        "the complex example nests composition several levels deep: depth {deepest}"
    );

    // The example is a valid scene document, not just illustrative JSON.
    let scene = vectr_core::parse(&complex.to_string()).expect("the example parses");
    let diagnostics = vectr_core::validate(&scene);
    assert!(
        !diagnostics.has_errors(),
        "the complex example validates: {diagnostics}"
    );
}

/// The backticked tokens in `text` that begin with `prefix`.
///
/// The skill names its packaged files as inline code (`references/rules.md`,
/// `assets/scene.template.json`), so a backtick-delimited scan collects exactly
/// the pointers the entry point carries.
fn backticked_paths(text: &str, prefix: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let token = &after[..end];
        if token.starts_with(prefix) {
            paths.push(token.to_string());
        }
        rest = &after[end + 1..];
    }
    paths
}

#[test]
fn the_skill_entry_point_carries_the_always_read_surfaces_and_routes_to_every_reference() {
    // FEAT-020's packaging amendment: the skill is a small always-read entry
    // point carrying orientation, the workflow, the tool surface, the version
    // check, and pointers to its references; the depth is loaded only when a
    // step needs it (D-042, D-043).
    let dir_skill = skill_dir();
    let skill = skill_text();

    // Orientation, then the four named surfaces besides it.
    for heading in [
        "# Vectr",
        "## Workflow",
        "## Tool surface",
        "## Version compatibility",
        "## References",
    ] {
        assert!(
            skill.contains(heading),
            "the entry point carries `{heading}`: {skill}"
        );
    }

    // The version check names both surfaces an agent compares.
    assert!(
        skill.contains("vectr --version") && skill.contains("serverInfo.version"),
        "the entry point carries the version check"
    );

    // Pointers: every on-demand reference the package ships is named, and every
    // path the entry point names resolves. A reference added without a pointer
    // is depth the model never loads; a pointer with no file is a broken route.
    let mut on_disk: Vec<String> = fs::read_dir(dir_skill.join("references"))
        .expect("the references directory")
        .map(|entry| {
            let entry = entry.expect("a reference entry");
            format!("references/{}", entry.file_name().to_string_lossy())
        })
        .collect();
    on_disk.sort();
    assert!(!on_disk.is_empty(), "the skill ships on-demand references");

    let mut named = backticked_paths(&skill, "references/");
    named.sort();
    named.dedup();
    for path in &on_disk {
        assert!(
            named.contains(path),
            "the entry point points at `{path}`: {named:?}"
        );
    }
    for path in &named {
        assert!(
            dir_skill.join(path).is_file(),
            "the entry point's pointer `{path}` resolves"
        );
    }

    // The assets and examples the entry point names resolve too.
    for path in backticked_paths(&skill, "assets/") {
        assert!(
            dir_skill.join(&path).is_file(),
            "the entry point's asset pointer `{path}` resolves"
        );
    }
    for path in backticked_paths(&skill, "examples/") {
        assert!(
            dir_skill.join(&path).is_file(),
            "the entry point's example pointer `{path}` resolves"
        );
    }

    // Each reference is routed on a condition, so the depth is loaded only when
    // the step needs it rather than up front.
    assert!(
        skill.contains("Read it when"),
        "the entry point routes each reference on when to read it"
    );
}

#[test]
fn the_skill_entry_point_routes_the_depth_to_its_on_demand_references() {
    // The entry point is always-read; the procedure, the worked examples, the
    // rules the schema does not state, the one method, the inspect-and-correct
    // loop, the failure catalogue, and the defaults live in the references and
    // the examples, read only when the step needs them (FEAT-020, D-042, D-046).
    let dir_skill = skill_dir();
    let skill = skill_text();

    // The entry point embeds no worked scene: every worked example lives under
    // `examples/`, so it is not loaded before the step that needs it.
    assert!(
        fenced_json_blocks(&skill).is_empty(),
        "the entry point embeds no worked example: {skill}"
    );
    for worked_id in ["\"id\": \"habit-logo\"", "\"id\": \"alpine-lake\""] {
        assert!(
            !skill.contains(worked_id),
            "the worked scene `{worked_id}` is not in the entry point"
        );
    }

    // The failure catalogue is a reference's depth, not the entry point's: the
    // catalogue codes are absent from the always-read file. (`E_SCHEMA_VERSION`
    // stays: it is the version-check result, not a catalogue entry.)
    for code in [
        "`E_SCHEMA`",
        "`E_PARSE`",
        "`E_FORMAT_VERSION`",
        "`E_INVALID_COLOR`",
    ] {
        assert!(
            !skill.contains(code),
            "the failure catalogue code {code} is not in the entry point"
        );
    }

    // Each reference the entry point routes to carries the depth it is named
    // for, so the split is real and not an empty pointer.
    let expected: &[(&str, &str)] = &[
        ("references/rules.md", "Rules the schema does not state"),
        (
            "references/reusable-parts.md",
            "Reusable parts: definitions and instances",
        ),
        (
            "references/depth-and-structure.md",
            "Convey depth through structure",
        ),
        ("references/inspect-and-correct.md", "Inspect and correct"),
        (
            "references/defaults.md",
            "Defaults for an ambiguous request",
        ),
        ("references/failures.md", "When authoring fails"),
        ("references/licensing.md", "Licensing and cost"),
    ];
    for (path, marker) in expected {
        let text = reference(path.strip_prefix("references/").expect("a reference path"));
        assert!(
            text.contains(marker),
            "the reference `{path}` carries its depth (`{marker}`)"
        );
    }

    // The worked examples and the failure catalogue are the references' depth:
    // the examples ship the worked scenes, and the failure catalogue is in the
    // reference the entry point names for a finding.
    let habit =
        fs::read_to_string(dir_skill.join("examples/habit-logo.md")).expect("the simple example");
    let alpine =
        fs::read_to_string(dir_skill.join("examples/alpine-lake.md")).expect("the complex example");
    assert!(
        habit.contains("\"id\": \"habit-logo\"") && alpine.contains("\"id\": \"alpine-lake\""),
        "the examples ship the worked scenes"
    );
    let failures = reference("failures.md");
    assert!(
        failures.contains("`E_SCHEMA`") && failures.contains("`E_PARSE`"),
        "the failure catalogue ships the diagnostics"
    );
}
