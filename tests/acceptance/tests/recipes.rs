//! Acceptance tests for the four version-one style recipes (FEAT-007–FEAT-010).
//!
//! Drives the shipped library through its public API (C-002) and the `vectr`
//! binary as a subprocess (C-004). A recipe is applied by `compile_with_style`
//! through the caller's [`StyleContext`]; these tests prove each recipe's
//! acceptance criteria and edge cases on the render model and the exported SVG,
//! and that a recipe selected through a project reaches compiled output.

mod common;

use common::*;
use serde_json::json;
use vectr_core::style::{
    FREEFORM_CURVE, GRID_SNAPPED, GRID_TOO_FINE, ISOMETRIC_OFF_AXIS, LINE_ART_EMPTY,
    STROKE_WEIGHT_CLAMPED, TEXTURE_UNSUPPORTED,
};
use vectr_core::{
    DiagnosticCode, Diagnostics, Gradient, Paint, Palette, Shape, StrokeProfile, StyleContext,
    StyleRecipe,
};

/// A style recipe document with the given name and parameters.
fn recipe(name: &str, parameters: serde_json::Value) -> StyleRecipe {
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

/// A style context over the assets a test supplies, with no fonts (no scene
/// here draws text).
fn context<'a>(
    palette: Option<&'a Palette>,
    strokes: &'a [StrokeProfile],
    gradients: &'a [Gradient],
    recipe: Option<&'a StyleRecipe>,
) -> StyleContext<'a> {
    StyleContext {
        palette,
        strokes,
        gradients,
        fonts: &[],
        recipe,
    }
}

/// Whether the findings carry a warning with the given code.
fn warns(diagnostics: &Diagnostics, code: DiagnosticCode) -> bool {
    diagnostics.warnings().any(|warning| warning.code == code)
}

/// The concrete point list of a polygon or line node, panicking otherwise.
fn points(node: &vectr_core::ResolvedNode) -> Vec<[f64; 2]> {
    match node.geometry.as_ref().expect("a geometry node") {
        Shape::Polygon(polygon) => polygon.points.clone(),
        Shape::Line(line) => line.points.clone(),
        other => panic!("expected a point list, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// FEAT-007 — Flat / 2D recipe
// ---------------------------------------------------------------------------

#[test]
fn a_flat_scene_fills_with_solid_palette_colours_and_honours_an_explicit_gradient() {
    let mut solid = rect("solid", 0, 0.0, 0.0, 40.0, 40.0);
    solid["fill"] = token_paint("accent");
    let mut faded = rect("faded", 1, 50.0, 0.0, 40.0, 40.0);
    faded["fill"] = gradient_paint("fade");
    let document = scene_with(vec![solid, faded], None, Some("brand"));

    let palette = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#e94560"), ("ink", "#1a1a2e")],
    ))
    .expect("a palette");
    let gradients = [vectr_core::parse_gradient(
        &json!({
            "id": "fade", "projectId": "p", "name": "Fade", "type": "linear",
            "stops": [{ "offset": 0, "token": "accent" }, { "offset": 1, "token": "ink" }]
        })
        .to_string(),
    )
    .expect("a gradient")];
    let flat = recipe("flat", json!({}));
    let style = context(Some(&palette), &[], &gradients, Some(&flat));

    let model = compile_with(&document, &style).expect("compiles");
    assert_eq!(model.meta.recipe.as_deref(), Some("flat"));

    // A plain fill is the palette colour, not a texture or a gradient.
    assert_eq!(fill_color(model.node("solid").unwrap()), Some("#e94560"));
    // An element that explicitly requests a gradient is honoured as the one
    // exception; the stops resolve to the tokens they name.
    let Paint::Gradient(gradient) = model.node("faded").unwrap().paint.fill.as_ref().unwrap()
    else {
        panic!("expected the explicit gradient to survive the flat recipe");
    };
    assert_eq!(gradient.stops[0].color, "#e94560");
    assert_eq!(gradient.stops[1].color, "#1a1a2e");

    // No texture: the language has no texture request, so no node is a raster
    // layer and the document carries no filter or turbulence.
    assert!(model.nodes.iter().all(|node| node.kind != "raster"));
    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(!svg.contains("feTurbulence"), "{svg}");
    assert!(!svg.contains("<filter"), "{svg}");
}

#[test]
fn every_colour_in_a_flat_scene_comes_from_the_palette() {
    let mut first = rect("first", 0, 0.0, 0.0, 40.0, 40.0);
    first["fill"] = token_paint("accent");
    let mut second = rect("second", 1, 20.0, 20.0, 40.0, 40.0);
    second["fill"] = token_paint("ink");
    let third = {
        let mut element = rect("third", 2, 10.0, 10.0, 40.0, 40.0);
        element["stroke"] = stroke("outline", "accent");
        element
    };
    let document = scene_with(vec![first, second, third], None, Some("brand"));

    let palette = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#e94560"), ("ink", "#1a1a2e")],
    ))
    .expect("a palette");
    let outline =
        vectr_core::parse_stroke_profile(&stroke_profile("outline", 2.0, "round", "round"))
            .expect("a profile");
    let flat = recipe("flat", json!({}));
    let style = context(
        Some(&palette),
        std::slice::from_ref(&outline),
        &[],
        Some(&flat),
    );

    let model = compile_with(&document, &style).expect("compiles");
    let palette_values = ["#e94560", "#1a1a2e"];
    for node in &model.nodes {
        if let Some(fill) = node.paint.fill.as_ref().and_then(color) {
            assert!(
                palette_values.contains(&fill),
                "fill `{fill}` is not a palette colour"
            );
        }
        if let Some(stroke) = &node.paint.stroke {
            let value = color(&stroke.paint).expect("a solid stroke colour");
            assert!(
                palette_values.contains(&value),
                "stroke `{value}` is not a palette colour"
            );
        }
    }
}

#[test]
fn overlapping_flat_shapes_keep_clean_edges_with_no_unintended_blending() {
    let mut under = rect("under", 0, 0.0, 0.0, 60.0, 60.0);
    under["fill"] = token_paint("accent");
    let mut over = rect("over", 1, 30.0, 30.0, 60.0, 60.0);
    over["fill"] = token_paint("ink");
    let document = scene_with(vec![under, over], None, Some("brand"));

    let palette = vectr_core::parse_palette(&palette(
        "brand",
        &[("accent", "#e94560"), ("ink", "#1a1a2e")],
    ))
    .expect("a palette");
    let flat = recipe("flat", json!({}));
    let style = context(Some(&palette), &[], &[], Some(&flat));

    let model = compile_with(&document, &style).expect("compiles");
    for node in &model.nodes {
        assert_eq!(node.opacity, 1.0, "the flat look adds no opacity blending");
    }

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(!svg.contains("mix-blend-mode"), "{svg}");
    assert!(!svg.contains("opacity="), "{svg}");
    assert!(!svg.contains("<filter"), "{svg}");
}

#[test]
fn a_transparent_fill_is_supported_and_rendered_as_transparency() {
    let mut card = rect("card", 0, 0.0, 0.0, 40.0, 40.0);
    card["fill"] = token_paint("clear");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette = vectr_core::parse_palette(&palette("brand", &[("clear", "transparent")]))
        .expect("a palette");
    let flat = recipe("flat", json!({}));
    let style = context(Some(&palette), &[], &[], Some(&flat));

    let model = compile_with(&document, &style).expect("compiles");
    assert_eq!(fill_color(model.node("card").unwrap()), Some("transparent"));
    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("fill=\"transparent\""), "{svg}");
}

#[test]
fn a_flat_recipe_that_requests_texture_reports_it_and_stays_flat() {
    let mut card = rect("card", 0, 0.0, 0.0, 40.0, 40.0);
    card["fill"] = token_paint("accent");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#e94560")])).expect("a palette");
    let flat = recipe("flat", json!({ "shading": "raster" }));
    let style = context(Some(&palette), &[], &[], Some(&flat));

    let model = compile_with(&document, &style).expect("a texture request is not fatal");
    assert!(
        warns(&model.diagnostics, TEXTURE_UNSUPPORTED),
        "the unexpressible texture is reported: {:?}",
        model.diagnostics
    );
    assert!(model.nodes.iter().all(|node| node.kind != "raster"));
}

// ---------------------------------------------------------------------------
// FEAT-008 — Line-art recipe
// ---------------------------------------------------------------------------

#[test]
fn a_line_art_scene_fixes_the_stroke_weight_and_takes_paint_from_the_palette() {
    let mut card = rect("card", 0, 0.0, 0.0, 40.0, 40.0);
    card["stroke"] = stroke("hairline", "accent");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#0000ff")])).expect("a palette");
    let hairline =
        vectr_core::parse_stroke_profile(&stroke_profile("hairline", 0.0, "round", "bevel"))
            .expect("a profile");
    let line_art = recipe("line-art", json!({ "strokeWeight": 2.5 }));
    let style = context(
        Some(&palette),
        std::slice::from_ref(&hairline),
        &[],
        Some(&line_art),
    );

    let model = compile_with(&document, &style).expect("compiles");
    assert_eq!(model.meta.recipe.as_deref(), Some("line-art"));
    let stroke = model
        .node("card")
        .unwrap()
        .paint
        .stroke
        .as_ref()
        .expect("the stroke");
    assert_eq!(stroke.width, 2.5, "the recipe fills a weight left unset");
    assert!(matches!(stroke.cap, vectr_core::StrokeCap::Round));
    assert!(matches!(stroke.join, vectr_core::StrokeJoin::Bevel));
    assert_eq!(color(&stroke.paint), Some("#0000ff"));

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("stroke=\"#0000ff\""), "{svg}");
    assert!(svg.contains("stroke-width=\"2.5\""), "{svg}");
}

#[test]
fn a_line_art_scene_honours_an_explicitly_varied_weight() {
    let mut thin = rect("thin", 0, 0.0, 0.0, 20.0, 20.0);
    thin["stroke"] = stroke("p2", "accent");
    let mut thick = rect("thick", 1, 30.0, 0.0, 20.0, 20.0);
    thick["stroke"] = stroke("p6", "accent");
    let document = scene_with(vec![thin, thick], None, Some("brand"));

    let palette =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#0000ff")])).expect("a palette");
    let strokes = [
        vectr_core::parse_stroke_profile(&stroke_profile("p2", 2.0, "butt", "miter"))
            .expect("a profile"),
        vectr_core::parse_stroke_profile(&stroke_profile("p6", 6.0, "butt", "miter"))
            .expect("a profile"),
    ];
    let line_art = recipe("line-art", json!({ "strokeWeight": 2.5 }));
    let style = context(Some(&palette), &strokes, &[], Some(&line_art));

    let model = compile_with(&document, &style).expect("compiles");
    let width = |id: &str| model.node(id).unwrap().paint.stroke.as_ref().unwrap().width;
    assert_eq!(width("thin"), 2.0, "an explicit weight is honoured");
    assert_eq!(width("thick"), 6.0, "an explicit weight is honoured");
}

#[test]
fn a_fill_disabled_shape_in_a_line_art_scene_renders_only_its_outline() {
    let mut card = rect("card", 0, 0.0, 0.0, 40.0, 40.0);
    card["stroke"] = stroke("hairline", "accent");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#0000ff")])).expect("a palette");
    let hairline =
        vectr_core::parse_stroke_profile(&stroke_profile("hairline", 0.0, "butt", "miter"))
            .expect("a profile");
    let line_art = recipe("line-art", json!({ "strokeWeight": 1.0 }));
    let style = context(
        Some(&palette),
        std::slice::from_ref(&hairline),
        &[],
        Some(&line_art),
    );

    let model = compile_with(&document, &style).expect("compiles");
    let node = model.node("card").unwrap();
    assert!(node.paint.fill.is_none(), "no fill, so only the outline");
    assert!(node.paint.stroke.is_some());

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert!(svg.contains("fill=\"none\""), "{svg}");
}

#[test]
fn a_line_art_weight_below_the_minimum_renderable_unit_is_clamped_and_warned() {
    let mut card = rect("card", 0, 0.0, 0.0, 40.0, 40.0);
    card["stroke"] = stroke("hairline", "accent");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#0000ff")])).expect("a palette");
    // A profile that states a positive width below the minimum is an explicit
    // but unrenderable weight.
    let hairline =
        vectr_core::parse_stroke_profile(&stroke_profile("hairline", 0.01, "butt", "miter"))
            .expect("a profile");
    let line_art = recipe("line-art", json!({ "strokeWeight": 2.0 }));
    let style = context(
        Some(&palette),
        std::slice::from_ref(&hairline),
        &[],
        Some(&line_art),
    );

    let model = compile_with(&document, &style).expect("compiles");
    let width = model
        .node("card")
        .unwrap()
        .paint
        .stroke
        .as_ref()
        .unwrap()
        .width;
    assert_eq!(width, vectr_core::style::MIN_STROKE_WEIGHT);
    assert!(
        warns(&model.diagnostics, STROKE_WEIGHT_CLAMPED),
        "{:?}",
        model.diagnostics
    );
}

#[test]
fn a_self_intersecting_stroke_path_renders_without_failing() {
    let mut bowtie = element(
        "bowtie",
        0,
        "path",
        json!({ "pathData": "M0 0 L20 20 L20 0 L0 20 Z" }),
    );
    bowtie["stroke"] = stroke("hairline", "accent");
    let document = scene_with(vec![bowtie], None, Some("brand"));

    let palette =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#0000ff")])).expect("a palette");
    let hairline =
        vectr_core::parse_stroke_profile(&stroke_profile("hairline", 0.0, "butt", "miter"))
            .expect("a profile");
    let line_art = recipe("line-art", json!({ "strokeWeight": 2.0 }));
    let style = context(
        Some(&palette),
        std::slice::from_ref(&hairline),
        &[],
        Some(&line_art),
    );

    let model = compile_with(&document, &style).expect("a self-intersection is not an error");
    assert!(model.node("bowtie").unwrap().paint.stroke.is_some());
}

#[test]
fn a_line_art_scene_with_no_strokes_is_reported_as_empty() {
    let mut card = rect("card", 0, 0.0, 0.0, 40.0, 40.0);
    card["fill"] = token_paint("accent");
    let document = scene_with(vec![card], None, Some("brand"));

    let palette =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#0000ff")])).expect("a palette");
    let line_art = recipe("line-art", json!({ "strokeWeight": 2.0 }));
    let style = context(Some(&palette), &[], &[], Some(&line_art));

    let model = compile_with(&document, &style).expect("compiles");
    assert!(
        warns(&model.diagnostics, LINE_ART_EMPTY),
        "{:?}",
        model.diagnostics
    );
}

// ---------------------------------------------------------------------------
// FEAT-009 — Geometric recipe
// ---------------------------------------------------------------------------

#[test]
fn a_geometric_recipe_snaps_an_off_grid_element_and_warns() {
    let mut off = rect("off", 0, 0.0, 0.0, 10.0, 10.0);
    off["transform"]["translateX"] = json!(13.0);
    off["transform"]["translateY"] = json!(27.0);
    let mut on = rect("on", 1, 0.0, 0.0, 10.0, 10.0);
    on["transform"]["translateX"] = json!(20.0);
    on["transform"]["translateY"] = json!(20.0);
    let document = scene(vec![off, on]);

    let geometric = recipe("geometric", json!({ "gridSize": 10.0 }));
    let model =
        compile_with(&document, &context(None, &[], &[], Some(&geometric))).expect("compiles");

    assert_eq!(
        model.node("off").unwrap().transform.apply([0.0, 0.0]),
        [10.0, 30.0],
        "the off-grid element snaps to the nearest intersection"
    );
    assert!(
        model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == GRID_SNAPPED && warning.message.contains("off")),
        "{:?}",
        model.diagnostics
    );
    assert_eq!(
        model.node("on").unwrap().transform.apply([0.0, 0.0]),
        [20.0, 20.0],
        "an element already on the grid does not move"
    );
    assert!(
        !model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == GRID_SNAPPED && warning.message.contains("`on`")),
        "an on-grid element is not reported: {:?}",
        model.diagnostics
    );
}

#[test]
fn a_set_of_polygons_shares_consistent_vertices_on_the_grid() {
    let first = element(
        "first",
        0,
        "polygon",
        json!({ "points": [[0.1, 0.1], [20.2, 0.1], [10.1, 10.2]] }),
    );
    let second = element(
        "second",
        1,
        "polygon",
        json!({ "points": [[0.2, 0.2], [20.1, 0.2], [10.2, 10.1]] }),
    );
    let document = scene(vec![first, second]);

    let geometric = recipe("geometric", json!({ "gridSize": 10.0 }));
    let model =
        compile_with(&document, &context(None, &[], &[], Some(&geometric))).expect("compiles");

    // Near-identical declared vertices land on the same grid intersections, so
    // the two polygons share consistent, grid-defined edges.
    assert_eq!(
        points(model.node("first").unwrap()),
        vec![[0.0, 0.0], [20.0, 0.0], [10.0, 10.0]]
    );
    assert_eq!(
        points(model.node("second").unwrap()),
        vec![[0.0, 0.0], [20.0, 0.0], [10.0, 10.0]]
    );
}

#[test]
fn a_geometric_recipe_snaps_an_element_placed_by_its_geometry_origin() {
    // schema.md places a rect and an ellipse by the x and y origin of their
    // bounding box, so the geometric grid must snap that placement (FEAT-009).
    let card = rect("card", 0, 13.0, 27.0, 10.0, 10.0);
    let document = scene(vec![card]);

    let geometric = recipe("geometric", json!({ "gridSize": 10.0 }));
    let model =
        compile_with(&document, &context(None, &[], &[], Some(&geometric))).expect("compiles");

    let node = model.node("card").unwrap();
    let Shape::Rect(rect) = node.geometry.as_ref().unwrap() else {
        panic!("expected a rect");
    };
    let origin = node.transform.apply([rect.x, rect.y]);
    let on_grid = |value: f64| {
        let steps = value / 10.0;
        (steps - steps.round()).abs() < 1e-6
    };
    assert!(
        on_grid(origin[0]) && on_grid(origin[1]),
        "the placed origin {origin:?} should snap to a grid intersection"
    );
}

#[test]
fn a_freeform_curve_is_kept_as_an_explicit_exception_in_a_geometric_scene() {
    let curve = element(
        "curve",
        0,
        "path",
        json!({ "pathData": "M0 0 C 10 0 10 10 0 10" }),
    );
    let straight = element(
        "straight",
        1,
        "path",
        json!({ "pathData": "M0 0 L10 0 L10 10 Z" }),
    );
    let document = scene(vec![curve, straight]);

    let geometric = recipe("geometric", json!({ "gridSize": 10.0 }));
    let model = compile_with(&document, &context(None, &[], &[], Some(&geometric)))
        .expect("a declared curve is allowed");

    assert!(
        model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == FREEFORM_CURVE && warning.message.contains("curve")),
        "the explicitly requested curve is reported: {:?}",
        model.diagnostics
    );
    assert!(
        !model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == FREEFORM_CURVE && warning.message.contains("straight")),
        "a straight-edged path is not a freeform curve: {:?}",
        model.diagnostics
    );
    assert!(
        matches!(
            model.node("curve").unwrap().geometry.as_ref().unwrap(),
            Shape::Path(_)
        ),
        "the curve is kept, not silently replaced"
    );
}

#[test]
fn a_grid_finer_than_the_renderable_resolution_is_a_performance_warning() {
    let card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    let document = scene(vec![card]);

    let geometric = recipe("geometric", json!({ "gridSize": 0.01 }));
    let model = compile_with(&document, &context(None, &[], &[], Some(&geometric)))
        .expect("a fine grid is not fatal");

    assert!(
        warns(&model.diagnostics, GRID_TOO_FINE),
        "{:?}",
        model.diagnostics
    );
}

// ---------------------------------------------------------------------------
// FEAT-010 — Isometric recipe
// ---------------------------------------------------------------------------

#[test]
fn an_isometric_recipe_snaps_an_off_axis_element_onto_the_axes_and_warns() {
    let mut off = rect("off", 0, 0.0, 0.0, 10.0, 10.0);
    off["transform"]["translateX"] = json!(1.0);
    off["transform"]["translateY"] = json!(1.0);
    let mut on = rect("on", 1, 0.0, 0.0, 10.0, 10.0);
    on["transform"]["translateY"] = json!(10.0);
    let document = scene(vec![off, on]);

    let isometric = recipe("isometric", json!({ "gridSize": 10.0 }));
    let model =
        compile_with(&document, &context(None, &[], &[], Some(&isometric))).expect("compiles");

    assert_eq!(
        model.node("off").unwrap().transform.apply([0.0, 0.0]),
        [0.0, 0.0],
        "an off-axis point snaps to the nearest lattice intersection"
    );
    assert!(
        model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == ISOMETRIC_OFF_AXIS && warning.message.contains("off")),
        "{:?}",
        model.diagnostics
    );
    assert_eq!(
        model.node("on").unwrap().transform.apply([0.0, 0.0]),
        [0.0, 10.0],
        "an element already on the axes stays put"
    );
    assert!(
        !model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == ISOMETRIC_OFF_AXIS && warning.message.contains("`on`")),
        "{:?}",
        model.diagnostics
    );
}

#[test]
fn an_isometric_recipe_aligns_an_element_placed_by_its_geometry_origin() {
    // An ellipse is placed by the x and y origin of its bounding box
    // (schema.md), so the isometric grid must align that placement (FEAT-010).
    let disc = element(
        "disc",
        0,
        "ellipse",
        json!({ "x": 1.0, "y": 1.0, "width": 10.0, "height": 10.0 }),
    );
    let document = scene(vec![disc]);

    let isometric = recipe("isometric", json!({ "gridSize": 10.0 }));
    let model =
        compile_with(&document, &context(None, &[], &[], Some(&isometric))).expect("compiles");

    let node = model.node("disc").unwrap();
    let Shape::Ellipse(ellipse) = node.geometry.as_ref().unwrap() else {
        panic!("expected an ellipse");
    };
    // The element is placed by its bounding-box origin; alignment to the
    // isometric axes means that origin lies on the lattice the 30° axes span.
    let origin = node
        .transform
        .apply([ellipse.cx - ellipse.rx, ellipse.cy - ellipse.ry]);
    let cos = 3.0_f64.sqrt() / 2.0;
    let i = origin[0] / (2.0 * 10.0 * cos) + origin[1] / 10.0;
    let j = -origin[0] / (2.0 * 10.0 * cos) + origin[1] / 10.0;
    let on_lattice = |value: f64| (value - value.round()).abs() < 1e-6;
    assert!(
        on_lattice(i) && on_lattice(j),
        "the placed origin {origin:?} should snap onto the isometric axes"
    );
}

#[test]
fn stacked_elements_paint_back_to_front_by_isometric_depth() {
    // Declared front, back, then middle; depth ordering must reorder them.
    let mut front = rect("front", 0, 0.0, 0.0, 10.0, 10.0);
    front["transform"]["translateY"] = json!(20.0);
    let back = rect("back", 1, 0.0, 0.0, 10.0, 10.0);
    let mut middle = rect("middle", 2, 0.0, 0.0, 10.0, 10.0);
    middle["transform"]["translateY"] = json!(10.0);
    let document = scene(vec![front, back, middle]);

    let isometric = recipe("isometric", json!({ "gridSize": 10.0 }));
    let model =
        compile_with(&document, &context(None, &[], &[], Some(&isometric))).expect("compiles");

    let order: Vec<&str> = model.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(
        order,
        vec!["back", "middle", "front"],
        "the stack paints from the farthest depth forward"
    );
    assert!(
        model
            .nodes
            .windows(2)
            .all(|pair| pair[0].order < pair[1].order),
        "paint order increases with depth"
    );
}

#[test]
fn a_flat_shape_projected_through_an_isometric_projection_becomes_an_isometric_face() {
    let projection = element("proj", 0, "projection", json!({ "axis": "isometric" }));
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["parentId"] = json!("proj");
    let document = scene(vec![projection, card]);

    let isometric = recipe("isometric", json!({}));
    let model =
        compile_with(&document, &context(None, &[], &[], Some(&isometric))).expect("compiles");

    let transform = model.node("card").unwrap().transform;
    let axis = 3.0_f64.sqrt() / 2.0;
    assert!(close(transform.a, axis), "{transform:?}");
    assert!(close(transform.b, 0.5), "{transform:?}");
    assert!(close(transform.c, -axis), "{transform:?}");
    assert!(close(transform.d, 0.5), "{transform:?}");
    assert_ne!(transform, vectr_core::Affine::IDENTITY);
}

#[test]
fn a_non_isometric_shape_in_an_isometric_scene_is_allowed_as_a_billboard() {
    let mut disc = element(
        "disc",
        0,
        "ellipse",
        json!({ "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0 }),
    );
    disc["transform"]["translateY"] = json!(10.0);
    let document = scene(vec![disc]);

    let isometric = recipe("isometric", json!({ "gridSize": 10.0 }));
    let model =
        compile_with(&document, &context(None, &[], &[], Some(&isometric))).expect("compiles");

    let node = model.node("disc").unwrap();
    assert_eq!(
        node.geometry,
        Some(Shape::Ellipse(vectr_core::Ellipse {
            cx: 5.0,
            cy: 5.0,
            rx: 5.0,
            ry: 5.0,
        })),
        "the face geometry is left unprojected"
    );
    assert_eq!(node.transform.apply([0.0, 0.0]), [0.0, 10.0]);
}

#[test]
fn an_ambiguous_isometric_depth_keeps_document_order_deterministically() {
    let first = rect("first", 0, 0.0, 0.0, 10.0, 10.0);
    let second = rect("second", 1, 0.0, 0.0, 10.0, 10.0);
    let document = scene(vec![first, second]);

    let isometric = recipe("isometric", json!({ "gridSize": 10.0 }));
    let style = context(None, &[], &[], Some(&isometric));
    let model = compile_with(&document, &style).expect("compiles");
    let again = compile_with(&document, &style).expect("compiles");

    let order: Vec<&str> = model.nodes.iter().map(|node| node.id.as_str()).collect();
    assert_eq!(
        order,
        vec!["first", "second"],
        "a tie keeps the order the elements were declared in"
    );
    assert_eq!(model, again, "the tie-break is deterministic (NFR-010)");
}

// ---------------------------------------------------------------------------
// C-004 — Recipe selection through a project
// ---------------------------------------------------------------------------

/// A project whose scene names `recipe` and whose elements stroke a width-zero
/// profile, so the recipe's `strokeWeight` is the weight that reaches the model.
fn recipe_project(tag: &str, recipe_id: Option<&str>, default_recipe: Option<&str>) -> TempDir {
    let dir = TempDir::new(tag);
    let mut config = json!({});
    if let Some(default) = default_recipe {
        config["defaultRecipeId"] = json!(default);
    }
    dir.write("vectr.project.json", &config.to_string());
    dir.write(
        "recipes/line.json",
        &json!({
            "id": "line", "projectId": "p", "name": "line-art",
            "parameters": { "strokeWeight": 3 }
        })
        .to_string(),
    );
    dir.write(
        "strokes/hairline.json",
        &json!({
            "id": "hairline", "projectId": "p", "name": "Hairline",
            "width": 0, "cap": "butt", "join": "miter"
        })
        .to_string(),
    );
    dir.write(
        "palettes/brand.json",
        &palette("brand", &[("accent", "#0000ff")]),
    );
    let mut scene = json!({
        "id": "s", "projectId": "p", "name": "S", "formatVersion": VERSION,
        "paletteId": "brand",
        "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
        "elements": [{
            "id": "r1", "sceneId": "s", "order": 0, "kind": "rect",
            "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
            "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
            "stroke": { "profileId": "hairline", "paint": { "kind": "token", "ref": "accent" } },
            "opacity": 1, "visible": true
        }]
    });
    if let Some(recipe_id) = recipe_id {
        scene["recipeId"] = json!(recipe_id);
    }
    dir.write("scenes/s.json", &scene.to_string());
    dir
}

#[test]
fn a_scene_selected_recipe_reaches_the_compiled_output() {
    let dir = recipe_project("recipe-scene", Some("line"), None);
    let output = run_vectr(dir.path(), &["compile", "s", "--out", "model.json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = std::fs::read_to_string(dir.path().join("model.json")).expect("the model");
    let model = vectr_core::render::parse(&text).expect("a render model");
    assert_eq!(model.meta.recipe.as_deref(), Some("line-art"));
    let stroke = model.nodes[0]
        .paint
        .stroke
        .as_ref()
        .expect("the recipe's weight reaches the stroke");
    assert_eq!(stroke.width, 3.0);
    assert_eq!(common::color(&stroke.paint), Some("#0000ff"));
}

#[test]
fn a_recipe_selected_through_a_project_reaches_exported_svg() {
    let dir = recipe_project("recipe-export", Some("line"), None);
    let output = run_vectr(
        dir.path(),
        &["export", "s", "--format", "svg", "--out", "scene.svg"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let svg = std::fs::read_to_string(dir.path().join("scene.svg")).expect("the svg");
    assert!(svg.contains("stroke-width=\"3\""), "{svg}");
    assert!(svg.contains("stroke=\"#0000ff\""), "{svg}");
}

#[test]
fn a_project_default_recipe_applies_when_the_scene_names_none() {
    let dir = recipe_project("recipe-default", None, Some("line"));
    let output = run_vectr(dir.path(), &["compile", "s", "--out", "model.json"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = std::fs::read_to_string(dir.path().join("model.json")).expect("the model");
    let model = vectr_core::render::parse(&text).expect("a render model");
    assert_eq!(model.meta.recipe.as_deref(), Some("line-art"));
}

#[test]
fn an_unresolvable_recipe_is_reported_as_missing_input() {
    let dir = recipe_project("recipe-missing", Some("absent"), None);
    let output = run_vectr(dir.path(), &["compile", "s", "--check"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(stderr(&output).contains("absent"), "{}", stderr(&output));
}

#[test]
fn an_invalid_recipe_document_is_reported_with_a_location() {
    let dir = TempDir::new("recipe-invalid");
    dir.write("vectr.project.json", "{}");
    dir.write(
        "recipes/bad.json",
        &json!({
            "id": "bad", "projectId": "p", "name": "geometric",
            "parameters": { "gridSize": -1 }
        })
        .to_string(),
    );
    dir.write(
        "scenes/s.json",
        &json!({
            "id": "s", "projectId": "p", "name": "S", "formatVersion": VERSION,
            "recipeId": "bad",
            "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
            "elements": [{
                "id": "r1", "sceneId": "s", "order": 0, "kind": "rect",
                "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
                "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
                "opacity": 1, "visible": true
            }]
        })
        .to_string(),
    );

    let output = run_vectr(dir.path(), &["compile", "s", "--check"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(stderr(&output).contains("gridSize"), "{}", stderr(&output));
}

#[test]
fn a_recipe_compiled_scene_is_byte_identical_across_runs() {
    let mut off = rect("off", 0, 0.0, 0.0, 10.0, 10.0);
    off["transform"]["translateX"] = json!(13.0);
    off["fill"] = token_paint("accent");
    let polygon = element(
        "poly",
        1,
        "polygon",
        json!({ "points": [[0.1, 0.1], [20.2, 0.1], [10.1, 10.2]] }),
    );
    let document = scene_with(vec![off, polygon], None, Some("brand"));

    let palette =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#e94560")])).expect("a palette");
    let geometric = recipe("geometric", json!({ "gridSize": 10.0 }));
    let style = context(Some(&palette), &[], &[], Some(&geometric));

    let first = compile_with(&document, &style).expect("compiles");
    let second = compile_with(&document, &style).expect("compiles");
    assert_eq!(first, second, "the render model is deterministic (NFR-010)");
    assert_eq!(
        vectr_core::export_svg(&first, &Default::default()).expect("exports"),
        vectr_core::export_svg(&second, &Default::default()).expect("exports"),
        "the exported document is byte-identical (NFR-010)"
    );
}
