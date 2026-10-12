//! Shared raster-layer findings across the export targets (FEAT-015, C-003).
//!
//! A render model node of kind `raster` carries the placed image bytes alongside
//! its bounding-box geometry. PNG composites the layer, while the vector targets
//! omit it rather than approximating it, so the omission is reported on every
//! vector export. The size finding belongs to the exporter that actually places
//! the layer: an asset large enough to bloat the file is reported where it
//! still renders.
//!
//! The omission code is read by both vector exporters and the size code by the
//! PNG exporter, so each message is defined once and stays identical everywhere
//! it is raised.

use crate::render::ResolvedNode;
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, Location};

/// A raster layer a vector target could not represent, omitted with a warning.
pub const OMITTED: DiagnosticCode = DiagnosticCode::new("W_RASTER_OMITTED");

/// A raster asset large enough to bloat the output, warned but still placed.
pub const LARGE: DiagnosticCode = DiagnosticCode::new("W_RASTER_LARGE");

/// The largest raster asset side the exporter passes without a size warning.
///
/// A layer wider or taller than this is reported because a photographic asset
/// packed into a scene file is a file-size tax; it is a warning, so the layer
/// still renders (FEAT-015's size edge case).
pub const MAX_RASTER_SIDE: u32 = 4_096;

/// The largest raster asset file the exporter passes without a size warning.
pub const MAX_RASTER_BYTES: usize = 8 * 1024 * 1024;

/// Records that a vector target omitted a raster layer, naming the element and
/// the asset it places (FEAT-015, C-003).
///
/// The caller passes the resolved asset identifier rather than reading it off
/// the node, so a raster node whose image bytes are missing stays a distinct,
/// generic omission rather than being reported as a named asset.
pub(crate) fn warn_omitted(
    diagnostics: &mut Diagnostics,
    node: &ResolvedNode,
    asset_id: &str,
    target: &str,
) {
    diagnostics.push(
        Diagnostic::warning(
            OMITTED,
            format!(
                "element `{}` places raster asset `{asset_id}`, which {target} omits to stay pure vector",
                node.id
            ),
        )
        .with_location(Location::element(node.id.clone())),
    );
}

/// Records every placed raster layer whose asset is large enough to bloat the
/// file, naming the layer, the asset, and the measured value (FEAT-015).
///
/// The measurement is the asset's pixel dimensions when they can be read and
/// its byte length always; the finding is a warning, so the layer is not
/// dropped and the export still succeeds.
pub(crate) fn warn_large(model: &crate::render::RenderModel, diagnostics: &mut Diagnostics) {
    for node in &model.nodes {
        let Some(image) = &node.raster else {
            continue;
        };
        let dimensions = image_dimensions(&image.data);
        let over_pixels = dimensions
            .is_some_and(|(width, height)| width > MAX_RASTER_SIDE || height > MAX_RASTER_SIDE);
        let over_bytes = image.data.len() > MAX_RASTER_BYTES;
        if !over_pixels && !over_bytes {
            continue;
        }
        let measured = match dimensions {
            Some((width, height)) => {
                format!("{width}x{height} pixels and {} bytes", image.data.len())
            }
            None => format!("{} bytes", image.data.len()),
        };
        diagnostics.push(
            Diagnostic::warning(
                LARGE,
                format!(
                    "element `{}` places raster asset `{}` measuring {measured}, above the {} pixel / {} byte budget",
                    node.id, image.asset_id, MAX_RASTER_SIDE, MAX_RASTER_BYTES
                ),
            )
            .with_location(Location::element(node.id.clone())),
        );
    }
}

/// The pixel dimensions an image file declares in its header, when readable.
///
/// Only a header is read, so an oversized asset is measured without decoding
/// it; the four formats the rasterizer accepts (PNG, JPEG, GIF, WebP) are
/// recognized by their magic bytes, and anything else yields `None`, leaving
/// the byte measurement to stand alone.
fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    png_dimensions(data)
        .or_else(|| gif_dimensions(data))
        .or_else(|| jpeg_dimensions(data))
        .or_else(|| webp_dimensions(data))
}

fn png_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if data.len() < 24 || &data[..8] != SIGNATURE || &data[12..16] != b"IHDR" {
        return None;
    }
    Some((be32(data, 16)?, be32(data, 20)?))
}

fn gif_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 10 || (&data[..6] != b"GIF87a" && &data[..6] != b"GIF89a") {
        return None;
    }
    Some((
        u32::from(u16::from_le_bytes([data[6], data[7]])),
        u32::from(u16::from_le_bytes([data[8], data[9]])),
    ))
}

/// The dimensions a JPEG's start-of-frame marker declares.
///
/// The marker segments are walked to the first frame header (SOF0..SOF15,
/// excluding the DHT, DAC and restart markers that share the range), where the
/// sample precision, height and width are written.
fn jpeg_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 4 || data[0] != 0xff || data[1] != 0xd8 {
        return None;
    }
    let mut index = 2;
    while index + 3 < data.len() {
        if data[index] != 0xff {
            index += 1;
            continue;
        }
        let marker = data[index + 1];
        // Standalone markers carry no length; padding 0xff bytes have none
        // either.
        if marker == 0xff || (0xd0..=0xd9).contains(&marker) {
            index += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([data[index + 2], data[index + 3]]));
        let is_frame =
            (0xc0..=0xcf).contains(&marker) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc;
        if is_frame && index + 9 < data.len() {
            let height = u32::from(u16::from_be_bytes([data[index + 5], data[index + 6]]));
            let width = u32::from(u16::from_be_bytes([data[index + 7], data[index + 8]]));
            return Some((width, height));
        }
        index += 2 + length;
    }
    None
}

fn webp_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 16 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return None;
    }
    match &data[12..16] {
        b"VP8 " => {
            // A lossy bitstream stores the dimensions after the start code.
            if data.len() < 30 {
                return None;
            }
            let width = u32::from(u16::from_le_bytes([data[26], data[27]])) & 0x3fff;
            let height = u32::from(u16::from_le_bytes([data[28], data[29]])) & 0x3fff;
            Some((width, height))
        }
        b"VP8L" => {
            if data.len() < 25 {
                return None;
            }
            let bits = u32::from_le_bytes([data[21], data[22], data[23], data[24]]);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => {
            if data.len() < 30 {
                return None;
            }
            let width = u32::from_le_bytes([data[24], data[25], data[26], 0]) + 1;
            let height = u32::from_le_bytes([data[27], data[28], data[29], 0]) + 1;
            Some((width, height))
        }
        _ => None,
    }
}

fn be32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::Affine;
    use crate::primitives::{Rect, Shape};
    use crate::render::{RasterImage, RenderCanvas, RenderMeta, RenderModel};

    fn raster_node(id: &str, asset_id: &str, data: Vec<u8>) -> ResolvedNode {
        ResolvedNode {
            id: id.to_string(),
            name: None,
            accessible_name: None,
            order: 0,
            kind: "raster".to_string(),
            groups: Vec::new(),
            geometry: Some(Shape::Rect(Rect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                rx: 0.0,
                ry: 0.0,
            })),
            text: None,
            raster: Some(RasterImage {
                asset_id: asset_id.to_string(),
                data,
            }),
            transform: Affine::IDENTITY,
            paint: Default::default(),
            opacity: 1.0,
            visible: true,
        }
    }

    fn model(nodes: Vec<ResolvedNode>) -> RenderModel {
        RenderModel {
            canvas: RenderCanvas {
                width: 10.0,
                height: 10.0,
                background: "#ffffff".to_string(),
            },
            nodes,
            meta: RenderMeta::default(),
            diagnostics: Diagnostics::new(),
            fonts: Vec::new(),
        }
    }

    /// A minimal PNG header declaring `width` by `height`; the pixel data is
    /// not needed to measure it.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&height.to_be_bytes());
        data
    }

    #[test]
    fn an_omitted_layer_names_the_element_and_its_asset() {
        let mut diagnostics = Diagnostics::new();
        let node = raster_node("r1", "photo", png_header(2, 2));
        warn_omitted(&mut diagnostics, &node, "photo", "SVG export");
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, OMITTED);
        assert!(warning.message.contains("r1"), "{}", warning.message);
        assert!(warning.message.contains("photo"), "{}", warning.message);
        assert!(
            warning.message.contains("SVG export"),
            "{}",
            warning.message
        );
    }

    #[test]
    fn a_small_asset_is_not_reported() {
        let mut diagnostics = Diagnostics::new();
        warn_large(
            &model(vec![raster_node("r1", "photo", png_header(64, 64))]),
            &mut diagnostics,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn an_asset_wider_than_the_budget_is_reported_with_its_measurement() {
        let mut diagnostics = Diagnostics::new();
        warn_large(
            &model(vec![raster_node("r1", "photo", png_header(5_000, 100))]),
            &mut diagnostics,
        );
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, LARGE);
        assert!(warning.message.contains("r1"), "{}", warning.message);
        assert!(warning.message.contains("photo"), "{}", warning.message);
        assert!(warning.message.contains("5000x100"), "{}", warning.message);
    }

    #[test]
    fn an_asset_larger_on_disk_than_the_budget_is_reported() {
        let mut diagnostics = Diagnostics::new();
        let mut data = png_header(32, 32);
        data.resize(MAX_RASTER_BYTES + 1, 0);
        warn_large(
            &model(vec![raster_node("r1", "photo", data)]),
            &mut diagnostics,
        );
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, LARGE);
        assert!(warning.message.contains("bytes"), "{}", warning.message);
    }

    #[test]
    fn headers_of_each_supported_format_are_measured() {
        assert_eq!(png_dimensions(&png_header(12, 34)), Some((12, 34)));

        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&12u16.to_le_bytes());
        gif.extend_from_slice(&34u16.to_le_bytes());
        assert_eq!(gif_dimensions(&gif), Some((12, 34)));

        // SOI, a SOF0 frame header declaring 34x12, then end of image.
        let jpeg = [
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x22, 0x00, 0x0c, 0x03, 0x01, 0x11,
            0x00, 0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xd9,
        ];
        assert_eq!(jpeg_dimensions(&jpeg), Some((12, 34)));

        let mut webp = b"RIFF\x16\x00\x00\x00WEBPVP8X".to_vec();
        webp.extend_from_slice(&[0x0a, 0x00, 0x00, 0x00]); // chunk size
        webp.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // flags
        webp.extend_from_slice(&[11, 0x00, 0x00]); // canvas width minus one
        webp.extend_from_slice(&[33, 0x00, 0x00]); // canvas height minus one
        assert_eq!(webp_dimensions(&webp), Some((12, 34)));
    }
}
