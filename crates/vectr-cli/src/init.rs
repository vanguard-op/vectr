//! Project scaffolding for `vectr init` (C-004).
//!
//! A project is a directory holding `vectr.project.json` and the entity folders
//! the scene model refers to: `scenes/`, `palettes/`, `strokes/`, `recipes/`,
//! with `assets/` for the fonts a scene may name and `dist/` for output.
//! Initializing writes the project configuration, a starter scene, and the
//! authoring guide a coding agent loads from the project root (FEAT-020);
//! running it again reports the project as already initialized and leaves every
//! file untouched.

use std::fs;
use std::path::Path;

use serde_json::json;
use vectr_core::scene::CURRENT_FORMAT_VERSION;
use vectr_core::{
    parse_palette, parse_style_recipe, validate_palette, validate_style_recipe, Canvas, Scene,
};

use crate::cli::{diagnostics_text, Report, EXIT_OUTPUT, EXIT_SUCCESS};
use crate::output::write_atomic;
use vectr_project::{authoring_guide, AUTHORING_GUIDE_FILE};

/// The identifier every scaffolded project and its starter scene share.
const PROJECT_ID: &str = "project";

/// The starter scene's file name within `scenes/`.
const STARTER_SCENE_FILE: &str = "example.json";

/// The starter scene's stable identifier.
const STARTER_SCENE_ID: &str = "example";

/// The default recipe a new project renders in (FEAT-007, D-009).
const DEFAULT_RECIPE_ID: &str = "flat";

/// The default recipe's file name within `recipes/`.
const DEFAULT_RECIPE_FILE: &str = "flat.json";

/// The default palette a new project resolves its tokens against (FEAT-005,
/// FEAT-016, D-039).
const DEFAULT_PALETTE_ID: &str = "brand";

/// The default palette's file name within `palettes/`.
const DEFAULT_PALETTE_FILE: &str = "brand.json";

/// The starter palette document a new project ships.
///
/// Kept as source text so the scaffold writes exactly what a project document
/// holds; a test parses and validates it so the starter project always
/// compiles.
const DEFAULT_PALETTE_JSON: &str = r##"{
  "id": "brand",
  "projectId": "project",
  "name": "Brand",
  "tokens": [
    { "name": "ink", "value": "#111111" },
    { "name": "paper", "value": "#ffffff" }
  ]
}"##;

/// The flat recipe document a new project ships, ready to compile against.
///
/// Kept as source text so the scaffold writes exactly what a project document
/// holds; a test parses and validates it so the starter project always compiles.
const DEFAULT_RECIPE_JSON: &str = r#"{
  "id": "flat",
  "projectId": "project",
  "name": "flat",
  "parameters": {}
}"#;

/// The entity directories a project holds, alongside `dist/` for output and
/// `assets/` for the fonts and images a scene may reference.
const PROJECT_DIRS: [&str; 8] = [
    "scenes",
    "palettes",
    "strokes",
    "gradients",
    "recipes",
    "definitions",
    "assets",
    "dist",
];

/// Creates a project scaffold in `dir`.
pub fn scaffold(dir: &Path) -> Report {
    let config_path = dir.join("vectr.project.json");
    if config_path.exists() {
        return Report {
            code: EXIT_SUCCESS,
            stdout: format!("project already initialized at {}\n", dir.display()),
            stderr: String::new(),
        };
    }

    let config = json!({
        "id": PROJECT_ID,
        "name": project_name(dir),
        "formatVersion": CURRENT_FORMAT_VERSION,
        "defaultSceneId": STARTER_SCENE_ID,
        "defaultRecipeId": DEFAULT_RECIPE_ID,
        "defaultPaletteId": DEFAULT_PALETTE_ID,
        "output": {
            "format": "svg",
            "width": 512,
            "height": 512,
            "density": 1,
            "background": "#ffffff"
        }
    });
    let config_text = match serde_json::to_string_pretty(&config) {
        Ok(text) => text,
        Err(error) => return write_failure(&config_path, &error.to_string()),
    };

    // The default recipe and palette are validated before anything is written,
    // so a scaffold never ships a project whose starter style cannot compile
    // (NFR-011).
    if let Err(diagnostics) = validate_default_recipe() {
        return Report::failure(EXIT_OUTPUT, diagnostics_text(&diagnostics));
    }
    if let Err(diagnostics) = validate_default_palette() {
        return Report::failure(EXIT_OUTPUT, diagnostics_text(&diagnostics));
    }

    let scene_text = match starter_scene().to_json_pretty() {
        Ok(text) => text,
        Err(diagnostics) => return Report::failure(EXIT_OUTPUT, diagnostics_text(&diagnostics)),
    };

    if let Err(error) = fs::create_dir_all(dir) {
        return write_failure(dir, &error.to_string());
    }
    for sub in PROJECT_DIRS {
        let path = dir.join(sub);
        if let Err(error) = fs::create_dir_all(&path) {
            return write_failure(&path, &error.to_string());
        }
    }

    if let Err(error) = write_atomic(&config_path, config_text.as_bytes()) {
        return write_failure(&config_path, &error.to_string());
    }

    let scene_path = dir.join("scenes").join(STARTER_SCENE_FILE);
    if !scene_path.exists() {
        if let Err(error) = write_atomic(&scene_path, scene_text.as_bytes()) {
            return write_failure(&scene_path, &error.to_string());
        }
    }

    let recipe_path = dir.join("recipes").join(DEFAULT_RECIPE_FILE);
    if !recipe_path.exists() {
        if let Err(error) = write_atomic(&recipe_path, DEFAULT_RECIPE_JSON.as_bytes()) {
            return write_failure(&recipe_path, &error.to_string());
        }
    }

    // The project names a starter palette as its default, so a scene that names
    // none resolves its tokens and an isolated definition has a palette to
    // render against (FEAT-005, FEAT-016).
    let palette_path = dir.join("palettes").join(DEFAULT_PALETTE_FILE);
    if !palette_path.exists() {
        if let Err(error) = write_atomic(&palette_path, DEFAULT_PALETTE_JSON.as_bytes()) {
            return write_failure(&palette_path, &error.to_string());
        }
    }

    // The authoring guide a coding agent loads from the project root. An
    // existing file is left alone: a project the user already runs has its own
    // agent instructions, and the scaffold never clobbers them (FEAT-020).
    let guide_path = dir.join(AUTHORING_GUIDE_FILE);
    if !guide_path.exists() {
        if let Err(error) = write_atomic(&guide_path, authoring_guide().as_bytes()) {
            return write_failure(&guide_path, &error.to_string());
        }
    }

    Report {
        code: EXIT_SUCCESS,
        stdout: format!("initialized project at {}\n", dir.display()),
        stderr: String::new(),
    }
}

/// Checks the bundled default recipe parses and validates (FEAT-007).
fn validate_default_recipe() -> Result<(), vectr_core::Diagnostics> {
    let recipe = parse_style_recipe(DEFAULT_RECIPE_JSON)?;
    let findings = validate_style_recipe(&recipe);
    if findings.has_errors() {
        Err(findings)
    } else {
        Ok(())
    }
}

/// Checks the bundled default palette parses and validates (FEAT-005).
fn validate_default_palette() -> Result<(), vectr_core::Diagnostics> {
    let palette = parse_palette(DEFAULT_PALETTE_JSON)?;
    let findings = validate_palette(&palette);
    if findings.has_errors() {
        Err(findings)
    } else {
        Ok(())
    }
}

/// The project's human-readable name, taken from its directory.
fn project_name(dir: &Path) -> String {
    let direct = dir
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..");
    if let Some(name) = direct {
        return name.to_string();
    }
    std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "Untitled".to_string())
}

/// A minimal, valid scene that opens the project with something to build.
fn starter_scene() -> Scene {
    Scene {
        id: STARTER_SCENE_ID.to_string(),
        project_id: PROJECT_ID.to_string(),
        name: "Example".to_string(),
        format_version: CURRENT_FORMAT_VERSION.to_string(),
        canvas: Canvas {
            width: 512.0,
            height: 512.0,
            background: "#ffffff".to_string(),
        },
        palette_id: None,
        recipe_id: None,
        seed: None,
        title: None,
        description: None,
        elements: Vec::new(),
        constraints: None,
    }
}

fn write_failure(path: &Path, error: &str) -> Report {
    Report::failure(
        EXIT_OUTPUT,
        format!("error: cannot write `{}`: {error}\n", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;
    use vectr_core::parse;

    #[test]
    fn writes_a_project_config_a_starter_scene_and_the_entity_dirs() {
        let dir = TempDir::new("init");
        let target = dir.path().join("habit-tracker");
        let report = scaffold(&target);
        assert_eq!(report.code, EXIT_SUCCESS);

        let config = fs::read_to_string(target.join("vectr.project.json")).expect("config");
        let value: serde_json::Value = serde_json::from_str(&config).expect("valid JSON");
        assert_eq!(value["id"], "project");
        assert_eq!(value["name"], "habit-tracker");
        assert_eq!(value["formatVersion"], CURRENT_FORMAT_VERSION);
        assert_eq!(value["defaultRecipeId"], "flat");
        assert_eq!(value["defaultPaletteId"], DEFAULT_PALETTE_ID);
        assert_eq!(value["output"]["format"], "svg");
        // The project names its starter scene as the default, and the scene
        // lives under the scene directory, not at the project root (FEAT-016,
        // D-032).
        assert_eq!(value["defaultSceneId"], STARTER_SCENE_ID);
        // The starter scene's file is named for its identifier, so the default
        // id resolves to it (D-032).
        assert_eq!(STARTER_SCENE_FILE, format!("{STARTER_SCENE_ID}.json"));
        assert!(
            !target.join(format!("{STARTER_SCENE_ID}.json")).exists(),
            "no scene document at the project root"
        );

        let scene_text =
            fs::read_to_string(target.join("scenes").join("example.json")).expect("scene");
        let scene = parse(&scene_text).expect("the starter scene is valid");
        assert_eq!(scene.id, STARTER_SCENE_ID);
        assert_eq!(scene.project_id, PROJECT_ID);

        // The starter project ships a flat recipe and names it as the default,
        // so a scaffolded scene compiles in the flat look without the author
        // adding anything (FEAT-007, D-009).
        let recipe_text =
            fs::read_to_string(target.join("recipes").join(DEFAULT_RECIPE_FILE)).expect("recipe");
        let recipe = vectr_core::parse_style_recipe(&recipe_text).expect("a valid recipe");
        assert!(recipe.is_flat());
        assert!(vectr_core::validate_style_recipe(&recipe).is_empty());

        // The starter project names a starter palette as its default, so a
        // scene that names none resolves its tokens and an isolated definition
        // has a palette to render against (FEAT-005, FEAT-016).
        let palette_text = fs::read_to_string(target.join("palettes").join(DEFAULT_PALETTE_FILE))
            .expect("palette");
        let palette = vectr_core::parse_palette(&palette_text).expect("a valid palette");
        assert_eq!(palette.id, DEFAULT_PALETTE_ID);
        assert!(!palette.tokens.is_empty());

        for sub in PROJECT_DIRS {
            assert!(target.join(sub).is_dir(), "{sub} is created");
        }
    }

    #[test]
    fn writes_an_authoring_guide_a_coding_agent_loads() {
        let dir = TempDir::new("init-guide");
        let target = dir.path().join("habit-tracker");
        assert_eq!(scaffold(&target).code, EXIT_SUCCESS);

        // The guide the scaffold writes is the workflow an agent follows from
        // the project root, and it names the tool's own format version so a
        // mismatch is caught before authoring (FEAT-020).
        let guide = fs::read_to_string(target.join(AUTHORING_GUIDE_FILE)).expect("guide");
        assert!(guide.contains("vectr validate"), "{guide}");
        assert!(guide.contains(CURRENT_FORMAT_VERSION), "{guide}");
        assert!(guide.contains(env!("CARGO_PKG_VERSION")), "{guide}");
    }

    #[test]
    fn an_existing_authoring_guide_is_left_untouched() {
        let dir = TempDir::new("init-guide-existing");
        let target = dir.path().join("habit-tracker");
        fs::create_dir_all(&target).expect("creates the target");
        let guide_path = target.join(AUTHORING_GUIDE_FILE);
        fs::write(&guide_path, "hand-written instructions").expect("seeds the guide");

        assert_eq!(scaffold(&target).code, EXIT_SUCCESS);
        assert_eq!(
            fs::read_to_string(&guide_path).expect("reads"),
            "hand-written instructions",
            "a project's own agent guide is not overwritten"
        );
    }

    #[test]
    fn initializing_twice_leaves_the_project_untouched() {
        let dir = TempDir::new("init-again");
        let target = dir.path().join("project");
        assert_eq!(scaffold(&target).code, EXIT_SUCCESS);

        let scene_path = target.join("scenes").join("example.json");
        fs::write(&scene_path, "hand-edited").expect("the author edits the scene");

        let report = scaffold(&target);
        assert_eq!(report.code, EXIT_SUCCESS);
        assert!(
            report.stdout.contains("already initialized"),
            "{}",
            report.stdout
        );
        assert_eq!(
            fs::read_to_string(&scene_path).expect("reads"),
            "hand-edited",
            "an existing file is not overwritten"
        );
    }

    #[test]
    fn the_starter_scene_is_empty_and_compiles() {
        let scene = starter_scene();
        assert!(scene.elements.is_empty());
        vectr_core::compile(&scene).expect("the starter scene compiles");
    }
}
