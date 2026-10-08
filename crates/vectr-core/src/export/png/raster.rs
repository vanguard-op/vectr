//! The rasterizer seam behind PNG export (FEAT-013).
//!
//! PNG output is produced by rasterizing the same SVG document the vector
//! exporter emits, so a raster image is the vector rendering at the requested
//! size rather than a second, independent interpretation of the render model.
//! The rasterizer is `resvg` over `tiny-skia` (D-007); it is compiled in behind
//! the `rasterizer` feature, and a build without it reports the missing
//! capability instead of failing silently (NFR-004's degradation table).

use crate::scene::Diagnostics;

use super::RASTERIZER;

/// Rasterizes an SVG document to PNG bytes at an exact pixel size.
#[cfg(feature = "rasterizer")]
pub(crate) fn rasterize(svg: &str, width: u32, height: u32) -> Result<Vec<u8>, Diagnostics> {
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

    pixmap.encode_png().map_err(|error| {
        failure(format!(
            "the rasterizer could not encode the image: {error}"
        ))
    })
}

/// Reports the missing rasterizer when the crate is built without one.
#[cfg(not(feature = "rasterizer"))]
pub(crate) fn rasterize(_svg: &str, _width: u32, _height: u32) -> Result<Vec<u8>, Diagnostics> {
    use crate::scene::Diagnostic;

    Err(Diagnostics::from(Diagnostic::error(
        RASTERIZER,
        "PNG export requires the rasterizer, which is not available in this build",
    )))
}

#[cfg(feature = "rasterizer")]
fn failure(message: String) -> Diagnostics {
    use crate::scene::Diagnostic;

    Diagnostics::from(Diagnostic::error(RASTERIZER, message))
}
