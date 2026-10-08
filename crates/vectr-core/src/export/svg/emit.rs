//! SVG document emission (FEAT-012).
//!
//! The writer is a pure function of the render model and the resolved output
//! size: it walks the flat node list in paint order, rebuilds the named-group
//! nesting each node's ancestor chain declares, wraps each node in a group
//! carrying its stable identifier and name, and turns its concrete geometry and
//! paint into native SVG. Every value is formatted by one deterministic number
//! routine and every text or attribute is XML-escaped, so the same model always
//! yields the same bytes and a name can never introduce markup (NFR-010,
//! NFR-023).

use std::collections::HashSet;
use std::fmt::Write as _;

use crate::composition::Affine;
use crate::fonts::FontLibrary;
use crate::primitives::{Path as PathGeometry, Rect, Segment, Shape};
use crate::render::{NodeGroup, Paint, RenderModel, ResolvedNode};
use crate::scene::{Diagnostic, Diagnostics, Location};
use crate::style::{GradientType, Spread, StrokeCap, StrokeJoin};

use super::UNSUPPORTED;

/// The rendered width and height of the output document.
pub(crate) struct Size {
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// Renders the whole SVG document.
///
/// Nodes the target cannot represent are omitted and reported on `diagnostics`;
/// the caller decides whether the collection is fatal (it never is for warnings
/// alone).
pub(crate) fn document(
    model: &RenderModel,
    size: &Size,
    background: &str,
    diagnostics: &mut Diagnostics,
) -> String {
    let canvas = &model.canvas;
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        number(size.width),
        number(size.height),
        number(canvas.width),
        number(canvas.height),
    );

    if let Some(title) = &model.meta.title {
        let _ = writeln!(out, "  <title>{}</title>", escape_text(title));
    }
    if let Some(description) = &model.meta.description {
        let _ = writeln!(out, "  <desc>{}</desc>", escape_text(description));
    }

    if !is_transparent(background) {
        let _ = writeln!(
            out,
            "  <rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
            number(canvas.width),
            number(canvas.height),
            escape_attr(background),
        );
    }

    let mut ids = IdAllocator::default();
    let fonts = FontLibrary::new(&model.fonts);
    let defs = GradientDefs::build(&model.nodes, &mut ids);
    if !defs.is_empty() {
        let _ = writeln!(out, "  <defs>");
        defs.write(&mut out);
        let _ = writeln!(out, "  </defs>");
    }
    emit_nodes(&mut out, &model.nodes, &fonts, &defs, &mut ids, diagnostics);

    out.push_str("</svg>\n");
    out
}

/// The gradient definitions a document needs, each with a stable identifier
/// shared by every node that uses it (FEAT-027).
///
/// The model's gradients are concrete by the time they reach here, so a
/// definition is written once per distinct paint and referenced by
/// `url(#id)`; identical gradients share an identifier, keeping the output
/// deterministic and compact.
#[derive(Default)]
struct GradientDefs {
    entries: Vec<(Paint, String)>,
}

impl GradientDefs {
    /// Registers every gradient paint in the model, in node order.
    fn build(nodes: &[ResolvedNode], ids: &mut IdAllocator) -> Self {
        let mut defs = GradientDefs::default();
        for node in nodes {
            if let Some(fill) = &node.paint.fill {
                defs.register(fill, ids);
            }
            if let Some(stroke) = &node.paint.stroke {
                defs.register(&stroke.paint, ids);
            }
        }
        defs
    }

    fn register(&mut self, paint: &Paint, ids: &mut IdAllocator) {
        if !matches!(paint, Paint::Gradient(_)) {
            return;
        }
        if self.entries.iter().any(|(existing, _)| existing == paint) {
            return;
        }
        let id = ids.allocate(&format!("gradient-{}", self.entries.len() + 1));
        self.entries.push((paint.clone(), id));
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The identifier for a gradient paint, or `None` for a colour.
    fn id(&self, paint: &Paint) -> Option<&str> {
        if !matches!(paint, Paint::Gradient(_)) {
            return None;
        }
        self.entries
            .iter()
            .find(|(existing, _)| existing == paint)
            .map(|(_, id)| id.as_str())
    }

    /// Writes each gradient definition, indented inside `<defs>`.
    fn write(&self, out: &mut String) {
        for (paint, id) in &self.entries {
            let Paint::Gradient(gradient) = paint else {
                continue;
            };
            let spread = spread_name(gradient.spread);
            match gradient.gradient_type {
                GradientType::Linear => {
                    let _ = writeln!(
                        out,
                        "    <linearGradient id=\"{id}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" spreadMethod=\"{spread}\">",
                        number(gradient.x1.unwrap_or(0.0)),
                        number(gradient.y1.unwrap_or(0.0)),
                        number(gradient.x2.unwrap_or(1.0)),
                        number(gradient.y2.unwrap_or(0.0)),
                    );
                }
                GradientType::Radial => {
                    let _ = writeln!(
                        out,
                        "    <radialGradient id=\"{id}\" cx=\"{}\" cy=\"{}\" r=\"{}\" fx=\"{}\" fy=\"{}\" spreadMethod=\"{spread}\">",
                        number(gradient.cx.unwrap_or(0.5)),
                        number(gradient.cy.unwrap_or(0.5)),
                        number(gradient.r.unwrap_or(0.5)),
                        number(gradient.fx.unwrap_or(0.5)),
                        number(gradient.fy.unwrap_or(0.5)),
                    );
                }
            }
            for stop in &gradient.stops {
                let _ = write!(
                    out,
                    "      <stop offset=\"{}\" stop-color=\"{}\"",
                    number(stop.offset),
                    escape_attr(&stop.color),
                );
                if stop.opacity < 1.0 {
                    let _ = write!(out, " stop-opacity=\"{}\"", number(stop.opacity));
                }
                out.push_str("/>\n");
            }
            let close = match gradient.gradient_type {
                GradientType::Linear => "linearGradient",
                GradientType::Radial => "radialGradient",
            };
            let _ = writeln!(out, "    </{close}>");
        }
    }
}

fn spread_name(spread: Spread) -> &'static str {
    match spread {
        Spread::Pad => "pad",
        Spread::Reflect => "reflect",
        Spread::Repeat => "repeat",
    }
}

/// Emits every node in paint order, opening and closing the named-group
/// elements their ancestor chains imply (FEAT-012).
///
/// The render model is flat, but each node carries its ancestor group chain
/// outermost first. Walking the list in paint order and keeping the chain that
/// is currently open reconstructs the nesting: a shared prefix stays open, a
/// shorter chain closes the groups it no longer needs, and a longer one opens
/// the groups it adds. Because the compiler emits a group's subtree as one
/// contiguous run, a group is opened once and spans all of its drawing
/// descendants. An unnamed group carries no name to preserve and no visual
/// state of its own — its transform and opacity are already resolved into its
/// nodes — so it is collapsed, as FEAT-012 permits; a named group nested inside
/// one still lands at the correct level.
///
/// A text node carries its string and resolved layout, not geometry (C-003), so
/// its glyphs are outlined here against the model's fonts and emitted as a
/// path; a run that draws nothing, or whose font is missing, opens no group and
/// is reported on `diagnostics` (FEAT-024).
fn emit_nodes(
    out: &mut String,
    nodes: &[ResolvedNode],
    fonts: &FontLibrary<'_>,
    defs: &GradientDefs,
    ids: &mut IdAllocator,
    diagnostics: &mut Diagnostics,
) {
    let mut open: Vec<String> = Vec::new();
    for node in nodes {
        if let Some(reason) = unsupported(node) {
            omit(diagnostics, node, reason);
            continue;
        }
        let Some(shape) = shape_for(node, fonts, defs, diagnostics) else {
            continue;
        };

        let chain: Vec<&NodeGroup> = node
            .groups
            .iter()
            .filter(|group| group.name.is_some())
            .collect();
        let common = open
            .iter()
            .zip(&chain)
            .take_while(|(id, group)| id.as_str() == group.id.as_str())
            .count();
        while open.len() > common {
            open.pop();
            let _ = writeln!(out, "{}</g>", indent(open.len() + 1));
        }
        for group in &chain[common..] {
            let id = ids.allocate(&group.id);
            let name = group.name.as_deref().unwrap_or_default();
            let _ = writeln!(
                out,
                "{}<g id=\"{}\" data-name=\"{}\">",
                indent(open.len() + 1),
                id,
                escape_attr(name),
            );
            open.push(group.id.clone());
        }

        emit_node(out, node, &shape, &indent(open.len() + 1), ids);
    }
    while !open.is_empty() {
        open.pop();
        let _ = writeln!(out, "{}</g>", indent(open.len() + 1));
    }
}

/// The reason a node cannot be represented, or `None` when it can.
fn unsupported(node: &ResolvedNode) -> Option<&'static str> {
    if node.kind == "raster" {
        Some("is a raster layer, which SVG export omits")
    } else if !is_finite(node) {
        Some("has non-finite geometry or transform, so SVG export omits it")
    } else {
        None
    }
}

/// The SVG shape for a node: a text node's outlined glyphs, or a shape's native
/// element. `None` when the node draws nothing, in which case its warnings have
/// already been recorded.
fn shape_for(
    node: &ResolvedNode,
    fonts: &FontLibrary<'_>,
    defs: &GradientDefs,
    diagnostics: &mut Diagnostics,
) -> Option<String> {
    match &node.text {
        Some(run) => {
            let outlined = fonts.outline(run, &node.id);
            diagnostics.extend(outlined.diagnostics);
            if outlined.path.is_empty() {
                None
            } else {
                Some(text_element(node, &outlined.path, defs))
            }
        }
        None => shape_element(node, defs),
    }
}

/// Emits one node as a group carrying its identifier and name.
fn emit_node(
    out: &mut String,
    node: &ResolvedNode,
    shape: &str,
    indent: &str,
    ids: &mut IdAllocator,
) {
    let id = ids.allocate(&node.id);
    let mut group = format!("<g id=\"{id}\"");
    if let Some(name) = &node.name {
        let _ = write!(group, " data-name=\"{}\"", escape_attr(name));
    }
    let opacity = node.opacity.clamp(0.0, 1.0);
    if opacity < 1.0 {
        let _ = write!(group, " opacity=\"{}\"", number(opacity));
    }
    if !node.visible {
        group.push_str(" display=\"none\"");
    }
    group.push('>');

    let _ = writeln!(out, "{indent}{group}");
    if let Some(name) = &node.name {
        let _ = writeln!(out, "{indent}  <title>{}</title>", escape_text(name));
    }
    // A text node's string is otherwise lost once its glyphs become outlines, so
    // carry it as the element's accessible description (FEAT-012).
    if let Some(text) = &node.text {
        let _ = writeln!(out, "{indent}  <desc>{}</desc>", escape_text(&text.value));
    }
    let _ = writeln!(out, "{indent}  {shape}");
    let _ = writeln!(out, "{indent}</g>");
}

/// The two-space indentation of a nesting level (level 1 is the document root's
/// children).
fn indent(level: usize) -> String {
    "  ".repeat(level)
}

fn omit(diagnostics: &mut Diagnostics, node: &ResolvedNode, reason: &str) {
    diagnostics.push(
        Diagnostic::warning(UNSUPPORTED, format!("element `{}` {reason}", node.id))
            .with_location(Location::element(node.id.clone())),
    );
}

/// The SVG shape element for a node, or `None` when it draws nothing.
fn shape_element(node: &ResolvedNode, defs: &GradientDefs) -> Option<String> {
    let shape = node.geometry.as_ref()?;
    let mut attrs = paint_attributes(node, defs);
    if node.transform != Affine::IDENTITY {
        let transform = node.transform;
        let _ = write!(
            attrs,
            " transform=\"matrix({} {} {} {} {} {})\"",
            number(transform.a),
            number(transform.b),
            number(transform.c),
            number(transform.d),
            number(transform.e),
            number(transform.f),
        );
    }

    let element = match shape {
        Shape::Rect(rect) => rect_element(rect, &attrs),
        Shape::Ellipse(ellipse) => format!(
            "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"{attrs}/>",
            number(ellipse.cx),
            number(ellipse.cy),
            number(ellipse.rx.abs()),
            number(ellipse.ry.abs()),
        ),
        Shape::Polygon(polygon) => {
            format!("<polygon points=\"{}\"{attrs}/>", points(&polygon.points))
        }
        Shape::Line(line) => {
            format!("<polyline points=\"{}\"{attrs}/>", points(&line.points))
        }
        Shape::Path(path) => {
            let data = path_data(path);
            if data.is_empty() {
                return None;
            }
            format!("<path d=\"{data}\"{attrs}/>")
        }
    };
    Some(element)
}

/// The SVG path element for an outlined text node (FEAT-024).
///
/// The glyph contours are concrete geometry by the time they reach here, so a
/// text node emits exactly like any other path: its paint, its resolved
/// transform, and one `d` attribute carrying every contour.
fn text_element(node: &ResolvedNode, path: &PathGeometry, defs: &GradientDefs) -> String {
    let data = path_data(path);
    let mut attrs = paint_attributes(node, defs);
    if node.transform != Affine::IDENTITY {
        let transform = node.transform;
        let _ = write!(
            attrs,
            " transform=\"matrix({} {} {} {} {} {})\"",
            number(transform.a),
            number(transform.b),
            number(transform.c),
            number(transform.d),
            number(transform.e),
            number(transform.f),
        );
    }
    format!("<path d=\"{data}\"{attrs}/>")
}

/// A rectangle, with a negative extent normalised to a positive one so the
/// element stays valid (an out-of-range coordinate is handled, not emitted
/// broken).
fn rect_element(rect: &Rect, attrs: &str) -> String {
    let (x, width) = normalize_extent(rect.x, rect.width);
    let (y, height) = normalize_extent(rect.y, rect.height);
    let mut element = format!(
        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
        number(x),
        number(y),
        number(width),
        number(height),
    );
    if rect.rx != 0.0 || rect.ry != 0.0 {
        let _ = write!(
            element,
            " rx=\"{}\" ry=\"{}\"",
            number(rect.rx.abs()),
            number(rect.ry.abs()),
        );
    }
    element.push_str(attrs);
    element.push_str("/>");
    element
}

fn normalize_extent(origin: f64, extent: f64) -> (f64, f64) {
    if extent < 0.0 {
        (origin + extent, -extent)
    } else {
        (origin, extent)
    }
}

/// The fill and stroke attributes for a node.
///
/// A concrete colour is written as a value; a gradient is written as a
/// `url(#id)` reference to the definition [`GradientDefs`] emitted (FEAT-027).
fn paint_attributes(node: &ResolvedNode, defs: &GradientDefs) -> String {
    let mut attrs = String::new();
    match &node.paint.fill {
        Some(fill) => write_paint(&mut attrs, "fill", fill, defs),
        None => attrs.push_str(" fill=\"none\""),
    }
    if let Some(stroke) = &node.paint.stroke {
        write_paint(&mut attrs, "stroke", &stroke.paint, defs);
        let _ = write!(
            attrs,
            " stroke-width=\"{}\" stroke-linecap=\"{}\" stroke-linejoin=\"{}\"",
            number(stroke.width),
            cap_name(stroke.cap),
            join_name(stroke.join),
        );
    }
    attrs
}

/// Writes one paint as an attribute: a colour value or a gradient reference.
fn write_paint(attrs: &mut String, name: &str, paint: &Paint, defs: &GradientDefs) {
    match defs.id(paint) {
        Some(id) => {
            let _ = write!(attrs, " {name}=\"url(#{id})\"");
        }
        None => {
            let value = match paint {
                Paint::Color { value } => value.as_str(),
                // A gradient always has a definition; unreachable in practice.
                Paint::Gradient(_) => "none",
            };
            let _ = write!(attrs, " {name}=\"{}\"", escape_attr(value));
        }
    }
}

/// SVG path data for a concrete path, with absolute commands.
fn path_data(path: &PathGeometry) -> String {
    let mut data = String::new();
    for subpath in &path.subpaths {
        if subpath.segments.is_empty() {
            continue;
        }
        if !data.is_empty() {
            data.push(' ');
        }
        let _ = write!(
            data,
            "M {} {}",
            number(subpath.start[0]),
            number(subpath.start[1])
        );
        for segment in &subpath.segments {
            match segment {
                Segment::Line { to } => {
                    let _ = write!(data, " L {} {}", number(to[0]), number(to[1]));
                }
                Segment::Cubic { ctrl1, ctrl2, to } => {
                    let _ = write!(
                        data,
                        " C {} {} {} {} {} {}",
                        number(ctrl1[0]),
                        number(ctrl1[1]),
                        number(ctrl2[0]),
                        number(ctrl2[1]),
                        number(to[0]),
                        number(to[1]),
                    );
                }
                Segment::Quadratic { ctrl, to } => {
                    let _ = write!(
                        data,
                        " Q {} {} {} {}",
                        number(ctrl[0]),
                        number(ctrl[1]),
                        number(to[0]),
                        number(to[1]),
                    );
                }
                Segment::Arc {
                    rx,
                    ry,
                    x_rotation,
                    large_arc,
                    sweep,
                    to,
                } => {
                    let _ = write!(
                        data,
                        " A {} {} {} {} {} {} {}",
                        number(*rx),
                        number(*ry),
                        number(*x_rotation),
                        flag(*large_arc),
                        flag(*sweep),
                        number(to[0]),
                        number(to[1]),
                    );
                }
            }
        }
        if subpath.closed {
            data.push_str(" Z");
        }
    }
    data
}

fn points(points: &[[f64; 2]]) -> String {
    let mut out = String::new();
    for (index, point) in points.iter().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{},{}", number(point[0]), number(point[1]));
    }
    out
}

fn flag(value: bool) -> &'static str {
    if value {
        "1"
    } else {
        "0"
    }
}

fn cap_name(cap: StrokeCap) -> &'static str {
    match cap {
        StrokeCap::Butt => "butt",
        StrokeCap::Round => "round",
        StrokeCap::Square => "square",
    }
}

fn join_name(join: StrokeJoin) -> &'static str {
    match join {
        StrokeJoin::Miter => "miter",
        StrokeJoin::Round => "round",
        StrokeJoin::Bevel => "bevel",
    }
}

pub(super) fn is_transparent(background: &str) -> bool {
    background.is_empty()
        || background.eq_ignore_ascii_case("transparent")
        || background.eq_ignore_ascii_case("none")
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

/// Formats a coordinate compactly and deterministically.
///
/// The shortest round-tripping decimal is used for the ordinary range, `-0` is
/// normalised to `0`, and very large or very small magnitudes use exponent form
/// so a pathological coordinate cannot balloon the file. The result is always a
/// valid SVG number.
fn number(value: f64) -> String {
    if !value.is_finite() {
        return "0".to_string();
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let magnitude = value.abs();
    if magnitude >= 1e15 || magnitude < 1e-4 {
        format!("{value:e}")
    } else {
        format!("{value}")
    }
}

/// Escapes text content so no value can introduce markup (NFR-023).
fn escape_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ if !is_xml_char(ch) => {}
            _ => out.push(ch),
        }
    }
    out
}

/// Escapes an attribute value, including the quote that delimits it.
fn escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            _ if !is_xml_char(ch) => {}
            _ => out.push(ch),
        }
    }
    out
}

/// Whether a character may appear in an XML 1.0 document.
fn is_xml_char(ch: char) -> bool {
    matches!(ch, '\u{9}' | '\u{A}' | '\u{D}')
        || ('\u{20}'..='\u{D7FF}').contains(&ch)
        || ('\u{E000}'..='\u{FFFD}').contains(&ch)
        || ('\u{10000}'..='\u{10FFFF}').contains(&ch)
}

/// Hands out XML identifiers that are valid names and unique within a document.
#[derive(Default)]
struct IdAllocator {
    used: HashSet<String>,
}

impl IdAllocator {
    fn allocate(&mut self, raw: &str) -> String {
        let base = xml_id(raw);
        if self.used.insert(base.clone()) {
            return base;
        }
        let mut suffix = 2usize;
        loop {
            let candidate = format!("{base}-{suffix}");
            if self.used.insert(candidate.clone()) {
                return candidate;
            }
            suffix += 1;
        }
    }
}

/// Coerces a scene identifier into a valid XML name (`NCName`).
fn xml_id(raw: &str) -> String {
    let mut chars = raw.chars();
    let mut id = String::with_capacity(raw.len() + 6);
    match chars.next() {
        Some(first) if first.is_alphabetic() || first == '_' => id.push(first),
        Some(first) => {
            id.push_str("vectr-");
            if first.is_alphanumeric() || matches!(first, '-' | '.') {
                id.push(first);
            }
        }
        None => return "vectr".to_string(),
    }
    for ch in chars {
        if ch.is_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            id.push(ch);
        } else {
            id.push('-');
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_compact_and_normalise_negative_zero() {
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(1.0), "1");
        assert_eq!(number(2.5), "2.5");
        assert_eq!(number(-3.25), "-3.25");
    }

    #[test]
    fn extreme_numbers_use_exponent_form_and_stay_finite() {
        assert_eq!(number(1e300), "1e300");
        assert_eq!(number(-1e300), "-1e300");
        assert_eq!(number(1e-9), "1e-9");
        assert_eq!(number(f64::INFINITY), "0");
        assert_eq!(number(f64::NAN), "0");
    }

    #[test]
    fn text_and_attributes_are_escaped() {
        assert_eq!(escape_text("a & b < c > d"), "a &amp; b &lt; c &gt; d");
        assert_eq!(escape_attr("\"x\" 'y'"), "&quot;x&quot; &apos;y&apos;");
        assert_eq!(escape_text("a\u{0}b"), "ab");
        assert_eq!(escape_attr("line\nbreak"), "line&#10;break");
    }

    #[test]
    fn identifiers_are_valid_xml_names() {
        assert_eq!(xml_id("logo"), "logo");
        assert_eq!(xml_id("_x-1.2"), "_x-1.2");
        assert_eq!(xml_id("1 bad"), "vectr-1-bad");
        assert_eq!(xml_id(""), "vectr");
    }
}
