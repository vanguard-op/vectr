//! Project scaffolding for `vectr init` (C-004).
//!
//! A project is a directory holding `vectr.project.json` and the entity folders
//! the scene model refers to: `scenes/`, `palettes/`, `strokes/`, `recipes/`,
//! with `assets/` for the fonts a scene may name and `dist/` for output.
//! Initializing writes the project configuration and a starter scene; running
//! it again reports the project as already initialized and leaves every file
//! untouched.

use std::fs;
use std::path::Path;

use serde_json::json;
use vectr_core::scene::CURRENT_FORMAT_VERSION;
use vectr_core::{Canvas, Scene};

use crate::cli::{diagnostics_text, Report, EXIT_OUTPUT, EXIT_SUCCESS};
use crate::output::write_atomic;

/// The identifier every scaffolded project and its starter scene share.
const PROJECT_ID: &str = "project";

/// The starter scene's file name within `scenes/`.
const STARTER_SCENE_FILE: &str = "example.json";

/// The starter scene's stable identifier.
const STARTER_SCENE_ID: &str = "example";

/// The entity directories a project holds, alongside `dist/` for output and
/// `assets/` for the fonts and images a scene may reference.
const PROJECT_DIRS: [&str; 7] = [
    "scenes",
    "palettes",
    "strokes",
    "gradients",
    "recipes",
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

    Report {
        code: EXIT_SUCCESS,
        stdout: format!("initialized project at {}\n", dir.display()),
        stderr: String::new(),
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
        assert_eq!(value["output"]["format"], "svg");

        let scene_text =
            fs::read_to_string(target.join("scenes").join("example.json")).expect("scene");
        let scene = parse(&scene_text).expect("the starter scene is valid");
        assert_eq!(scene.project_id, PROJECT_ID);

        for sub in PROJECT_DIRS {
            assert!(target.join(sub).is_dir(), "{sub} is created");
        }
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
