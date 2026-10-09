//! Scene addressing within a project (FEAT-016, D-032).
//!
//! A command names a scene by its identifier, not by a file path: the document
//! lives at `scenes/<id>.json` under the project root, and a command that names
//! none uses the project's `defaultSceneId` (docs/Vectr/schema.md,
//! "ProjectConfig"). A project that names no default reports that no scene was
//! selected rather than choosing among its scenes, and a default that resolves
//! to no document names the missing scene (FEAT-016, C-004).
//!
//! The identifier is untrusted input like any other scene reference: it must be
//! a plain file stem, so a scene cannot name its way out of the project's scene
//! directory (NFR-021).

use std::fs;
use std::path::{Path, PathBuf};

use vectr_core::{Diagnostic, DiagnosticCode, Diagnostics, Scene};

use crate::assets::{is_plain_id, project_string_field};

/// The folder holding a project's scene documents (D-032).
pub const SCENE_DIR: &str = "scenes";

/// A scene a command named, or the project's default, could not be resolved.
pub const SCENE: DiagnosticCode = DiagnosticCode::new("E_SCENE");

/// The project configuration field naming the default scene.
const DEFAULT_SCENE_FIELD: &str = "defaultSceneId";

/// A scene document resolved within its project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectScene {
    id: String,
    path: PathBuf,
    root: PathBuf,
}

impl ProjectScene {
    /// The scene's stable identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The document's path, `scenes/<id>.json` under the project root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The project root that holds the scene.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Reads the document's source text.
    pub fn source(&self) -> Result<String, Diagnostics> {
        fs::read_to_string(&self.path).map_err(|error| {
            scene_error(format!(
                "cannot read scene `{}`: {error}",
                self.path.display()
            ))
        })
    }

    /// Reads and parses the scene document.
    pub fn parse(&self) -> Result<Scene, Diagnostics> {
        let source = self.source()?;
        vectr_core::parse(&source)
    }
}

/// Resolves the scene a command names, or the project's default.
///
/// `requested` is the identifier a command named, or `None` to use the project's
/// `defaultSceneId`. The scene document is `root/scenes/<id>.json` (D-032). A
/// project that names no default when none is requested, an identifier that is
/// not a plain file stem, and an identifier no document provides are each a
/// located error naming the scene, so a command never picks a scene by accident
/// (FEAT-016, C-004).
pub fn resolve_scene(root: &Path, requested: Option<&str>) -> Result<ProjectScene, Diagnostics> {
    let id = match requested {
        Some(id) => id.to_string(),
        None => match project_string_field(root, DEFAULT_SCENE_FIELD)? {
            Some(id) => id,
            None => {
                return Err(scene_error(format!(
                    "no scene was selected and the project at `{}` names no default scene",
                    root.display()
                )))
            }
        },
    };

    if !is_plain_id(&id) {
        return Err(scene_error(format!(
            "`{id}` is not a valid scene identifier"
        )));
    }

    let path = root.join(SCENE_DIR).join(format!("{id}.json"));
    if !path.is_file() {
        return Err(scene_error(format!(
            "scene `{id}` was not found at `{}`",
            path.display()
        )));
    }

    Ok(ProjectScene {
        id,
        path,
        root: root.to_path_buf(),
    })
}

/// The project's scenes, the default first, each resolved by its identifier.
///
/// Part resolution locates an element subtree across the whole project without
/// the caller naming a scene, so it needs every scene document. The order is
/// deterministic — the default scene first, then the rest by file name — so the
/// same identifier resolves the same way every run (FEAT-031, D-039, NFR-010).
pub fn project_scenes(root: &Path) -> Vec<ProjectScene> {
    let mut ids: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(root.join(SCENE_DIR)) {
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(std::ffi::OsStr::to_str)
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            })
            .collect();
        paths.sort();
        for path in paths {
            if let Some(stem) = path.file_stem().and_then(std::ffi::OsStr::to_str) {
                ids.push(stem.to_string());
            }
        }
    }

    if let Ok(default) = resolve_scene(root, None) {
        let default = default.id().to_string();
        ids.retain(|id| id != &default);
        ids.insert(0, default);
    }

    ids.into_iter()
        .filter_map(|id| resolve_scene(root, Some(&id)).ok())
        .collect()
}

fn scene_error(message: impl Into<String>) -> Diagnostics {
    Diagnostics::from(Diagnostic::error(SCENE, message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const SCENE_JSON: &str = r##"{
      "id": "logo",
      "projectId": "p",
      "name": "Logo",
      "formatVersion": "0.2",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": []
    }"##;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vectr-project-scene-{}-{label}-{unique}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("creates the temporary directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("creates the parent");
        }
        fs::write(&path, text).expect("writes the file");
        path
    }

    #[test]
    fn a_named_scene_resolves_to_its_document() {
        let dir = TempDir::new("named");
        write(dir.path(), "vectr.project.json", "{}");
        write(dir.path(), "scenes/logo.json", SCENE_JSON);

        let scene = resolve_scene(dir.path(), Some("logo")).expect("resolves");
        assert_eq!(scene.id(), "logo");
        assert_eq!(scene.path(), dir.path().join("scenes/logo.json"));
        assert_eq!(scene.root(), dir.path());
        assert_eq!(scene.parse().expect("parses").id, "logo");
    }

    #[test]
    fn an_omitted_scene_uses_the_project_default() {
        let dir = TempDir::new("default");
        write(
            dir.path(),
            "vectr.project.json",
            r#"{"defaultSceneId":"logo"}"#,
        );
        write(dir.path(), "scenes/logo.json", SCENE_JSON);

        let scene = resolve_scene(dir.path(), None).expect("resolves the default");
        assert_eq!(scene.id(), "logo");
    }

    #[test]
    fn a_project_with_no_default_names_no_scene() {
        let dir = TempDir::new("no-default");
        write(dir.path(), "vectr.project.json", "{}");
        write(dir.path(), "scenes/logo.json", SCENE_JSON);

        let diagnostics = resolve_scene(dir.path(), None).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, SCENE);
        assert!(
            error.message.contains("no default scene"),
            "{}",
            error.message
        );
    }

    #[test]
    fn a_default_that_resolves_to_no_document_names_it() {
        let dir = TempDir::new("missing-default");
        write(
            dir.path(),
            "vectr.project.json",
            r#"{"defaultSceneId":"absent"}"#,
        );

        let diagnostics = resolve_scene(dir.path(), None).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, SCENE);
        assert!(error.message.contains("absent"), "{}", error.message);
    }

    #[test]
    fn a_named_scene_no_document_provides_is_reported() {
        let dir = TempDir::new("missing-named");
        write(dir.path(), "vectr.project.json", "{}");

        let diagnostics = resolve_scene(dir.path(), Some("absent")).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, SCENE);
        assert!(error.message.contains("absent"), "{}", error.message);
    }

    #[test]
    fn an_identifier_that_escapes_the_scene_directory_is_refused() {
        let dir = TempDir::new("escape");
        write(dir.path(), "vectr.project.json", "{}");

        for id in ["../secret", "nested/scene", ".", ".."] {
            let diagnostics = resolve_scene(dir.path(), Some(id)).expect_err("refused");
            assert_eq!(
                diagnostics.errors().next().map(|error| error.code.clone()),
                Some(SCENE),
                "`{id}` must be refused"
            );
        }
    }
}
