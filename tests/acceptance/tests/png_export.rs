//! Acceptance tests for PNG export (FEAT-013).
//!
//! Verifies that the raster matches the vector rendering at the requested size,
//! that background and scale options are honoured, that text rasterizes, and the
//! failure states — an invalid size and a size beyond the defined budget.

mod common;

use common::*;
use serde_json::json;
use vectr_core::export::png::{OPTIONS, SIZE_LIMIT};
use vectr_core::render::{Paint, RenderCanvas, RenderMeta, RenderModel, ResolvedNode};
use vectr_core::{Affine, Diagnostics, FontAsset, RasterOptions, Rect, Shape, StyleContext};

fn half_red() -> RenderModel {
    let mut shape = ResolvedNode {
        id: "e1".to_string(),
        name: None,
        order: 0,
        kind: "rect".to_string(),
        groups: Vec::new(),
        geometry: Some(Shape::Rect(Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
            rx: 0.0,
            ry: 0.0,
        })),
        text: None,
        transform: Affine::IDENTITY,
        paint: Paint {
            fill: Some("#ff0000".to_string()),
            stroke: None,
        },
        opacity: 1.0,
        visible: true,
    };
    shape.order = 0;
    RenderModel {
        canvas: RenderCanvas {
            width: 100.0,
            height: 50.0,
            background: "#ffffff".to_string(),
        },
        nodes: vec![shape],
        meta: RenderMeta::default(),
        diagnostics: Diagnostics::new(),
        fonts: Vec::new(),
    }
}

fn decode(bytes: &[u8]) -> tiny_skia::Pixmap {
    tiny_skia::Pixmap::decode_png(bytes).expect("a PNG")
}

#[test]
fn the_png_matches_the_vector_rendering_at_the_requested_size() {
    let bytes = vectr_core::export_png(&half_red(), &RasterOptions::default()).expect("exports");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");

    let image = decode(&bytes);
    assert_eq!((image.width(), image.height()), (100, 50), "canvas size");

    let inside = image.pixel(10, 10).expect("in bounds").demultiply();
    assert_eq!(
        (inside.red(), inside.green(), inside.blue(), inside.alpha()),
        (255, 0, 0, 255),
        "the filled half is red"
    );
    let outside = image.pixel(80, 10).expect("in bounds").demultiply();
    assert_eq!(
        (outside.red(), outside.green(), outside.blue()),
        (255, 255, 255),
        "the canvas background fills the rest"
    );
}

#[test]
fn a_background_override_and_transparency_are_honoured() {
    let transparent = vectr_core::export_png(
        &half_red(),
        &RasterOptions {
            background: Some("transparent".to_string()),
            ..RasterOptions::default()
        },
    )
    .expect("exports");
    let image = decode(&transparent);
    assert_eq!(image.pixel(80, 10).expect("in bounds").alpha(), 0);

    let black = vectr_core::export_png(
        &half_red(),
        &RasterOptions {
            background: Some("#000000".to_string()),
            ..RasterOptions::default()
        },
    )
    .expect("exports");
    let image = decode(&black);
    let pixel = image.pixel(80, 10).expect("in bounds").demultiply();
    assert_eq!((pixel.red(), pixel.green(), pixel.blue()), (0, 0, 0));
}

#[test]
fn a_scale_factor_scales_the_output_resolution() {
    let bytes = vectr_core::export_png(
        &half_red(),
        &RasterOptions {
            density: Some(2.0),
            ..RasterOptions::default()
        },
    )
    .expect("exports");
    let image = decode(&bytes);
    assert_eq!((image.width(), image.height()), (200, 100));
}

#[test]
fn a_single_output_dimension_preserves_the_aspect_ratio() {
    let bytes = vectr_core::export_png(
        &half_red(),
        &RasterOptions {
            width: Some(200.0),
            ..RasterOptions::default()
        },
    )
    .expect("exports");
    let image = decode(&bytes);
    assert_eq!((image.width(), image.height()), (200, 100));
}

#[test]
fn a_text_element_rasterizes_its_glyphs() {
    let mut wordmark = text("wordmark", 0, 5.0, 45.0, "I", 40.0);
    wordmark["fillToken"] = json!("ink");
    let mut document = scene_with(vec![wordmark], None, Some("brand"));
    document["canvas"] = json!({ "width": 64.0, "height": 64.0, "background": "#ffffff" });
    let palette =
        vectr_core::parse_palette(&palette("brand", &[("ink", "#000000")])).expect("a palette");
    let fonts = [FontAsset::new(
        vectr_core::DEFAULT_FONT_ID,
        "Inter",
        font_bytes("Inter.ttf"),
    )];
    let style = StyleContext {
        palette: Some(&palette),
        strokes: &[],
        fonts: &fonts,
    };
    let model = compile_with(&document, &style).expect("compiles");

    let bytes = vectr_core::export_png(&model, &RasterOptions::default()).expect("exports");
    let image = decode(&bytes);
    assert_eq!((image.width(), image.height()), (64, 64));
    let inked = (0..image.height()).any(|y| {
        (0..image.width()).any(|x| {
            let pixel = image.pixel(x, y).expect("in bounds").demultiply();
            pixel.red() < 64 && pixel.green() < 64 && pixel.blue() < 64 && pixel.alpha() > 0
        })
    });
    assert!(inked, "the glyph rasterizes to visible pixels");
}

#[test]
fn a_zero_or_negative_size_is_refused_naming_the_invalid_size() {
    for options in [
        RasterOptions {
            width: Some(0.0),
            ..RasterOptions::default()
        },
        RasterOptions {
            height: Some(-1.0),
            ..RasterOptions::default()
        },
        RasterOptions {
            density: Some(0.0),
            ..RasterOptions::default()
        },
    ] {
        let diagnostics = vectr_core::export_png(&half_red(), &options).expect_err("refused");
        assert_eq!(
            diagnostics.errors().next().map(|error| error.code.clone()),
            Some(OPTIONS),
            "{options:?}"
        );
    }
}

#[test]
fn a_size_beyond_the_available_budget_reports_a_clear_limit() {
    let diagnostics = vectr_core::export_png(
        &half_red(),
        &RasterOptions {
            width: Some(100_000.0),
            ..RasterOptions::default()
        },
    )
    .expect_err("refused");
    let error = diagnostics.errors().next().expect("an error");
    assert_eq!(error.code, SIZE_LIMIT);
    assert!(
        error.message.contains("exceeds the limit"),
        "{}",
        error.message
    );
}

#[test]
fn png_export_is_byte_identical_across_runs() {
    let options = RasterOptions {
        density: Some(1.5),
        ..RasterOptions::default()
    };
    let first = vectr_core::export_png(&half_red(), &options).expect("exports");
    let second = vectr_core::export_png(&half_red(), &options).expect("exports");
    assert_eq!(
        first, second,
        "identical input yields identical bytes (NFR-010)"
    );
}
