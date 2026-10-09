//! Icon-set export for `vectr icon-set export` (C-004, FEAT-025).
//!
//! A project's icon sets live as documents under `icon-sets/`, one per set
//! named for its identifier, parallel to its scenes and definitions. The command
//! resolves the set, loads the style it shares, renders every icon in process
//! through [`vectr_core::export_icon_set`], and writes each to a file named by
//! the set's naming pattern under the output directory.
//!
//! Every icon is rendered before any file is written, so a set that fails to
//! resolve or render leaves no partial output (NFR-011). An identifier that is
//! not a plain file stem is refused so a set cannot name its way out of its
//! project (NFR-021, NFR-024).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use vectr_core::{
    export_icon_set, parse_icon_set, Canvas, Definition, Diagnostic, DiagnosticCode, Diagnostics,
    ElementKind, ExportOptions, IconFormat, IconSet, PaintKind, PaintValue, Scene,
    CURRENT_FORMAT_VERSION,
};
use vectr_project::{project_root_from, ProjectAssets, STYLE_ASSET};

use crate::cli::{
    asset_exit_code, diagnostics_text, export_exit_code, Format, Report, EXIT_OUTPUT, EXIT_SUCCESS,
    EXIT_USAGE,
};
use crate::output::write_atomic;

/// The folder holding icon-set documents (FEAT-025).
const ICON_SET_DIR: &str = "icon-sets";

/// An icon set that cannot be read or resolved.
pub const ICON_SET: DiagnosticCode = DiagnosticCode::new("E_ICON_SET");

/// Renders and writes every icon in the resolved set (FEAT-025).
pub fn export(cwd: &Path, requested: Option<&str>, format: Format, out_dir: &Path) -> Report {
    let root = project_root_from(cwd);

    let sets = match load_icon_sets(&root) {
        Ok(sets) => sets,
        Err(diagnostics) => return Report::failure(EXIT_USAGE, diagnostics_text(&diagnostics)),
    };
    let set = match select_set(&sets, requested, &root) {
        Ok(set) => set,
        Err(diagnostics) => return Report::failure(EXIT_USAGE, diagnostics_text(&diagnostics)),
    };

    // The set carries the palette and recipe its icons resolve against; a
    // synthetic scene hands those selections to the shared project loader, so
    // the icon set and a scene resolve their assets the same way (FEAT-016).
    let assets = match ProjectAssets::load(&root, &style_scene(set)) {
        Ok(assets) => assets,
        Err(diagnostics) => {
            return Report::failure(
                asset_exit_code(&diagnostics),
                diagnostics_text(&diagnostics),
            )
        }
    };

    // A set that names no palette and a project that names no default leaves
    // token paints unresolvable; report the missing palette rather than letting
    // the compiler report an unresolved token (FEAT-005, D-039).
    if assets.palette().is_none() && set_uses_tokens(set, assets.definitions()) {
        let diagnostics = Diagnostics::from(
            Diagnostic::error(
                STYLE_ASSET,
                format!(
                    "icon set `{}` renders token paints, but the project at `{}` names no default palette and the set names none",
                    set.id,
                    root.display()
                ),
            )
            .at_path("/paletteId"),
        );
        return Report::failure(EXIT_USAGE, diagnostics_text(&diagnostics));
    }

    let options = ExportOptions {
        format: icon_format(format),
        width: None,
        height: None,
        density: None,
        background: None,
        profile: None,
    };
    let exports =
        match export_icon_set(set, assets.definitions(), &assets.style_context(), &options) {
            Ok(exports) => exports,
            Err(diagnostics) => {
                return Report::failure(
                    export_exit_code(&diagnostics),
                    diagnostics_text(&diagnostics),
                )
            }
        };

    write_exports(out_dir, &exports)
}

/// Writes every rendered icon, reporting each path and its warnings.
///
/// Every icon is already rendered, so the only failure left is an unwritable
/// destination; a file name that would escape the output directory is refused
/// before anything is written (NFR-024).
fn write_exports(out_dir: &Path, exports: &[vectr_core::IconExport]) -> Report {
    for export in exports {
        if !is_plain_file_name(&export.file_name) {
            let diagnostics = Diagnostics::from(
                Diagnostic::error(
                    ICON_SET,
                    format!(
                        "icon `{}` exports to `{}`, which is not a plain file name",
                        export.name, export.file_name
                    ),
                )
                .at_path("/namePattern"),
            );
            return Report::failure(EXIT_USAGE, diagnostics_text(&diagnostics));
        }
    }

    let mut stdout = String::new();
    let mut stderr = String::new();
    for export in exports {
        let path = out_dir.join(&export.file_name);
        match write_atomic(&path, &export.bytes) {
            Ok(()) => {
                stdout.push_str(&format!("wrote {}\n", path.display()));
                stderr.push_str(&diagnostics_text(&export.diagnostics));
            }
            Err(error) => {
                return Report::failure(
                    EXIT_OUTPUT,
                    format!(
                        "{stderr}error: cannot write output `{}`: {error}\n",
                        path.display()
                    ),
                );
            }
        }
    }
    Report {
        code: EXIT_SUCCESS,
        stdout,
        stderr,
    }
}

/// Loads every icon-set document the project provides, in a fixed order.
fn load_icon_sets(root: &Path) -> Result<Vec<IconSet>, Diagnostics> {
    let dir = root.join(ICON_SET_DIR);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && has_json_extension(path))
        .collect();
    paths.sort();

    let mut sets = Vec::new();
    let mut diagnostics = Diagnostics::new();
    for path in paths {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                diagnostics.push(Diagnostic::error(
                    ICON_SET,
                    format!("cannot read icon set `{}`: {error}", path.display()),
                ));
                continue;
            }
        };
        match parse_icon_set(&source) {
            Ok(set) => sets.push(set),
            Err(findings) => {
                let mut combined = Diagnostics::from(Diagnostic::error(
                    ICON_SET,
                    format!("invalid icon set `{}`", path.display()),
                ));
                combined.extend(findings);
                diagnostics.extend(combined);
            }
        }
    }

    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(sets)
    }
}

/// Resolves the requested icon set, or the project's only set when none is
/// named (FEAT-025).
fn select_set<'a>(
    sets: &'a [IconSet],
    requested: Option<&str>,
    root: &Path,
) -> Result<&'a IconSet, Diagnostics> {
    match requested {
        Some(id) => sets.iter().find(|set| set.id == id).ok_or_else(|| {
            Diagnostics::from(Diagnostic::error(
                ICON_SET,
                format!(
                    "icon set `{id}` resolves to no document in the project at `{}`",
                    root.display()
                ),
            ))
        }),
        None => match sets {
            [only] => Ok(only),
            [] => Err(Diagnostics::from(Diagnostic::error(
                ICON_SET,
                format!(
                    "the project at `{}` declares no icon set; name one or add a document under `{ICON_SET_DIR}/`",
                    root.display()
                ),
            ))),
            many => Err(Diagnostics::from(Diagnostic::error(
                ICON_SET,
                format!(
                    "the project at `{}` holds {} icon sets; name the one to export",
                    root.display(),
                    many.len()
                ),
            ))),
        },
    }
}

/// The synthetic scene that carries the set's palette and recipe selections to
/// the project loader, which resolves assets the same way for a scene.
fn style_scene(set: &IconSet) -> Scene {
    Scene {
        id: set.id.clone(),
        project_id: set.project_id.clone(),
        name: set.name.clone(),
        format_version: CURRENT_FORMAT_VERSION.to_string(),
        canvas: Canvas {
            width: set.canvas.width,
            height: set.canvas.height,
            background: set.canvas.background.clone(),
        },
        palette_id: set.palette_id.clone(),
        recipe_id: set.recipe_id.clone(),
        seed: None,
        title: None,
        description: None,
        elements: Vec::new(),
        constraints: None,
    }
}

/// Whether any icon's definition draws a palette token paint, directly or
/// through a definition it places.
fn set_uses_tokens(set: &IconSet, definitions: &[Definition]) -> bool {
    set.icons.iter().any(|icon| {
        definitions
            .iter()
            .filter(|definition| definition.id == icon.definition_ref)
            .any(|definition| {
                let mut visited = HashSet::new();
                definition_uses_tokens(definition, definitions, &mut visited)
            })
    })
}

/// Whether a definition, or a definition it places, paints with a palette
/// token.
fn definition_uses_tokens(
    definition: &Definition,
    definitions: &[Definition],
    visited: &mut HashSet<String>,
) -> bool {
    if !visited.insert(definition.id.clone()) {
        return false;
    }
    definition.elements.iter().any(|element| {
        let fill = element
            .fill
            .as_ref()
            .and_then(PaintValue::literal)
            .is_some_and(|paint| paint.kind == PaintKind::Token);
        let stroke = element
            .stroke
            .as_ref()
            .and_then(|stroke| stroke.paint.literal())
            .is_some_and(|paint| paint.kind == PaintKind::Token);
        fill || stroke
            || (element.kind == ElementKind::Instance
                && element
                    .definition_ref
                    .as_deref()
                    .and_then(|reference| {
                        definitions
                            .iter()
                            .find(|candidate| candidate.id == reference)
                    })
                    .is_some_and(|nested| definition_uses_tokens(nested, definitions, visited)))
    })
}

/// The core export format the CLI format names.
fn icon_format(format: Format) -> IconFormat {
    match format {
        Format::Svg => IconFormat::Svg,
        Format::Png => IconFormat::Png,
        Format::Pdf => IconFormat::Pdf,
    }
}

/// Whether a path is a plain file name: no separators and no `.`/`..` segment.
fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

/// Whether a path carries a `.json` extension, whatever its case.
fn has_json_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}
