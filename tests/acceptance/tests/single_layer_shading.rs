//! Acceptance tests for single-layer shading (FEAT-028).
//!
//! Drives the shipped library through its public API (C-002), the `vectr`
//! binary as a subprocess (C-004), and the SVG, PNG and PDF exporters (C-003).
//! A flat scene that selects the look derives exactly one hard-edged flat
//! shading layer per filled shape, on the side facing away from the scene's
//! light, bounded by the shape's own outline; the tests prove the look on the
//! render model and prove it reaches all three export targets as pure vector.

mod common;

use common::*;
use serde_json::{json, Value};
use vectr_core::render::{NodeShading, Paint};
use vectr_core::style::{SHADING, SHADING_UNSUPPORTED};
use vectr_core::{
    Definition, Gradient, LightDirection, Palette, ResolvedNode, Shape, StyleContext, StyleRecipe,
};

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// A style recipe document with the given name and parameters.
fn recipe(name: &str, parameters: Value) -> StyleRecipe {
    vectr_core::parse_style_recipe(
        &json!({
            "id": "r",
            "projectId": "p",
            "name": name,
            "parameters": parameters
        })
        .to_string(),
    )
    .unwrap_or_else(|diagnostics| panic!("expected a valid {name} recipe: {diagnostics}"))
}

/// A flat recipe requesting single-layer shading in `token`, optionally with a
/// light direction; omitting the direction leaves the documented default.
fn single_layer(token: &str, light: Option<&str>) -> StyleRecipe {
    let mut parameters = json!({ "shading": "single-layer", "shadeToken": token });
    if let Some(direction) = light {
        parameters["lightDirection"] = json!(direction);
    }
    recipe("flat", parameters)
}

/// A style context over the assets a test supplies.
fn context<'a>(
    palette: Option<&'a Palette>,
    strokes: &'a [vectr_core::StrokeProfile],
    gradients: &'a [Gradient],
    recipe: Option<&'a StyleRecipe>,
    definitions: &'a [Definition],
) -> StyleContext<'a> {
    StyleContext {
        palette,
        strokes,
        gradients,
        fonts: &[],
        recipe,
        definitions,
        rasters: &[],
        raster_layers: false,
    }
}

/// A palette with a base fill and a shade tone.
fn shading_palette() -> Palette {
    vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#ff0000"), ("shadow", "#111111")],
    ))
    .expect("a palette")
}

/// The shading layers a model carries, each with the node it belongs to.
fn shading_layers(model: &vectr_core::RenderModel) -> Vec<(&str, &NodeShading)> {
    model
        .nodes
        .iter()
        .filter_map(|node| {
            node.shading
                .as_ref()
                .map(|shading| (node.id.as_str(), shading))
        })
        .collect()
}

/// The world points a node's shading layer spans, under the node's transform.
fn shading_world_points(node: &ResolvedNode) -> Vec<[f64; 2]> {
    let shading = node.shading.as_ref().expect("a shading layer");
    vectr_core::flatten_shape(&shading.geometry)
        .into_iter()
        .flatten()
        .map(|point| node.transform.apply(point))
        .collect()
}

/// The area of a node's shading region in world coordinates.
fn shading_area(node: &ResolvedNode) -> f64 {
    shape_area(&node.shading.as_ref().expect("a shading layer").geometry)
}

/// The `(p - b) · light` sign of every point of a node's shading region, where
/// `b` is the centre of the node's own bounding box in world coordinates.
fn shade_side(node: &ResolvedNode, light: LightDirection) -> Vec<f64> {
    let geometry = node.geometry.as_ref().expect("a fill area");
    let world: Vec<[f64; 2]> = vectr_core::flatten_shape(geometry)
        .into_iter()
        .flatten()
        .map(|point| node.transform.apply(point))
        .collect();
    let (min_x, max_x) = world.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
        (lo.min(p[0]), hi.max(p[0]))
    });
    let (min_y, max_y) = world.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
        (lo.min(p[1]), hi.max(p[1]))
    });
    let centre = [(min_x + max_x) / 2.0, (min_y + max_y) / 2.0];
    let vector = vectr_core::light_vector(light);
    shading_world_points(node)
        .into_iter()
        .map(|p| (p[0] - centre[0]) * vector[0] + (p[1] - centre[1]) * vector[1])
        .collect()
}

/// A scene with one accent-filled rect of the given size.
fn shaded_rect_scene() -> Value {
    let mut card = rect("card", 0, 0.0, 0.0, 100.0, 100.0);
    card["fill"] = token_paint("accent");
    scene_with(vec![card], None, Some("brand"))
}

/// Decodes a PNG to a pixmap.
fn decode_png(bytes: &[u8]) -> tiny_skia::Pixmap {
    tiny_skia::Pixmap::decode_png(bytes).expect("a PNG")
}

/// The PDF document as text, for content-stream inspection.
fn pdf_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The content stream of the document's first form XObject, if any.
fn pdf_form_stream(text: &str) -> Option<&str> {
    let start = text.find("/Subtype /Form")?;
    let stream = start + text[start..].find("stream\n")? + "stream\n".len();
    let end = stream + text[stream..].find("\nendstream")?;
    Some(&text[stream..end])
}

// ---------------------------------------------------------------------------
// Acceptance criteria
// ---------------------------------------------------------------------------

#[test]
fn every_filled_shape_renders_its_base_fill_plus_one_shading_layer_on_the_far_side() {
    // A filled rect, a stroke-only rect and a text run: only the filled rect is
    // a fill area for this feature.
    let mut filled = rect("filled", 0, 0.0, 0.0, 100.0, 100.0);
    filled["fill"] = token_paint("accent");
    let mut outline = rect("outline", 1, 0.0, 0.0, 50.0, 50.0);
    outline["stroke"] = stroke("outline", "accent");
    let mut label = text("label", 2, 0.0, 20.0, "Hi", 8.0);
    label["fill"] = token_paint("accent");
    let document = scene_with(vec![filled, outline, label], None, Some("brand"));

    let palette = shading_palette();
    let profile =
        vectr_core::parse_stroke_profile(&stroke_profile("outline", 1.0, "butt", "miter"))
            .expect("a profile");
    let look = single_layer("shadow", Some("top"));
    let style = context(
        Some(&palette),
        std::slice::from_ref(&profile),
        &[],
        Some(&look),
        &[],
    );

    let model = compile_with(&document, &style).expect("compiles");

    // Exactly one node draws a shading layer, and it is the filled shape.
    let layers = shading_layers(&model);
    assert_eq!(
        layers.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec!["filled"],
        "a stroke-only shape and a text run derive no shading: {:?}",
        model.nodes
    );

    let node = model.node("filled").expect("the filled node");
    let shading = node.shading.as_ref().expect("one shading layer");
    assert!(
        matches!(shading.paint, Paint::Color { .. }),
        "the shading layer is a flat fill, not a gradient: {:?}",
        shading.paint
    );
    assert_eq!(
        common::color(&shading.paint),
        Some("#111111"),
        "the shading layer is painted in the shade tone"
    );
    assert!(
        matches!(shading.geometry, Shape::Path(_)),
        "the derived region is a concrete hard-edged path: {:?}",
        shading.geometry
    );

    // The region is half the shape, on the far side (the bottom, under a top
    // light), and its edge is the straight line through the bounding-box centre.
    assert!(
        (shading_area(node) - 5_000.0).abs() < 1e-6,
        "the far half of a 100x100 shape is 5000: {}",
        shading_area(node)
    );
    let sides = shade_side(node, LightDirection::Top);
    assert!(
        sides.iter().all(|signed| *signed <= 1e-9),
        "every point of the region is on the far side of the light line: {sides:?}"
    );
    assert!(
        sides.iter().any(|signed| *signed < -1e-9),
        "the region is non-empty"
    );
}

#[test]
fn the_shading_region_is_bounded_by_the_outline_and_cut_by_one_straight_edge() {
    // A triangle whose far side is clipped by the line through its bounding-box
    // centre: the region is a subset of the fill area, so it never extends past
    // the shape's own outline.
    let mut triangle = element(
        "tri",
        0,
        "polygon",
        json!({ "points": [[0.0, 0.0], [100.0, 0.0], [20.0, 80.0]] }),
    );
    triangle["fill"] = token_paint("accent");
    let document = scene_with(vec![triangle], None, Some("brand"));

    let palette = shading_palette();
    let look = single_layer("shadow", Some("right"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("compiles");
    let node = model.node("tri").expect("the triangle");
    let fill_area = shape_area(node.geometry.as_ref().expect("an outline"));
    let region = shading_area(node);
    assert!(
        region > 0.0 && region < fill_area,
        "the region is a proper subset of the fill area: region {region}, fill {fill_area}"
    );

    // Every point of the region is inside the triangle's bounding box, so the
    // region is clipped to the shape and not invented beside it.
    let sides = shade_side(node, LightDirection::Right);
    assert!(
        sides.iter().all(|signed| *signed <= 1e-9),
        "the region never covers the lit side: {sides:?}"
    );
}

#[test]
fn a_single_layer_request_without_a_shade_token_is_a_located_error_naming_it() {
    // The shade tone is required alongside the request, so the recipe is
    // refused by name rather than compiled without one.
    let missing = recipe(
        "flat",
        json!({ "shading": "single-layer", "lightDirection": "top" }),
    );
    let diagnostics = vectr_core::validate_style_recipe(&missing);
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, SHADING);
    assert!(
        error.message.contains("shadeToken"),
        "the error names the missing setting: {}",
        error.message
    );
    assert_eq!(
        error
            .location
            .as_ref()
            .and_then(|location| location.json_path.as_deref()),
        Some("/parameters/shadeToken")
    );
}

#[test]
fn a_shade_token_the_palette_does_not_define_is_refused_naming_it() {
    let document = shaded_rect_scene();
    let palette = shading_palette();
    let look = single_layer("absent", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let diagnostics = compile_with(&document, &style).expect_err("an undefined shade token");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, SHADING);
    assert!(
        error.message.contains("absent"),
        "the error names the token: {}",
        error.message
    );
    assert!(
        error.message.contains("shade token") || error.message.contains("shadeToken"),
        "the error names the setting: {}",
        error.message
    );
}

#[test]
fn a_light_direction_that_is_not_a_compass_point_is_refused_naming_it() {
    let diagnostics = vectr_core::parse_style_recipe(
        &json!({
            "id": "r", "projectId": "p", "name": "flat",
            "parameters": {
                "shading": "single-layer", "shadeToken": "shadow",
                "lightDirection": "diagonal"
            }
        })
        .to_string(),
    )
    .expect_err("an unknown direction");
    let error = diagnostics.errors().next().expect("an error");
    assert!(
        error.message.contains("lightDirection") && error.message.contains("diagonal"),
        "the error names the invalid setting and its value: {}",
        error.message
    );
}

#[test]
fn the_documented_default_light_direction_applies_when_none_is_set() {
    // No lightDirection: the documented default is top-left, which shades the
    // side facing bottom-right.
    let document = shaded_rect_scene();
    let palette = shading_palette();
    let look = single_layer("shadow", None);
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("compiles");
    let node = model.node("card").expect("the card");
    assert_eq!(
        look.light_direction(),
        LightDirection::TopLeft,
        "the recipe's resolved direction is the documented default"
    );
    let sides = shade_side(node, LightDirection::TopLeft);
    assert!(
        sides.iter().all(|signed| *signed <= 1e-9),
        "the region falls on the side away from the default light: {sides:?}"
    );
    assert!(sides.iter().any(|signed| *signed < -1e-9));
}

#[test]
fn a_shaded_shape_translates_its_region_with_it() {
    let mut near = rect("near", 0, 0.0, 0.0, 40.0, 40.0);
    near["fill"] = token_paint("accent");
    let mut far = rect("far", 1, 0.0, 0.0, 40.0, 40.0);
    far["fill"] = token_paint("accent");
    far["transform"]["translateX"] = json!(200.0);
    far["transform"]["translateY"] = json!(120.0);
    let document = scene_with(vec![near, far], None, Some("brand"));

    let palette = shading_palette();
    let look = single_layer("shadow", Some("bottom"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("compiles");
    let offset = [200.0, 120.0];
    for (id, _) in shading_layers(&model) {
        let node = model.node(id).unwrap();
        let points = shading_world_points(node);
        // The translated copy's region is its own region moved by the same
        // amount; both sit in the far (top) half of their own shape.
        assert!(
            points
                .iter()
                .all(|p| p[1] <= node.transform.apply([0.0, 20.0])[1] + 1e-6),
            "`{id}` shades the far side: {points:?}"
        );
    }
    // The two regions are the same shape, apart by the translation.
    let near_points = shading_world_points(model.node("near").unwrap());
    let far_points = shading_world_points(model.node("far").unwrap());
    let far_expected: Vec<[f64; 2]> = near_points
        .iter()
        .map(|p| [p[0] + offset[0], p[1] + offset[1]])
        .collect();
    assert_eq!(far_points.len(), far_expected.len());
    for (actual, expected) in far_points.iter().zip(far_expected.iter()) {
        assert!(
            close(actual[0], expected[0]) && close(actual[1], expected[1]),
            "the translated region moves with the shape: {actual:?} vs {expected:?}"
        );
    }
}

#[test]
fn a_gradient_filled_shape_keeps_its_gradient_and_gains_one_flat_shade() {
    let mut faded = rect("faded", 0, 0.0, 0.0, 60.0, 60.0);
    faded["fill"] = gradient_paint("fade");
    let document = scene_with(vec![faded], None, Some("brand"));

    let palette = shading_palette();
    let gradients = [vectr_core::parse_gradient(
        &json!({
            "id": "fade", "projectId": "p", "name": "Fade", "type": "linear",
            "stops": [{ "offset": 0, "token": "accent" }, { "offset": 1, "token": "shadow" }]
        })
        .to_string(),
    )
    .expect("a gradient")];
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &gradients, Some(&look), &[]);

    let model = compile_with(&document, &style).expect("compiles");
    let node = model.node("faded").expect("the faded node");
    assert!(
        matches!(node.paint.fill, Some(Paint::Gradient(_))),
        "the gradient base fill survives: {:?}",
        node.paint.fill
    );
    assert_eq!(
        node.shading.as_ref().expect("a shading layer").paint,
        Paint::Color {
            value: "#111111".to_string()
        },
        "the shading layer itself carries no gradient"
    );

    // The SVG carries the gradient definition for the base fill and a separate
    // flat-filled path for the shading layer.
    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("<linearGradient"), "{svg}");
    assert!(svg.contains("fill=\"#111111\""), "{svg}");
}

#[test]
fn a_shape_whose_shading_region_is_empty_is_reported_and_render_its_base_fill() {
    // A degenerate polygon encloses no area on either side of the light line, so
    // the derived region is empty. The shape still renders its base fill and the
    // empty region is named, never silent.
    let mut flat = element(
        "flat",
        0,
        "polygon",
        json!({ "points": [[0.0, 0.0], [10.0, 0.0], [20.0, 0.0]] }),
    );
    flat["fill"] = token_paint("accent");
    let document = scene_with(vec![flat], None, Some("brand"));

    let palette = shading_palette();
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("the shape still compiles");
    assert!(
        model.node("flat").unwrap().shading.is_none(),
        "an empty region derives no shading layer"
    );
    assert_eq!(
        common::fill_color(model.node("flat").unwrap()),
        Some("#ff0000"),
        "the shape renders its base fill"
    );
    let warning = model
        .diagnostics
        .warnings()
        .find(|warning| warning.code == SHADING_UNSUPPORTED)
        .expect("the empty region is reported, not silent");
    assert!(
        warning.message.contains("flat"),
        "the warning names the shape: {}",
        warning.message
    );
}

#[test]
fn a_line_that_encloses_no_fill_area_is_reported_rather_than_left_unshaded() {
    let mut line = line("line1", 0, json!([[0.0, 0.0], [10.0, 10.0]]));
    line["fill"] = token_paint("accent");
    let document = scene_with(vec![line], None, Some("brand"));

    let palette = shading_palette();
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("the line still compiles");
    assert!(model.node("line1").unwrap().shading.is_none());
    let warning = model
        .diagnostics
        .warnings()
        .find(|warning| warning.code == SHADING_UNSUPPORTED)
        .expect("the unshadable shape is named");
    assert!(warning.message.contains("line1"), "{}", warning.message);
}

#[test]
fn a_composition_resolving_to_a_single_fill_area_is_shaded_once() {
    // A boolean union lowers to one resolved fill area, so it carries exactly
    // one shading layer, not one per child operand.
    let mut union = element("u", 0, "boolean", json!({ "operation": "union" }));
    union["fill"] = token_paint("accent");
    let mut left = rect("left", 0, 0.0, 0.0, 60.0, 60.0);
    left["parentId"] = json!("u");
    let mut right = rect("right", 1, 40.0, 0.0, 60.0, 60.0);
    right["parentId"] = json!("u");
    let document = scene_with(vec![union, left, right], None, Some("brand"));

    let palette = shading_palette();
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("compiles");
    assert_eq!(
        model.nodes.len(),
        1,
        "the union lowers to one node: {:?}",
        model.nodes
    );
    assert_eq!(
        shading_layers(&model).len(),
        1,
        "the single fill area is shaded once"
    );
}

#[test]
fn a_repeat_shades_every_copy_on_its_own_outline() {
    let repeat = element("row", 0, "repeat", json!({ "count": 3, "spacing": 40.0 }));
    let mut cell = rect("cell", 0, 0.0, 0.0, 20.0, 20.0);
    cell["parentId"] = json!("row");
    cell["fill"] = token_paint("accent");
    let document = scene_with(vec![repeat, cell], None, Some("brand"));

    let palette = shading_palette();
    let look = single_layer("shadow", Some("right"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("compiles");
    let layers = shading_layers(&model);
    assert_eq!(
        layers.len(),
        3,
        "every copy is shaded on its own outline: {:?}",
        model.nodes
    );
    for (id, _) in layers {
        let node = model.node(id).unwrap();
        let sides = shade_side(node, LightDirection::Right);
        assert!(
            sides.iter().all(|signed| *signed <= 1e-9),
            "`{id}` shades its own far side: {sides:?}"
        );
    }
}

#[test]
fn a_definition_instance_shades_every_fill_area_of_its_definition() {
    let definitions = [vectr_core::parse_definition(
        &json!({
            "id": "chip", "projectId": "p", "name": "chip",
            "parameters": [], "origin": { "x": 0.0, "y": 0.0 },
            "elements": [{
                "id": "body", "definitionId": "chip", "order": 0, "kind": "rect",
                "geometry": { "x": 0.0, "y": 0.0, "width": 30.0, "height": 30.0 },
                "transform": {
                    "translateX": 0.0, "translateY": 0.0, "rotate": 0.0,
                    "scaleX": 1.0, "scaleY": 1.0
                },
                "fill": { "kind": "token", "ref": "accent" },
                "opacity": 1.0, "visible": true
            }]
        })
        .to_string(),
    )
    .expect("a definition")];

    let mut placed = element("place", 0, "instance", json!({}));
    placed["definitionRef"] = json!("chip");
    let document = scene_with(vec![placed], None, Some("brand"));

    let palette = shading_palette();
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &definitions);

    let model = compile_with(&document, &style).expect("compiles");
    assert_eq!(
        shading_layers(&model).len(),
        1,
        "the definition's fill area is shaded: {:?}",
        model.nodes
    );
}

#[test]
fn a_change_to_the_shade_token_restyles_every_shaded_shape_after_one_recompile() {
    let mut first = rect("first", 0, 0.0, 0.0, 30.0, 30.0);
    first["fill"] = token_paint("accent");
    let mut second = rect("second", 1, 40.0, 0.0, 30.0, 30.0);
    second["fill"] = token_paint("accent");
    let document = scene_with(vec![first, second], None, Some("brand"));

    let before = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#ff0000"), ("shadow", "#111111")],
    ))
    .expect("a palette");
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&before), &[], &[], Some(&look), &[]);
    let model = compile_with(&document, &style).expect("compiles");
    for (_, shading) in shading_layers(&model) {
        assert_eq!(common::color(&shading.paint), Some("#111111"));
    }

    // One recompile with the new palette value restyles every shaded shape.
    let after = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#ff0000"), ("shadow", "#00ff00")],
    ))
    .expect("a palette");
    let style = context(Some(&after), &[], &[], Some(&look), &[]);
    let recoloured = compile_with(&document, &style).expect("compiles");
    let layers = shading_layers(&recoloured);
    assert_eq!(layers.len(), 2);
    for (_, shading) in layers {
        assert_eq!(common::color(&shading.paint), Some("#00ff00"));
    }
}

#[test]
fn a_shaded_scene_exports_to_svg_png_and_pdf_as_pure_vector() {
    let document = shaded_rect_scene();
    let palette = shading_palette();
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);
    let model = compile_with(&document, &style).expect("compiles");

    // SVG: the flat shade fill is present and the document carries no raster.
    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports svg");
    assert!(svg.contains("fill=\"#111111\""), "{svg}");
    assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
    assert!(!svg.contains("<image"), "the shading is pure vector: {svg}");
    assert!(!svg.contains("feTurbulence"), "{svg}");

    // PNG: the far half is the shade tone and the lit half the base fill.
    let bytes = vectr_core::export_png(&model, &Default::default()).expect("exports png");
    let image = decode_png(&bytes);
    let lit = image.pixel(50, 20).expect("in bounds").demultiply();
    assert_eq!(
        (lit.red(), lit.green(), lit.blue()),
        (255, 0, 0),
        "the half toward the light keeps the base fill"
    );
    let shaded = image.pixel(50, 80).expect("in bounds").demultiply();
    assert_eq!(
        (shaded.red(), shaded.green(), shaded.blue()),
        (17, 17, 17),
        "the far half carries the shade tone"
    );

    // PDF: the shade colour is written as a vector fill and the document embeds
    // no raster image.
    let pdf = vectr_core::export_pdf(&model, &Default::default()).expect("exports pdf");
    assert!(pdf.starts_with(b"%PDF-"), "a PDF document");
    let text = pdf_text(&pdf);
    assert!(
        text.contains("0.066667 0.066667 0.066667 rg"),
        "the shade tone is a vector fill: {text}"
    );
    assert!(!text.contains("/Subtype /Image"), "no raster asset: {text}");
    assert!(!text.contains("/Image"), "no raster asset: {text}");
}

#[test]
fn the_same_shaded_scene_compiles_and_exports_identically_twice() {
    let document = shaded_rect_scene();
    let palette = shading_palette();
    let look = single_layer("shadow", Some("bottomRight"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let first = compile_with(&document, &style).expect("compiles");
    let second = compile_with(&document, &style).expect("compiles");
    assert_eq!(first, second, "the shaded model is deterministic (NFR-010)");

    assert_eq!(
        vectr_core::export_svg(&first, &Default::default()).expect("svg"),
        vectr_core::export_svg(&second, &Default::default()).expect("svg"),
        "SVG is byte-identical (NFR-010)"
    );
    assert_eq!(
        vectr_core::export_png(&first, &Default::default()).expect("png"),
        vectr_core::export_png(&second, &Default::default()).expect("png"),
        "PNG is byte-identical (NFR-010)"
    );
    assert_eq!(
        vectr_core::export_pdf(&first, &Default::default()).expect("pdf"),
        vectr_core::export_pdf(&second, &Default::default()).expect("pdf"),
        "PDF is byte-identical (NFR-010)"
    );
}

#[test]
fn the_shading_composites_above_the_fill_and_below_the_stroke() {
    let mut card = rect("card", 0, 0.0, 0.0, 100.0, 100.0);
    card["fill"] = token_paint("accent");
    card["stroke"] = stroke("outline", "shadow");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette = shading_palette();
    let profile =
        vectr_core::parse_stroke_profile(&stroke_profile("outline", 2.0, "butt", "miter"))
            .expect("a profile");
    let look = single_layer("shadow", Some("top"));
    let style = context(
        Some(&palette),
        std::slice::from_ref(&profile),
        &[],
        Some(&look),
        &[],
    );

    let model = compile_with(&document, &style).expect("compiles");
    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    let fill = svg.find("fill=\"#ff0000\"").expect("the base fill");
    let shade = svg.find("fill=\"#111111\"").expect("the shading layer");
    let stroke = svg.find("stroke=\"#111111\"").expect("the stroke");
    assert!(
        fill < shade && shade < stroke,
        "paint order is fill, shading, stroke: {svg}"
    );
}

#[test]
fn an_along_path_shades_every_copy_on_its_own_outline() {
    let along = element(
        "follow",
        0,
        "alongPath",
        json!({ "count": 2, "pathData": "M0 0 L100 0" }),
    );
    let mut cell = rect("cell", 0, 0.0, 0.0, 20.0, 20.0);
    cell["parentId"] = json!("follow");
    cell["fill"] = token_paint("accent");
    let document = scene_with(vec![along, cell], None, Some("brand"));

    let palette = shading_palette();
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("compiles");
    let filled = model
        .nodes
        .iter()
        .filter(|node| node.paint.fill.is_some())
        .count();
    assert!(
        filled >= 2,
        "the alongPath lowers to copies: {:?}",
        model.nodes
    );
    assert_eq!(
        shading_layers(&model).len(),
        filled,
        "every fill area the alongPath yields is shaded: {:?}",
        model.nodes
    );
}

#[test]
fn a_procedural_element_shades_every_generated_area() {
    let mut procedural = element(
        "p1",
        0,
        "procedural",
        json!({ "procedure": "stippling", "count": 12, "spacing": 3.0 }),
    );
    procedural["fill"] = token_paint("accent");
    let mut region = rect("region", 0, 60.0, 60.0, 80.0, 80.0);
    region["parentId"] = json!("p1");
    let mut document = scene_with(vec![procedural, region], None, Some("brand"));
    document["seed"] = json!(7);

    let palette = shading_palette();
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("compiles");
    let filled = model
        .nodes
        .iter()
        .filter(|node| node.paint.fill.is_some())
        .count();
    assert!(
        filled > 1,
        "the stippling generates fill areas: {:?}",
        model.nodes
    );
    assert_eq!(
        shading_layers(&model).len(),
        filled,
        "every generated fill area is shaded on its own outline"
    );
}

// ---------------------------------------------------------------------------
// Element opacity and paint alpha
// ---------------------------------------------------------------------------

#[test]
fn element_opacity_and_paint_alpha_composite_as_one_element_paint() {
    // Base fill alpha and shade-tone alpha each apply to their own paint, and
    // the element's whole-element opacity applies to the composite (FEAT-028).
    let mut card = rect("card", 0, 0.0, 0.0, 100.0, 100.0);
    card["fill"] = token_paint("accent");
    card["opacity"] = json!(0.5);
    let document = scene_with(vec![card], None, Some("brand"));

    let palette = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#ff0000cc"), ("shadow", "#11111180")],
    ))
    .expect("a palette");
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);

    let model = compile_with(&document, &style).expect("compiles");
    let node = model.node("card").expect("the card");
    assert_eq!(node.opacity, 0.5, "the element opacity stays on the node");
    assert_eq!(
        common::fill_color(node),
        Some("#ff0000cc"),
        "the fill token's own alpha applies to the base fill"
    );
    assert_eq!(
        common::color(&node.shading.as_ref().expect("a shading layer").paint),
        Some("#11111180"),
        "the shade token's own alpha applies to the shading layer"
    );

    // The SVG applies the element opacity once, on the group wrapping both
    // paints, rather than on either paint alone.
    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("opacity=\"0.5\""), "{svg}");
    assert_eq!(
        svg.matches("opacity=\"0.5\"").count(),
        1,
        "element opacity is applied once to the composite: {svg}"
    );
    assert!(svg.contains("fill=\"#ff0000cc\""), "{svg}");
    assert!(svg.contains("fill=\"#11111180\""), "{svg}");
}

#[test]
fn an_element_opacity_below_one_composites_svg_and_png_as_one_element_paint() {
    // Opaque base and shade tones with element opacity 0.5 over a white
    // background: the composite is painted first and then faded, so the shaded
    // half is the shade tone over the base fill, then faded, rather than each
    // paint faded independently.
    let mut card = rect("card", 0, 0.0, 0.0, 100.0, 100.0);
    card["fill"] = token_paint("accent");
    card["opacity"] = json!(0.5);
    let document = scene_with(vec![card], None, Some("brand"));

    let palette = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#ff0000"), ("shadow", "#0000ff")],
    ))
    .expect("a palette");
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);
    let model = compile_with(&document, &style).expect("compiles");

    let image = decode_png(&vectr_core::export_png(&model, &Default::default()).expect("png"));
    // A one-step fade of an opaque composite rounds to 128; the double-fade a
    // per-paint opacity would produce is tens of levels away, so a small
    // tolerance still catches it.
    let close8 = |actual: (u8, u8, u8), expected: (u8, u8, u8)| {
        actual
            .0
            .abs_diff(expected.0)
            .max(actual.1.abs_diff(expected.1))
            .max(actual.2.abs_diff(expected.2))
            <= 2
    };
    let lit = image.pixel(50, 20).expect("in bounds").demultiply();
    let lit = (lit.red(), lit.green(), lit.blue());
    assert!(
        close8(lit, (255, 128, 128)),
        "the lit half is the base fill faded once: {lit:?}"
    );
    let shaded = image.pixel(50, 80).expect("in bounds").demultiply();
    let shaded = (shaded.red(), shaded.green(), shaded.blue());
    assert!(
        close8(shaded, (128, 128, 255)),
        "the shaded half is the shade tone over the base fill, faded once: {shaded:?}"
    );
}

#[test]
fn the_element_opacity_is_applied_once_to_the_pdf_composite() {
    // FEAT-028's one-element-paint rule across the PDF target: a shaded node
    // below full opacity paints its fill and shading layer into an isolated
    // transparency group and fades the group once, rather than fading each
    // paint (which would double-fade the shade where it overlaps the base).
    let mut card = rect("card", 0, 0.0, 0.0, 100.0, 100.0);
    card["fill"] = token_paint("accent");
    card["opacity"] = json!(0.5);
    let document = scene_with(vec![card], None, Some("brand"));

    let palette = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#ff0000"), ("shadow", "#0000ff")],
    ))
    .expect("a palette");
    let look = single_layer("shadow", Some("top"));
    let style = context(Some(&palette), &[], &[], Some(&look), &[]);
    let model = compile_with(&document, &style).expect("compiles");

    let pdf = pdf_text(&vectr_core::export_pdf(&model, &Default::default()).expect("pdf"));

    // The element alpha is the group's alpha, and the group is an isolated
    // transparency group invoked once under that alpha.
    assert!(pdf.contains("/ca 0.5 /CA 0.5"), "{pdf}");
    assert!(
        pdf.contains("/Group << /Type /Group /S /Transparency") && pdf.contains("/I true"),
        "the composite is an isolated transparency group: {pdf}"
    );
    assert!(pdf.contains("/Subtype /Form"), "{pdf}");
    assert!(pdf.contains("/XObject <<"), "{pdf}");

    // Inside the group each paint carries only its own alpha, with the base fill
    // below the shading layer; the element opacity is not folded into a paint.
    let form = pdf_form_stream(&pdf).expect("a form XObject");
    assert!(
        form.contains("1 0 0 rg"),
        "the base fill is inside the group: {form}"
    );
    assert!(
        form.contains("0 0 1 rg"),
        "the shading layer is inside the group: {form}"
    );
    let fill = form.find("1 0 0 rg").expect("the base fill");
    let shade = form.find("0 0 1 rg").expect("the shading layer");
    assert!(fill < shade, "the shading paints above the fill: {form}");
    assert!(
        !form.contains("/ca 0.5"),
        "the element opacity is not applied to either paint alone: {form}"
    );
}

// ---------------------------------------------------------------------------
// C-004 — the look reaches a project's output
// ---------------------------------------------------------------------------

/// A project whose scene selects a flat recipe requesting single-layer shading.
fn shading_project(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    dir.write(
        "vectr.project.json",
        r#"{"defaultSceneId":"main","defaultPaletteId":"brand"}"#,
    );
    dir.write(
        "recipes/shaded.json",
        &json!({
            "id": "shaded", "projectId": "p", "name": "flat",
            "parameters": { "shading": "single-layer", "lightDirection": "top", "shadeToken": "shadow" }
        })
        .to_string(),
    );
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "#ff0000"), ("shadow", "#111111")]),
    );
    let mut card = rect("card", 0, 0.0, 0.0, 100.0, 100.0);
    card["fill"] = token_paint("accent");
    let mut document = scene_with(vec![card], None, Some("brand"));
    document["recipeId"] = json!("shaded");
    write_scene_as(&dir, "main", document);
    dir
}

#[test]
fn a_scene_selected_shading_recipe_reaches_compiled_output_and_svg() {
    let dir = shading_project("shading-cli-compile");
    let output = run_vectr(dir.path(), &["compile", "main", "--out", "model.json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = std::fs::read_to_string(dir.path().join("model.json")).expect("the model");
    let model = vectr_core::render::parse(&text).expect("a render model");
    assert!(
        model.nodes[0].shading.is_some(),
        "the project's recipe reaches the compiled model: {:?}",
        model.nodes
    );

    let output = run_vectr(
        dir.path(),
        &["export", "main", "--format", "svg", "--out", "scene.svg"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let svg = std::fs::read_to_string(dir.path().join("scene.svg")).expect("the svg");
    assert!(svg.contains("fill=\"#111111\""), "{svg}");
}
