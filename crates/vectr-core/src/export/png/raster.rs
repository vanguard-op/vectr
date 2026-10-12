//! The rasterizer seam behind PNG export (FEAT-013, FEAT-015).
//!
//! PNG output is produced by rasterizing the same SVG document the vector
//! exporter emits, so a raster image is the vector rendering at the requested
//! size rather than a second, independent interpretation of the render model.
//! The rasterizer is `resvg` over `tiny-skia` (D-007); it is compiled in behind
//! the `rasterizer` feature, and a build without it reports the missing
//! capability instead of failing silently (NFR-004's degradation table).
//!
//! A raster-assisted layer is composited here rather than by the rasterizer,
//! which is built without its raster-image decoder (D-007): the vector content
//! is rasterized in bands around each layer and the decoded layer is drawn
//! between them, so the layer paints in document order (FEAT-015).

use crate::export::svg::{self, RasterMode};
use crate::primitives::Shape;
use crate::render::{RenderModel, ResolvedNode};
use crate::scene::{Diagnostic, Diagnostics};

use super::RASTERIZER;

/// Rasterizes an SVG document to PNG bytes at an exact pixel size.
#[cfg(feature = "rasterizer")]
pub(crate) fn rasterize(svg: &str, width: u32, height: u32) -> Result<Vec<u8>, Diagnostics> {
    render(svg, width, height)?.encode_png().map_err(|error| {
        failure(format!(
            "the rasterizer could not encode the image: {error}"
        ))
    })
}

/// Rasterizes an SVG document to a decoded pixel buffer at an exact pixel size.
#[cfg(feature = "rasterizer")]
fn render(svg: &str, width: u32, height: u32) -> Result<resvg::tiny_skia::Pixmap, Diagnostics> {
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_str(svg, &options).map_err(|error| {
        failure(format!(
            "the rasterizer could not read the drawing: {error}"
        ))
    })?;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(|| {
        failure("the rasterizer could not allocate the output surface".to_string())
    })?;

    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// Composites a render model's raster layers into a PNG at an exact pixel size.
///
/// The background is painted once, the vector content is rasterized in bands
/// around each layer, and each layer's decoded image is drawn at its bounding
/// box under its resolved transform, so the layer falls in document order
/// (FEAT-015). A layer whose asset is not a decodable image is a located error
/// and no image, rather than a silently dropped layer (NFR-011).
#[cfg(feature = "rasterizer")]
pub(crate) fn compose(
    model: &RenderModel,
    width: u32,
    height: u32,
    background: &str,
    diagnostics: &mut Diagnostics,
) -> Result<Vec<u8>, Diagnostics> {
    let (pixel_width, pixel_height) = (f64::from(width), f64::from(height));

    let mut canvas = resvg::tiny_skia::Pixmap::new(width, height).ok_or_else(|| {
        failure("the rasterizer could not allocate the output surface".to_string())
    })?;

    // The background is drawn once, on its own, so a translucent canvas colour
    // is not stacked by the bands that follow.
    let backdrop = svg::emit_band(
        model,
        &[],
        pixel_width,
        pixel_height,
        background,
        RasterMode::Skip,
        diagnostics,
    );
    draw(&mut canvas, render(&backdrop, width, height)?);

    let (scale_x, scale_y) = scene_scale(model, pixel_width, pixel_height);
    let mut start = 0usize;
    for (index, node) in model.nodes.iter().enumerate() {
        if node.raster.is_none() {
            continue;
        }
        draw_vector_band(
            &mut canvas,
            model,
            &model.nodes[start..index],
            width,
            height,
            diagnostics,
        )?;
        draw_layer(&mut canvas, node, scale_x, scale_y)?;
        start = index + 1;
    }
    draw_vector_band(
        &mut canvas,
        model,
        &model.nodes[start..],
        width,
        height,
        diagnostics,
    )?;

    canvas.encode_png().map_err(|error| {
        failure(format!(
            "the rasterizer could not encode the image: {error}"
        ))
    })
}

/// Rasterizes one band of vector content, with no background, onto `canvas`.
#[cfg(feature = "rasterizer")]
fn draw_vector_band(
    canvas: &mut resvg::tiny_skia::Pixmap,
    model: &RenderModel,
    nodes: &[ResolvedNode],
    width: u32,
    height: u32,
    diagnostics: &mut Diagnostics,
) -> Result<(), Diagnostics> {
    if nodes.is_empty() {
        return Ok(());
    }
    let svg = svg::emit_band(
        model,
        nodes,
        f64::from(width),
        f64::from(height),
        "transparent",
        RasterMode::Skip,
        diagnostics,
    );
    draw(canvas, render(&svg, width, height)?);
    Ok(())
}

/// Draws one raster layer's decoded image at its placement and size.
#[cfg(feature = "rasterizer")]
fn draw_layer(
    canvas: &mut resvg::tiny_skia::Pixmap,
    node: &ResolvedNode,
    scale_x: f64,
    scale_y: f64,
) -> Result<(), Diagnostics> {
    if !node.visible || node.opacity <= 0.0 {
        return Ok(());
    }
    let image = node.raster.as_ref().expect("a raster layer");
    let Some(Shape::Rect(rect)) = node.geometry.as_ref() else {
        return Err(failure(format!(
            "raster layer `{}` has no bounding box to place it",
            node.id
        )));
    };
    let pixmap = resvg::tiny_skia::Pixmap::decode_png(&image.data).map_err(|error| {
        failure(format!(
            "raster asset `{}` placed by element `{}` is not a PNG image: {error}",
            image.asset_id, node.id
        ))
    })?;
    let (image_width, image_height) = (f64::from(pixmap.width()), f64::from(pixmap.height()));
    if image_width <= 0.0 || image_height <= 0.0 {
        return Ok(());
    }

    // Image pixel -> bounding box local point -> scene point -> output pixel.
    let placement = crate::composition::Affine {
        a: rect.width / image_width,
        b: 0.0,
        c: 0.0,
        d: rect.height / image_height,
        e: rect.x,
        f: rect.y,
    };
    let composed = crate::composition::Affine::scale(scale_x, scale_y)
        .then(node.transform)
        .then(placement);
    if !composed.is_finite() {
        return Err(failure(format!(
            "raster layer `{}` has a non-finite transform",
            node.id
        )));
    }
    let transform = resvg::tiny_skia::Transform::from_row(
        composed.a as f32,
        composed.b as f32,
        composed.c as f32,
        composed.d as f32,
        composed.e as f32,
        composed.f as f32,
    );
    let paint = resvg::tiny_skia::PixmapPaint {
        opacity: node.opacity.clamp(0.0, 1.0) as f32,
        ..Default::default()
    };
    canvas.draw_pixmap(0, 0, pixmap.as_ref(), &paint, transform, None);
    Ok(())
}

/// Copies `layer` onto `canvas` at one to one.
#[cfg(feature = "rasterizer")]
fn draw(canvas: &mut resvg::tiny_skia::Pixmap, layer: resvg::tiny_skia::Pixmap) {
    canvas.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &resvg::tiny_skia::PixmapPaint::default(),
        resvg::tiny_skia::Transform::identity(),
        None,
    );
}

/// The scale from scene units to output pixels along each axis.
#[cfg(feature = "rasterizer")]
fn scene_scale(model: &RenderModel, width: f64, height: f64) -> (f64, f64) {
    let canvas = &model.canvas;
    if canvas.width > 0.0 && canvas.height > 0.0 {
        (width / canvas.width, height / canvas.height)
    } else {
        (1.0, 1.0)
    }
}

/// Reports the missing rasterizer when the crate is built without one.
#[cfg(not(feature = "rasterizer"))]
pub(crate) fn rasterize(_svg: &str, _width: u32, _height: u32) -> Result<Vec<u8>, Diagnostics> {
    Err(missing_rasterizer())
}

/// Reports the missing rasterizer for a composited raster export.
#[cfg(not(feature = "rasterizer"))]
pub(crate) fn compose(
    _model: &RenderModel,
    _width: u32,
    _height: u32,
    _background: &str,
    _diagnostics: &mut Diagnostics,
) -> Result<Vec<u8>, Diagnostics> {
    Err(missing_rasterizer())
}

#[cfg(not(feature = "rasterizer"))]
fn missing_rasterizer() -> Diagnostics {
    Diagnostics::from(Diagnostic::error(
        RASTERIZER,
        "PNG export requires the rasterizer, which is not available in this build",
    ))
}

#[cfg(feature = "rasterizer")]
fn failure(message: String) -> Diagnostics {
    Diagnostics::from(Diagnostic::error(RASTERIZER, message))
}
