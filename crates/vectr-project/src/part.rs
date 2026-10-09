//! Part resolution in a project's shared definition-and-element namespace
//! (FEAT-031, D-039).
//!
//! A part is a reusable definition or a named element subtree, addressed by its
//! identifier on its own so it can be previewed before it is composed. A
//! project's definitions and its elements share one identifier namespace, so a
//! part identifier resolves to a definition first and otherwise to the element
//! subtree; a subtree is located across the project's scenes without the caller
//! naming one (FEAT-016, FEAT-031).
//!
//! An identifier that is not unique — it names a definition and an element, an
//! element in more than one scene, or two definitions — is reported as a
//! duplicate before anything is rendered, rather than resolved by an arbitrary
//! ordering (FEAT-016, FEAT-031). Resolution is deterministic: scenes are read
//! in a fixed order and the same project and identifier resolve the same way
//! every run (NFR-010).

use std::path::Path;

use vectr_core::{
    parse as parse_scene, Definition, Diagnostic, DiagnosticCode, Diagnostics, Scene, PART,
};

use crate::assets::{load_definitions, ProjectAssets};
use crate::scene::project_scenes;

/// A part resolved within a project, with the assets it renders against.
#[derive(Debug)]
pub enum ResolvedPart {
    /// A reusable definition, previewed against the project's default palette
    /// and default recipe (FEAT-005, D-039).
    Definition {
        definition: Definition,
        assets: ProjectAssets,
    },
    /// An element subtree inside one of the project's scenes, previewed against
    /// the style of the scene that owns it.
    Element { scene: Scene, assets: ProjectAssets },
}

/// Resolves a part identifier to a definition or an element subtree.
///
/// The project's definitions are gathered first, then every scene is searched
/// for an element of that identifier. A definition wins only when the
/// identifier is unique across both; a collision is a duplicate error naming
/// the identifier (FEAT-031). A definition renders against the project's
/// default palette and default recipe; an element subtree against the style of
/// the scene that owns it (FEAT-005, D-039).
pub fn resolve_part(root: &Path, part: &str) -> Result<ResolvedPart, Diagnostics> {
    let definitions = load_definitions(root)?;

    let mut definition: Option<&Definition> = None;
    let mut definition_count = 0usize;
    for candidate in &definitions {
        if candidate.id == part {
            definition_count += 1;
            definition.get_or_insert(candidate);
        }
    }

    let mut element_scene: Option<Scene> = None;
    let mut element_count = 0usize;
    for scene in project_scenes(root) {
        let source = scene.source()?;
        let Ok(parsed) = parse_scene(&source) else {
            // A scene that does not parse cannot own a valid subtree; it is
            // reported by its own validation, not as a part-resolution failure.
            continue;
        };
        let count = parsed
            .elements
            .iter()
            .filter(|element| element.id == part)
            .count();
        if count > 0 {
            element_count += count;
            if element_scene.is_none() {
                element_scene = Some(parsed);
            }
        }
    }

    match definition_count + element_count {
        0 => Err(no_such_part(root, part)),
        1 => {
            if let Some(definition) = definition {
                let assets = ProjectAssets::load_for_definition(root, definition)?;
                Ok(ResolvedPart::Definition {
                    definition: definition.clone(),
                    assets,
                })
            } else {
                let scene = element_scene.expect("one element match was found");
                let assets = ProjectAssets::load(root, &scene)?;
                Ok(ResolvedPart::Element { scene, assets })
            }
        }
        _ => Err(duplicate_part(part)),
    }
}

/// A part identifier no definition or element provides.
fn no_such_part(root: &Path, part: &str) -> Diagnostics {
    Diagnostics::from(Diagnostic::error(
        PART,
        format!(
            "part `{part}` resolves to no definition or element in the project at `{}`",
            root.display()
        ),
    ))
}

/// A part identifier that is not unique across the project's namespace.
fn duplicate_part(part: &str) -> Diagnostics {
    Diagnostics::from(Diagnostic::error(
        DiagnosticCode::DUPLICATE_ID,
        format!(
            "part identifier `{part}` is not unique across the project's definitions and elements"
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use vectr_core::{DiagnosticCode, PART};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vectr-part-{}-{label}-{unique}",
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

    const PALETTE: &str = r##"{"id":"brand","projectId":"p","name":"Brand","tokens":[{"name":"accent","value":"#ff0000"}]}"##;

    fn definition(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","projectId":"p","name":"{id}","parameters":[],"origin":{{"x":0,"y":0}},"elements":[]}}"#
        )
    }

    fn scene(id: &str, element_id: &str) -> String {
        format!(
            r##"{{"id":"{id}","projectId":"p","name":"{id}","formatVersion":"0.2","canvas":{{"width":10,"height":10,"background":"#ffffff"}},"elements":[{{"id":"{element_id}","sceneId":"{id}","order":0,"kind":"rect","geometry":{{"x":0,"y":0,"width":1,"height":1}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}]}}"##
        )
    }

    fn project(tag: &str, config: &str) -> TempDir {
        let dir = TempDir::new(tag);
        write(dir.path(), "vectr.project.json", config);
        write(dir.path(), "palettes/brand.json", PALETTE);
        dir
    }

    #[test]
    fn a_definition_wins_over_no_element() {
        let dir = project("definition", r#"{"defaultPaletteId":"brand"}"#);
        write(dir.path(), "definitions/badge.json", &definition("badge"));
        let resolved = resolve_part(dir.path(), "badge").expect("resolves");
        assert!(matches!(resolved, ResolvedPart::Definition { .. }));
    }

    #[test]
    fn an_element_subtree_is_located_without_naming_a_scene() {
        let dir = project("element", r#"{"defaultPaletteId":"brand"}"#);
        write(dir.path(), "scenes/main.json", &scene("main", "mark"));
        let resolved = resolve_part(dir.path(), "mark").expect("resolves");
        match resolved {
            ResolvedPart::Element { scene, .. } => assert_eq!(scene.id, "main"),
            _ => panic!("expected an element subtree"),
        }
    }

    #[test]
    fn an_unknown_part_is_named() {
        let dir = project("unknown", r#"{"defaultPaletteId":"brand"}"#);
        let diagnostics = resolve_part(dir.path(), "absent").expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, PART);
        assert!(error.message.contains("absent"), "{}", error.message);
    }

    #[test]
    fn an_identifier_shared_by_a_definition_and_an_element_is_a_duplicate() {
        let dir = project("duplicate", r#"{"defaultPaletteId":"brand"}"#);
        write(dir.path(), "definitions/shared.json", &definition("shared"));
        write(dir.path(), "scenes/main.json", &scene("main", "shared"));

        let diagnostics = resolve_part(dir.path(), "shared").expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::DUPLICATE_ID);
        assert!(error.message.contains("shared"), "{}", error.message);
    }

    #[test]
    fn an_element_in_two_scenes_is_a_duplicate() {
        let dir = project("duplicate-scene", r#"{"defaultPaletteId":"brand"}"#);
        write(dir.path(), "scenes/one.json", &scene("one", "mark"));
        write(dir.path(), "scenes/two.json", &scene("two", "mark"));

        let diagnostics = resolve_part(dir.path(), "mark").expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::DUPLICATE_ID);
    }

    #[test]
    fn a_definition_without_a_default_palette_is_reported_naming_the_project() {
        let dir = project("no-palette", r#"{"defaultSceneId":"main"}"#);
        write(dir.path(), "definitions/badge.json", &definition("badge"));
        let diagnostics = resolve_part(dir.path(), "badge").expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert!(
            error.message.contains("no default palette"),
            "{}",
            error.message
        );
    }
}
