//! Acceptance tests for raster-assisted layers (FEAT-015) and the raster cases
//! of scene validation (FEAT-018, C-001).
//!
//! Drives the shipped library, the `vectr` binary and the `vectr-mcp` server:
//! a raster layer composites into PNG at its placement and size, in paint
//! order, over or under the vector artwork; a vector target omits it with a
//! warning naming the element and its asset, staying pure vector; a recipe that
//! designates raster shading is fulfilled by the scene's raster layers or its
//! unhonored request is reported; and each raster reference failure — missing,
//! wrong kind, a file that is not a PNG, an asset outside its scene, a raster
//! inside a reusable definition, or a raster as a composition operand — is
//! reported with its location before anything renders. The `enable_raster_layers`
//! rollout flag (off by default) gates the capability at the front ends.

mod common;

use common::*;
use serde_json::{json, Value};
use vectr_core::export::pdf::{export_pdf_reporting, PdfOptions};
use vectr_core::export::png::{export_png_reporting, RasterOptions};
use vectr_core::export::raster::{LARGE, MAX_RASTER_BYTES, MAX_RASTER_SIDE, OMITTED};
use vectr_core::export::svg::{export_svg_reporting, SvgOptions};
use vectr_core::style::TEXTURE_UNSUPPORTED;
use vectr_core::{
    compile_with_style, parse, parse_definition, parse_style_recipe, Diagnostic, DiagnosticCode,
    Diagnostics, Palette, RasterAsset, StyleContext, StyleRecipe,
};
use vectr_project::ProjectAssets;

/// The environment variable that enables the raster-layers rollout flag at the
/// front ends; off unless it names one of the documented truthy values
/// (FEAT-015, C-004, C-005).
const RASTER_FLAG: &str = "VECTR_ENABLE_RASTER_LAYERS";

/// A raster element that names its asset and occupies its bounding box
/// (FEAT-015, C-001).
fn raster_layer(id: &str, order: i64, asset_id: &str, x: f64, y: f64, w: f64, h: f64) -> Value {
    let mut value = element(
        id,
        order,
        "raster",
        json!({ "x": x, "y": y, "width": w, "height": h }),
    );
    value["assetId"] = json!(asset_id);
    value
}

/// An 8-bit RGBA PNG of one solid colour, so the compositing path decodes a real
/// image rather than placeholder bytes (FEAT-013, NFR-022).
fn solid_png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("a pixmap");
    pixmap.fill(tiny_skia::Color::from_rgba8(
        rgba[0], rgba[1], rgba[2], rgba[3],
    ));
    pixmap.encode_png().expect("encodes a PNG")
}

/// A PNG of `width` by `height` deterministic noise, used where an asset must
/// exceed the byte budget while staying within the pixel budget (FEAT-015).
fn noise_png(width: u32, height: u32) -> Vec<u8> {
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("a pixmap");
    let mut state: u32 = 0x1234_5678;
    let mut next = || {
        // A xorshift generator, seeded so the fixture is reproducible (NFR-010).
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };
    for pixel in pixmap.data_mut().as_chunks_mut::<4>().0 {
        pixel[0] = next() as u8;
        pixel[1] = next() as u8;
        pixel[2] = next() as u8;
        pixel[3] = 255;
    }
    pixmap.encode_png().expect("encodes a PNG")
}

/// The style context a scene compiles against with raster layers enabled.
fn style<'a>(
    palette: Option<&'a Palette>,
    recipe: Option<&'a StyleRecipe>,
    rasters: &'a [RasterAsset],
) -> StyleContext<'a> {
    StyleContext {
        palette,
        strokes: &[],
        gradients: &[],
        fonts: &[],
        recipe,
        definitions: &[],
        rasters,
        raster_layers: true,
    }
}

/// A project palette with the two tokens the layered scene resolves.
fn layered_palette() -> Palette {
    vectr_core::parse_palette(&palette(
        "brand",
        &[("red", "#ff0000"), ("green", "#00ff00")],
    ))
    .expect("a palette")
}

/// A scene that places a red rect under a blue raster layer under a green rect,
/// so a single PNG proves placement, size, and paint order (FEAT-015).
fn layered_scene() -> Value {
    let mut under = rect("under", 0, 0.0, 0.0, 60.0, 60.0);
    under["fill"] = token_paint("red");
    let mut over = rect("over", 2, 60.0, 60.0, 20.0, 20.0);
    over["fill"] = token_paint("green");
    let mut document = scene(vec![
        under,
        raster_layer("photo", 1, "photo", 40.0, 40.0, 40.0, 40.0),
        over,
    ]);
    document["canvas"] = json!({ "width": 100.0, "height": 100.0, "background": "#ffffff" });
    document
}

/// A blue raster asset the layered scene places.
fn blue_asset() -> RasterAsset {
    RasterAsset::new("photo", "Photo", solid_png(8, 8, [0, 0, 255, 255]))
}

/// The pixel's demultiplied (red, green, blue, alpha) at `(x, y)`.
fn pixel(bytes: &[u8], x: u32, y: u32) -> (u8, u8, u8, u8) {
    let image = tiny_skia::Pixmap::decode_png(bytes).expect("a PNG");
    let pixel = image.pixel(x, y).expect("in bounds").demultiply();
    (pixel.red(), pixel.green(), pixel.blue(), pixel.alpha())
}

fn export_png(model: &vectr_core::RenderModel) -> Vec<u8> {
    export_png_reporting(model, &RasterOptions::default())
        .expect("exports")
        .png
}

// ---------------------------------------------------------------------------
// Raster layers in PNG output (FEAT-015)
// ---------------------------------------------------------------------------

#[test]
fn a_raster_layer_composites_at_its_placement_and_size_in_paint_order() {
    let brand = layered_palette();
    let rasters = [blue_asset()];
    let model = compile_with_style(
        &parse_scene(&layered_scene()),
        &style(Some(&brand), None, &rasters),
    )
    .expect("compiles");

    // Paint order follows `order`: the rect under the layer, the layer, then the
    // rect over it.
    let kinds: Vec<&str> = model.nodes.iter().map(|node| node.kind.as_str()).collect();
    assert_eq!(kinds, vec!["rect", "raster", "rect"]);

    let bytes = export_png(&model);

    // The under-rect shows where the layer does not cover it; the layer shows
    // over the under-rect; the over-rect shows over the layer; and the canvas
    // background shows outside both (FEAT-015).
    assert_eq!(pixel(&bytes, 10, 10), (255, 0, 0, 255), "under the layer");
    assert_eq!(
        pixel(&bytes, 45, 45),
        (0, 0, 255, 255),
        "the layer over the rect"
    );
    assert_eq!(
        pixel(&bytes, 70, 70),
        (0, 255, 0, 255),
        "the rect over the layer"
    );
    assert_eq!(
        pixel(&bytes, 85, 85),
        (255, 255, 255, 255),
        "the background"
    );

    // The layer occupies its declared box and no more: 40..80 on both axes.
    assert_eq!(
        pixel(&bytes, 79, 45),
        (0, 0, 255, 255),
        "the layer's far edge"
    );
    assert_eq!(
        pixel(&bytes, 81, 81),
        (255, 255, 255, 255),
        "outside the layer"
    );
}

#[test]
fn a_raster_layer_is_omitted_from_svg_with_a_warning_naming_element_and_asset() {
    let brand = layered_palette();
    let rasters = [blue_asset()];
    let model = compile_with_style(
        &parse_scene(&layered_scene()),
        &style(Some(&brand), None, &rasters),
    )
    .expect("compiles");

    let export = export_svg_reporting(&model, &SvgOptions::default()).expect("exports");

    // The vector target is pure vector: no image element and no raster content
    // (FEAT-012, FEAT-015).
    assert!(!export.svg.contains("<image"), "{}", export.svg);
    assert!(export.svg.contains("<svg"), "well-formed: {}", export.svg);
    assert!(
        export.svg.contains("#ff0000"),
        "the rest renders: {}",
        export.svg
    );

    let warning = export
        .diagnostics
        .warnings()
        .find(|finding| finding.code == OMITTED)
        .expect("the omission is reported");
    assert_eq!(
        warning
            .location
            .as_ref()
            .and_then(|location| location.element_id.as_deref()),
        Some("photo"),
        "the warning is located at the omitted element"
    );
    assert!(warning.message.contains("photo"), "{}", warning.message);
    assert!(warning.message.contains("SVG"), "{}", warning.message);
}

#[test]
fn a_raster_layer_is_omitted_from_pdf_with_a_warning_naming_element_and_asset() {
    let brand = layered_palette();
    let rasters = [blue_asset()];
    let model = compile_with_style(
        &parse_scene(&layered_scene()),
        &style(Some(&brand), None, &rasters),
    )
    .expect("compiles");

    let export = export_pdf_reporting(&model, &PdfOptions::default()).expect("exports");

    // The PDF carries vector content only, never the raster as an image
    // (FEAT-014, FEAT-015).
    let text = String::from_utf8_lossy(&export.pdf);
    assert!(!text.contains("/Subtype /Image"), "vector only: {text}");
    assert!(text.starts_with("%PDF-"), "a PDF signature");

    let warning = export
        .diagnostics
        .warnings()
        .find(|finding| finding.code == OMITTED)
        .expect("the omission is reported");
    assert!(warning.message.contains("photo"), "{}", warning.message);
    assert!(warning.message.contains("PDF"), "{}", warning.message);
}

#[test]
fn a_scene_without_raster_layers_stays_pure_vector_and_unchanged() {
    let brand = layered_palette();
    let mut card = rect("card", 0, 0.0, 0.0, 30.0, 30.0);
    card["fill"] = token_paint("red");
    let document = scene(vec![card]);
    let scene = parse_scene(&document);

    // Supplying a raster asset the scene does not place changes nothing: a
    // scene with no raster layers keeps its pure-vector output (FEAT-015).
    let unused = [blue_asset()];
    let without = compile_with_style(&scene, &style(Some(&brand), None, &[])).expect("compiles");
    let with = compile_with_style(&scene, &style(Some(&brand), None, &unused)).expect("compiles");

    assert!(without.nodes.iter().all(|node| node.raster.is_none()));
    assert!(with.nodes.iter().all(|node| node.raster.is_none()));

    let first = export_svg_reporting(&without, &SvgOptions::default()).expect("exports");
    let second = export_svg_reporting(&with, &SvgOptions::default()).expect("exports");
    assert_eq!(first.svg, second.svg, "an unused raster asset is inert");
    assert!(
        first
            .diagnostics
            .warnings()
            .all(|finding| finding.code != OMITTED),
        "no omission is reported when nothing is omitted"
    );
}

// ---------------------------------------------------------------------------
// The recipe's shading model (FEAT-015, FEAT-007, FEAT-028)
// ---------------------------------------------------------------------------

fn raster_shading_recipe() -> StyleRecipe {
    parse_style_recipe(
        r#"{"id":"shaded","projectId":"p","name":"flat","parameters":{"shading":"raster"}}"#,
    )
    .expect("a recipe")
}

fn unshaded_recipe() -> StyleRecipe {
    parse_style_recipe(
        r#"{"id":"plain","projectId":"p","name":"flat","parameters":{"shading":"none"}}"#,
    )
    .expect("a recipe")
}

#[test]
fn a_recipe_requesting_raster_shading_is_honored_by_the_scene_s_layers() {
    let brand = layered_palette();
    let recipe = raster_shading_recipe();
    let rasters = [blue_asset()];
    let model = compile_with_style(
        &parse_scene(&layered_scene()),
        &style(Some(&brand), Some(&recipe), &rasters),
    )
    .expect("compiles");

    assert!(!model.diagnostics.has_errors(), "{:?}", model.diagnostics);
    assert!(
        model
            .diagnostics
            .warnings()
            .all(|finding| finding.code != TEXTURE_UNSUPPORTED),
        "a request the layers fulfill is not reported unhonored: {:?}",
        model.diagnostics
    );
    assert!(model.nodes.iter().any(|node| node.raster.is_some()));
}

#[test]
fn a_recipe_requesting_raster_shading_with_no_layer_is_reported_not_ignored() {
    let brand = layered_palette();
    let recipe = raster_shading_recipe();
    let mut card = rect("card", 0, 0.0, 0.0, 30.0, 30.0);
    card["fill"] = token_paint("red");
    let document = scene(vec![card]);
    let model = compile_with_style(
        &parse_scene(&document),
        &style(Some(&brand), Some(&recipe), &[]),
    )
    .expect("compiles");

    let warning = model
        .diagnostics
        .warnings()
        .find(|finding| finding.code == TEXTURE_UNSUPPORTED)
        .expect("the unhonored request is named");
    assert!(warning.message.contains("shaded"), "{}", warning.message);
    assert!(
        warning.message.contains("no raster layer"),
        "{}",
        warning.message
    );
    // The scene renders with no raster shading: no layer is invented.
    assert!(model.nodes.iter().all(|node| node.raster.is_none()));
}

#[test]
fn a_raster_layer_renders_even_when_the_recipe_does_not_request_raster_shading() {
    let brand = layered_palette();
    let recipe = unshaded_recipe();
    let rasters = [blue_asset()];
    let model = compile_with_style(
        &parse_scene(&layered_scene()),
        &style(Some(&brand), Some(&recipe), &rasters),
    )
    .expect("compiles");

    // The layer still renders, and the recipe's shading model is unaffected: no
    // unhonored-raster-shading warning and no derived shading layer (FEAT-015,
    // FEAT-028).
    assert!(
        model.nodes.iter().any(|node| node.raster.is_some()),
        "the layer is carried into the model"
    );
    assert!(
        model
            .diagnostics
            .warnings()
            .all(|finding| finding.code != TEXTURE_UNSUPPORTED),
        "{:?}",
        model.diagnostics
    );
    assert_eq!(pixel(&export_png(&model), 45, 45), (0, 0, 255, 255));
}

// ---------------------------------------------------------------------------
// Determinism and the size budget (FEAT-015, NFR-010)
// ---------------------------------------------------------------------------

#[test]
fn a_raster_scene_compiles_and_exports_identically_across_runs() {
    let brand = layered_palette();
    let rasters = [blue_asset()];

    let first = compile_with_style(
        &parse_scene(&layered_scene()),
        &style(Some(&brand), None, &rasters),
    )
    .expect("compiles");
    let second = compile_with_style(
        &parse_scene(&layered_scene()),
        &style(Some(&brand), None, &rasters),
    )
    .expect("compiles");

    assert_eq!(
        first.to_json_string().expect("serializable"),
        second.to_json_string().expect("serializable"),
        "identical input yields an identical model (NFR-010)"
    );
    assert_eq!(
        export_png(&first),
        export_png(&second),
        "identical PNG bytes"
    );
    assert_eq!(
        export_svg_reporting(&first, &SvgOptions::default())
            .expect("exports")
            .svg,
        export_svg_reporting(&second, &SvgOptions::default())
            .expect("exports")
            .svg,
        "identical SVG text"
    );
}

/// A scene placing an asset that names the given image file.
fn oversized_scene() -> Value {
    let mut document = scene(vec![raster_layer(
        "photo", 0, "photo", 0.0, 0.0, 40.0, 40.0,
    )]);
    document["canvas"] = json!({ "width": 40.0, "height": 40.0, "background": "#ffffff" });
    document
}

#[test]
fn an_asset_wider_than_the_pixel_budget_is_warned_but_still_rendered() {
    let wide = RasterAsset::new(
        "photo",
        "Photo",
        solid_png(MAX_RASTER_SIDE + 1, 4, [0, 0, 255, 255]),
    );
    let rasters = [wide];
    let model = compile_with_style(
        &parse_scene(&oversized_scene()),
        &style(None, None, &rasters),
    )
    .expect("compiles");

    let export = export_png_reporting(&model, &RasterOptions::default()).expect("exports");
    let warning = export
        .diagnostics
        .warnings()
        .find(|finding| finding.code == LARGE)
        .expect("an oversized asset is reported");
    assert!(warning.message.contains("photo"), "{}", warning.message);
    assert!(
        warning
            .message
            .contains(&format!("{}x4", MAX_RASTER_SIDE + 1)),
        "the measured value is named: {}",
        warning.message
    );
    // It is a warning, not an error: the layer still renders (FEAT-015).
    assert_eq!(pixel(&export.png, 20, 20), (0, 0, 255, 255));
}

#[test]
fn an_asset_larger_on_disk_than_the_byte_budget_is_warned_but_still_rendered() {
    // 4096 by 768 noise is within the pixel budget but over the byte budget, so
    // the byte branch alone triggers the warning (FEAT-015).
    let data = noise_png(4096, 768);
    assert!(
        data.len() > MAX_RASTER_BYTES,
        "the fixture exceeds the byte budget: {} bytes",
        data.len()
    );
    let rasters = [RasterAsset::new("photo", "Photo", data)];
    let model = compile_with_style(
        &parse_scene(&oversized_scene()),
        &style(None, None, &rasters),
    )
    .expect("compiles");

    let export = export_png_reporting(&model, &RasterOptions::default()).expect("exports");
    let warning = export
        .diagnostics
        .warnings()
        .find(|finding| finding.code == LARGE)
        .expect("an oversized asset is reported");
    assert!(warning.message.contains("photo"), "{}", warning.message);
    assert!(warning.message.contains("bytes"), "{}", warning.message);
    assert!(export.png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

// ---------------------------------------------------------------------------
// Raster reference validation (FEAT-015, FEAT-018, C-001)
// ---------------------------------------------------------------------------

/// A project holding one scene whose single raster element `layer` references
/// asset `photo`, plus an optional asset document and image file.
fn project_with_raster_asset(
    tag: &str,
    asset: Option<Value>,
    file: Option<(&str, &[u8])>,
) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", r#"{"defaultSceneId":"s"}"#);
    let mut document = scene(vec![raster_layer(
        "layer", 0, "photo", 0.0, 0.0, 10.0, 10.0,
    )]);
    document["canvas"] = json!({ "width": 100.0, "height": 100.0, "background": "#ffffff" });
    write_scene(&dir, &document);
    if let Some(asset) = asset {
        let id = asset["id"].as_str().expect("an asset id");
        dir.write(&format!("assets/{id}.json"), &asset.to_string());
    }
    if let Some((path, bytes)) = file {
        dir.write_bytes(path, bytes);
    }
    dir
}

/// The reference findings the shipped project loader reports for the scene at
/// `scenes/s.json` in `dir` (FEAT-018).
fn reference_findings(dir: &TempDir) -> Diagnostics {
    let text = std::fs::read_to_string(dir.path().join("scenes/s.json")).expect("reads the scene");
    let scene = parse(&text).expect("the scene parses");
    let assets = ProjectAssets::load(dir.path(), &scene).expect("the project loads");
    assets.check_references(&scene)
}

fn error_with<'a>(diagnostics: &'a Diagnostics, code: &DiagnosticCode) -> &'a Diagnostic {
    diagnostics
        .errors()
        .find(|finding| finding.code == *code)
        .unwrap_or_else(|| panic!("expected {code}, got: {diagnostics}"))
}

#[test]
fn a_missing_raster_asset_is_reported_naming_asset_and_element() {
    let dir = project_with_raster_asset("raster-missing", None, None);
    let findings = reference_findings(&dir);

    let error = error_with(&findings, &vectr_core::MISSING_ASSET);
    assert!(error.message.contains("layer"), "{}", error.message);
    assert!(error.message.contains("photo"), "{}", error.message);
    let location = error.location.as_ref().expect("the reference is located");
    assert_eq!(location.element_id.as_deref(), Some("layer"));
    assert_eq!(location.json_path.as_deref(), Some("/assetId"));
}

#[test]
fn a_raster_asset_of_another_kind_is_reported_naming_the_element() {
    let asset = json!({
        "id": "photo", "sceneId": "s", "kind": "font",
        "path": "assets/Brand.ttf", "license": "OFL"
    });
    let dir = project_with_raster_asset("raster-kind", Some(asset), None);
    let findings = reference_findings(&dir);

    let error = error_with(&findings, &vectr_core::ASSET_KIND);
    assert!(error.message.contains("layer"), "{}", error.message);
    assert!(error.message.contains("photo"), "{}", error.message);
    assert!(error.message.contains("not a raster"), "{}", error.message);
}

#[test]
fn a_raster_file_that_is_not_a_png_is_reported_naming_asset_and_element() {
    let asset = json!({
        "id": "photo", "sceneId": "s", "kind": "raster",
        "path": "assets/photo.jpg", "license": "CC0"
    });
    // JPEG magic bytes: a valid image in a format the raster layer rejects.
    let dir = project_with_raster_asset(
        "raster-format",
        Some(asset),
        Some(("assets/photo.jpg", b"\xff\xd8\xff\xe0not-a-png")),
    );
    let findings = reference_findings(&dir);

    let error = error_with(&findings, &vectr_project::RASTER_FORMAT);
    assert!(error.message.contains("layer"), "{}", error.message);
    assert!(error.message.contains("photo"), "{}", error.message);
    assert!(error.message.contains("PNG"), "{}", error.message);
    let location = error.location.as_ref().expect("the reference is located");
    assert_eq!(location.element_id.as_deref(), Some("layer"));
}

#[test]
fn a_raster_asset_declared_for_another_scene_does_not_resolve() {
    let asset = json!({
        "id": "photo", "sceneId": "other", "kind": "raster",
        "path": "assets/photo.png", "license": "CC0"
    });
    let dir = project_with_raster_asset(
        "raster-outside",
        Some(asset),
        Some(("assets/photo.png", &solid_png(2, 2, [0, 0, 255, 255]))),
    );
    let findings = reference_findings(&dir);

    // An asset belongs to exactly one scene; one declared for another scene
    // does not resolve here (FEAT-018).
    let error = error_with(&findings, &vectr_core::MISSING_ASSET);
    assert!(error.message.contains("layer"), "{}", error.message);
}

#[test]
fn a_resolvable_png_raster_asset_passes_validation() {
    let asset = json!({
        "id": "photo", "sceneId": "s", "kind": "raster",
        "path": "assets/photo.png", "license": "CC0"
    });
    let dir = project_with_raster_asset(
        "raster-valid",
        Some(asset),
        Some(("assets/photo.png", &solid_png(2, 2, [0, 0, 255, 255]))),
    );
    let findings = reference_findings(&dir);
    assert!(!findings.has_errors(), "{findings}");
}

#[test]
fn a_raster_element_owned_by_a_definition_is_refused() {
    let mut raster = def_element(
        "r1",
        "chip",
        0,
        "raster",
        json!({ "x": 0.0, "y": 0.0, "width": 4.0, "height": 4.0 }),
    );
    raster["assetId"] = json!("photo");
    let document = definition("chip", json!([]), vec![raster]);

    let diagnostics =
        parse_definition(&document.to_string()).expect_err("a raster in a definition is refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, DiagnosticCode::SCHEMA);
    assert!(error.message.contains("r1"), "{}", error.message);
    assert!(error.message.contains("definition"), "{}", error.message);
}

#[test]
fn a_raster_element_is_refused_as_each_composition_operand() {
    // Every composition kind the language names: a raster element cannot be
    // lowered into any of them, and the combination is named rather than the
    // element dropped (FEAT-015, FEAT-018).
    let parents: [(&str, Value); 6] = [
        ("boolean", json!({ "operation": "union" })),
        ("alongPath", json!({ "count": 2, "pathData": "M0 0 L10 0" })),
        ("offset", json!({ "distance": 5.0 })),
        ("projection", json!({ "axis": "x" })),
        (
            "procedural",
            json!({ "procedure": "stippling", "count": 4 }),
        ),
        ("repeat", json!({ "count": 2, "spacing": 10.0 })),
    ];

    for (kind, geometry) in parents {
        let parent = element("parent", 0, kind, geometry);
        let mut child = raster_layer("layer", 0, "photo", 0.0, 0.0, 4.0, 4.0);
        child["parentId"] = json!("parent");
        let document = scene(vec![parent, child]);

        let diagnostics = parse(&document.to_string()).expect_err("a raster operand is refused");
        assert!(
            diagnostics.errors().any(|error| {
                error.message.contains("raster element `layer`") && error.message.contains(kind)
            }),
            "a raster element as a `{kind}` operand is refused naming the combination: {diagnostics}"
        );
    }
}

// ---------------------------------------------------------------------------
// The rollout flag at the command line and over MCP (C-004, C-005)
// ---------------------------------------------------------------------------

/// A project whose default scene `s` places one full-canvas blue raster layer
/// resolved by `assets/photo.png`.
fn cli_raster_project(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write("vectr.project.json", r#"{"defaultSceneId":"s"}"#);
    let mut document = scene(vec![raster_layer(
        "layer", 0, "photo", 0.0, 0.0, 100.0, 100.0,
    )]);
    document["canvas"] = json!({ "width": 100.0, "height": 100.0, "background": "#ffffff" });
    write_scene(&dir, &document);
    dir.write(
        "assets/photo.json",
        r#"{"id":"photo","sceneId":"s","kind":"raster","path":"assets/photo.png","license":"CC0"}"#,
    );
    dir.write_bytes("assets/photo.png", &solid_png(4, 4, [0, 0, 255, 255]));
    dir
}

#[test]
fn the_raster_layers_flag_gates_the_capability_at_the_command_line() {
    let dir = cli_raster_project("cli-raster-flag");

    // Off by default: a scene carrying a raster layer is an unsupported feature
    // and nothing is written (FEAT-015, C-004).
    let disabled = run_vectr_env(
        dir.path(),
        &["export", "--format", "png", "--out", "off.png"],
        &[RASTER_FLAG],
        &[],
    );
    assert_eq!(code(&disabled), 3, "{}", stderr(&disabled));
    assert!(
        stderr(&disabled).contains("E_UNSUPPORTED"),
        "{}",
        stderr(&disabled)
    );
    assert!(
        !dir.path().join("off.png").exists(),
        "no partial output while the flag is off"
    );

    // Validation is structural, so it still succeeds while rendering is gated.
    let validated = run_vectr_env(dir.path(), &["validate"], &[RASTER_FLAG], &[]);
    assert_eq!(code(&validated), 0, "{}", stderr(&validated));

    // Enabled: the layer composites into PNG.
    let png = run_vectr_env(
        dir.path(),
        &["export", "--format", "png", "--out", "out.png"],
        &[RASTER_FLAG],
        &[(RASTER_FLAG, "1")],
    );
    assert_eq!(code(&png), 0, "{}", stderr(&png));
    let bytes = std::fs::read(dir.path().join("out.png")).expect("reads the png");
    assert_eq!(pixel(&bytes, 50, 50), (0, 0, 255, 255));

    // Enabled: the SVG target omits the layer with a warning and stays pure
    // vector.
    let svg = run_vectr_env(
        dir.path(),
        &["export", "--format", "svg", "--out", "out.svg"],
        &[RASTER_FLAG],
        &[(RASTER_FLAG, "1")],
    );
    assert_eq!(code(&svg), 0, "{}", stderr(&svg));
    assert!(
        stderr(&svg).contains("W_RASTER_OMITTED"),
        "{}",
        stderr(&svg)
    );
    let svg_text = std::fs::read_to_string(dir.path().join("out.svg")).expect("reads the svg");
    assert!(!svg_text.contains("<image"), "{svg_text}");
}

#[test]
fn the_command_line_reports_a_non_png_raster_asset_as_an_invalid_scene() {
    let dir = TempDir::new("cli-raster-format");
    dir.write("vectr.project.json", r#"{"defaultSceneId":"s"}"#);
    let mut document = scene(vec![raster_layer(
        "layer", 0, "photo", 0.0, 0.0, 10.0, 10.0,
    )]);
    document["canvas"] = json!({ "width": 100.0, "height": 100.0, "background": "#ffffff" });
    write_scene(&dir, &document);
    dir.write(
        "assets/photo.json",
        r#"{"id":"photo","sceneId":"s","kind":"raster","path":"assets/photo.jpg","license":"CC0"}"#,
    );
    dir.write_bytes("assets/photo.jpg", b"\xff\xd8\xff\xe0not-a-png");

    // Exit 1: a raster asset whose file is not a PNG is an invalid scene,
    // reported before anything renders (C-004, FEAT-018).
    let output = run_vectr_env(dir.path(), &["validate"], &[RASTER_FLAG], &[]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("E_RASTER_FORMAT"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn mcp_compile_gates_raster_layers_and_carries_the_raster_node_when_enabled() {
    let dir = cli_raster_project("mcp-raster");
    let call = mcp_tool_call(1, "compile", json!({ "scene": "s" }));

    // Off: the capability is an unsupported feature (C-005, FEAT-015).
    let disabled = run_mcp_session_with_env(
        dir.path(),
        &[],
        &[(RASTER_FLAG, "0")],
        std::slice::from_ref(&call),
    );
    let disabled_text = serde_json::to_string(&disabled.responses).expect("serializable");
    assert!(disabled_text.contains("E_UNSUPPORTED"), "{}", disabled_text);

    // On: compile returns the scene's raster node.
    let enabled = run_mcp_session_with_env(dir.path(), &[], &[(RASTER_FLAG, "1")], &[call]);
    assert_eq!(enabled.code, 0, "{}", enabled.stderr);
    let model = &enabled.responses[0]["result"]["structuredContent"];
    let nodes = model["model"]["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["kind"], "raster");
    assert_eq!(nodes[0]["raster"]["assetId"], "photo");
}

#[test]
fn mcp_render_omits_a_raster_layer_from_svg_with_a_warning() {
    let dir = cli_raster_project("mcp-raster-svg");
    let run = run_mcp_session_with_env(
        dir.path(),
        &[],
        &[(RASTER_FLAG, "1")],
        &[mcp_tool_call(
            1,
            "render",
            json!({ "scene": "s", "format": "svg", "out": "s.svg" }),
        )],
    );
    assert_eq!(run.code, 0, "{}", run.stderr);

    let result = &run.responses[0]["result"]["structuredContent"];
    let diagnostics = result["diagnostics"].as_array().expect("diagnostics");
    assert!(
        diagnostics.iter().any(|finding| {
            finding["severity"] == "warning" && finding["code"] == "W_RASTER_OMITTED"
        }),
        "{diagnostics:?}"
    );
    let svg = std::fs::read_to_string(dir.path().join("s.svg")).expect("reads the svg");
    assert!(!svg.contains("<image"), "{svg}");
}

/// Enabling the capability does not invent an asset a project does not provide:
/// the missing reference is still reported and nothing is written (FEAT-015,
/// FEAT-018).
#[test]
fn the_flag_does_not_make_a_missing_asset_resolve() {
    let dir = cli_raster_project("cli-raster-missing");
    std::fs::remove_file(dir.path().join("assets/photo.json")).expect("removes the asset");

    let output = run_vectr_env(
        dir.path(),
        &["export", "--format", "png", "--out", "out.png"],
        &[RASTER_FLAG],
        &[(RASTER_FLAG, "1")],
    );
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("E_MISSING_ASSET"),
        "{}",
        stderr(&output)
    );
    assert!(!dir.path().join("out.png").exists());
}
