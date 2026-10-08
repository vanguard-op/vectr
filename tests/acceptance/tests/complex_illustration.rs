//! Acceptance tests for composition primitives and the complex-illustration
//! class (FEAT-003, FEAT-011).
//!
//! FEAT-003's composition primitives — grouping, transforms, repetition and
//! grids, booleans, placement along a path, outline offsetting and projection —
//! are the axis along which scene complexity grows. This file exercises each
//! primitive's acceptance criterion, then compounds them: a deeply nested
//! composition, a scene that combines a grid, a boolean and an along-path
//! placement, and the documented complex-illustration class — a dense
//! composition resolving to the large-scene element count deterministically
//! with no element silently dropped (FEAT-011, nfr.md "Complex illustration").

mod common;

use common::*;
use serde_json::json;
use vectr_core::{DiagnosticCode, Shape, StyleContext};

/// A rect placed by its transform, so a transform's effect on a primitive is
/// read from the resolved world transform.
fn shape_extent_x(node: &vectr_core::ResolvedNode) -> (f64, f64) {
    let Shape::Rect(rect) = node.geometry.as_ref().expect("concrete geometry") else {
        panic!("expected a resolved rect: {:?}", node.geometry);
    };
    let left = node.transform.apply([rect.x, rect.y]);
    let right = node.transform.apply([rect.x + rect.width, rect.y]);
    (left[0].min(right[0]), left[0].max(right[0]))
}

#[test]
fn a_translate_transform_displaces_geometry_by_the_stated_offset() {
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["transform"]["translateX"] = json!(30.0);
    card["transform"]["translateY"] = json!(40.0);
    let document = scene(vec![card]);

    let model = compile_doc(&document);
    let node = model.node("card").expect("the card");
    let origin = node.transform.apply([0.0, 0.0]);
    assert!(
        close(origin[0], 30.0) && close(origin[1], 40.0),
        "the stated offset displaces the geometry: {origin:?}"
    );
}

#[test]
fn a_rotate_transform_rotates_geometry_about_its_origin() {
    let mut card = rect("card", 0, 10.0, 0.0, 10.0, 10.0);
    card["transform"]["rotate"] = json!(90.0);
    let document = scene(vec![card]);

    let model = compile_doc(&document);
    let node = model.node("card").expect("the card");
    // The local point (10, 0) turns 90° about the origin to (0, 10).
    let turned = node.transform.apply([10.0, 0.0]);
    assert!(
        close(turned[0], 0.0) && close(turned[1], 10.0),
        "the geometry rotates about its origin: {turned:?}"
    );
}

#[test]
fn a_scale_transform_scales_geometry_about_its_origin() {
    let mut card = rect("card", 0, 10.0, 0.0, 10.0, 10.0);
    card["transform"]["scaleX"] = json!(2.0);
    card["transform"]["scaleY"] = json!(3.0);
    let document = scene(vec![card]);

    let model = compile_doc(&document);
    let node = model.node("card").expect("the card");
    let scaled = node.transform.apply([10.0, 10.0]);
    assert!(
        close(scaled[0], 20.0) && close(scaled[1], 30.0),
        "the geometry scales about its origin: {scaled:?}"
    );
}

#[test]
fn a_skew_transform_shears_geometry_about_its_origin() {
    let mut card = rect("card", 0, 0.0, 0.0, 10.0, 10.0);
    card["transform"]["skewX"] = json!(45.0);
    let document = scene(vec![card]);

    let model = compile_doc(&document);
    let node = model.node("card").expect("the card");
    let sheared = node.transform.apply([0.0, 10.0]);
    assert!(
        close(sheared[0], 10.0) && close(sheared[1], 10.0),
        "a skewX of 45° shears x by y: {sheared:?}"
    );
}

#[test]
fn a_negative_scale_reflects_geometry_across_that_axis() {
    let mut card = rect("card", 0, 10.0, 0.0, 5.0, 10.0);
    card["transform"]["scaleX"] = json!(-1.0);
    let document = scene(vec![card]);

    let model = compile_doc(&document);
    let node = model.node("card").expect("the card");
    let (left, right) = shape_extent_x(node);
    assert!(
        close(left, -15.0) && close(right, -10.0),
        "the geometry reflects across the y axis: [{left}, {right}]"
    );
}

#[test]
fn a_mirrored_copy_is_symmetric_about_its_shared_axis() {
    // The motif covers x in [10, 15]; its mirror about the y axis covers
    // x in [-15, -10], the point reflection of the motif's span.
    let mut mirror = rect("mirror", 1, 10.0, 0.0, 5.0, 10.0);
    mirror["transform"]["scaleX"] = json!(-1.0);
    let document = scene(vec![
        group("pair", 0, Some("Pair")),
        {
            let mut motif = rect("motif", 0, 10.0, 0.0, 5.0, 10.0);
            motif["parentId"] = json!("pair");
            motif
        },
        {
            mirror["parentId"] = json!("pair");
            mirror
        },
    ]);

    let model = compile_doc(&document);
    let (left, right) = shape_extent_x(model.node("motif").expect("the motif"));
    let (mirror_left, mirror_right) = shape_extent_x(model.node("mirror").expect("the mirror"));
    assert!(
        close(mirror_left, -right) && close(mirror_right, -left),
        "the copy is symmetric about the shared axis: motif [{left}, {right}], mirror [{mirror_left}, {mirror_right}]"
    );
}

#[test]
fn a_repeat_nested_in_a_repeat_renders_an_instance_at_every_grid_position() {
    // Outer rows of 3 at 20 units, inner columns of 2 at 10 units: 6 copies
    // land on the lattice {0, 10, 20, 30, 40, 50}.
    let document = scene(vec![
        element("rows", 0, "repeat", json!({ "count": 3, "spacing": 20.0 })),
        {
            let mut columns = element(
                "columns",
                0,
                "repeat",
                json!({ "count": 2, "spacing": 10.0 }),
            );
            columns["parentId"] = json!("rows");
            columns
        },
        {
            let mut cell = rect("cell", 0, 0.0, 0.0, 5.0, 5.0);
            cell["parentId"] = json!("columns");
            cell
        },
    ]);

    let model = compile_doc(&document);
    assert_eq!(model.nodes.len(), 6, "{:?}", model.nodes);
    let mut xs: Vec<f64> = model
        .nodes
        .iter()
        .map(|node| node.transform.apply([0.0, 0.0])[0])
        .collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let expected = [0.0, 10.0, 20.0, 30.0, 40.0, 50.0];
    assert!(
        xs.iter().zip(expected).all(|(got, want)| close(*got, want)),
        "an instance renders at every grid position: {xs:?}"
    );
}

#[test]
fn a_boolean_union_combines_both_operands() {
    let boolean = element("merge", 0, "boolean", json!({ "operation": "union" }));
    let mut left = rect("left", 0, 0.0, 0.0, 100.0, 100.0);
    left["parentId"] = json!("merge");
    let mut right = rect("right", 1, 50.0, 0.0, 100.0, 100.0);
    right["parentId"] = json!("merge");
    let document = scene(vec![boolean, left, right]);

    let model = compile_doc(&document);
    assert_eq!(model.nodes.len(), 1, "{:?}", model.nodes);
    let geometry = model.nodes[0].geometry.as_ref().expect("resolved geometry");
    // Two 100×100 squares overlapping over 50×100 leave 15,000 units visible.
    assert!(
        close(shape_area(geometry), 15_000.0),
        "the union is A and B combined: {}",
        shape_area(geometry)
    );
}

#[test]
fn a_boolean_intersect_keeps_only_the_overlap() {
    let boolean = element("overlap", 0, "boolean", json!({ "operation": "intersect" }));
    let mut left = rect("left", 0, 0.0, 0.0, 100.0, 100.0);
    left["parentId"] = json!("overlap");
    let mut right = rect("right", 1, 50.0, 0.0, 100.0, 100.0);
    right["parentId"] = json!("overlap");
    let document = scene(vec![boolean, left, right]);

    let model = compile_doc(&document);
    assert_eq!(model.nodes.len(), 1, "{:?}", model.nodes);
    let geometry = model.nodes[0].geometry.as_ref().expect("resolved geometry");
    assert!(
        close(shape_area(geometry), 5_000.0),
        "only the 50×100 overlap is visible: {}",
        shape_area(geometry)
    );
}

#[test]
fn an_element_placed_along_a_path_follows_the_guide_with_the_stated_count() {
    let guide = element(
        "trail",
        0,
        "alongPath",
        json!({ "count": 3, "pathData": "M0 0 L30 0" }),
    );
    let mut dot = rect("dot", 0, 0.0, 0.0, 2.0, 2.0);
    dot["parentId"] = json!("trail");
    let document = scene(vec![guide, dot]);

    let model = compile_doc(&document);
    assert_eq!(model.nodes.len(), 3, "{:?}", model.nodes);
    let mut xs: Vec<f64> = model
        .nodes
        .iter()
        .map(|node| node.transform.apply([0.0, 0.0])[0])
        .collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(
        close(xs[0], 0.0) && close(xs[1], 15.0) && close(xs[2], 30.0),
        "the copies follow the path: {xs:?}"
    );
}

#[test]
fn an_outline_offset_grows_and_shrinks_the_outline_by_the_distance() {
    let outward = {
        let offset = element("grow", 0, "offset", json!({ "distance": 5.0 }));
        let mut square = rect("square", 0, 0.0, 0.0, 10.0, 10.0);
        square["parentId"] = json!("grow");
        scene(vec![offset, square])
    };
    let grown = compile_doc(&outward);
    let area = shape_area(grown.nodes[0].geometry.as_ref().expect("geometry"));
    assert!(
        close(area, 400.0),
        "a 10×10 square offset outward by 5 grows to 20×20: {area}"
    );

    let inward = {
        let offset = element("shrink", 0, "offset", json!({ "distance": -2.0 }));
        let mut square = rect("square", 0, 0.0, 0.0, 10.0, 10.0);
        square["parentId"] = json!("shrink");
        scene(vec![offset, square])
    };
    let shrunk = compile_doc(&inward);
    let area = shape_area(shrunk.nodes[0].geometry.as_ref().expect("geometry"));
    assert!(
        close(area, 36.0),
        "a 10×10 square offset inward by 2 shrinks to 6×6: {area}"
    );
}

#[test]
fn children_are_mapped_onto_the_projection_axes() {
    let x_projection = {
        let projection = element("proj", 0, "projection", json!({ "axis": "x" }));
        let mut card = rect("card", 0, 10.0, 20.0, 5.0, 5.0);
        card["parentId"] = json!("proj");
        scene(vec![projection, card])
    };
    let model = compile_doc(&x_projection);
    let node = model.node("card").expect("the projected card");
    let mapped = node.transform.apply([10.0, 20.0]);
    assert!(
        close(mapped[0], 10.0) && close(mapped[1], 0.0),
        "the x projection keeps x and drops y: {mapped:?}"
    );

    let isometric = {
        let projection = element("proj", 0, "projection", json!({ "axis": "isometric" }));
        let mut card = rect("card", 0, 10.0, 20.0, 5.0, 5.0);
        card["parentId"] = json!("proj");
        scene(vec![projection, card])
    };
    let model = compile_doc(&isometric);
    let node = model.node("card").expect("the projected card");
    let x_axis = node.transform.apply([1.0, 0.0]);
    let y_axis = node.transform.apply([0.0, 1.0]);
    assert!(
        x_axis[0] > 0.0 && x_axis[1] > 0.0,
        "the isometric x axis rises to the right: {x_axis:?}"
    );
    assert!(
        y_axis[0] < 0.0 && y_axis[1] > 0.0,
        "the isometric y axis rises to the left: {y_axis:?}"
    );
}

#[test]
fn a_composition_nested_several_levels_deep_resolves_completely_in_paint_order() {
    // outer group > inner group > repeat > cell group > boolean, two copies.
    let document = scene(vec![
        group("outer", 0, Some("Outer")),
        {
            let mut mid = group("mid", 0, Some("Mid"));
            mid["parentId"] = json!("outer");
            mid
        },
        {
            let mut rows = element("rows", 0, "repeat", json!({ "count": 2, "spacing": 30.0 }));
            rows["parentId"] = json!("mid");
            rows
        },
        {
            let mut cell = group("cell", 0, Some("Cell"));
            cell["parentId"] = json!("rows");
            cell
        },
        {
            let mut merge = element("merge", 0, "boolean", json!({ "operation": "union" }));
            merge["parentId"] = json!("cell");
            merge
        },
        {
            let mut a = rect("a", 0, 0.0, 0.0, 10.0, 10.0);
            a["parentId"] = json!("merge");
            a
        },
        {
            let mut b = rect("b", 1, 5.0, 5.0, 10.0, 10.0);
            b["parentId"] = json!("merge");
            b
        },
    ]);

    let model = compile_doc(&document);
    assert!(!model.diagnostics.has_errors(), "{:?}", model.diagnostics);
    assert_eq!(model.nodes.len(), 2, "{:?}", model.nodes);
    for node in &model.nodes {
        assert!(
            node.geometry.is_some(),
            "every level resolves to concrete geometry: {node:?}"
        );
        let chain: Vec<&str> = node.groups.iter().map(|group| group.id.as_str()).collect();
        assert_eq!(
            chain,
            vec!["outer", "mid", "cell"],
            "each node carries the full ancestor group chain, outermost first"
        );
    }
    // Paint order is document order of the repeat copies.
    let xs: Vec<f64> = model
        .nodes
        .iter()
        .map(|node| node.transform.apply([0.0, 0.0])[0])
        .collect();
    assert!(
        close(xs[0], 0.0) && close(xs[1], 30.0),
        "the copies paint in order: {xs:?}"
    );
}

#[test]
fn a_scene_combining_a_grid_a_boolean_and_an_along_path_placement_renders() {
    let document = scene(vec![
        // A repeated grid of three unioned badges.
        element("grid", 0, "repeat", json!({ "count": 3, "spacing": 20.0 })),
        {
            let mut badge = element("badge", 0, "boolean", json!({ "operation": "union" }));
            badge["parentId"] = json!("grid");
            badge
        },
        {
            let mut a = rect("a", 0, 0.0, 0.0, 10.0, 10.0);
            a["parentId"] = json!("badge");
            a
        },
        {
            let mut b = rect("b", 1, 5.0, 5.0, 10.0, 10.0);
            b["parentId"] = json!("badge");
            b
        },
        // A placement along a path of two dots.
        element(
            "trail",
            1,
            "alongPath",
            json!({ "count": 2, "pathData": "M0 40 L30 40" }),
        ),
        {
            let mut dot = rect("dot", 0, 0.0, 0.0, 2.0, 2.0);
            dot["parentId"] = json!("trail");
            dot
        },
    ]);

    let model = compile_doc(&document);
    assert!(!model.diagnostics.has_errors(), "{:?}", model.diagnostics);
    assert_eq!(model.nodes.len(), 5, "{:?}", model.nodes);
    for node in &model.nodes {
        assert!(node.geometry.is_some(), "no operation is dropped: {node:?}");
    }

    let svg = vectr_core::export_svg(&model, &Default::default()).expect("exports");
    assert_eq!(
        svg.matches("<g id=").count(),
        5,
        "every operation renders: {svg}"
    );
    assert!(svg.contains("<g id=\"badge-grid-0\""), "{svg}");
    assert!(svg.contains("<g id=\"badge-grid-2\""), "{svg}");
    assert!(svg.contains("<g id=\"dot-trail-1\""), "{svg}");
}

/// A dense composition: a repeat of `rows` rows, each a repeat of `columns`
/// cells. The scene holds a handful of elements and expands to
/// `rows * columns` render nodes — the complex-illustration class nfr.md
/// documents, where the count arises from dense composition.
fn dense_illustration(rows: u32, columns: u32) -> serde_json::Value {
    scene_with(
        vec![
            {
                let mut rows_element = element(
                    "rows",
                    0,
                    "repeat",
                    json!({ "count": rows, "spacing": 6.0 }),
                );
                rows_element["name"] = json!("Rows");
                rows_element
            },
            {
                let mut columns_element = element(
                    "columns",
                    0,
                    "repeat",
                    json!({ "count": columns, "spacing": 4.0 }),
                );
                columns_element["parentId"] = json!("rows");
                columns_element
            },
            {
                let mut cell = rect("cell", 0, 0.0, 0.0, 3.0, 3.0);
                cell["parentId"] = json!("columns");
                cell["fill"] = token_paint("accent");
                cell
            },
        ],
        None,
        Some("brand"),
    )
}

#[test]
fn the_documented_large_scene_compiles_completely_and_deterministically() {
    // 200 rows × 250 columns = 50,000 rendered elements: the documented
    // complex-illustration count (nfr.md, "Complex illustration").
    let document = dense_illustration(200, 250);
    let brand =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#abcdef")])).expect("a palette");
    let style = StyleContext {
        palette: Some(&brand),
        strokes: &[],
        gradients: &[],
        fonts: &[],
        recipe: None,
    };

    let first = compile_with(&document, &style).expect("a complex illustration compiles");
    assert!(!first.diagnostics.has_errors(), "{:?}", first.diagnostics);
    assert_eq!(
        first.nodes.len(),
        50_000,
        "the render model is complete: no element dropped"
    );
    for node in &first.nodes {
        assert!(
            node.geometry.is_some(),
            "every element resolves to concrete geometry: {node:?}"
        );
        assert_eq!(fill_color(node), Some("#abcdef"), "{node:?}");
    }

    let second = compile_with(&document, &style).expect("compiles again");
    assert_eq!(
        first.to_json_string().expect("serializable"),
        second.to_json_string().expect("serializable"),
        "identical input yields identical output (NFR-010)"
    );
}

#[test]
fn a_composition_above_the_documented_large_scene_count_is_warned_not_silent() {
    // 200 rows × 300 columns = 60,000 rendered elements, above the documented
    // 50,000 complex-illustration count (nfr.md, "Complex illustration"): the
    // scaling trigger says such a scene is processed with a warning rather than
    // silently (NFR-011, nfr.md "Scaling trigger").
    let document = dense_illustration(200, 300);
    let brand =
        vectr_core::parse_palette(&palette("brand", &[("accent", "#abcdef")])).expect("a palette");
    let style = StyleContext {
        palette: Some(&brand),
        strokes: &[],
        gradients: &[],
        fonts: &[],
        recipe: None,
    };

    let model = compile_with(&document, &style).expect("a dense composition compiles");
    assert_eq!(model.nodes.len(), 60_000, "the render model is complete");
    assert!(
        model
            .diagnostics
            .iter()
            .any(|finding| finding.code.as_str() == "W_LARGE_SCENE"),
        "a composition above the documented large-scene count is processed with a warning, not silently: {:?}",
        model.diagnostics
    );
}

#[test]
fn a_large_composition_reports_an_unprocessable_element_rather_than_dropping_it() {
    // A raster layer the compiler does not yet accept, buried under the dense
    // composition: it is named as an error, never silently dropped (FEAT-011).
    let document = scene(vec![
        element("rows", 0, "repeat", json!({ "count": 4, "spacing": 10.0 })),
        {
            let mut columns = element(
                "columns",
                0,
                "repeat",
                json!({ "count": 4, "spacing": 10.0 }),
            );
            columns["parentId"] = json!("rows");
            columns
        },
        {
            let mut tile = element("tile", 0, "raster", json!({}));
            tile["parentId"] = json!("columns");
            tile
        },
    ]);

    let scene = parse_scene(&document);
    let diagnostics = vectr_core::compile(&scene).expect_err("the raster is refused");
    let error = diagnostics.errors().next().expect("an error");
    assert!(
        error.message.contains("tile"),
        "the unprocessable element is named: {}",
        error.message
    );
    assert!(
        error.message.contains("raster"),
        "the unsupported feature is named: {}",
        error.message
    );
}

#[test]
fn a_composition_whose_expansion_exceeds_the_limit_is_refused_naming_it() {
    // Each individual count is under the per-element budget, but the nested
    // expansion is above it: the error must name the composition that
    // overflows, not report a bare limit (FEAT-003 edge case).
    let document = scene(vec![
        element(
            "rows",
            0,
            "repeat",
            json!({ "count": 2_000, "spacing": 1.0 }),
        ),
        {
            let mut columns = element(
                "columns",
                0,
                "repeat",
                json!({ "count": 1_000, "spacing": 1.0 }),
            );
            columns["parentId"] = json!("rows");
            columns
        },
        {
            let mut cell = rect("cell", 0, 0.0, 0.0, 1.0, 1.0);
            cell["parentId"] = json!("columns");
            cell
        },
    ]);

    let scene = parse_scene(&document);
    let diagnostics = vectr_core::compile(&scene).expect_err("the expansion is bounded");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, DiagnosticCode::SIZE_LIMIT);
    assert!(
        error.message.contains("rows") || error.message.contains("columns"),
        "the defined size limit names the composition that overflows: {}",
        error.message
    );
}
