//! PDF export: the render model as a vector PDF for print (FEAT-014).
//!
//! [`export_pdf`] is the library entry point (C-002). It writes one page whose
//! vector content stays vector — geometry is emitted as PDF path operators and
//! a text run's glyphs are outlined, so the document renders without the font
//! installed — and it never goes through the rasterizer. The page is sized by
//! the options, the named print colour profile is honoured, and the background
//! is validated exactly as SVG's is, so an unusable colour is refused before
//! any document is produced (FEAT-018).
//!
//! # Page size and profile
//!
//! `pageWidth`/`pageHeight` are in PDF points; the canvas is mapped onto the
//! page, so a page larger or smaller than the canvas scales the artwork rather
//! than clipping it. A single dimension scales the other to keep the canvas
//! aspect ratio. When neither is given, the canvas size is used and a warning
//! is raised, because a print page that nobody chose is worth reporting.
//!
//! `profile` names the output colour space: `srgb` (the default) writes device
//! RGB and carries transparency; `cmyk` writes device CMYK and flattens
//! transparency, reporting that it did. An unknown profile is a located error
//! and no document (NFR-011).
//!
//! # Failure and degradation
//!
//! A non-positive or non-finite page dimension, or an unusable background, is a
//! located error and no document. Content the vector target cannot represent —
//! a raster layer, or geometry that is not finite — is omitted with a warning,
//! exactly as the SVG writer does (C-003).

use crate::render::RenderModel;
use crate::scene::{validate_color, Diagnostic, DiagnosticCode, Diagnostics};

mod color;
mod emit;

/// A requested page dimension is not usable.
pub const OPTIONS: DiagnosticCode = DiagnosticCode::new("E_PDF_OPTIONS");

/// The named print colour profile is not one this exporter supports.
pub const PROFILE: DiagnosticCode = DiagnosticCode::new("E_PDF_PROFILE");

/// A node the PDF target cannot represent, omitted with a warning.
pub const UNSUPPORTED: DiagnosticCode = DiagnosticCode::new("W_UNSUPPORTED_PDF");

/// No page size was requested, so the canvas size was applied.
pub const NO_PAGE_SIZE: DiagnosticCode = DiagnosticCode::new("W_PDF_NO_PAGE_SIZE");

/// Transparency the chosen profile cannot carry was flattened.
pub const TRANSPARENCY_FLATTENED: DiagnosticCode =
    DiagnosticCode::new("W_PDF_TRANSPARENCY_FLATTENED");

/// Options controlling PDF output (C-002).
///
/// The defaults reproduce the canvas as the page, use the sRGB profile, and
/// draw the canvas's own background. `transparent` (or `none`, or empty) as the
/// background leaves the page unpainted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdfOptions {
    /// Page width in PDF points; the canvas width when absent.
    pub page_width: Option<f64>,
    /// Page height in PDF points; the canvas height when absent.
    pub page_height: Option<f64>,
    /// The output colour profile: `srgb` (default) or `cmyk`.
    pub profile: Option<String>,
    /// Background override; the canvas background when absent.
    pub background: Option<String>,
}

/// An exported PDF document and the findings recorded while writing it.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfExport {
    /// The encoded PDF bytes.
    pub pdf: Vec<u8>,
    /// Warnings for content the target could not represent; errors never reach
    /// this type, because an export that cannot proceed returns `Err` instead.
    pub diagnostics: Diagnostics,
}

/// Exports a render model as a vector PDF (C-002).
///
/// The document is returned as bytes; a request the exporter cannot satisfy is
/// a located diagnostic and no document. Content the target cannot represent is
/// omitted; use [`export_pdf_reporting`] to observe those warnings.
pub fn export_pdf(model: &RenderModel, options: &PdfOptions) -> Result<Vec<u8>, Diagnostics> {
    export_pdf_reporting(model, options).map(|export| export.pdf)
}

/// Exports a render model as a vector PDF, returning the warnings alongside the
/// bytes.
///
/// This is the same emission as [`export_pdf`]; it exists because the frozen
/// signature returns only the bytes and would otherwise drop the warnings an
/// exporter records when it omits content the target cannot represent (C-003).
pub fn export_pdf_reporting(
    model: &RenderModel,
    options: &PdfOptions,
) -> Result<PdfExport, Diagnostics> {
    let profile = resolve_profile(options.profile.as_deref())?;
    let mut diagnostics = Diagnostics::new();
    let size = resolve_page(model, options, &mut diagnostics)?;

    let background = match options.background.as_deref() {
        Some(value) => value,
        None => model.canvas.background.as_str(),
    };
    if !emit::is_transparent(background) {
        let (what, path) = if options.background.is_some() {
            ("export background", "/background")
        } else {
            ("canvas background", "/canvas/background")
        };
        validate_color(&mut diagnostics, background, what, path);
    }
    if diagnostics.has_errors() {
        return Err(diagnostics);
    }

    let pdf = emit::document(model, &size, background, profile, &mut diagnostics);
    Ok(PdfExport { pdf, diagnostics })
}

/// The output colour profile (FEAT-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Profile {
    /// Device RGB with transparency.
    Srgb,
    /// Device CMYK with transparency flattened.
    Cmyk,
}

impl Profile {
    /// Whether the profile can carry per-paint transparency.
    pub(crate) fn supports_transparency(self) -> bool {
        matches!(self, Profile::Srgb)
    }
}

/// Resolves the named profile, refusing an unknown one.
fn resolve_profile(name: Option<&str>) -> Result<Profile, Diagnostics> {
    match name.map(str::trim) {
        None => Ok(Profile::Srgb),
        Some(value) if value.eq_ignore_ascii_case("srgb") => Ok(Profile::Srgb),
        Some(value) if value.eq_ignore_ascii_case("cmyk") => Ok(Profile::Cmyk),
        Some(value) => Err(Diagnostics::from(
            Diagnostic::error(
                PROFILE,
                format!(
                    "`profile` names `{value}`, which PDF export does not support; expected `srgb` or `cmyk`"
                ),
            )
            .at_path("/profile"),
        )),
    }
}

/// The resolved page size in PDF points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Page {
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// Resolves the requested page size, applying the canvas size when none is
/// given and warning that it did.
fn resolve_page(
    model: &RenderModel,
    options: &PdfOptions,
    diagnostics: &mut Diagnostics,
) -> Result<Page, Diagnostics> {
    if let Some(width) = options.page_width {
        validate_dimension(diagnostics, width, "pageWidth");
    }
    if let Some(height) = options.page_height {
        validate_dimension(diagnostics, height, "pageHeight");
    }
    if diagnostics.has_errors() {
        return Err(std::mem::take(diagnostics));
    }

    let canvas = &model.canvas;
    let (width, height) = match (options.page_width, options.page_height) {
        (Some(width), Some(height)) => (width, height),
        (Some(width), None) => {
            let height = scale_other(width, canvas.width, canvas.height);
            (width, height)
        }
        (None, Some(height)) => {
            let width = scale_other(height, canvas.height, canvas.width);
            (width, height)
        }
        (None, None) => {
            let width = if canvas.width.is_finite() && canvas.width > 0.0 {
                canvas.width
            } else {
                1.0
            };
            let height = if canvas.height.is_finite() && canvas.height > 0.0 {
                canvas.height
            } else {
                1.0
            };
            diagnostics.push(
                Diagnostic::warning(
                    NO_PAGE_SIZE,
                    format!("no page size was requested; using the canvas size {width} x {height}"),
                )
                .at_path("/pageWidth"),
            );
            (width, height)
        }
    };

    if !width.is_finite() || width <= 0.0 || !height.is_finite() || height <= 0.0 {
        return Err(Diagnostics::from(
            Diagnostic::error(
                OPTIONS,
                "the resolved page size is not a pair of positive finite numbers",
            )
            .at_path("/pageWidth"),
        ));
    }
    Ok(Page { width, height })
}

/// Scales the other dimension to keep the canvas aspect ratio.
fn scale_other(given: f64, given_canvas: f64, other_canvas: f64) -> f64 {
    if given_canvas.is_finite() && given_canvas > 0.0 && other_canvas.is_finite() {
        given * other_canvas / given_canvas
    } else {
        given
    }
}

fn validate_dimension(diagnostics: &mut Diagnostics, value: f64, field: &str) {
    if !value.is_finite() || value <= 0.0 {
        diagnostics.push(
            Diagnostic::error(
                OPTIONS,
                format!("`{field}` must be a finite number greater than zero, got {value}"),
            )
            .at_path(format!("/{field}")),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::composition::Affine;
    use crate::primitives::{Rect, Shape};
    use crate::render::{NodePaint, Paint, RenderCanvas, RenderMeta, ResolvedNode};

    fn model() -> RenderModel {
        let shape = ResolvedNode {
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
            paint: NodePaint {
                fill: Some(Paint::Color {
                    value: "#ff0000".to_string(),
                }),
                stroke: None,
            },
            opacity: 1.0,
            visible: true,
        };
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

    #[test]
    fn a_pdf_is_emitted_with_the_canvas_as_the_default_page() {
        let export = export_pdf_reporting(&model(), &PdfOptions::default()).expect("exports a pdf");
        assert!(export.pdf.starts_with(b"%PDF-"));
        assert!(export.pdf.ends_with(b"%%EOF\n"));
        assert_eq!(
            export.diagnostics.warnings().next().map(|w| w.code.clone()),
            Some(NO_PAGE_SIZE)
        );
    }

    #[test]
    fn an_explicit_page_size_is_honoured() {
        let export = export_pdf_reporting(
            &model(),
            &PdfOptions {
                page_width: Some(612.0),
                page_height: Some(792.0),
                ..PdfOptions::default()
            },
        )
        .expect("exports");
        let text = String::from_utf8_lossy(&export.pdf).into_owned();
        assert!(text.contains("/MediaBox [0 0 612 792]"), "{text}");
        assert!(!export
            .diagnostics
            .warnings()
            .any(|w| w.code == NO_PAGE_SIZE));
    }

    #[test]
    fn a_single_page_dimension_preserves_the_canvas_aspect_ratio() {
        let export = export_pdf_reporting(
            &model(),
            &PdfOptions {
                page_width: Some(200.0),
                ..PdfOptions::default()
            },
        )
        .expect("exports");
        let text = String::from_utf8_lossy(&export.pdf).into_owned();
        assert!(text.contains("/MediaBox [0 0 200 100]"), "{text}");
    }

    #[test]
    fn the_profile_selects_the_output_colour_space() {
        let cmyk = export_pdf_reporting(
            &model(),
            &PdfOptions {
                profile: Some("cmyk".to_string()),
                ..PdfOptions::default()
            },
        )
        .expect("exports");
        let text = String::from_utf8_lossy(&cmyk.pdf).into_owned();
        assert!(text.contains(" k\n") || text.contains(" K\n"), "{text}");
        assert!(!text.contains(" rg\n"), "{text}");
    }

    #[test]
    fn an_unknown_profile_is_refused_naming_it() {
        let diagnostics = export_pdf(
            &model(),
            &PdfOptions {
                profile: Some("adobe-rgb".to_string()),
                ..PdfOptions::default()
            },
        )
        .expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, PROFILE);
        assert!(error.message.contains("adobe-rgb"), "{}", error.message);
    }

    #[test]
    fn an_invalid_page_dimension_is_refused() {
        for (width, height) in [
            (Some(0.0), None),
            (None, Some(-1.0)),
            (Some(f64::NAN), None),
        ] {
            let diagnostics = export_pdf(
                &model(),
                &PdfOptions {
                    page_width: width,
                    page_height: height,
                    ..PdfOptions::default()
                },
            )
            .expect_err("refused");
            assert_eq!(
                diagnostics.errors().next().map(|e| e.code.clone()),
                Some(OPTIONS)
            );
        }
    }

    #[test]
    fn an_invalid_background_is_refused_with_its_location() {
        let diagnostics = export_pdf(
            &model(),
            &PdfOptions {
                background: Some("not-a-colour".to_string()),
                ..PdfOptions::default()
            },
        )
        .expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, crate::scene::INVALID_COLOR);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/background")
        );
    }

    #[test]
    fn a_canvas_background_that_is_not_a_colour_is_refused_at_its_own_path() {
        let mut document = model();
        document.canvas.background = "not-a-colour".to_string();
        let diagnostics = export_pdf(&document, &PdfOptions::default()).expect_err("refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, crate::scene::INVALID_COLOR);
        assert!(
            error.message.contains("canvas background"),
            "{}",
            error.message
        );
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/canvas/background")
        );
    }

    #[test]
    fn repeated_exports_are_byte_identical() {
        let document = model();
        let options = PdfOptions::default();
        assert_eq!(
            export_pdf(&document, &options).expect("exports"),
            export_pdf(&document, &options).expect("exports")
        );
    }
}
