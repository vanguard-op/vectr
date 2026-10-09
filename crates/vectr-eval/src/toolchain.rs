//! Running an authored scene through the toolchain (FEAT-023).
//!
//! A model returns a JSON object carrying a Vectr scene and, for a complex
//! graphic, the reusable definitions the scene places. The harness materializes
//! that bundle into a fresh evaluation project — a fixed palette and the flat
//! recipe, so a scene that names no style resolves against known assets — and
//! runs the real pipeline: parse, validate, resolve references, compile. A
//! scene that fails any step is a compile failure with its diagnostics; nothing
//! is written outside the scratch directory (NFR-011).
//!
//! The evaluation palette carries the atmospheric tokens the hard-end guidance
//! points a model at, so depth and perspective are expressible from the
//! documented palette (FEAT-032).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use vectr_core::{compile_with_style, parse, Diagnostic, Diagnostics};

use crate::json::extract_json;
use crate::provider::Authored;
use vectr_project::ProjectAssets;

/// The identifier of the evaluation project's palette.
pub const PALETTE_ID: &str = "eval";

/// The identifier of the evaluation project's recipe.
pub const RECIPE_ID: &str = "flat";

/// The palette the evaluation project supplies, with atmospheric variants so a
/// scene can express depth and distance (FEAT-032).
pub const PALETTE_TOKENS: &[(&str, &str)] = &[
    ("ink", "#111111"),
    ("paper", "#ffffff"),
    ("accent", "#6d5efc"),
    ("accentDeep", "#3a2fb0"),
    ("accentSoft", "#b9b2ff"),
    ("highlight", "#ffd166"),
    ("shadow", "#1a1a2e"),
    ("sky", "#8ecae6"),
    ("skyFar", "#cfe8f3"),
    ("haze", "#e8f1f5"),
    ("ground", "#5c8a3a"),
    ("groundFar", "#a8c48a"),
    ("foliage", "#2f6b2f"),
    ("foliageFar", "#7aa87a"),
    ("water", "#2a7fa3"),
    ("waterFar", "#9fc9d9"),
    ("warm", "#e07a3f"),
    ("cool", "#3f7de0"),
    ("neutral", "#8a8a8a"),
];

/// The outcome of running one authored scene through the toolchain.
#[derive(Debug, Clone, PartialEq)]
pub struct CompileOutcome {
    /// Whether the scene parsed, validated, resolved and compiled.
    pub compiled: bool,
    /// The findings the toolchain produced, in order.
    pub diagnostics: Vec<Diagnostic>,
    /// Why the scene did not compile, when it did not.
    pub failure: Option<String>,
}

/// Materializes an authored bundle and compiles it.
///
/// `authored.text` is expected to hold the JSON object the authoring prompt
/// asks for. Text with no JSON, a scene that does not parse, a reference that
/// does not resolve, and a compile error are each a compile failure carrying
/// the reason and, where the toolchain produced them, the diagnostics.
pub fn evaluate(authored: &Authored) -> CompileOutcome {
    let Some(value) = extract_json(&authored.text) else {
        return failure("the model returned no JSON object");
    };
    let Some((scene_value, definitions)) = split_bundle(&value) else {
        return failure("the model's JSON carried no `scene` object");
    };

    let scene_text = match serde_json::to_string(&scene_value) {
        Ok(text) => text,
        Err(error) => return failure(format!("the scene could not be serialized: {error}")),
    };
    let scene = match parse(&scene_text) {
        Ok(scene) => scene,
        Err(diagnostics) => return from_diagnostics(diagnostics),
    };

    let scratch = Scratch::new();
    if let Err(error) = write_project(&scratch.path, &scene_text, &definitions) {
        return failure(error);
    }

    let assets = match ProjectAssets::load(&scratch.path, &scene) {
        Ok(assets) => assets,
        Err(diagnostics) => return from_diagnostics(diagnostics),
    };

    let references = assets.check_references(&scene);
    if references.has_errors() {
        return from_diagnostics(references);
    }

    match compile_with_style(&scene, &assets.style_context()) {
        Ok(model) => {
            let mut diagnostics: Vec<Diagnostic> = references.into_iter().collect();
            diagnostics.extend(model.diagnostics.iter().cloned());
            CompileOutcome {
                compiled: true,
                diagnostics,
                failure: None,
            }
        }
        Err(diagnostics) => from_diagnostics(diagnostics),
    }
}

/// Splits the authored JSON into its scene and its reusable definitions.
fn split_bundle(value: &Value) -> Option<(Value, Vec<Value>)> {
    let object = value.as_object()?;
    if let Some(scene) = object.get("scene") {
        if !scene.is_object() {
            return None;
        }
        let definitions = object
            .get("definitions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        return Some((scene.clone(), definitions));
    }
    // A bare scene document is accepted too.
    Some((value.clone(), Vec::new()))
}

fn from_diagnostics(diagnostics: Diagnostics) -> CompileOutcome {
    let failure = diagnostics
        .errors()
        .next()
        .map(|error| error.to_string())
        .unwrap_or_else(|| "the scene did not compile".to_string());
    CompileOutcome {
        compiled: false,
        diagnostics: diagnostics.into_iter().collect(),
        failure: Some(failure),
    }
}

fn failure(message: impl Into<String>) -> CompileOutcome {
    CompileOutcome {
        compiled: false,
        diagnostics: Vec::new(),
        failure: Some(message.into()),
    }
}

/// Writes the evaluation project and the authored documents into `root`.
///
/// Definition files are named by position rather than by the authored
/// identifier, so an untrusted identifier cannot name its way out of the
/// directory (NFR-021).
fn write_project(root: &Path, scene_text: &str, definitions: &[Value]) -> Result<(), String> {
    for sub in [
        "scenes",
        "palettes",
        "strokes",
        "gradients",
        "recipes",
        "definitions",
        "assets",
        "dist",
    ] {
        fs::create_dir_all(root.join(sub))
            .map_err(|error| format!("cannot create the evaluation project: {error}"))?;
    }

    let config = format!(
        r#"{{
  "id": "eval",
  "name": "Evaluation",
  "formatVersion": "0.2",
  "defaultPaletteId": "{PALETTE_ID}",
  "defaultRecipeId": "{RECIPE_ID}"
}}"#
    );
    let tokens: Vec<String> = PALETTE_TOKENS
        .iter()
        .map(|(name, value)| format!(r#"{{"name":"{name}","value":"{value}"}}"#))
        .collect();
    let palette = format!(
        r#"{{"id":"{PALETTE_ID}","projectId":"eval","name":"Evaluation","tokens":[{}]}}"#,
        tokens.join(",")
    );
    let recipe =
        format!(r#"{{"id":"{RECIPE_ID}","projectId":"eval","name":"flat","parameters":{{}}}}"#);

    write(&root.join("vectr.project.json"), &config)?;
    write(
        &root.join("palettes").join(format!("{PALETTE_ID}.json")),
        &palette,
    )?;
    write(
        &root.join("recipes").join(format!("{RECIPE_ID}.json")),
        &recipe,
    )?;
    write(&root.join("scenes").join("scene.json"), scene_text)?;
    for (index, definition) in definitions.iter().enumerate() {
        let text = serde_json::to_string(definition)
            .map_err(|error| format!("a definition could not be serialized: {error}"))?;
        write(
            &root.join("definitions").join(format!("{index}.json")),
            &text,
        )?;
    }
    Ok(())
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|error| format!("cannot write `{}`: {error}", path.display()))
}

/// A per-scene scratch directory removed when the outcome is returned.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vectr-eval-scratch-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        let _ = fs::create_dir_all(&path);
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authored(text: &str) -> Authored {
        Authored {
            text: text.to_string(),
            tokens: None,
        }
    }

    const RECT_SCENE: &str = r##"{
      "scene": {
        "id": "s",
        "projectId": "eval",
        "name": "Rect",
        "formatVersion": "0.2",
        "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
        "elements": [
          {
            "id": "r1", "sceneId": "s", "order": 0, "kind": "rect",
            "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
            "fill": { "kind": "token", "ref": "accent" },
            "opacity": 1, "visible": true
          }
        ]
      }
    }"##;

    #[test]
    fn a_valid_authored_scene_compiles() {
        let outcome = evaluate(&authored(RECT_SCENE));
        assert!(outcome.compiled, "{:?}", outcome);
        assert!(outcome.failure.is_none());
    }

    #[test]
    fn a_bare_scene_document_is_accepted() {
        let direct = r##"{
          "id": "s", "projectId": "eval", "name": "Rect", "formatVersion": "0.2",
          "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
          "elements": []
        }"##;
        let outcome = evaluate(&authored(direct));
        assert!(outcome.compiled, "{:?}", outcome);
    }

    #[test]
    fn an_unknown_palette_token_fails_the_reference_check() {
        let bad = RECT_SCENE.replace("\"ref\": \"accent\"", "\"ref\": \"absent\"");
        let outcome = evaluate(&authored(&bad));
        assert!(!outcome.compiled);
        assert!(outcome.failure.is_some());
    }

    #[test]
    fn prose_with_no_json_is_a_failure() {
        let outcome = evaluate(&authored("I could not draw that."));
        assert!(!outcome.compiled);
        assert!(outcome.failure.expect("a reason").contains("no JSON"));
    }

    #[test]
    fn a_scene_that_does_not_parse_reports_its_diagnostics() {
        let outcome = evaluate(&authored(r#"{"scene": {"id": 1}}"#));
        assert!(!outcome.compiled);
        assert!(!outcome.diagnostics.is_empty());
    }
}
