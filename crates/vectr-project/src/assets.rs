//! Project asset loading for the front ends (D-009, FEAT-016, FEAT-024).
//!
//! A project is a directory holding `vectr.project.json` and the entity folders
//! the scene model refers to. Before compiling, a front end loads the assets a
//! scene names: the palette the scene selects, the style recipe it renders in,
//! the stroke profiles and gradients its elements reference, and the fonts its
//! text elements name. The two open-licensed fonts Vectr ships are supplied for
//! any scene with text, so a text element that names no font renders with the
//! default and a glyph the chosen font lacks is covered by the fallback
//! (FEAT-024, D-017, D-018).
//!
//! References resolve by identifier, not by file name: palettes live under
//! `palettes/`, style recipes under `recipes/`, stroke profiles under `strokes/`,
//! gradients under `gradients/`, and a user-supplied font is an `Asset` document
//! under `assets/` whose `path` points at the font file. A document that cannot
//! be read is a located diagnostic and no compilation happens; nothing is
//! substituted silently (NFR-011, FEAT-005).
//!
//! A scene renders in the recipe it names, or, when it names none, the project's
//! `defaultRecipeId` (docs/Vectr/schema.md, "ProjectConfig"). The recipe is
//! resolved the same way a palette is: by identifier from its folder, an
//! unresolvable id a located error rather than a scene that silently renders
//! flat (FEAT-007–FEAT-010).
//!
//! Because both an identifier in a scene and a path in an asset document are
//! untrusted input, the loader refuses an identifier that is not a plain file
//! stem and an asset file that resolves outside the project root, so a scene
//! cannot name its way out of its project (NFR-021, NFR-024).
//!
//! The loader is deterministic (NFR-010): directory entries are read in sorted
//! order and the bundled fonts are carried in a fixed order, so the same project
//! and scene always produce the same style context.

use std::fs;
use std::path::{Path, PathBuf};

use vectr_core::compiler::FONT;
use vectr_core::style::{UNDEFINED_GRADIENT, UNDEFINED_STROKE};
use vectr_core::{
    parse_gradient, parse_palette, parse_stroke_profile, parse_style_recipe, validate_gradient,
    validate_palette, validate_stroke_profile, validate_style_recipe, Diagnostic, DiagnosticCode,
    Diagnostics, ElementKind, FontAsset, Gradient, Location, PaintKind, Palette, Scene,
    StrokeProfile, StyleContext, StyleRecipe, DEFAULT_FONT_ID, FALLBACK_FONT_ID,
};

/// The project configuration that marks a directory as a project root.
pub(crate) const PROJECT_FILE: &str = "vectr.project.json";

/// The folder holding palette documents.
const PALETTE_DIR: &str = "palettes";

/// The folder holding stroke-profile documents.
const STROKE_DIR: &str = "strokes";

/// The folder holding gradient documents.
const GRADIENT_DIR: &str = "gradients";

/// The folder holding style-recipe documents (FEAT-007–FEAT-010).
const RECIPE_DIR: &str = "recipes";

/// The folder holding asset documents, including the fonts a scene may name.
const ASSET_DIR: &str = "assets";

/// A project style asset (a palette, recipe, gradient or stroke profile) could
/// not be read or parsed, so the project an input refers to is broken.
pub const STYLE_ASSET: DiagnosticCode = DiagnosticCode::new("E_PROJECT_ASSET");

/// The bundled default font's family name.
const DEFAULT_FONT_NAME: &str = "Inter";

/// The bundled fallback font's family name.
const FALLBACK_FONT_NAME: &str = "Noto Sans";

/// The bundled default sans, read into the binary at build time (A-001).
const DEFAULT_FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/Inter.ttf");

/// The bundled fallback sans, read into the binary at build time (A-002).
const FALLBACK_FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/NotoSans.ttf");

/// The assets a scene's project provides, ready to resolve into a
/// [`StyleContext`] (C-002).
#[derive(Debug)]
pub struct ProjectAssets {
    palette: Option<Palette>,
    recipe: Option<StyleRecipe>,
    strokes: Vec<StrokeProfile>,
    gradients: Vec<Gradient>,
    fonts: Vec<FontAsset>,
}

impl ProjectAssets {
    /// Loads the assets the project rooted at `root` provides for `scene`.
    ///
    /// A caller that resolves the root itself — the MCP server, whose root also
    /// anchors its filesystem scope — passes it in. A caller that only has a
    /// scene file uses [`ProjectAssets::load_for_scene`].
    ///
    /// A palette, style-recipe, stroke-profile, gradient, or asset document
    /// that cannot be read is reported rather than skipped, so a broken project
    /// fails loudly instead of compiling against stale or partial assets
    /// (NFR-011).
    pub fn load(root: &Path, scene: &Scene) -> Result<Self, Diagnostics> {
        let mut diagnostics = Diagnostics::new();

        let palette = match scene.palette_id.as_deref() {
            Some(id) => match load_palette(root, id) {
                Ok(palette) => Some(palette),
                Err(findings) => {
                    diagnostics.extend(findings);
                    None
                }
            },
            None => None,
        };

        // A scene renders in the recipe it names, or, when it names none, the
        // project's default; a project without a configuration names neither and
        // the scene compiles with no recipe (docs/Vectr/schema.md,
        // "ProjectConfig").
        let default_recipe = match load_default_recipe_id(root) {
            Ok(id) => id,
            Err(findings) => {
                diagnostics.extend(findings);
                None
            }
        };
        let recipe_id = scene.recipe_id.clone().or(default_recipe);
        let recipe = match recipe_id.as_deref() {
            Some(id) => match load_recipe(root, id) {
                Ok(recipe) => Some(recipe),
                Err(findings) => {
                    diagnostics.extend(findings);
                    None
                }
            },
            None => None,
        };

        let strokes = match load_strokes(root) {
            Ok(strokes) => strokes,
            Err(findings) => {
                diagnostics.extend(findings);
                Vec::new()
            }
        };

        let gradients = match load_gradients(root) {
            Ok(gradients) => gradients,
            Err(findings) => {
                diagnostics.extend(findings);
                Vec::new()
            }
        };

        // The bundled fonts are only meaningful for a scene with text; carrying
        // them into a scene with none would embed a font file nothing uses.
        let mut fonts = Vec::new();
        if has_text(scene) {
            fonts.extend(bundled_fonts());
            match load_font_assets(root) {
                Ok(user) => fonts.extend(user),
                Err(findings) => diagnostics.extend(findings),
            }
        }

        if diagnostics.has_errors() {
            return Err(diagnostics);
        }
        Ok(Self {
            palette,
            recipe,
            strokes,
            gradients,
            fonts,
        })
    }

    /// Loads the assets for a scene file, resolving its project root first.
    ///
    /// The root is the nearest ancestor holding the project configuration, or
    /// the scene's own directory when there is none ([`project_root`]).
    pub fn load_for_scene(scene_path: &Path, scene: &Scene) -> Result<Self, Diagnostics> {
        Self::load(&project_root(scene_path), scene)
    }

    /// The style context the compiler resolves the scene against (D-013).
    pub fn style_context(&self) -> StyleContext<'_> {
        StyleContext {
            palette: self.palette.as_ref(),
            strokes: &self.strokes,
            gradients: &self.gradients,
            fonts: &self.fonts,
            recipe: self.recipe.as_ref(),
        }
    }

    /// The palette the scene selected, when the project provides it.
    pub fn palette(&self) -> Option<&Palette> {
        self.palette.as_ref()
    }

    /// The gradients the project defines (FEAT-027).
    pub fn gradients(&self) -> &[Gradient] {
        &self.gradients
    }

    /// Checks a scene's stroke, gradient and font references against the loaded
    /// assets.
    ///
    /// A referenced stroke profile, gradient or font that no asset provides is
    /// an error naming the element, so an unresolvable reference fails before
    /// anything is compiled and never degrades to a missing stroke or a dropped
    /// run (FEAT-005, FEAT-024).
    pub fn check_references(&self, scene: &Scene) -> Diagnostics {
        let mut diagnostics = Diagnostics::new();
        for element in &scene.elements {
            if let Some(stroke) = &element.stroke {
                if !self
                    .strokes
                    .iter()
                    .any(|profile| profile.id == stroke.profile_id)
                {
                    diagnostics.push(
                        Diagnostic::error(
                            UNDEFINED_STROKE,
                            format!(
                                "element `{}` references undefined stroke profile `{}`",
                                element.id, stroke.profile_id
                            ),
                        )
                        .with_location(Location::element_at(
                            element.id.clone(),
                            "/stroke/profileId",
                        )),
                    );
                }
            }
            for (paint, field) in [
                (element.fill.as_ref(), "fill"),
                (
                    element.stroke.as_ref().map(|stroke| &stroke.paint),
                    "stroke/paint",
                ),
            ] {
                let Some(paint) = paint else {
                    continue;
                };
                if paint.kind != PaintKind::Gradient {
                    continue;
                }
                if !self
                    .gradients
                    .iter()
                    .any(|gradient| gradient.id == paint.reference)
                {
                    diagnostics.push(
                        Diagnostic::error(
                            UNDEFINED_GRADIENT,
                            format!(
                                "element `{}` references undefined gradient `{}`",
                                element.id, paint.reference
                            ),
                        )
                        .with_location(Location::element_at(
                            element.id.clone(),
                            format!("/{field}"),
                        )),
                    );
                }
            }
            if element.kind == ElementKind::Text {
                let requested = element.font_id.as_deref().unwrap_or(DEFAULT_FONT_ID);
                if !self.fonts.iter().any(|font| font.id == requested) {
                    diagnostics.push(
                        Diagnostic::error(
                            FONT,
                            format!(
                                "text element `{}` references undefined font `{requested}`",
                                element.id
                            ),
                        )
                        .with_location(Location::element_at(element.id.clone(), "/fontId")),
                    );
                }
            }
        }
        diagnostics
    }
}

/// Finds the project root for a scene: the nearest ancestor holding the project
/// configuration, or the scene's own directory when there is none.
pub fn project_root(scene_path: &Path) -> PathBuf {
    let start = match scene_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    project_root_from(&start)
}

/// Finds the project root at or above `start`: the nearest ancestor holding the
/// project configuration, or `start` itself when there is none.
///
/// A command that addresses a scene by identifier discovers its project from the
/// working directory, so it walks up from there (FEAT-016, C-004).
pub fn project_root_from(start: &Path) -> PathBuf {
    let mut current: Option<&Path> = Some(start);
    while let Some(dir) = current {
        if dir.join(PROJECT_FILE).is_file() {
            return dir.to_path_buf();
        }
        current = dir.parent();
    }
    start.to_path_buf()
}

/// Whether a scene draws any text, and so needs a font at all.
fn has_text(scene: &Scene) -> bool {
    scene
        .elements
        .iter()
        .any(|element| element.kind == ElementKind::Text)
}

/// The two open-licensed fonts Vectr ships, under the default and fallback ids.
fn bundled_fonts() -> Vec<FontAsset> {
    vec![
        FontAsset::new(
            DEFAULT_FONT_ID,
            DEFAULT_FONT_NAME,
            DEFAULT_FONT_BYTES.to_vec(),
        ),
        FontAsset::new(
            FALLBACK_FONT_ID,
            FALLBACK_FONT_NAME,
            FALLBACK_FONT_BYTES.to_vec(),
        ),
    ]
}

/// Whether an identifier is a plain file stem: non-empty, no path separators,
/// and no `.`/`..` component.
///
/// Shared by every identifier-addressed document — palettes, recipes, scenes —
/// so an id from an untrusted scene or command line cannot name its way out of
/// its directory (NFR-021, NFR-024).
pub(crate) fn is_plain_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && !id.contains('/')
        && !id.contains('\\')
        && Path::new(id)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

/// Refuses an identifier that is not a plain file stem, so a scene cannot name
/// its way out of the asset directory (NFR-021, NFR-024).
pub(crate) fn safe_id(id: &str) -> Result<(), Diagnostics> {
    if is_plain_id(id) {
        Ok(())
    } else {
        Err(style_error(format!(
            "`{id}` is not a valid asset identifier"
        )))
    }
}

/// Loads the palette the scene selected by id.
fn load_palette(root: &Path, id: &str) -> Result<Palette, Diagnostics> {
    safe_id(id)?;
    let dir = root.join(PALETTE_DIR);
    let direct = dir.join(format!("{id}.json"));
    if direct.is_file() {
        let palette = read_document(&direct, "palette", parse_palette, validate_palette)?;
        if palette.id != id {
            return Err(style_error(format!(
                "palette `{}` declares id `{}`, not `{id}`",
                direct.display(),
                palette.id
            )));
        }
        return Ok(palette);
    }
    for path in json_files(&dir) {
        if let Ok(palette) = read_document(&path, "palette", parse_palette, validate_palette) {
            if palette.id == id {
                return Ok(palette);
            }
        }
    }
    Err(style_error(format!(
        "palette `{id}` was not found under `{}`",
        dir.display()
    )))
}

/// Reads the project's default style-recipe id, when its configuration names one
/// (docs/Vectr/schema.md, "ProjectConfig").
///
/// A project configuration that cannot be read or parsed is reported rather than
/// ignored, so a mistyped `defaultRecipeId` cannot silently leave the project
/// rendering without the recipe it meant to apply (NFR-011).
fn load_default_recipe_id(root: &Path) -> Result<Option<String>, Diagnostics> {
    project_string_field(root, "defaultRecipeId")
}

/// Reads a string field from the project configuration, if the configuration
/// names it.
///
/// A project configuration that cannot be read or parsed is reported rather than
/// ignored, so a mistyped `defaultRecipeId` or `defaultSceneId` cannot silently
/// leave the project without the default it meant to name (NFR-011).
pub(crate) fn project_string_field(
    root: &Path,
    field: &str,
) -> Result<Option<String>, Diagnostics> {
    let path = root.join(PROJECT_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let source = fs::read_to_string(&path).map_err(|error| {
        style_error(format!(
            "cannot read project configuration `{}`: {error}",
            path.display()
        ))
    })?;
    let value: serde_json::Value = serde_json::from_str(&source).map_err(|error| {
        // serde_json appends " at line N column M" to its message; the location
        // is dropped so the finding reads as a project diagnostic, like the
        // style documents' parse findings (NFR-011).
        let raw = error.to_string();
        let message = raw.split(" at line ").next().unwrap_or(&raw);
        style_error(format!(
            "project configuration `{}` is not valid JSON: {message}",
            path.display()
        ))
    })?;
    let Some(raw) = value.get(field) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    match raw.as_str() {
        Some(id) if !id.is_empty() => Ok(Some(id.to_string())),
        Some(_) => Err(style_error(format!(
            "project configuration `{}` has an empty `{field}`",
            path.display()
        ))),
        None => Err(style_error(format!(
            "project configuration `{}` has a non-string `{field}`",
            path.display()
        ))),
    }
}

/// Loads the style recipe the scene renders in, by id (FEAT-007–FEAT-010).
fn load_recipe(root: &Path, id: &str) -> Result<StyleRecipe, Diagnostics> {
    safe_id(id)?;
    let dir = root.join(RECIPE_DIR);
    let direct = dir.join(format!("{id}.json"));
    if direct.is_file() {
        let recipe = read_document(
            &direct,
            "style recipe",
            parse_style_recipe,
            validate_style_recipe,
        )?;
        if recipe.id != id {
            return Err(style_error(format!(
                "style recipe `{}` declares id `{}`, not `{id}`",
                direct.display(),
                recipe.id
            )));
        }
        return Ok(recipe);
    }
    for path in json_files(&dir) {
        if let Ok(recipe) = read_document(
            &path,
            "style recipe",
            parse_style_recipe,
            validate_style_recipe,
        ) {
            if recipe.id == id {
                return Ok(recipe);
            }
        }
    }
    Err(style_error(format!(
        "style recipe `{id}` was not found under `{}`",
        dir.display()
    )))
}

/// Loads every stroke profile the project defines.
fn load_strokes(root: &Path) -> Result<Vec<StrokeProfile>, Diagnostics> {
    let dir = root.join(STROKE_DIR);
    let mut profiles = Vec::new();
    let mut diagnostics = Diagnostics::new();
    for path in json_files(&dir) {
        match read_document(
            &path,
            "stroke profile",
            parse_stroke_profile,
            validate_stroke_profile,
        ) {
            Ok(profile) => profiles.push(profile),
            Err(findings) => diagnostics.extend(findings),
        }
    }
    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(profiles)
    }
}

/// Loads every gradient the project defines.
fn load_gradients(root: &Path) -> Result<Vec<Gradient>, Diagnostics> {
    let dir = root.join(GRADIENT_DIR);
    let mut gradients = Vec::new();
    let mut diagnostics = Diagnostics::new();
    for path in json_files(&dir) {
        match read_document(&path, "gradient", parse_gradient, validate_gradient) {
            Ok(gradient) => gradients.push(gradient),
            Err(findings) => diagnostics.extend(findings),
        }
    }
    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(gradients)
    }
}

/// Loads the user-supplied font assets the project declares.
///
/// Each `Asset` document of kind `font` names its font file in `path`, resolved
/// relative to the project root or, failing that, to the document itself. The
/// font file is read as data and never executed (NFR-022), and must resolve
/// inside the project root so an asset document cannot point elsewhere
/// (NFR-024).
fn load_font_assets(root: &Path) -> Result<Vec<FontAsset>, Diagnostics> {
    let dir = root.join(ASSET_DIR);
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut fonts = Vec::new();
    let mut diagnostics = Diagnostics::new();
    for path in json_files(&dir) {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                diagnostics.push(font_error(format!(
                    "cannot read asset `{}`: {error}",
                    path.display()
                )));
                continue;
            }
        };
        let value: serde_json::Value = match serde_json::from_str(&source) {
            Ok(value) => value,
            Err(error) => {
                diagnostics.push(font_error(format!(
                    "asset `{}` is not valid JSON: {error}",
                    path.display()
                )));
                continue;
            }
        };
        if value.get("kind").and_then(serde_json::Value::as_str) != Some("font") {
            continue;
        }
        let Some(id) = value.get("id").and_then(serde_json::Value::as_str) else {
            diagnostics.push(font_error(format!(
                "font asset `{}` declares no id",
                path.display()
            )));
            continue;
        };
        let Some(relative) = value.get("path").and_then(serde_json::Value::as_str) else {
            diagnostics.push(font_error(format!("font asset `{id}` declares no path")));
            continue;
        };
        let Some(file) = asset_file(root, &path, relative) else {
            diagnostics.push(font_error(format!(
                "font `{id}` names `{relative}`, which is not a file"
            )));
            continue;
        };
        // The resolved file must stay inside the project root (NFR-024).
        let canonical = fs::canonicalize(&file).unwrap_or_else(|_| file.clone());
        if !canonical.starts_with(&canonical_root) {
            diagnostics.push(font_error(format!(
                "font `{id}` names `{relative}`, which is outside the project"
            )));
            continue;
        }
        match fs::read(&file) {
            Ok(data) => {
                let name = value
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(id)
                    .to_string();
                fonts.push(FontAsset::new(id, name, data));
            }
            Err(error) => diagnostics.push(font_error(format!(
                "cannot read font `{id}` from `{}`: {error}",
                file.display()
            ))),
        }
    }
    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(fonts)
    }
}

/// Resolves an asset's `path` against the project root, then the document.
fn asset_file(root: &Path, document: &Path, relative: &str) -> Option<PathBuf> {
    let from_root = root.join(relative);
    if from_root.is_file() {
        return Some(from_root);
    }
    let from_document = document.parent()?.join(relative);
    from_document.is_file().then_some(from_document)
}

/// Reads and validates one style document, naming the file on any failure.
fn read_document<T>(
    path: &Path,
    kind: &str,
    parse: fn(&str) -> Result<T, Diagnostics>,
    validate: fn(&T) -> Diagnostics,
) -> Result<T, Diagnostics> {
    let source = fs::read_to_string(path).map_err(|error| {
        style_error(format!("cannot read {kind} `{}`: {error}", path.display()))
    })?;
    let document = parse(&source).map_err(|findings| invalid(path, kind, findings))?;
    let findings = validate(&document);
    if findings.has_errors() {
        return Err(invalid(path, kind, findings));
    }
    Ok(document)
}

/// Names the offending file ahead of a document's own findings.
fn invalid(path: &Path, kind: &str, findings: Diagnostics) -> Diagnostics {
    let mut combined = style_error(format!("invalid {kind} `{}`", path.display()));
    combined.extend(findings);
    combined
}

/// The JSON documents in a directory, in a deterministic order.
fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && has_json_extension(path))
        .collect();
    paths.sort();
    paths
}

/// Whether a path carries a `.json` extension, whatever its case.
fn has_json_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn style_error(message: impl Into<String>) -> Diagnostics {
    Diagnostics::from(Diagnostic::error(STYLE_ASSET, message))
}

fn font_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(FONT, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use vectr_core::{compile_with_style, parse as parse_scene};

    const TEXT_SCENE: &str = r##"{
      "id": "s",
      "projectId": "p",
      "name": "Text",
      "formatVersion": "0.2",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "t1", "sceneId": "s", "order": 0, "kind": "text",
          "geometry": { "text": "Hi", "fontSize": 12, "x": 0, "y": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    const RECT_SCENE: &str = r##"{
      "id": "s",
      "projectId": "p",
      "name": "Rect",
      "formatVersion": "0.2",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "r1", "sceneId": "s", "order": 0, "kind": "rect",
          "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    /// A unique directory that removes itself when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vectr-project-{}-{label}-{unique}",
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

    fn bundled(file: &str) -> Vec<u8> {
        let path = format!("{}/../../assets/fonts/{file}", env!("CARGO_MANIFEST_DIR"));
        fs::read(&path).unwrap_or_else(|error| panic!("could not read {path}: {error}"))
    }

    #[test]
    fn a_text_scene_carries_the_bundled_default_and_fallback_fonts() {
        let dir = TempDir::new("assets-bundled");
        let scene_path = write(dir.path(), "scene.json", TEXT_SCENE);
        let scene = parse_scene(TEXT_SCENE).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        let model = compile_with_style(&scene, &assets.style_context()).expect("compiles");
        let ids: Vec<&str> = model.fonts.iter().map(|font| font.id.as_str()).collect();
        assert_eq!(ids, vec![DEFAULT_FONT_ID, FALLBACK_FONT_ID]);
        assert_eq!(model.fonts[0].data, bundled("Inter.ttf"));
        assert_eq!(model.fonts[1].data, bundled("NotoSans.ttf"));
    }

    #[test]
    fn a_scene_without_text_carries_no_font() {
        let dir = TempDir::new("assets-no-text");
        let scene_path = write(dir.path(), "scene.json", RECT_SCENE);
        let scene = parse_scene(RECT_SCENE).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        let model = compile_with_style(&scene, &assets.style_context()).expect("compiles");
        assert!(model.fonts.is_empty(), "{:?}", model.fonts);
    }

    #[test]
    fn a_project_palette_resolves_a_fill_token() {
        let dir = TempDir::new("assets-palette");
        write(dir.path(), "vectr.project.json", "{}");
        write(
            dir.path(),
            "palettes/brand.json",
            r##"{"id":"brand","projectId":"p","name":"Brand","tokens":[{"name":"accent","value":"#ff0000"}]}"##,
        );
        let scene_text = TEXT_SCENE.replace(
            r##""elements": ["##,
            r##""paletteId": "brand", "elements": ["##,
        );
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        assert_eq!(
            assets.palette().map(|palette| palette.id.as_str()),
            Some("brand")
        );
    }

    #[test]
    fn a_missing_palette_is_a_project_asset_error() {
        let dir = TempDir::new("assets-missing-palette");
        write(dir.path(), "vectr.project.json", "{}");
        let scene_text = RECT_SCENE.replace(
            r##""elements": ["##,
            r##""paletteId": "brand", "elements": ["##,
        );
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let diagnostics = ProjectAssets::load_for_scene(&scene_path, &scene).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, STYLE_ASSET);
        assert!(error.message.contains("brand"), "{}", error.message);
    }

    #[test]
    fn a_palette_id_that_escapes_the_root_is_refused() {
        let dir = TempDir::new("assets-escape-id");
        write(dir.path(), "vectr.project.json", "{}");
        write(
            dir.path(),
            "palettes/../secret.json",
            r##"{"id":"../secret","projectId":"p","name":"S","tokens":[]}"##,
        );
        let scene_text = RECT_SCENE.replace(
            r##""elements": ["##,
            r##""paletteId": "../secret", "elements": ["##,
        );
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let diagnostics = ProjectAssets::load_for_scene(&scene_path, &scene).expect_err("refused");
        assert_eq!(
            diagnostics.errors().next().map(|error| error.code.clone()),
            Some(STYLE_ASSET)
        );
    }

    #[test]
    fn a_missing_stroke_profile_is_reported_against_the_element() {
        let dir = TempDir::new("assets-missing-stroke");
        write(dir.path(), "vectr.project.json", "{}");
        let scene_text = RECT_SCENE.replace(
            r##""kind": "rect","##,
            r##""kind": "rect", "stroke": {"profileId": "outline", "paint": {"kind": "token", "ref": "accent"}},"##,
        );
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        let diagnostics = assets.check_references(&scene);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNDEFINED_STROKE);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.element_id.as_deref()),
            Some("r1")
        );
    }

    #[test]
    fn a_project_stroke_profile_resolves() {
        let dir = TempDir::new("assets-stroke");
        write(dir.path(), "vectr.project.json", "{}");
        write(
            dir.path(),
            "strokes/outline.json",
            r#"{"id":"outline","projectId":"p","name":"Outline","width":3,"cap":"butt","join":"miter"}"#,
        );
        write(
            dir.path(),
            "palettes/brand.json",
            r##"{"id":"brand","projectId":"p","name":"Brand","tokens":[{"name":"accent","value":"#0000ff"}]}"##,
        );
        let scene_text = RECT_SCENE.replace(
            r##""kind": "rect","##,
            r##""kind": "rect", "stroke": {"profileId": "outline", "paint": {"kind": "token", "ref": "accent"}},"##,
        );
        let scene_text = scene_text.replace(
            r##""elements": ["##,
            r##""paletteId": "brand", "elements": ["##,
        );
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        assert!(assets.check_references(&scene).is_empty());
        let model = compile_with_style(&scene, &assets.style_context()).expect("compiles");
        let stroke = model.nodes[0]
            .paint
            .stroke
            .as_ref()
            .expect("a resolved stroke");
        assert_eq!(stroke.width, 3.0);
        assert_eq!(
            stroke.paint,
            vectr_core::render::Paint::Color {
                value: "#0000ff".to_string()
            }
        );
    }

    #[test]
    fn a_user_font_asset_is_loaded_and_carried() {
        let dir = TempDir::new("assets-user-font");
        write(dir.path(), "vectr.project.json", "{}");
        fs::create_dir_all(dir.path().join("assets")).expect("creates assets");
        fs::write(dir.path().join("assets/Brand.ttf"), bundled("Inter.ttf"))
            .expect("writes the font file");
        write(
            dir.path(),
            "assets/brand.json",
            r#"{"id":"brand","sceneId":"s","kind":"font","path":"assets/Brand.ttf","license":"OFL"}"#,
        );
        let scene_text = TEXT_SCENE.replace(
            r##""kind": "text","##,
            r##""kind": "text", "fontId": "brand","##,
        );
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        assert!(assets.check_references(&scene).is_empty());
        let model = compile_with_style(&scene, &assets.style_context()).expect("compiles");
        let brand = model
            .fonts
            .iter()
            .find(|font| font.id == "brand")
            .expect("the user font is carried");
        assert_eq!(brand.data, bundled("Inter.ttf"));
    }

    #[test]
    fn a_missing_font_is_reported_against_the_element() {
        let dir = TempDir::new("assets-missing-font");
        write(dir.path(), "vectr.project.json", "{}");
        let scene_text = TEXT_SCENE.replace(
            r##""kind": "text","##,
            r##""kind": "text", "fontId": "absent","##,
        );
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        let diagnostics = assets.check_references(&scene);
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, FONT);
        assert!(error.message.contains("absent"), "{}", error.message);
    }

    #[test]
    fn a_font_asset_whose_file_is_missing_is_a_dependency_error() {
        let dir = TempDir::new("assets-font-file");
        write(dir.path(), "vectr.project.json", "{}");
        write(
            dir.path(),
            "assets/brand.json",
            r#"{"id":"brand","sceneId":"s","kind":"font","path":"assets/Brand.ttf","license":"OFL"}"#,
        );
        let scene_text = TEXT_SCENE.replace(
            r##""kind": "text","##,
            r##""kind": "text", "fontId": "brand","##,
        );
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let diagnostics = ProjectAssets::load_for_scene(&scene_path, &scene).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, FONT);
    }

    #[test]
    fn a_font_file_outside_the_project_root_is_refused() {
        let dir = TempDir::new("assets-font-escape");
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).expect("creates the outside directory");
        fs::write(outside.join("Brand.ttf"), bundled("Inter.ttf")).expect("writes the font file");
        let project = dir.path().join("project");
        write(&project, "vectr.project.json", "{}");
        write(
            &project,
            "assets/brand.json",
            r#"{"id":"brand","sceneId":"s","kind":"font","path":"../outside/Brand.ttf","license":"OFL"}"#,
        );
        let scene_text = TEXT_SCENE.replace(
            r##""kind": "text","##,
            r##""kind": "text", "fontId": "brand","##,
        );
        let scene_path = write(&project, "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let diagnostics = ProjectAssets::load_for_scene(&scene_path, &scene).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, FONT);
        assert!(error.message.contains("outside"), "{}", error.message);
    }

    #[test]
    fn the_project_root_is_the_nearest_ancestor_with_the_configuration() {
        let dir = TempDir::new("assets-root");
        let project = dir.path().join("habit");
        write(&project, "vectr.project.json", "{}");
        let scene = write(&project, "scenes/deep/scene.json", RECT_SCENE);
        assert_eq!(project_root(&scene), project);
    }

    #[test]
    fn loading_is_deterministic() {
        let dir = TempDir::new("assets-determinism");
        write(dir.path(), "vectr.project.json", "{}");
        write(
            dir.path(),
            "strokes/a.json",
            r#"{"id":"a","projectId":"p","name":"A","width":1,"cap":"butt","join":"miter"}"#,
        );
        write(
            dir.path(),
            "strokes/b.json",
            r#"{"id":"b","projectId":"p","name":"B","width":2,"cap":"round","join":"bevel"}"#,
        );
        let scene_path = write(dir.path(), "scene.json", RECT_SCENE);
        let scene = parse_scene(RECT_SCENE).expect("a valid scene");

        let first = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        let second = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        let first_ids: Vec<&str> = first.strokes.iter().map(|p| p.id.as_str()).collect();
        let second_ids: Vec<&str> = second.strokes.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(first_ids, second_ids);
        assert_eq!(first_ids, vec!["a", "b"]);
    }

    const LINE_RECIPE: &str =
        r#"{"id":"line","projectId":"p","name":"line-art","parameters":{"strokeWeight":2.5}}"#;

    const FLAT_RECIPE: &str = r#"{"id":"flat","projectId":"p","name":"flat","parameters":{}}"#;

    const HAIRLINE: &str = r#"{"id":"hairline","projectId":"p","name":"Hairline","width":0,"cap":"butt","join":"miter"}"#;

    const INK_PALETTE: &str = r##"{"id":"brand","projectId":"p","name":"Brand","tokens":[{"name":"accent","value":"#ff0000"}]}"##;

    /// A rect with a stroke whose profile leaves the weight unset, so a line-art
    /// recipe's `strokeWeight` is the one that reaches the render model.
    fn scene_naming(recipe: &str) -> String {
        RECT_SCENE
            .replace(
                r##""kind": "rect","##,
                r##""kind": "rect", "stroke": {"profileId": "hairline", "paint": {"kind": "token", "ref": "accent"}},"##,
            )
            .replace(
                r##""formatVersion": "0.2","##,
                &format!(r##""formatVersion": "0.2", "recipeId": "{recipe}","##),
            )
    }

    #[test]
    fn a_scene_naming_a_recipe_loads_and_applies_it() {
        let dir = TempDir::new("recipe-scene");
        write(dir.path(), "vectr.project.json", "{}");
        write(dir.path(), "recipes/line.json", LINE_RECIPE);
        write(dir.path(), "strokes/hairline.json", HAIRLINE);
        write(dir.path(), "palettes/brand.json", INK_PALETTE);
        let scene_text = scene_naming("line");
        let scene_text = scene_text.replace(
            r##""elements": ["##,
            r##""paletteId": "brand", "elements": ["##,
        );
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        let model = compile_with_style(&scene, &assets.style_context()).expect("compiles");
        assert_eq!(model.meta.recipe.as_deref(), Some("line-art"));
        let stroke = model.nodes[0]
            .paint
            .stroke
            .as_ref()
            .expect("the recipe's weight reaches the stroke");
        assert_eq!(stroke.width, 2.5);
    }

    #[test]
    fn the_project_default_recipe_applies_when_the_scene_names_none() {
        let dir = TempDir::new("recipe-default");
        write(
            dir.path(),
            "vectr.project.json",
            r#"{"defaultRecipeId":"line"}"#,
        );
        write(dir.path(), "recipes/line.json", LINE_RECIPE);
        let scene_path = write(dir.path(), "scenes/scene.json", RECT_SCENE);
        let scene = parse_scene(RECT_SCENE).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        let model = compile_with_style(&scene, &assets.style_context()).expect("compiles");
        assert_eq!(model.meta.recipe.as_deref(), Some("line-art"));
    }

    #[test]
    fn a_scene_recipe_overrides_the_project_default() {
        let dir = TempDir::new("recipe-override");
        write(
            dir.path(),
            "vectr.project.json",
            r#"{"defaultRecipeId":"flat"}"#,
        );
        write(dir.path(), "recipes/flat.json", FLAT_RECIPE);
        write(dir.path(), "recipes/line.json", LINE_RECIPE);
        let scene_text = scene_naming("line");
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        let model = compile_with_style(&scene, &assets.style_context()).expect("compiles");
        assert_eq!(model.meta.recipe.as_deref(), Some("line-art"));
    }

    #[test]
    fn a_scene_without_a_recipe_loads_no_recipe() {
        let dir = TempDir::new("recipe-none");
        let scene_path = write(dir.path(), "scene.json", RECT_SCENE);
        let scene = parse_scene(RECT_SCENE).expect("a valid scene");

        let assets = ProjectAssets::load_for_scene(&scene_path, &scene).expect("loads");
        assert!(assets.style_context().recipe.is_none());
    }

    #[test]
    fn a_missing_recipe_is_a_project_asset_error() {
        let dir = TempDir::new("recipe-missing");
        write(dir.path(), "vectr.project.json", "{}");
        let scene_text = scene_naming("absent");
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let diagnostics = ProjectAssets::load_for_scene(&scene_path, &scene).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, STYLE_ASSET);
        assert!(error.message.contains("absent"), "{}", error.message);
    }

    #[test]
    fn an_invalid_recipe_document_is_refused() {
        let dir = TempDir::new("recipe-invalid");
        write(dir.path(), "vectr.project.json", "{}");
        write(
            dir.path(),
            "recipes/bad.json",
            r#"{"id":"bad","projectId":"p","name":"geometric","parameters":{"gridSize":-1}}"#,
        );
        let scene_text = scene_naming("bad");
        let scene_path = write(dir.path(), "scenes/scene.json", &scene_text);
        let scene = parse_scene(&scene_text).expect("a valid scene");

        let diagnostics = ProjectAssets::load_for_scene(&scene_path, &scene).expect_err("refused");
        assert!(
            diagnostics
                .errors()
                .any(|error| error.code == DiagnosticCode::SCHEMA),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn a_malformed_project_configuration_is_reported() {
        let dir = TempDir::new("recipe-bad-config");
        write(dir.path(), "vectr.project.json", "{ not json");
        let scene_path = write(dir.path(), "scene.json", RECT_SCENE);
        let scene = parse_scene(RECT_SCENE).expect("a valid scene");

        let diagnostics = ProjectAssets::load_for_scene(&scene_path, &scene).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, STYLE_ASSET);
    }
}
