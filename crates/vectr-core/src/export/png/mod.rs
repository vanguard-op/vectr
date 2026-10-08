//! PNG export: the render model rasterized to a bitmap (FEAT-013).
//!
//! [`export_png`] is the library entry point (C-002). It resolves the requested
//! pixel size, emits the model through the SVG writer at that size, and
//! rasterizes the result, so a PNG is the vector rendering at the requested
//! resolution rather than a second interpretation of the model. Width, height
//! and `density` compose into the output size: a single dimension scales the
//! other to keep the canvas aspect ratio, and `density` multiplies the resolved
//! size so a scale factor raises the resolution without changing the framing.
//!
//! # Failure and degradation
//!
//! A non-positive or non-finite dimension, or a size beyond the rasterizer's
//! defined budget, is a located error and no image (NFR-011). When the crate is
//! built without its rasterizer, PNG export reports the missing capability and
//! leaves every other stage — validation, compilation and SVG export — working
//! (NFR-004's degradation table).
//!
//! # Warnings
//!
//! PNG is rasterized from the SVG document, so content the SVG writer omits is
//! omitted here for the same reason; [`export_png_reporting`] returns those
//! warnings alongside the bytes, while [`export_png`] keeps the frozen shape.

use crate::export::svg::{export_svg_reporting, SvgOptions};
use crate::render::RenderModel;
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics};

mod raster;

/// The rasterizer is not available in this build, or could not produce an image.
pub const RASTERIZER: DiagnosticCode = DiagnosticCode::new("E_RASTERIZER");

/// A requested output dimension or scale factor is not usable.
pub const OPTIONS: DiagnosticCode = DiagnosticCode::new("E_PNG_OPTIONS");

/// The requested raster size exceeds the defined pixel budget.
pub const SIZE_LIMIT: DiagnosticCode = DiagnosticCode::new("E_RASTER_LIMIT");

/// The default scale factor when no density is requested.
const DEFAULT_DENSITY: f64 = 1.0;

/// The largest raster the exporter will attempt, in pixels.
///
/// The budget bounds the output buffer and the rasterizer's working memory so a
/// size that cannot be satisfied fails with a clear limit instead of exhausting
/// memory (FEAT-013's failure states). At four bytes per pixel this is roughly
/// 400 MB.
pub const MAX_RASTER_PIXELS: u64 = 100_000_000;

/// The largest raster the exporter will attempt, per side.
pub const MAX_RASTER_DIMENSION: u32 = 16_384;

/// Options controlling PNG output.
///
/// The defaults reproduce the canvas exactly at one pixel per scene unit: no
/// size override, a density of one, and the canvas's own background. A width or
/// height alone scales the other dimension to keep the canvas aspect ratio;
/// `density` then multiplies the resolved size. `transparent` (or `none`, or
/// empty) as the background leaves the image transparent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RasterOptions {
    /// Output width in pixels; the canvas width when absent.
    pub width: Option<f64>,
    /// Output height in pixels; the canvas height when absent.
    pub height: Option<f64>,
    /// Scale factor applied to the resolved size; one when absent.
    pub density: Option<f64>,
    /// Background override; the canvas background when absent.
    pub background: Option<String>,
}

/// An exported PNG image and the findings recorded while writing it.
#[derive(Debug, Clone, PartialEq)]
pub struct PngExport {
    /// The encoded PNG bytes.
    pub png: Vec<u8>,
    /// Warnings for content the target could not represent; errors never reach
    /// this type, because an export that cannot proceed returns `Err` instead.
    pub diagnostics: Diagnostics,
}

/// Exports a render model as PNG (C-002).
///
/// The image is returned as bytes; a request the exporter cannot satisfy is a
/// located diagnostic and no image. Content the raster target cannot represent
/// is omitted; use [`export_png_reporting`] to observe those warnings.
pub fn export_png(model: &RenderModel, options: &RasterOptions) -> Result<Vec<u8>, Diagnostics> {
    export_png_reporting(model, options).map(|export| export.png)
}

/// Exports a render model as PNG, returning the warnings alongside the bytes.
///
/// This is the same emission as [`export_png`]; it exists because the frozen
/// signature returns only the bytes and would otherwise drop the warnings the
/// SVG writer records for omitted content (C-003).
pub fn export_png_reporting(
    model: &RenderModel,
    options: &RasterOptions,
) -> Result<PngExport, Diagnostics> {
    let pixels = resolve_pixels(model, options)?;
    let background = options
        .background
        .as_deref()
        .unwrap_or(model.canvas.background.as_str());

    // Emit at the exact pixel size so the raster matches the vector rendering at
    // that size; the view box stays in canvas coordinates.
    let svg = export_svg_reporting(
        model,
        &SvgOptions {
            width: Some(f64::from(pixels.width)),
            height: Some(f64::from(pixels.height)),
            background: Some(background.to_string()),
        },
    )?;

    let png = raster::rasterize(&svg.svg, pixels.width, pixels.height)?;
    Ok(PngExport {
        png,
        diagnostics: svg.diagnostics,
    })
}

/// The exact pixel size of the output image.
struct Pixels {
    width: u32,
    height: u32,
}

/// Resolves the requested size into an exact, budgeted pixel count.
fn resolve_pixels(model: &RenderModel, options: &RasterOptions) -> Result<Pixels, Diagnostics> {
    let mut diagnostics = Diagnostics::new();

    if let Some(width) = options.width {
        validate_positive(&mut diagnostics, width, "width");
    }
    if let Some(height) = options.height {
        validate_positive(&mut diagnostics, height, "height");
    }
    let density = options.density.unwrap_or(DEFAULT_DENSITY);
    if !density.is_finite() || density <= 0.0 {
        diagnostics.push(Diagnostic::error(
            OPTIONS,
            format!("`density` must be a finite number greater than zero, got {density}"),
        ));
    }
    if diagnostics.has_errors() {
        return Err(diagnostics);
    }

    let canvas = &model.canvas;
    let mut width = options.width.unwrap_or(canvas.width);
    let mut height = options.height.unwrap_or(canvas.height);

    // A single dimension scales the other to preserve the canvas aspect ratio.
    let scalable = canvas.width > 0.0 && canvas.height > 0.0;
    match (options.width, options.height) {
        (Some(width), None) if scalable => height = width * canvas.height / canvas.width,
        (None, Some(height)) if scalable => width = height * canvas.width / canvas.height,
        _ => {}
    }

    let pixel_width = width * density;
    let pixel_height = height * density;
    if !pixel_width.is_finite() || pixel_width <= 0.0 {
        diagnostics.push(Diagnostic::error(
            OPTIONS,
            "the resolved raster width is not a positive finite number",
        ));
    }
    if !pixel_height.is_finite() || pixel_height <= 0.0 {
        diagnostics.push(Diagnostic::error(
            OPTIONS,
            "the resolved raster height is not a positive finite number",
        ));
    }
    if diagnostics.has_errors() {
        return Err(diagnostics);
    }

    let pixels = Pixels {
        width: to_pixels(pixel_width),
        height: to_pixels(pixel_height),
    };
    if pixels.width > MAX_RASTER_DIMENSION
        || pixels.height > MAX_RASTER_DIMENSION
        || u64::from(pixels.width) * u64::from(pixels.height) > MAX_RASTER_PIXELS
    {
        return Err(Diagnostics::from(Diagnostic::error(
            SIZE_LIMIT,
            format!(
                "the requested raster size {}x{} exceeds the limit of {} pixels with a maximum side of {}",
                pixels.width, pixels.height, MAX_RASTER_PIXELS, MAX_RASTER_DIMENSION
            ),
        )));
    }

    Ok(pixels)
}

fn validate_positive(diagnostics: &mut Diagnostics, value: f64, field: &str) {
    if !value.is_finite() || value <= 0.0 {
        diagnostics.push(Diagnostic::error(
            OPTIONS,
            format!("`{field}` must be a finite number greater than zero, got {value}"),
        ));
    }
}

/// Rounds a positive pixel extent to at least one whole pixel.
fn to_pixels(value: f64) -> u32 {
    let rounded = value.round();
    if rounded < 1.0 {
        1
    } else {
        rounded as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::Affine;
    use crate::primitives::{Rect, Shape};
    use crate::render::{Paint, RenderCanvas, RenderMeta, ResolvedNode};

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Shape {
        Shape::Rect(Rect {
            x,
            y,
            width,
            height,
            rx: 0.0,
            ry: 0.0,
        })
    }

    fn node(id: &str, geometry: Shape) -> ResolvedNode {
        ResolvedNode {
            id: id.to_string(),
            name: None,
            order: 0,
            kind: geometry.kind().to_string(),
            groups: Vec::new(),
            geometry,
            transform: Affine::IDENTITY,
            paint: Paint::default(),
            opacity: 1.0,
            visible: true,
        }
    }

    fn model(nodes: Vec<ResolvedNode>) -> RenderModel {
        RenderModel {
            canvas: RenderCanvas {
                width: 100.0,
                height: 50.0,
                background: "#ffffff".to_string(),
            },
            nodes,
            meta: RenderMeta::default(),
            diagnostics: Diagnostics::new(),
        }
    }

    /// A model with a red rectangle covering the left half of the canvas.
    fn half_red() -> RenderModel {
        let mut shape = node("e1", rect(0.0, 0.0, 50.0, 50.0));
        shape.paint.fill = Some("#ff0000".to_string());
        model(vec![shape])
    }

    #[cfg(feature = "rasterizer")]
    fn decode(bytes: &[u8]) -> resvg::tiny_skia::Pixmap {
        resvg::tiny_skia::Pixmap::decode_png(bytes).expect("a PNG")
    }

    #[cfg(feature = "rasterizer")]
    #[test]
    fn a_compiled_scene_exports_a_png_with_the_canvas_size() {
        let bytes = export_png(&half_red(), &RasterOptions::default()).expect("exports");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
        let image = decode(&bytes);
        assert_eq!((image.width(), image.height()), (100, 50));
    }

    #[cfg(feature = "rasterizer")]
    #[test]
    fn the_raster_matches_the_vector_rendering_at_that_size() {
        let bytes = export_png(&half_red(), &RasterOptions::default()).expect("exports");
        let image = decode(&bytes);

        let inside = image.pixel(10, 10).expect("in bounds").demultiply();
        assert_eq!((inside.red(), inside.green(), inside.blue()), (255, 0, 0));
        assert_eq!(inside.alpha(), 255);

        let outside = image.pixel(80, 10).expect("in bounds").demultiply();
        assert_eq!(
            (outside.red(), outside.green(), outside.blue()),
            (255, 255, 255)
        );
    }

    #[cfg(feature = "rasterizer")]
    #[test]
    fn a_single_output_dimension_preserves_the_aspect_ratio() {
        let bytes = export_png(
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

    #[cfg(feature = "rasterizer")]
    #[test]
    fn an_explicit_size_overrides_the_canvas() {
        let bytes = export_png(
            &half_red(),
            &RasterOptions {
                width: Some(64.0),
                height: Some(64.0),
                ..RasterOptions::default()
            },
        )
        .expect("exports");
        let image = decode(&bytes);
        assert_eq!((image.width(), image.height()), (64, 64));
    }

    #[cfg(feature = "rasterizer")]
    #[test]
    fn a_scale_factor_scales_the_resolution() {
        let bytes = export_png(
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

    #[cfg(feature = "rasterizer")]
    #[test]
    fn a_transparent_background_leaves_the_image_transparent() {
        let bytes = export_png(
            &half_red(),
            &RasterOptions {
                background: Some("transparent".to_string()),
                ..RasterOptions::default()
            },
        )
        .expect("exports");
        let image = decode(&bytes);
        let outside = image.pixel(80, 10).expect("in bounds");
        assert_eq!(outside.alpha(), 0);
    }

    #[cfg(feature = "rasterizer")]
    #[test]
    fn a_background_override_is_honoured() {
        let bytes = export_png(
            &half_red(),
            &RasterOptions {
                background: Some("#000000".to_string()),
                ..RasterOptions::default()
            },
        )
        .expect("exports");
        let image = decode(&bytes);
        let outside = image.pixel(80, 10).expect("in bounds").demultiply();
        assert_eq!((outside.red(), outside.green(), outside.blue()), (0, 0, 0));
        assert_eq!(outside.alpha(), 255);
    }

    #[cfg(feature = "rasterizer")]
    #[test]
    fn repeated_exports_are_byte_identical() {
        let document = half_red();
        let options = RasterOptions {
            density: Some(1.5),
            ..RasterOptions::default()
        };
        assert_eq!(
            export_png(&document, &options).expect("exports"),
            export_png(&document, &options).expect("exports")
        );
    }

    #[cfg(feature = "rasterizer")]
    #[test]
    fn a_raster_node_is_omitted_with_a_warning() {
        let mut shape = node("r1", rect(0.0, 0.0, 1.0, 1.0));
        shape.kind = "raster".to_string();
        let export =
            export_png_reporting(&model(vec![shape]), &RasterOptions::default()).expect("exports");
        assert_eq!(
            export.diagnostics.warnings().next().map(|w| w.code.clone()),
            Some(crate::export::svg::UNSUPPORTED)
        );
    }

    #[test]
    fn an_invalid_size_or_scale_is_refused() {
        let document = model(Vec::new());
        let invalid = [
            RasterOptions {
                width: Some(0.0),
                ..RasterOptions::default()
            },
            RasterOptions {
                height: Some(-1.0),
                ..RasterOptions::default()
            },
            RasterOptions {
                width: Some(f64::NAN),
                ..RasterOptions::default()
            },
            RasterOptions {
                density: Some(0.0),
                ..RasterOptions::default()
            },
            RasterOptions {
                density: Some(f64::INFINITY),
                ..RasterOptions::default()
            },
        ];
        for options in invalid {
            let diagnostics = export_png(&document, &options).expect_err("refused");
            assert_eq!(
                diagnostics.errors().next().map(|e| e.code.clone()),
                Some(OPTIONS),
                "{options:?}"
            );
        }
    }

    #[test]
    fn a_size_beyond_the_budget_is_refused_with_a_limit() {
        let document = model(Vec::new());
        for options in [
            RasterOptions {
                width: Some(100_000.0),
                ..RasterOptions::default()
            },
            RasterOptions {
                width: Some(12_000.0),
                height: Some(12_000.0),
                ..RasterOptions::default()
            },
        ] {
            let diagnostics = export_png(&document, &options).expect_err("refused");
            let error = diagnostics.errors().next().expect("an error");
            assert_eq!(error.code, SIZE_LIMIT);
            assert!(
                error.message.contains("exceeds the limit"),
                "{}",
                error.message
            );
        }
    }

    #[test]
    fn a_degenerate_canvas_is_refused() {
        let mut document = model(Vec::new());
        document.canvas.width = 0.0;
        let diagnostics = export_png(&document, &RasterOptions::default()).expect_err("refused");
        assert_eq!(
            diagnostics.errors().next().map(|e| e.code.clone()),
            Some(OPTIONS)
        );
    }

    #[cfg(not(feature = "rasterizer"))]
    #[test]
    fn a_missing_rasterizer_is_reported() {
        let diagnostics = export_png(&half_red(), &RasterOptions::default()).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, RASTERIZER);
    }
}
