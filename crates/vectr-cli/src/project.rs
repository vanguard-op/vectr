//! Project asset loading for the commands (D-009, FEAT-016, FEAT-024).
//!
//! A project is a directory holding `vectr.project.json` and the entity folders
//! the scene model refers to. Before compiling, a command loads the assets a
//! scene names: the palette the scene selects, the stroke profiles its elements
//! reference, and the fonts its text elements name. The two open-licensed fonts
//! Vectr ships are supplied for any scene with text, so a text element that
//! names no font renders with the default and a glyph the chosen font lacks is
//! covered by the fallback (FEAT-024, D-017, D-018).
//!
//! References resolve by identifier, not by file name: palettes live under
//! `palettes/`, stroke profiles under `strokes/`, and a user-supplied font is an
//! `Asset` document under `assets/` whose `path` points at the font file. A
//! document that cannot be read is a located diagnostic and no compilation
//! happens; nothing is substituted silently (NFR-011, FEAT-005).
//!
//! The loader is deterministic (NFR-010): directory entries are read in sorted
//! order and the bundled fonts are carried in a fixed order, so the same project
//! and scene always produce the same style context.

use std::fs;
use std::path::{Path, PathBuf};

use vectr_core::compiler::FONT;
use vectr_core::style::{UNDEFINED_GRADIENT, UNDEFINED_STROKE};
use vectr_core::{
    parse_gradient, parse_palette, parse_stroke_profile, validate_gradient, validate_palette,
    validate_stroke_profile, Diagnostic, DiagnosticCode, Diagnostics, ElementKind, FontAsset,
    Gradient, Location, PaintKind, Palette, Scene, StrokeProfile, StyleContext, DEFAULT_FONT_ID,
    FALLBACK_FONT_ID,
};

/// The project configuration that marks a directory as a project root.
const PROJECT_FILE: &str = "vectr.project.json";

/// The folder holding palette documents.
const PALETTE_DIR: &str = "palettes";

/// The folder holding stroke-profile documents.
const STROKE_DIR: &str = "strokes";

/// The folder holding gradient documents.
const GRADIENT_DIR: &str = "gradients";

/// The folder holding asset documents, including the fonts a scene may name.
const ASSET_DIR: &str = "assets";

/// A project style asset (a palette or stroke profile) could not be read or
/// parsed, so the project an input refers to is broken.
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
    strokes: Vec<StrokeProfile>,
    gradients: Vec<Gradient>,
    fonts: Vec<FontAsset>,
}

impl ProjectAssets {
    /// Loads the assets the project at `scene_path` provides for `scene`.
    ///
    /// A palette, stroke-profile, or asset document that cannot be read is
    /// reported rather than skipped, so a broken project fails loudly instead of
    /// compiling against stale or partial assets (NFR-011).
    pub fn load(scene_path: &Path, scene: &Scene) -> Result<Self, Diagnostics> {
        let root = project_root(scene_path);
        let mut diagnostics = Diagnostics::new();

        let palette = match scene.palette_id.as_deref() {
            Some(id) => match load_palette(&root, id) {
                Ok(palette) => Some(palette),
                Err(findings) => {
                    diagnostics.extend(findings);
                    None
                }
            },
            None => None,
        };

        let strokes = match load_strokes(&root) {
            Ok(strokes) => strokes,
            Err(findings) => {
                diagnostics.extend(findings);
                Vec::new()
            }
        };

        let gradients = match load_gradients(&root) {
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
            match load_font_assets(&root) {
                Ok(user) => fonts.extend(user),
                Err(findings) => diagnostics.extend(findings),
            }
        }

        if diagnostics.has_errors() {
            return Err(diagnostics);
        }
        Ok(Self {
            palette,
            strokes,
            gradients,
            fonts,
        })
    }

    /// The style context the compiler resolves the scene against (D-013).
    pub fn style_context(&self) -> StyleContext<'_> {
        StyleContext {
            palette: self.palette.as_ref(),
            strokes: &self.strokes,
            gradients: &self.gradients,
            fonts: &self.fonts,
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

    /// Checks a scene's stroke and font references against the loaded assets.
    ///
    /// A referenced stroke profile or font that no asset provides is an error
    /// naming the element, so an unresolvable reference fails before anything is
    /// compiled and never degrades to a missing stroke or a dropped run
    /// (FEAT-005, FEAT-024).
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

/// Finds the project root for a scene: the nearest ancestor holding the project
/// configuration, or the scene's own directory when there is none.
fn project_root(scene_path: &Path) -> PathBuf {
    let start = match scene_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let mut current: Option<&Path> = Some(start.as_path());
    while let Some(dir) = current {
        if dir.join(PROJECT_FILE).is_file() {
            return dir.to_path_buf();
        }
        current = dir.parent();
    }
    start
}

/// Loads the palette the scene selected by id.
fn load_palette(root: &Path, id: &str) -> Result<Palette, Diagnostics> {
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
/// font file is read as data and never executed (NFR-022).
fn load_font_assets(root: &Path) -> Result<Vec<FontAsset>, Diagnostics> {
    let dir = root.join(ASSET_DIR);
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
        let file = match asset_file(root, &path, relative) {
            Some(file) => file,
            None => {
                diagnostics.push(font_error(format!(
                    "font `{id}` names `{relative}`, which is not a file"
                )));
                continue;
            }
        };
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
    use crate::testing::TempDir;
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

        let assets = ProjectAssets::load(&scene_path, &scene).expect("loads");
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

        let assets = ProjectAssets::load(&scene_path, &scene).expect("loads");
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

        let assets = ProjectAssets::load(&scene_path, &scene).expect("loads");
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

        let diagnostics = ProjectAssets::load(&scene_path, &scene).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, STYLE_ASSET);
        assert!(error.message.contains("brand"), "{}", error.message);
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

        let assets = ProjectAssets::load(&scene_path, &scene).expect("loads");
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

        let assets = ProjectAssets::load(&scene_path, &scene).expect("loads");
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

        let assets = ProjectAssets::load(&scene_path, &scene).expect("loads");
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

        let assets = ProjectAssets::load(&scene_path, &scene).expect("loads");
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

        let diagnostics = ProjectAssets::load(&scene_path, &scene).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, FONT);
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

        let first = ProjectAssets::load(&scene_path, &scene).expect("loads");
        let second = ProjectAssets::load(&scene_path, &scene).expect("loads");
        let first_ids: Vec<&str> = first.strokes.iter().map(|p| p.id.as_str()).collect();
        let second_ids: Vec<&str> = second.strokes.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(first_ids, second_ids);
        assert_eq!(first_ids, vec!["a", "b"]);
    }
}
