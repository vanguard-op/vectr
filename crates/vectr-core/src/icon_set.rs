//! Icon-set rendering: every icon in a set rendered on its own under the set's
//! shared canvas, palette, recipe, and stroke profile (C-002, FEAT-025).
//!
//! [`export_icon_set`] resolves each icon's reusable definition, compiles it as
//! if placed once with no bindings under the set's style, and exports it in the
//! requested format. Every icon shares one canvas, one palette, one recipe, and
//! one stroke profile, so restyling the set restyles every icon together.
//!
//! The whole set is rendered before any output is produced: a definition that
//! does not resolve, a stroke that names a profile the set does not share, or a
//! failure in any one icon is reported and no icon is exported, never a partial
//! set (NFR-011). Rendering is deterministic: icons are processed in document
//! order and the same set and style yield byte-identical output (NFR-010).

use std::collections::HashSet;

use crate::compiler::expand::{UNRESOLVED_DEFINITION, UNUSED_DEFINITION};
use crate::compiler::{compile_with_style, model_bounds, StyleContext};
use crate::export::pdf::{self as pdf_export, PdfOptions};
use crate::export::png::{self as png_export, RasterOptions};
use crate::export::svg::{self as svg_export, SvgOptions};
use crate::render::RenderModel;
use crate::scene::{
    validate_icon_set, BoolValue, Definition, Diagnostic, DiagnosticCode, Diagnostics, Element,
    ElementKind, Geometry, IconEntry, IconSet, Location, NumberValue, Scene, Transform,
    CURRENT_FORMAT_VERSION,
};
use crate::style::UNUSED_GRADIENT;

/// An icon's element strokes with a profile other than the set's shared one.
pub const ICON_STROKE: DiagnosticCode = DiagnosticCode::new("E_ICON_STROKE");

/// An icon's geometry extends beyond the shared canvas, so detail may be lost.
pub const ICON_DETAIL: DiagnosticCode = DiagnosticCode::new("W_ICON_DETAIL");

/// How close to the canvas edge an icon may reach before it is reported.
const CANVAS_EPSILON: f64 = 1e-6;

/// The output format an icon set exports to (C-002, FEAT-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconFormat {
    /// A portable SVG document.
    #[default]
    Svg,
    /// A rasterized PNG image.
    Png,
    /// A vector PDF document.
    Pdf,
}

/// The options an icon-set export applies to every icon (C-002).
///
/// The canvas, palette, recipe, and stroke profile come from the set itself; the
/// options here carry only the output target and its size overrides, mirroring
/// the single-output exporters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportOptions {
    /// The output format.
    pub format: IconFormat,
    /// Output width override, in scene units.
    pub width: Option<f64>,
    /// Output height override, in scene units.
    pub height: Option<f64>,
    /// Pixel density multiplier; PNG only.
    pub density: Option<f64>,
    /// Background override, or `transparent`.
    pub background: Option<String>,
    /// The print colour profile a PDF export targets.
    pub profile: Option<String>,
}

/// One rendered icon, ready to write to a file (C-002).
#[derive(Debug, Clone, PartialEq)]
pub struct IconExport {
    /// The icon's name within the set.
    pub name: String,
    /// The icon's accessible name, when it declares one.
    pub accessible_name: Option<String>,
    /// The file name the icon exports to, from the set's naming pattern.
    pub file_name: String,
    /// The rendered output bytes.
    pub bytes: Vec<u8>,
    /// The warnings the icon's compile and export produced.
    pub diagnostics: Diagnostics,
}

/// Renders every icon in a set on its own under the set's shared style
/// (C-002, FEAT-025).
///
/// Each icon's definition is compiled as if placed once with no bindings, so its
/// parameters take their declared defaults and any definition it places resolves
/// through `definitions`. The set's stroke profile is enforced: every stroke an
/// icon draws must name it, and the profile must be present in the style
/// context. An unresolved definition, a mismatched stroke, or any icon's failure
/// is reported and no icon is returned (NFR-011).
pub fn export_icon_set(
    set: &IconSet,
    definitions: &[Definition],
    style: &StyleContext<'_>,
    options: &ExportOptions,
) -> Result<Vec<IconExport>, Diagnostics> {
    let mut diagnostics = validate_icon_set(set);
    if diagnostics.has_errors() {
        return Err(diagnostics);
    }

    // Resolve every icon's definition before anything is rendered, so a missing
    // definition is reported once for the set rather than after some icons were
    // already produced (NFR-011).
    let mut resolved: Vec<(&IconEntry, &Definition)> = Vec::with_capacity(set.icons.len());
    for (index, icon) in set.icons.iter().enumerate() {
        match definitions
            .iter()
            .find(|definition| definition.id == icon.definition_ref)
        {
            Some(definition) => resolved.push((icon, definition)),
            None => diagnostics.push(
                Diagnostic::error(
                    UNRESOLVED_DEFINITION,
                    format!(
                        "icon `{}` references definition `{}`, which the project does not provide",
                        icon.name, icon.definition_ref
                    ),
                )
                .at_path(format!("/icons/{index}/definitionRef")),
            ),
        }
    }

    // The set shares one stroke profile: the profile must exist, and every
    // stroke an icon draws must name it (docs/Vectr/schema.md, "IconSet").
    if !style
        .strokes
        .iter()
        .any(|profile| profile.id == set.stroke_profile_id)
    {
        diagnostics.push(
            Diagnostic::error(
                ICON_STROKE,
                format!(
                    "icon set `{}` names stroke profile `{}`, which the project does not provide",
                    set.id, set.stroke_profile_id
                ),
            )
            .at_path("/strokeProfileId"),
        );
    } else {
        for (icon, definition) in &resolved {
            let mut visited = HashSet::new();
            check_icon_stroke(
                &mut diagnostics,
                set,
                icon,
                definition,
                definitions,
                &mut visited,
            );
        }
    }

    if diagnostics.has_errors() {
        return Err(diagnostics);
    }

    // The set's own definitions resolve the icons' nested placements, so the
    // context is pinned to the definitions the caller supplied.
    let context = StyleContext {
        definitions,
        ..*style
    };

    let mut exports = Vec::with_capacity(resolved.len());
    for (icon, definition) in resolved {
        let scene = icon_scene(set, icon, definition);
        let model = match compile_with_style(&scene, &context) {
            Ok(model) => model,
            Err(errors) => {
                diagnostics.extend(errors);
                continue;
            }
        };

        // An icon renders in isolation, so a definition the set's other icons
        // place — or a gradient the project carries but this icon does not
        // reference — is not unused and must not be reported against it
        // (FEAT-025, FEAT-030, FEAT-031).
        let mut warnings: Diagnostics = model
            .diagnostics
            .iter()
            .filter(|finding| finding.code != UNUSED_DEFINITION && finding.code != UNUSED_GRADIENT)
            .cloned()
            .collect();
        if let Some(warning) = icon_detail_warning(set, icon, &model) {
            warnings.push(warning);
        }

        match export_icon(&model, options, &mut warnings) {
            Ok(bytes) => exports.push(IconExport {
                name: icon.name.clone(),
                accessible_name: icon.accessible_name.clone(),
                file_name: set.file_name_for(icon, extension(options.format)),
                bytes,
                diagnostics: warnings,
            }),
            Err(errors) => diagnostics.extend(errors),
        }
    }

    if diagnostics.has_errors() {
        Err(diagnostics)
    } else {
        Ok(exports)
    }
}

/// Refuses a stroke an icon draws that names a profile the set does not share.
///
/// A definition's own elements and any definition it places are checked, so a
/// mismatched stroke written inside a nested part is reported like one written
/// directly (FEAT-030).
fn check_icon_stroke(
    diagnostics: &mut Diagnostics,
    set: &IconSet,
    icon: &IconEntry,
    definition: &Definition,
    definitions: &[Definition],
    visited: &mut HashSet<String>,
) {
    if !visited.insert(definition.id.clone()) {
        return;
    }
    for element in &definition.elements {
        if let Some(stroke) = &element.stroke {
            if stroke.profile_id != set.stroke_profile_id {
                diagnostics.push(
                    Diagnostic::error(
                        ICON_STROKE,
                        format!(
                            "icon `{}` element `{}` strokes with profile `{}`, but the set shares `{}`",
                            icon.name, element.id, stroke.profile_id, set.stroke_profile_id
                        ),
                    )
                    .with_location(Location::element_at(
                        element.id.clone(),
                        "/stroke/profileId",
                    )),
                );
            }
        }
        if element.kind == ElementKind::Instance {
            if let Some(reference) = element.definition_ref.as_deref() {
                if let Some(nested) = definitions
                    .iter()
                    .find(|candidate| candidate.id == reference)
                {
                    check_icon_stroke(diagnostics, set, icon, nested, definitions, visited);
                }
            }
        }
    }
}

/// The synthetic scene one icon renders as: the set's canvas and style, with
/// the icon's definition placed once with no bindings.
fn icon_scene(set: &IconSet, icon: &IconEntry, definition: &Definition) -> Scene {
    let instance = Element {
        id: definition.id.clone(),
        scene_id: Some(definition.id.clone()),
        definition_id: None,
        parent_id: None,
        order: 0,
        name: Some(icon.name.clone()),
        accessible_name: icon.accessible_name.clone(),
        kind: ElementKind::Instance,
        geometry: Geometry::default(),
        transform: Transform {
            translate_x: NumberValue::Literal(0.0),
            translate_y: NumberValue::Literal(0.0),
            rotate: NumberValue::Literal(0.0),
            scale_x: NumberValue::Literal(1.0),
            scale_y: NumberValue::Literal(1.0),
            skew_x: None,
            skew_y: None,
        },
        fill: None,
        stroke: None,
        font_id: None,
        opacity: NumberValue::Literal(1.0),
        visible: BoolValue::Literal(true),
        definition_ref: Some(definition.id.clone()),
        bindings: None,
    };

    Scene {
        id: format!("{}-{}", set.id, icon.name),
        project_id: set.project_id.clone(),
        name: icon.name.clone(),
        format_version: CURRENT_FORMAT_VERSION.to_string(),
        canvas: set.canvas.clone(),
        palette_id: set.palette_id.clone(),
        recipe_id: set.recipe_id.clone(),
        seed: None,
        title: Some(icon.name.clone()),
        description: icon.accessible_name.clone(),
        elements: vec![instance],
        constraints: None,
    }
}

/// Warns when an icon's geometry reaches beyond the shared canvas, where its
/// detail would be clipped rather than shown (FEAT-025).
fn icon_detail_warning(set: &IconSet, icon: &IconEntry, model: &RenderModel) -> Option<Diagnostic> {
    let (min, max) = model_bounds(model)?;
    let canvas = &set.canvas;
    let overflows = max[0] > canvas.width + CANVAS_EPSILON
        || max[1] > canvas.height + CANVAS_EPSILON
        || min[0] < -CANVAS_EPSILON
        || min[1] < -CANVAS_EPSILON;
    overflows.then(|| {
        Diagnostic::warning(
            ICON_DETAIL,
            format!(
                "icon `{}` extends beyond the {}x{} set canvas; detail may be lost",
                icon.name, canvas.width, canvas.height
            ),
        )
    })
}

/// Exports one compiled icon to bytes in the requested format.
fn export_icon(
    model: &RenderModel,
    options: &ExportOptions,
    warnings: &mut Diagnostics,
) -> Result<Vec<u8>, Diagnostics> {
    match options.format {
        IconFormat::Svg => {
            let options = SvgOptions {
                width: options.width,
                height: options.height,
                background: options.background.clone(),
            };
            let export = svg_export::export_svg_reporting(model, &options)?;
            warnings.extend(export.diagnostics);
            Ok(export.svg.into_bytes())
        }
        IconFormat::Png => {
            let options = RasterOptions {
                width: options.width,
                height: options.height,
                density: options.density,
                background: options.background.clone(),
            };
            let export = png_export::export_png_reporting(model, &options)?;
            warnings.extend(export.diagnostics);
            Ok(export.png)
        }
        IconFormat::Pdf => {
            let options = PdfOptions {
                page_width: options.width,
                page_height: options.height,
                profile: options.profile.clone(),
                background: options.background.clone(),
            };
            let export = pdf_export::export_pdf_reporting(model, &options)?;
            warnings.extend(export.diagnostics);
            Ok(export.pdf)
        }
    }
}

/// The file extension a format writes.
fn extension(format: IconFormat) -> &'static str {
    match format {
        IconFormat::Svg => "svg",
        IconFormat::Png => "png",
        IconFormat::Pdf => "pdf",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{parse_definition, parse_icon_set};
    use crate::style::{parse_palette, parse_stroke_profile};

    const TRANSFORM: &str =
        r#""transform":{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}"#;

    fn definition(id: &str, element: &str) -> Definition {
        parse_definition(&format!(
            r#"{{"id":"{id}","projectId":"p","name":"{id}","parameters":[],"origin":{{"x":0,"y":0}},"elements":[{element}]}}"#
        ))
        .expect("a valid definition")
    }

    fn rect(
        id: &str,
        owner: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        stroke: &str,
    ) -> String {
        format!(
            r#"{{"id":"{id}","definitionId":"{owner}","order":0,"kind":"rect","geometry":{{"x":{x},"y":{y},"width":{width},"height":{height}}},{TRANSFORM},"fill":{{"kind":"token","ref":"ink"}},"opacity":1,"visible":true{stroke}}}"#
        )
    }

    fn set(icons: &[(&str, &str)], pattern: Option<&str>, canvas: f64) -> IconSet {
        let entries: Vec<String> = icons
            .iter()
            .map(|(name, reference)| {
                format!(r#"{{"name":"{name}","definitionRef":"{reference}"}}"#)
            })
            .collect();
        let pattern = pattern
            .map(|pattern| format!(r#","namePattern":"{pattern}""#))
            .unwrap_or_default();
        parse_icon_set(&format!(
            r##"{{"id":"ui","projectId":"p","name":"UI","canvas":{{"width":{canvas},"height":{canvas},"background":"transparent"}},"strokeProfileId":"line","paletteId":"brand"{pattern},"icons":[{}]}}"##,
            entries.join(",")
        ))
        .expect("a valid icon set")
    }

    fn style<'a>(
        palette: &'a crate::style::Palette,
        strokes: &'a [crate::style::StrokeProfile],
    ) -> StyleContext<'a> {
        StyleContext {
            palette: Some(palette),
            strokes,
            gradients: &[],
            fonts: &[],
            recipe: None,
            definitions: &[],
        }
    }

    fn palette() -> crate::style::Palette {
        parse_palette(
            r##"{"id":"brand","projectId":"p","name":"Brand","tokens":[{"name":"ink","value":"#111111"}]}"##,
        )
        .expect("a palette")
    }

    fn stroke_profile(id: &str) -> crate::style::StrokeProfile {
        parse_stroke_profile(&format!(
            r#"{{"id":"{id}","projectId":"p","name":"{id}","width":2,"cap":"butt","join":"miter"}}"#
        ))
        .expect("a stroke profile")
    }

    fn gradient(id: &str) -> crate::style::Gradient {
        crate::style::parse_gradient(&format!(
            r##"{{"id":"{id}","projectId":"p","name":"{id}","type":"linear","stops":[{{"offset":0,"token":"ink"}},{{"offset":1,"token":"ink"}}]}}"##
        ))
        .expect("a gradient")
    }

    #[test]
    fn every_icon_exports_to_its_own_named_file() {
        let definitions = [
            definition("plus", &rect("plus-body", "plus", 0.0, 0.0, 10.0, 10.0, "")),
            definition(
                "minus",
                &rect("minus-body", "minus", 0.0, 0.0, 10.0, 10.0, ""),
            ),
        ];
        let set = set(&[("plus", "plus"), ("minus", "minus")], None, 24.0);
        let palette = palette();
        let strokes = [stroke_profile("line")];
        let options = ExportOptions {
            format: IconFormat::Svg,
            ..Default::default()
        };

        let exports = export_icon_set(&set, &definitions, &style(&palette, &strokes), &options)
            .expect("exports");

        assert_eq!(exports.len(), 2);
        assert_eq!(exports[0].file_name, "plus.svg");
        assert_eq!(exports[1].file_name, "minus.svg");
        assert!(
            exports[0].bytes.windows(4).any(|window| window == b"<svg"),
            "an SVG document"
        );
    }

    #[test]
    fn a_naming_pattern_names_every_file() {
        let definitions = [definition(
            "plus",
            &rect("plus-body", "plus", 0.0, 0.0, 10.0, 10.0, ""),
        )];
        let set = set(&[("plus", "plus")], Some("icon-{name}-24"), 24.0);
        let palette = palette();
        let strokes = [stroke_profile("line")];
        let exports = export_icon_set(
            &set,
            &definitions,
            &style(&palette, &strokes),
            &ExportOptions::default(),
        )
        .expect("exports");
        assert_eq!(exports[0].file_name, "icon-plus-24.svg");
    }

    #[test]
    fn every_icon_shares_the_sets_canvas_and_style() {
        let definitions = [
            definition("a", &rect("a-body", "a", 0.0, 0.0, 10.0, 10.0, "")),
            definition("b", &rect("b-body", "b", 2.0, 2.0, 10.0, 10.0, "")),
        ];
        let set = set(&[("a", "a"), ("b", "b")], None, 48.0);
        let palette = palette();
        let strokes = [stroke_profile("line")];
        let exports = export_icon_set(
            &set,
            &definitions,
            &style(&palette, &strokes),
            &ExportOptions::default(),
        )
        .expect("exports");

        for export in &exports {
            let svg = String::from_utf8(export.bytes.clone()).expect("utf-8 svg");
            assert!(
                svg.contains("viewBox=\"0 0 48 48\""),
                "shared canvas: {svg}"
            );
            assert!(svg.contains("#111111"), "shared palette: {svg}");
        }
    }

    #[test]
    fn a_restyle_updates_every_icon_together() {
        let definitions = [
            definition("a", &rect("a-body", "a", 0.0, 0.0, 10.0, 10.0, "")),
            definition("b", &rect("b-body", "b", 2.0, 2.0, 10.0, 10.0, "")),
        ];
        let set = set(&[("a", "a"), ("b", "b")], None, 48.0);
        let strokes = [stroke_profile("line")];

        let first = palette();
        let first = export_icon_set(
            &set,
            &definitions,
            &style(&first, &strokes),
            &ExportOptions::default(),
        )
        .expect("exports");

        let second = parse_palette(
            r##"{"id":"brand","projectId":"p","name":"Brand","tokens":[{"name":"ink","value":"#ff0000"}]}"##,
        )
        .expect("a palette");
        let second = export_icon_set(
            &set,
            &definitions,
            &style(&second, &strokes),
            &ExportOptions::default(),
        )
        .expect("exports");

        for (before, after) in first.iter().zip(second.iter()) {
            let before = String::from_utf8(before.bytes.clone()).expect("svg");
            let after = String::from_utf8(after.bytes.clone()).expect("svg");
            assert!(before.contains("#111111"));
            assert!(after.contains("#ff0000"), "the restyle reaches {after}");
        }
    }

    #[test]
    fn an_icon_does_not_warn_about_the_sets_other_definitions() {
        let definitions = [
            definition("plus", &rect("plus-body", "plus", 0.0, 0.0, 10.0, 10.0, "")),
            definition(
                "minus",
                &rect("minus-body", "minus", 0.0, 0.0, 10.0, 10.0, ""),
            ),
        ];
        let set = set(&[("plus", "plus"), ("minus", "minus")], None, 24.0);
        let palette = palette();
        let strokes = [stroke_profile("line")];

        let exports = export_icon_set(
            &set,
            &definitions,
            &style(&palette, &strokes),
            &ExportOptions::default(),
        )
        .expect("exports");

        for export in &exports {
            assert!(
                export
                    .diagnostics
                    .iter()
                    .all(|finding| finding.code != UNUSED_DEFINITION),
                "an isolated icon must not report the set's other definitions: {:?}",
                export.diagnostics
            );
        }
    }

    #[test]
    fn an_icon_does_not_warn_about_the_sets_other_gradients() {
        let definitions = [definition(
            "plus",
            &rect("plus-body", "plus", 0.0, 0.0, 10.0, 10.0, ""),
        )];
        let set = set(&[("plus", "plus")], None, 24.0);
        let palette = palette();
        let strokes = [stroke_profile("line")];
        let gradients = [gradient("halo")];

        let mut context = style(&palette, &strokes);
        context.gradients = &gradients;

        let exports = export_icon_set(&set, &definitions, &context, &ExportOptions::default())
            .expect("exports");

        for export in &exports {
            assert!(
                export
                    .diagnostics
                    .iter()
                    .all(|finding| finding.code != UNUSED_GRADIENT),
                "an isolated icon must not report the set's other gradients: {:?}",
                export.diagnostics
            );
        }
    }

    #[test]
    fn an_unresolved_definition_is_reported_and_nothing_is_exported() {
        let definitions = [definition(
            "plus",
            &rect("plus-body", "plus", 0.0, 0.0, 10.0, 10.0, ""),
        )];
        let set = set(&[("plus", "plus"), ("ghost", "absent")], None, 24.0);
        let palette = palette();
        let strokes = [stroke_profile("line")];

        let diagnostics = export_icon_set(
            &set,
            &definitions,
            &style(&palette, &strokes),
            &ExportOptions::default(),
        )
        .expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.code == UNRESOLVED_DEFINITION && error.message.contains("absent")));
    }

    #[test]
    fn a_stroke_that_names_a_different_profile_is_refused() {
        let definitions = [definition(
            "plus",
            &rect(
                "plus-body",
                "plus",
                0.0,
                0.0,
                10.0,
                10.0,
                r#","stroke":{"profileId":"bold","paint":{"kind":"token","ref":"ink"}}"#,
            ),
        )];
        let set = set(&[("plus", "plus")], None, 24.0);
        let palette = palette();
        let strokes = [stroke_profile("line"), stroke_profile("bold")];

        let diagnostics = export_icon_set(
            &set,
            &definitions,
            &style(&palette, &strokes),
            &ExportOptions::default(),
        )
        .expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.code == ICON_STROKE && error.message.contains("bold")));
    }

    #[test]
    fn an_icon_that_overflows_the_canvas_warns_that_detail_may_be_lost() {
        let definitions = [definition(
            "wide",
            &rect("wide-body", "wide", 0.0, 0.0, 40.0, 10.0, ""),
        )];
        let set = set(&[("wide", "wide")], None, 24.0);
        let palette = palette();
        let strokes = [stroke_profile("line")];

        let exports = export_icon_set(
            &set,
            &definitions,
            &style(&palette, &strokes),
            &ExportOptions::default(),
        )
        .expect("exports");
        assert!(exports[0]
            .diagnostics
            .warnings()
            .any(|warning| warning.code == ICON_DETAIL));
    }

    #[test]
    fn a_duplicate_icon_name_is_refused_at_parse() {
        let diagnostics = parse_icon_set(
            r##"{"id":"ui","projectId":"p","name":"UI","canvas":{"width":24,"height":24,"background":"transparent"},"strokeProfileId":"line","icons":[{"name":"plus","definitionRef":"a"},{"name":"plus","definitionRef":"b"}]}"##,
        )
        .expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.code == DiagnosticCode::DUPLICATE_NAME));
    }

    #[test]
    fn an_empty_icon_list_is_refused_at_parse() {
        let diagnostics = parse_icon_set(
            r##"{"id":"ui","projectId":"p","name":"UI","canvas":{"width":24,"height":24,"background":"transparent"},"strokeProfileId":"line","icons":[]}"##,
        )
        .expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.code == DiagnosticCode::SCHEMA));
    }

    #[test]
    fn a_naming_pattern_without_the_placeholder_is_refused_at_parse() {
        let diagnostics = parse_icon_set(
            r##"{"id":"ui","projectId":"p","name":"UI","canvas":{"width":24,"height":24,"background":"transparent"},"strokeProfileId":"line","namePattern":"icon","icons":[{"name":"plus","definitionRef":"a"}]}"##,
        )
        .expect_err("refused");
        assert!(diagnostics
            .errors()
            .any(|error| error.code == DiagnosticCode::SCHEMA));
    }

    #[test]
    fn the_naming_pattern_defaults_to_the_icon_name() {
        let set = set(&[("plus", "a")], None, 24.0);
        assert_eq!(set.name_pattern(), "{name}");
    }
}
