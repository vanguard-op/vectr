//! PDF document emission (FEAT-014).
//!
//! The writer is a pure function of the render model and the resolved page: it
//! walks the flat node list in paint order, turns each concrete shape into PDF
//! path operators, outlines each text run's glyphs, and writes solid paints as
//! device colour operators and gradients as PDF shadings. Vector content stays
//! vector, text carries no font dependency, and the same model always yields the
//! same bytes (NFR-010).
//!
//! The PDF coordinate system is y-up with its origin at the bottom-left, while
//! the render model is y-down; one base matrix flips and scales the canvas onto
//! the page, so every node's local transform composes in scene coordinates.

use std::fmt::Write as _;

use crate::composition::Affine;
use crate::fonts::FontLibrary;
use crate::primitives::{Path as PathGeometry, Segment, Shape};
use crate::render::{GradientPaint, NodeStroke, Paint, RenderModel, ResolvedNode};
use crate::scene::{Diagnostic, Diagnostics, Location};
use crate::style::{GradientType, Spread, StrokeCap, StrokeJoin};

use super::color::{self, Rgba, BLACK, WHITE};
use super::{Page, Profile, TRANSPARENCY_FLATTENED, UNSUPPORTED};

/// The two spaces of a stream, written as PDF `re`, `f`, `m`, `l`, `c`, `h`,
/// `W`, `n`, `S`, `sh` and `gs` operators.
pub(crate) fn is_transparent(background: &str) -> bool {
    background.is_empty()
        || background.eq_ignore_ascii_case("transparent")
        || background.eq_ignore_ascii_case("none")
}

/// Renders the whole PDF document.
pub(crate) fn document(
    model: &RenderModel,
    page: &Page,
    background: &str,
    profile: Profile,
    diagnostics: &mut Diagnostics,
) -> Vec<u8> {
    let fonts = FontLibrary::new(&model.fonts);
    let mut builder = Builder::default();
    let mut emitter = Emitter {
        profile,
        backdrop: backdrop(background),
        ext_states: Vec::new(),
        shadings: Vec::new(),
        flattened: 0,
    };

    let mut content = String::new();
    content.push_str("q\n");
    emitter.background(&mut content, page, background, &mut builder);

    let (sx, sy) = base_scale(model, page);
    let _ = writeln!(
        content,
        "{} 0 0 {} 0 {} cm",
        num(sx),
        num(-sy),
        num(page.height)
    );
    emitter.nodes(&mut content, model, &fonts, diagnostics, &mut builder);
    content.push_str("Q\n");

    if emitter.flattened > 0 {
        diagnostics.push(Diagnostic::warning(
            TRANSPARENCY_FLATTENED,
            format!(
                "the `{}` profile does not carry transparency; {} paint(s) were flattened against the background",
                profile_name(profile),
                emitter.flattened
            ),
        ));
    }

    let ext_states = emitter.ext_states;
    let shadings = emitter.shadings;

    let content_id = builder.stream(content.as_bytes());
    let page_id = builder.reserve();
    let pages_id = builder.reserve();
    builder.set(
        page_id,
        page_body(pages_id, content_id, page, &ext_states, &shadings),
    );
    builder.set(
        pages_id,
        format!("<< /Type /Pages /Kids [{page_id} 0 R] /Count 1 >>").into_bytes(),
    );
    let catalog_id =
        builder.alloc(format!("<< /Type /Catalog /Pages {pages_id} 0 R >>").into_bytes());
    builder.serialize(catalog_id)
}

/// The node's geometry: a concrete shape, or a text run's outlined glyphs.
enum Geometry<'a> {
    Shape(&'a Shape),
    Outline(PathGeometry),
}

/// A resolved paint colour in the output colour space.
enum Solid {
    Rgb(Rgba),
    Cmyk([f64; 4]),
}

/// Which painting operation a colour operator targets.
#[derive(Clone, Copy)]
enum Op {
    Fill,
    Stroke,
}

/// The per-document emission state: the profile, the backdrop transparency is
/// flattened against, and the resources discovered while writing.
struct Emitter {
    profile: Profile,
    backdrop: Rgba,
    /// ExtGState resources: the alpha value, its resource name and object id.
    ext_states: Vec<(f64, String, usize)>,
    /// Shading resources: the resource name and object id.
    shadings: Vec<(String, usize)>,
    /// How many paints were flattened because the profile lacks transparency.
    flattened: usize,
}

impl Emitter {
    /// Draws the page background, unless it is transparent.
    fn background(
        &mut self,
        out: &mut String,
        page: &Page,
        background: &str,
        builder: &mut Builder,
    ) {
        if is_transparent(background) {
            return;
        }
        let rgba = color::parse(background).unwrap_or(BLACK);
        let (solid, alpha) = self.resolve(rgba, 1.0, WHITE);
        if alpha <= 0.0 {
            return;
        }
        self.set_alpha(out, alpha, builder);
        write_color(out, &solid, Op::Fill);
        let _ = writeln!(out, "0 0 {} {} re f", num(page.width), num(page.height));
    }

    /// Emits every node in paint order.
    fn nodes(
        &mut self,
        out: &mut String,
        model: &RenderModel,
        fonts: &FontLibrary<'_>,
        diagnostics: &mut Diagnostics,
        builder: &mut Builder,
    ) {
        for node in &model.nodes {
            if node.kind == "raster" {
                warn(
                    diagnostics,
                    UNSUPPORTED,
                    node,
                    "is a raster layer, which PDF export omits",
                );
                continue;
            }
            if !is_finite(node) {
                warn(
                    diagnostics,
                    UNSUPPORTED,
                    node,
                    "has non-finite geometry or transform, so PDF export omits it",
                );
                continue;
            }
            if !node.visible || node.opacity <= 0.0 {
                continue;
            }

            let geometry = match &node.text {
                Some(run) => {
                    let outlined = fonts.outline(run, &node.id);
                    diagnostics.extend(outlined.diagnostics);
                    if outlined.path.is_empty() {
                        continue;
                    }
                    Geometry::Outline(outlined.path)
                }
                None => match &node.geometry {
                    Some(shape) => Geometry::Shape(shape),
                    None => continue,
                },
            };
            self.node(out, node, &geometry, diagnostics, builder);
        }
    }

    /// Emits one node: its fill, then its stroke.
    fn node(
        &mut self,
        out: &mut String,
        node: &ResolvedNode,
        geometry: &Geometry<'_>,
        diagnostics: &mut Diagnostics,
        builder: &mut Builder,
    ) {
        out.push_str("q\n");
        if node.transform != Affine::IDENTITY {
            let t = node.transform;
            let _ = writeln!(
                out,
                "{} {} {} {} {} {} cm",
                num(t.a),
                num(t.b),
                num(t.c),
                num(t.d),
                num(t.e),
                num(t.f)
            );
        }
        let opacity = node.opacity.clamp(0.0, 1.0);
        if let Some(fill) = &node.paint.fill {
            self.fill(out, node, geometry, fill, opacity, diagnostics, builder);
        }
        if let Some(stroke) = &node.paint.stroke {
            self.stroke(out, node, geometry, stroke, opacity, diagnostics, builder);
        }
        out.push_str("Q\n");
    }

    #[allow(clippy::too_many_arguments)]
    fn fill(
        &mut self,
        out: &mut String,
        node: &ResolvedNode,
        geometry: &Geometry<'_>,
        paint: &Paint,
        opacity: f64,
        diagnostics: &mut Diagnostics,
        builder: &mut Builder,
    ) {
        match paint {
            Paint::Color { value } => {
                let rgba = color::parse(value).unwrap_or(BLACK);
                let (solid, alpha) = self.resolve(rgba, opacity, self.backdrop);
                if alpha <= 0.0 {
                    return;
                }
                self.set_alpha(out, alpha, builder);
                write_color(out, &solid, Op::Fill);
                write_path(out, geometry);
                out.push_str("f\n");
            }
            Paint::Gradient(gradient) => {
                self.gradient_fill(out, node, geometry, gradient, opacity, diagnostics, builder)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn stroke(
        &mut self,
        out: &mut String,
        node: &ResolvedNode,
        geometry: &Geometry<'_>,
        stroke: &NodeStroke,
        opacity: f64,
        diagnostics: &mut Diagnostics,
        builder: &mut Builder,
    ) {
        let Paint::Color { value } = &stroke.paint else {
            warn(
                diagnostics,
                UNSUPPORTED,
                node,
                "has a gradient stroke, which PDF export cannot represent; the stroke is omitted",
            );
            return;
        };
        let rgba = color::parse(value).unwrap_or(BLACK);
        let (solid, alpha) = self.resolve(rgba, opacity, self.backdrop);
        if alpha <= 0.0 {
            return;
        }
        self.set_alpha(out, alpha, builder);
        write_color(out, &solid, Op::Stroke);
        let _ = writeln!(out, "{} w", num(stroke.width));
        let _ = writeln!(out, "{} J", cap_code(stroke.cap));
        let _ = writeln!(out, "{} j", join_code(stroke.join));
        write_path(out, geometry);
        out.push_str("S\n");
    }

    /// Emits a gradient fill as a PDF shading clipped to the shape.
    #[allow(clippy::too_many_arguments)]
    fn gradient_fill(
        &mut self,
        out: &mut String,
        node: &ResolvedNode,
        geometry: &Geometry<'_>,
        gradient: &GradientPaint,
        opacity: f64,
        diagnostics: &mut Diagnostics,
        builder: &mut Builder,
    ) {
        let Some(bbox) = bounding_box(geometry) else {
            warn(
                diagnostics,
                UNSUPPORTED,
                node,
                "has a gradient fill on geometry with no extent; the fill is omitted",
            );
            return;
        };
        if gradient.stops.len() < 2 {
            warn(
                diagnostics,
                UNSUPPORTED,
                node,
                "has a gradient fill with fewer than two stops; the fill is omitted",
            );
            return;
        }

        let stops: Vec<(f64, Vec<f64>)> = gradient
            .stops
            .iter()
            .map(|stop| {
                let rgba = color::parse(&stop.color).unwrap_or(BLACK);
                let effective = rgba.a * stop.opacity.clamp(0.0, 1.0);
                if effective < 1.0 {
                    self.flattened += 1;
                }
                (
                    stop.offset.clamp(0.0, 1.0),
                    self.components(rgba.over(self.backdrop)),
                )
            })
            .collect();

        let Some(name) = self.shading(gradient, &stops, bbox, node, diagnostics, builder) else {
            return;
        };

        let alpha = if self.profile.supports_transparency() {
            opacity
        } else {
            if opacity < 1.0 {
                self.flattened += 1;
            }
            1.0
        };
        self.set_alpha(out, alpha, builder);
        write_path(out, geometry);
        out.push_str("W n\n");
        let _ = writeln!(out, "/{name} sh");
    }

    /// Builds a shading resource and returns its name, or `None` when it cannot
    /// be represented.
    fn shading(
        &mut self,
        gradient: &GradientPaint,
        stops: &[(f64, Vec<f64>)],
        bbox: (f64, f64, f64, f64),
        node: &ResolvedNode,
        diagnostics: &mut Diagnostics,
        builder: &mut Builder,
    ) -> Option<String> {
        // One exponential function per adjacent stop pair, stitched over the
        // whole domain; the first stop anchors the low end and the last the high.
        let mut function_ids = Vec::with_capacity(stops.len() - 1);
        for pair in stops.windows(2) {
            let (_, c0) = &pair[0];
            let (_, c1) = &pair[1];
            let body = format!(
                "<< /FunctionType 2 /Domain [0 1] /C0 [{}] /C1 [{}] /N 1 >>",
                components(c0),
                components(c1)
            );
            function_ids.push(builder.alloc(body.into_bytes()));
        }
        let functions: Vec<String> = function_ids.iter().map(|id| format!("{id} 0 R")).collect();
        let bounds: Vec<String> = stops[1..stops.len() - 1]
            .iter()
            .map(|(offset, _)| num(*offset))
            .collect();
        let encode: Vec<String> = function_ids
            .iter()
            .flat_map(|_| ["0".to_string(), "1".to_string()])
            .collect();
        let stitching = format!(
            "<< /FunctionType 3 /Domain [0 1] /Functions [{}] /Bounds [{}] /Encode [{}] >>",
            functions.join(" "),
            bounds.join(" "),
            encode.join(" ")
        );
        let function_id = builder.alloc(stitching.into_bytes());

        let (min_x, min_y, max_x, max_y) = bbox;
        let width = max_x - min_x;
        let height = max_y - min_y;
        let map = |u: f64, v: f64| (min_x + u * width, min_y + v * height);

        let (shading_type, coords) = match gradient.gradient_type {
            GradientType::Linear => {
                let (x0, y0) = map(gradient.x1.unwrap_or(0.0), gradient.y1.unwrap_or(0.0));
                let (x1, y1) = map(gradient.x2.unwrap_or(1.0), gradient.y2.unwrap_or(0.0));
                (
                    2,
                    format!("{} {} {} {}", num(x0), num(y0), num(x1), num(y1)),
                )
            }
            GradientType::Radial => {
                let cx = gradient.cx.unwrap_or(0.5);
                let cy = gradient.cy.unwrap_or(0.5);
                let radius = gradient.r.unwrap_or(0.5) * (width.abs() + height.abs()) / 2.0;
                let (fx, fy) = map(gradient.fx.unwrap_or(cx), gradient.fy.unwrap_or(cy));
                let (ccx, ccy) = map(cx, cy);
                (
                    3,
                    format!(
                        "{} {} 0 {} {} {}",
                        num(fx),
                        num(fy),
                        num(ccx),
                        num(ccy),
                        num(radius)
                    ),
                )
            }
        };

        if gradient.spread != Spread::Pad {
            warn(
                diagnostics,
                UNSUPPORTED,
                node,
                "has a gradient whose spread is not `pad`, which PDF export approximates as `pad`",
            );
        }

        let colorspace = if self.profile.supports_transparency() {
            "/DeviceRGB"
        } else {
            "/DeviceCMYK"
        };
        let body = format!(
            "<< /ShadingType {shading_type} /ColorSpace {colorspace} /Coords [{coords}] /Function {function_id} 0 R /Extend [true true] >>"
        );
        let id = builder.alloc(body.into_bytes());
        let name = format!("Sh{}", self.shadings.len() + 1);
        self.shadings.push((name.clone(), id));
        Some(name)
    }

    /// Resolves a colour to the output space and its effective alpha, flattening
    /// against `flatten_backdrop` when the profile cannot carry transparency.
    fn resolve(&mut self, rgba: Rgba, opacity: f64, flatten_backdrop: Rgba) -> (Solid, f64) {
        let effective = rgba.a * opacity.clamp(0.0, 1.0);
        if self.profile.supports_transparency() {
            (Solid::Rgb(rgba), effective)
        } else {
            if effective < 1.0 {
                self.flattened += 1;
            }
            (Solid::Cmyk(cmyk(rgba.over(flatten_backdrop))), 1.0)
        }
    }

    /// The device components of a colour in the output space.
    fn components(&self, rgba: Rgba) -> Vec<f64> {
        if self.profile.supports_transparency() {
            vec![rgba.r, rgba.g, rgba.b]
        } else {
            cmyk(rgba).to_vec()
        }
    }

    /// Applies an alpha through an ExtGState, deduplicated by value.
    fn set_alpha(&mut self, out: &mut String, alpha: f64, builder: &mut Builder) {
        let alpha = alpha.clamp(0.0, 1.0);
        if alpha >= 1.0 {
            return;
        }
        if let Some((_, name, _)) = self
            .ext_states
            .iter()
            .find(|(value, _, _)| value.to_bits() == alpha.to_bits())
        {
            let _ = writeln!(out, "/{name} gs");
            return;
        }
        let name = format!("GS{}", self.ext_states.len() + 1);
        let body = format!(
            "<< /Type /ExtGState /ca {} /CA {} >>",
            num(alpha),
            num(alpha)
        );
        let id = builder.alloc(body.into_bytes());
        self.ext_states.push((alpha, name.clone(), id));
        let _ = writeln!(out, "/{name} gs");
    }
}

/// The page dictionary, carrying the resources discovered while emitting.
fn page_body(
    pages_id: usize,
    content_id: usize,
    page: &Page,
    ext_states: &[(f64, String, usize)],
    shadings: &[(String, usize)],
) -> Vec<u8> {
    let mut resources = String::from("<<");
    if !ext_states.is_empty() {
        resources.push_str(" /ExtGState <<");
        for (_, name, id) in ext_states {
            let _ = write!(resources, " /{name} {id} 0 R");
        }
        resources.push_str(" >>");
    }
    if !shadings.is_empty() {
        resources.push_str(" /Shading <<");
        for (name, id) in shadings {
            let _ = write!(resources, " /{name} {id} 0 R");
        }
        resources.push_str(" >>");
    }
    resources.push_str(" >>");
    format!(
        "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Resources {resources} /Contents {content_id} 0 R >>",
        num(page.width),
        num(page.height)
    )
    .into_bytes()
}

/// Writes one paint's colour operator.
fn write_color(out: &mut String, solid: &Solid, op: Op) {
    match (solid, op) {
        (Solid::Rgb(c), Op::Fill) => {
            let _ = writeln!(out, "{} {} {} rg", num(c.r), num(c.g), num(c.b));
        }
        (Solid::Rgb(c), Op::Stroke) => {
            let _ = writeln!(out, "{} {} {} RG", num(c.r), num(c.g), num(c.b));
        }
        (Solid::Cmyk(c), Op::Fill) => {
            let _ = writeln!(
                out,
                "{} {} {} {} k",
                num(c[0]),
                num(c[1]),
                num(c[2]),
                num(c[3])
            );
        }
        (Solid::Cmyk(c), Op::Stroke) => {
            let _ = writeln!(
                out,
                "{} {} {} {} K",
                num(c[0]),
                num(c[1]),
                num(c[2]),
                num(c[3])
            );
        }
    }
}

/// A naive device CMYK conversion, adequate for a print profile.
fn cmyk(c: Rgba) -> [f64; 4] {
    let k = 1.0 - c.r.max(c.g).max(c.b);
    if k >= 1.0 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let inverse = 1.0 - k;
    [
        (1.0 - c.r - k) / inverse,
        (1.0 - c.g - k) / inverse,
        (1.0 - c.b - k) / inverse,
        k,
    ]
}

fn components(values: &[f64]) -> String {
    values.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" ")
}

/// Writes a geometry's path operators, without a painting operator.
fn write_path(out: &mut String, geometry: &Geometry<'_>) {
    match geometry {
        Geometry::Shape(shape) => write_shape(out, shape),
        Geometry::Outline(path) => write_path_data(out, path),
    }
}

fn write_shape(out: &mut String, shape: &Shape) {
    match shape {
        Shape::Rect(rect) => {
            let (x, width) = normalize_extent(rect.x, rect.width);
            let (y, height) = normalize_extent(rect.y, rect.height);
            if rect.rx <= 0.0 && rect.ry <= 0.0 {
                let _ = writeln!(
                    out,
                    "{} {} {} {} re",
                    num(x),
                    num(y),
                    num(width),
                    num(height)
                );
            } else {
                write_rounded_rect(out, x, y, width, height, rect.rx, rect.ry);
            }
        }
        Shape::Ellipse(ellipse) => write_ellipse(
            out,
            ellipse.cx,
            ellipse.cy,
            ellipse.rx.abs(),
            ellipse.ry.abs(),
        ),
        Shape::Polygon(polygon) => write_point_list(out, &polygon.points, true),
        Shape::Line(line) => write_point_list(out, &line.points, false),
        Shape::Path(path) => write_path_data(out, path),
    }
}

fn write_rounded_rect(out: &mut String, x: f64, y: f64, w: f64, h: f64, rx: f64, ry: f64) {
    // A corner radius cannot exceed half the extent.
    let rx = rx.abs().min(w / 2.0);
    let ry = ry.abs().min(h / 2.0);
    const K: f64 = 0.552_284_749_830_793_6;
    let _ = writeln!(out, "{} {} m", num(x + rx), num(y));
    let _ = writeln!(out, "{} {} l", num(x + w - rx), num(y));
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(x + w - rx + rx * K),
        num(y),
        num(x + w),
        num(y + ry - ry * K),
        num(x + w),
        num(y + ry)
    );
    let _ = writeln!(out, "{} {} l", num(x + w), num(y + h - ry));
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(x + w),
        num(y + h - ry + ry * K),
        num(x + w - rx + rx * K),
        num(y + h),
        num(x + w - rx),
        num(y + h)
    );
    let _ = writeln!(out, "{} {} l", num(x + rx), num(y + h));
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(x + rx - rx * K),
        num(y + h),
        num(x),
        num(y + h - ry + ry * K),
        num(x),
        num(y + h - ry)
    );
    let _ = writeln!(out, "{} {} l", num(x), num(y + ry));
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(x),
        num(y + ry - ry * K),
        num(x + rx - rx * K),
        num(y),
        num(x + rx),
        num(y)
    );
    out.push_str("h\n");
}

fn write_ellipse(out: &mut String, cx: f64, cy: f64, rx: f64, ry: f64) {
    const K: f64 = 0.552_284_749_830_793_6;
    let _ = writeln!(out, "{} {} m", num(cx + rx), num(cy));
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(cx + rx),
        num(cy + ry * K),
        num(cx + rx * K),
        num(cy + ry),
        num(cx),
        num(cy + ry)
    );
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(cx - rx * K),
        num(cy + ry),
        num(cx - rx),
        num(cy + ry * K),
        num(cx - rx),
        num(cy)
    );
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(cx - rx),
        num(cy - ry * K),
        num(cx - rx * K),
        num(cy - ry),
        num(cx),
        num(cy - ry)
    );
    let _ = writeln!(
        out,
        "{} {} {} {} {} {} c",
        num(cx + rx * K),
        num(cy - ry),
        num(cx + rx),
        num(cy - ry * K),
        num(cx + rx),
        num(cy)
    );
    out.push_str("h\n");
}

fn write_point_list(out: &mut String, points: &[[f64; 2]], closed: bool) {
    let Some(first) = points.first() else {
        return;
    };
    let _ = writeln!(out, "{} {} m", num(first[0]), num(first[1]));
    for point in &points[1..] {
        let _ = writeln!(out, "{} {} l", num(point[0]), num(point[1]));
    }
    if closed {
        out.push_str("h\n");
    }
}

fn write_path_data(out: &mut String, path: &PathGeometry) {
    for subpath in &path.subpaths {
        if subpath.segments.is_empty() {
            continue;
        }
        let _ = writeln!(out, "{} {} m", num(subpath.start[0]), num(subpath.start[1]));
        let mut current = subpath.start;
        for segment in &subpath.segments {
            match segment {
                Segment::Line { to } => {
                    let _ = writeln!(out, "{} {} l", num(to[0]), num(to[1]));
                    current = *to;
                }
                Segment::Cubic { ctrl1, ctrl2, to } => {
                    let _ = writeln!(
                        out,
                        "{} {} {} {} {} {} c",
                        num(ctrl1[0]),
                        num(ctrl1[1]),
                        num(ctrl2[0]),
                        num(ctrl2[1]),
                        num(to[0]),
                        num(to[1])
                    );
                    current = *to;
                }
                Segment::Quadratic { ctrl, to } => {
                    let c1 = [
                        current[0] + 2.0 / 3.0 * (ctrl[0] - current[0]),
                        current[1] + 2.0 / 3.0 * (ctrl[1] - current[1]),
                    ];
                    let c2 = [
                        to[0] + 2.0 / 3.0 * (ctrl[0] - to[0]),
                        to[1] + 2.0 / 3.0 * (ctrl[1] - to[1]),
                    ];
                    let _ = writeln!(
                        out,
                        "{} {} {} {} {} {} c",
                        num(c1[0]),
                        num(c1[1]),
                        num(c2[0]),
                        num(c2[1]),
                        num(to[0]),
                        num(to[1])
                    );
                    current = *to;
                }
                Segment::Arc {
                    rx,
                    ry,
                    x_rotation,
                    large_arc,
                    sweep,
                    to,
                } => {
                    for (c1, c2, end) in
                        arc_cubics(current, *rx, *ry, *x_rotation, *large_arc, *sweep, *to)
                    {
                        let _ = writeln!(
                            out,
                            "{} {} {} {} {} {} c",
                            num(c1[0]),
                            num(c1[1]),
                            num(c2[0]),
                            num(c2[1]),
                            num(end[0]),
                            num(end[1])
                        );
                    }
                    current = *to;
                }
            }
        }
        if subpath.closed {
            out.push_str("h\n");
        }
    }
}

/// Approximates an SVG elliptical arc with cubic Bézier segments (the endpoint
/// to centre parameterization).
#[allow(clippy::too_many_arguments)]
fn arc_cubics(
    start: [f64; 2],
    rx: f64,
    ry: f64,
    x_rotation: f64,
    large_arc: bool,
    sweep: bool,
    end: [f64; 2],
) -> Vec<([f64; 2], [f64; 2], [f64; 2])> {
    let mut rx = rx.abs();
    let mut ry = ry.abs();
    if rx == 0.0 || ry == 0.0 || start == end {
        return vec![(start, end, end)];
    }
    let phi = x_rotation.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let dx = (start[0] - end[0]) / 2.0;
    let dy = (start[1] - end[1]) / 2.0;
    let x1p = cos_phi * dx + sin_phi * dy;
    let y1p = -sin_phi * dx + cos_phi * dy;

    let lambda = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }

    let numerator = (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p).max(0.0);
    let denominator = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let mut coefficient = if denominator == 0.0 {
        0.0
    } else {
        (numerator / denominator).sqrt()
    };
    if large_arc == sweep {
        coefficient = -coefficient;
    }
    let cxp = coefficient * rx * y1p / ry;
    let cyp = -coefficient * ry * x1p / rx;
    let cx = cos_phi * cxp - sin_phi * cyp + (start[0] + end[0]) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (start[1] + end[1]) / 2.0;

    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = uy.atan2(ux);
    let mut delta = (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    if !sweep && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f64::consts::TAU;
    }

    let count = ((delta.abs() / (std::f64::consts::PI / 2.0)).ceil() as usize).max(1);
    let step = delta / count as f64;
    let map = |u: f64, v: f64| {
        let ex = rx * u;
        let ey = ry * v;
        [
            cos_phi * ex - sin_phi * ey + cx,
            sin_phi * ex + cos_phi * ey + cy,
        ]
    };

    let mut cubics = Vec::with_capacity(count);
    let mut theta = theta1;
    for _ in 0..count {
        let next = theta + step;
        let alpha = 4.0 / 3.0 * (step / 4.0).tan();
        let (sin_a, cos_a) = theta.sin_cos();
        let (sin_b, cos_b) = next.sin_cos();
        let c1 = map(cos_a + alpha * -sin_a, sin_a + alpha * cos_a);
        let c2 = map(cos_b - alpha * -sin_b, sin_b - alpha * cos_b);
        let p1 = map(cos_b, sin_b);
        cubics.push((c1, c2, p1));
        theta = next;
    }
    cubics
}

/// The bounding box of a geometry, or `None` when it has no points.
fn bounding_box(geometry: &Geometry<'_>) -> Option<(f64, f64, f64, f64)> {
    let mut points: Vec<[f64; 2]> = Vec::new();
    match geometry {
        Geometry::Shape(shape) => collect_shape_points(shape, &mut points),
        Geometry::Outline(path) => collect_path_points(path, &mut points),
    }
    let first = points.first()?;
    let mut box_ = (first[0], first[1], first[0], first[1]);
    for point in &points[1..] {
        box_.0 = box_.0.min(point[0]);
        box_.1 = box_.1.min(point[1]);
        box_.2 = box_.2.max(point[0]);
        box_.3 = box_.3.max(point[1]);
    }
    Some(box_)
}

fn collect_shape_points(shape: &Shape, points: &mut Vec<[f64; 2]>) {
    match shape {
        Shape::Rect(rect) => {
            let (x, w) = normalize_extent(rect.x, rect.width);
            let (y, h) = normalize_extent(rect.y, rect.height);
            points.extend([[x, y], [x + w, y], [x, y + h], [x + w, y + h]]);
        }
        Shape::Ellipse(ellipse) => {
            let rx = ellipse.rx.abs();
            let ry = ellipse.ry.abs();
            points.extend([
                [ellipse.cx - rx, ellipse.cy - ry],
                [ellipse.cx + rx, ellipse.cy + ry],
            ]);
        }
        Shape::Polygon(polygon) => points.extend_from_slice(&polygon.points),
        Shape::Line(line) => points.extend_from_slice(&line.points),
        Shape::Path(path) => collect_path_points(path, points),
    }
}

fn collect_path_points(path: &PathGeometry, points: &mut Vec<[f64; 2]>) {
    for subpath in &path.subpaths {
        points.push(subpath.start);
        for segment in &subpath.segments {
            match segment {
                Segment::Line { to } => points.push(*to),
                Segment::Cubic { ctrl1, ctrl2, to } => {
                    points.push(*ctrl1);
                    points.push(*ctrl2);
                    points.push(*to);
                }
                Segment::Quadratic { ctrl, to } => {
                    points.push(*ctrl);
                    points.push(*to);
                }
                Segment::Arc { to, .. } => points.push(*to),
            }
        }
    }
}

fn normalize_extent(origin: f64, extent: f64) -> (f64, f64) {
    if extent < 0.0 {
        (origin + extent, -extent)
    } else {
        (origin, extent)
    }
}

fn cap_code(cap: StrokeCap) -> u8 {
    match cap {
        StrokeCap::Butt => 0,
        StrokeCap::Round => 1,
        StrokeCap::Square => 2,
    }
}

fn join_code(join: StrokeJoin) -> u8 {
    match join {
        StrokeJoin::Miter => 0,
        StrokeJoin::Round => 1,
        StrokeJoin::Bevel => 2,
    }
}

fn warn(
    diagnostics: &mut Diagnostics,
    code: crate::scene::DiagnosticCode,
    node: &ResolvedNode,
    reason: &str,
) {
    diagnostics.push(
        Diagnostic::warning(code, format!("element `{}` {reason}", node.id))
            .with_location(Location::element(node.id.clone())),
    );
}

fn base_scale(model: &RenderModel, page: &Page) -> (f64, f64) {
    let sx = if model.canvas.width.is_finite() && model.canvas.width > 0.0 {
        page.width / model.canvas.width
    } else {
        1.0
    };
    let sy = if model.canvas.height.is_finite() && model.canvas.height > 0.0 {
        page.height / model.canvas.height
    } else {
        1.0
    };
    (sx, sy)
}

/// The opaque colour transparency is flattened against: the page background when
/// it is opaque, otherwise white paper.
fn backdrop(background: &str) -> Rgba {
    if is_transparent(background) {
        return WHITE;
    }
    match color::parse(background) {
        Some(value) => value.over(WHITE),
        None => WHITE,
    }
}

fn profile_name(profile: Profile) -> &'static str {
    match profile {
        Profile::Srgb => "srgb",
        Profile::Cmyk => "cmyk",
    }
}

/// Whether every value the writer will format is finite.
fn is_finite(node: &ResolvedNode) -> bool {
    node.transform.is_finite()
        && node.opacity.is_finite()
        && node
            .paint
            .stroke
            .as_ref()
            .is_none_or(|stroke| stroke.width.is_finite())
        && node.geometry.as_ref().is_none_or(geometry_is_finite)
}

fn geometry_is_finite(shape: &Shape) -> bool {
    match shape {
        Shape::Rect(rect) => finite(&[rect.x, rect.y, rect.width, rect.height, rect.rx, rect.ry]),
        Shape::Ellipse(ellipse) => finite(&[ellipse.cx, ellipse.cy, ellipse.rx, ellipse.ry]),
        Shape::Polygon(polygon) => polygon.points.iter().all(|point| finite(point)),
        Shape::Line(line) => line.points.iter().all(|point| finite(point)),
        Shape::Path(path) => path.subpaths.iter().all(|subpath| {
            finite(&subpath.start) && subpath.segments.iter().all(segment_is_finite)
        }),
    }
}

fn segment_is_finite(segment: &Segment) -> bool {
    match segment {
        Segment::Line { to } => finite(to),
        Segment::Cubic { ctrl1, ctrl2, to } => finite(ctrl1) && finite(ctrl2) && finite(to),
        Segment::Quadratic { ctrl, to } => finite(ctrl) && finite(to),
        Segment::Arc {
            rx,
            ry,
            x_rotation,
            to,
            ..
        } => finite(&[*rx, *ry, *x_rotation]) && finite(to),
    }
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|value| value.is_finite())
}

/// Formats a coordinate deterministically, without exponent notation (PDF
/// content streams do not accept it) and with trailing zeros trimmed.
fn num(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let text = format!("{value:.6}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Accumulates indirect objects and serializes them with a cross-reference
/// table.
#[derive(Default)]
struct Builder {
    objects: Vec<Vec<u8>>,
}

impl Builder {
    fn alloc(&mut self, body: Vec<u8>) -> usize {
        self.objects.push(body);
        self.objects.len()
    }

    fn reserve(&mut self) -> usize {
        self.objects.push(Vec::new());
        self.objects.len()
    }

    fn set(&mut self, id: usize, body: Vec<u8>) {
        self.objects[id - 1] = body;
    }

    fn stream(&mut self, data: &[u8]) -> usize {
        let mut body = format!("<< /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.alloc(body)
    }

    fn serialize(&self, root: usize) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
        let count = self.objects.len();
        let mut offsets = vec![0usize; count + 1];
        for (index, body) in self.objects.iter().enumerate() {
            let id = index + 1;
            offsets[id] = out.len();
            out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", count + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {root} 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                count + 1
            )
            .as_bytes(),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{NodePaint, RenderCanvas, RenderMeta};
    use crate::style::StrokeCap;
    use crate::style::StrokeJoin;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Shape {
        Shape::Rect(crate::primitives::Rect {
            x,
            y,
            width: w,
            height: h,
            rx: 0.0,
            ry: 0.0,
        })
    }

    fn node(id: &str, geometry: Shape) -> ResolvedNode {
        ResolvedNode {
            id: id.to_string(),
            name: None,
            accessible_name: None,
            order: 0,
            kind: geometry.kind().to_string(),
            groups: Vec::new(),
            geometry: Some(geometry),
            text: None,
            transform: Affine::IDENTITY,
            paint: NodePaint::default(),
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
            fonts: Vec::new(),
        }
    }

    fn content(model: &RenderModel, profile: Profile) -> (String, Diagnostics) {
        let page = Page {
            width: model.canvas.width,
            height: model.canvas.height,
        };
        let mut diagnostics = Diagnostics::new();
        let bytes = document(
            model,
            &page,
            &model.canvas.background,
            profile,
            &mut diagnostics,
        );
        let text = String::from_utf8_lossy(&bytes).into_owned();
        (text, diagnostics)
    }

    #[test]
    fn a_rect_becomes_vector_path_operators() {
        let mut shape = node("e1", rect(1.0, 2.0, 30.0, 40.0));
        shape.paint.fill = Some(Paint::Color {
            value: "#ff0000".to_string(),
        });
        let (text, _) = content(&model(vec![shape]), Profile::Srgb);
        assert!(text.contains("1 2 30 40 re"), "{text}");
        assert!(text.contains("1 0 0 rg"), "{text}");
        assert!(!text.contains("/Subtype /Image"), "{text}");
    }

    #[test]
    fn the_page_is_sized_by_the_options_and_the_canvas_is_flipped_onto_it() {
        let page = Page {
            width: 200.0,
            height: 100.0,
        };
        let mut diagnostics = Diagnostics::new();
        let bytes = document(
            &model(Vec::new()),
            &page,
            "#ffffff",
            Profile::Srgb,
            &mut diagnostics,
        );
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("/MediaBox [0 0 200 100]"), "{text}");
        // 2x scale and a y flip about the page height.
        assert!(text.contains("2 0 0 -2 0 100 cm"), "{text}");
    }

    #[test]
    fn a_text_node_is_outlined_with_no_font_resource() {
        let path = format!(
            "{}/../../assets/fonts/Inter.ttf",
            env!("CARGO_MANIFEST_DIR")
        );
        let data = std::fs::read(&path).expect("the bundled font");
        let mut text_node = node("t1", rect(0.0, 0.0, 1.0, 1.0));
        text_node.kind = "text".to_string();
        text_node.geometry = None;
        text_node.text = Some(crate::render::TextRun {
            value: "O".to_string(),
            font_id: "body".to_string(),
            font_size: 40.0,
            align: crate::scene::TextAlign::Start,
            line_height: 40.0,
            letter_spacing: 0.0,
            width: None,
        });
        text_node.transform = Affine::translate(5.0, 45.0);
        text_node.paint.fill = Some(Paint::Color {
            value: "#000000".to_string(),
        });
        let mut document = model(vec![text_node]);
        document.canvas.width = 50.0;
        document.canvas.height = 50.0;
        document.fonts = vec![crate::render::ResolvedFont {
            id: "body".to_string(),
            name: "Inter".to_string(),
            data,
        }];

        let (text, diagnostics) = content(&document, Profile::Srgb);
        assert!(text.contains(" c\n"), "glyphs are curves: {text}");
        assert!(!text.contains("/Font"), "no font resource: {text}");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn a_missing_font_omits_the_text_with_a_warning() {
        let mut text_node = node("t1", rect(0.0, 0.0, 1.0, 1.0));
        text_node.kind = "text".to_string();
        text_node.geometry = None;
        text_node.text = Some(crate::render::TextRun {
            value: "Hi".to_string(),
            font_id: "ghost".to_string(),
            font_size: 12.0,
            align: crate::scene::TextAlign::Start,
            line_height: 12.0,
            letter_spacing: 0.0,
            width: None,
        });
        let (_, diagnostics) = content(&model(vec![text_node]), Profile::Srgb);
        assert_eq!(
            diagnostics.warnings().next().map(|w| w.code.clone()),
            Some(crate::fonts::FONT_MISSING)
        );
    }

    #[test]
    fn a_raster_node_is_omitted_with_a_warning() {
        let mut shape = node("r1", rect(0.0, 0.0, 1.0, 1.0));
        shape.kind = "raster".to_string();
        let (text, diagnostics) = content(&model(vec![shape]), Profile::Srgb);
        assert!(!text.contains("r1"), "{text}");
        assert_eq!(
            diagnostics.warnings().next().map(|w| w.code.clone()),
            Some(UNSUPPORTED)
        );
    }

    #[test]
    fn node_opacity_becomes_an_ext_gstate() {
        let mut shape = node("e1", rect(0.0, 0.0, 10.0, 10.0));
        shape.paint.fill = Some(Paint::Color {
            value: "#ff0000".to_string(),
        });
        shape.opacity = 0.5;
        let (text, _) = content(&model(vec![shape]), Profile::Srgb);
        assert!(text.contains("/ca 0.5 /CA 0.5"), "{text}");
        assert!(text.contains("/GS1 gs"), "{text}");
    }

    #[test]
    fn a_colour_alpha_becomes_an_ext_gstate() {
        let mut shape = node("e1", rect(0.0, 0.0, 10.0, 10.0));
        shape.paint.fill = Some(Paint::Color {
            value: "#ff000080".to_string(),
        });
        let (text, _) = content(&model(vec![shape]), Profile::Srgb);
        assert!(text.contains("/ca "), "{text}");
    }

    #[test]
    fn the_cmyk_profile_writes_cmyk_and_flattens_transparency() {
        let mut shape = node("e1", rect(0.0, 0.0, 10.0, 10.0));
        shape.paint.fill = Some(Paint::Color {
            value: "#ff0000".to_string(),
        });
        shape.opacity = 0.5;
        let (text, diagnostics) = content(&model(vec![shape]), Profile::Cmyk);
        assert!(text.contains(" k\n"), "{text}");
        assert!(!text.contains(" rg\n"), "{text}");
        assert!(text.contains("0 1 1 0 k"), "red is CMYK 0/1/1/0: {text}");
        assert!(
            diagnostics
                .warnings()
                .any(|w| w.code == TRANSPARENCY_FLATTENED),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn a_gradient_becomes_a_shading() {
        let mut shape = node("e1", rect(0.0, 0.0, 10.0, 10.0));
        shape.paint.fill = Some(Paint::Gradient(GradientPaint {
            gradient_type: GradientType::Linear,
            stops: vec![
                crate::render::ResolvedStop {
                    offset: 0.0,
                    color: "#ff0000".to_string(),
                    opacity: 1.0,
                },
                crate::render::ResolvedStop {
                    offset: 1.0,
                    color: "#0000ff".to_string(),
                    opacity: 1.0,
                },
            ],
            spread: Spread::Pad,
            x1: Some(0.0),
            y1: Some(0.0),
            x2: Some(1.0),
            y2: Some(0.0),
            cx: None,
            cy: None,
            r: None,
            fx: None,
            fy: None,
        }));
        let (text, _) = content(&model(vec![shape]), Profile::Srgb);
        assert!(text.contains("/ShadingType 2"), "{text}");
        assert!(text.contains("/Sh1 sh"), "{text}");
        assert!(text.contains("/Shading << /Sh1"), "{text}");
    }

    #[test]
    fn a_gradient_stroke_is_omitted_with_a_warning() {
        let mut shape = node("e1", rect(0.0, 0.0, 10.0, 10.0));
        shape.paint.stroke = Some(NodeStroke {
            paint: Paint::Gradient(GradientPaint {
                gradient_type: GradientType::Linear,
                stops: vec![
                    crate::render::ResolvedStop {
                        offset: 0.0,
                        color: "#ff0000".to_string(),
                        opacity: 1.0,
                    },
                    crate::render::ResolvedStop {
                        offset: 1.0,
                        color: "#0000ff".to_string(),
                        opacity: 1.0,
                    },
                ],
                spread: Spread::Pad,
                x1: None,
                y1: None,
                x2: None,
                y2: None,
                cx: None,
                cy: None,
                r: None,
                fx: None,
                fy: None,
            }),
            width: 1.0,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
        });
        let (_, diagnostics) = content(&model(vec![shape]), Profile::Srgb);
        assert!(diagnostics.warnings().any(|w| w.code == UNSUPPORTED));
    }

    #[test]
    fn a_stroke_carries_its_width_cap_and_join() {
        let mut shape = node("e1", rect(0.0, 0.0, 10.0, 10.0));
        shape.paint.stroke = Some(NodeStroke {
            paint: Paint::Color {
                value: "#0000ff".to_string(),
            },
            width: 2.5,
            cap: StrokeCap::Round,
            join: StrokeJoin::Bevel,
        });
        let (text, _) = content(&model(vec![shape]), Profile::Srgb);
        assert!(text.contains("2.5 w"), "{text}");
        assert!(text.contains("1 J"), "{text}");
        assert!(text.contains("2 j"), "{text}");
        assert!(text.contains("0 0 1 RG"), "{text}");
    }

    #[test]
    fn a_non_finite_node_is_omitted_with_a_warning() {
        let shape = node("e1", rect(f64::NAN, 0.0, 1.0, 1.0));
        let (_, diagnostics) = content(&model(vec![shape]), Profile::Srgb);
        assert!(diagnostics.warnings().any(|w| w.code == UNSUPPORTED));
    }

    #[test]
    fn an_invisible_or_fully_transparent_node_draws_nothing() {
        let mut invisible = node("e1", rect(0.0, 0.0, 10.0, 10.0));
        invisible.visible = false;
        let mut clear = node("e2", rect(0.0, 0.0, 10.0, 10.0));
        clear.opacity = 0.0;
        let (text, _) = content(&model(vec![invisible, clear]), Profile::Srgb);
        assert!(!text.contains("e1"), "{text}");
        assert!(!text.contains("e2"), "{text}");
    }

    #[test]
    fn an_arc_is_approximated_by_cubic_curves() {
        let path = PathGeometry {
            subpaths: vec![crate::primitives::SubPath {
                start: [0.0, 0.0],
                segments: vec![Segment::Arc {
                    rx: 5.0,
                    ry: 5.0,
                    x_rotation: 0.0,
                    large_arc: false,
                    sweep: true,
                    to: [10.0, 0.0],
                }],
                closed: false,
            }],
        };
        let mut shape = node("e1", Shape::Path(path));
        shape.paint.fill = None;
        shape.paint.stroke = Some(NodeStroke {
            paint: Paint::Color {
                value: "#000000".to_string(),
            },
            width: 1.0,
            cap: StrokeCap::Butt,
            join: StrokeJoin::Miter,
        });
        let (text, _) = content(&model(vec![shape]), Profile::Srgb);
        assert!(text.contains(" c\n"), "the arc becomes cubics: {text}");
    }

    #[test]
    fn the_pdf_is_well_formed_with_a_cross_reference_table() {
        let mut shape = node("e1", rect(0.0, 0.0, 10.0, 10.0));
        shape.paint.fill = Some(Paint::Color {
            value: "#000000".to_string(),
        });
        let (text, _) = content(&model(vec![shape]), Profile::Srgb);
        assert!(text.starts_with("%PDF-1.7\n"), "{text}");
        assert!(text.contains("\nxref\n"), "{text}");
        assert!(text.contains("/Root "), "{text}");
        assert!(text.ends_with("%%EOF\n"), "{text}");
    }

    #[test]
    fn numbers_avoid_exponent_notation() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(1e-9), "0");
        assert_eq!(num(f64::INFINITY), "0");
        assert!(!num(1e20).contains('e'));
    }
}
