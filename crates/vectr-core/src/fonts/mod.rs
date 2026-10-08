//! The Font & Asset Manager: resolves a render model's fonts and shapes its text
//! into glyph outlines (FEAT-024).
//!
//! The compiler resolves a text element's font reference and carries the font
//! file in the render model (C-003), leaving only glyph geometry for an exporter
//! to finalize (FEAT-011). This module supplies that geometry: it parses the
//! model's fonts, lays a run out into lines, wraps it to its declared width,
//! aligns it about the anchor, shapes it with `rustybuzz`, and reads each glyph's
//! contours with `ttf-parser` as concrete path geometry. Every exporter then
//! emits outlines, so SVG, PNG and PDF output does not depend on the font being
//! installed (FEAT-012, FEAT-013, NFR-040).
//!
//! # Fallback and licensing
//!
//! A character the chosen font does not cover is substituted from the model's
//! fallback font (D-004); a character no available font covers is reported as a
//! warning naming it, rather than rendered as a blank box (FEAT-024). Vectr
//! bundles no font of its own: every font arrives in the render model, so a
//! user-supplied font is used on the user's responsibility and never
//! redistributed (NFR-040). The same model always shapes to the same outlines
//! (NFR-010).

mod outline;

use crate::primitives::{Path, Segment, SubPath};
use crate::render::{ResolvedFont, TextRun};
use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, Location, TextAlign};

use outline::GlyphOutline;

/// The font asset id a run falls back to when its own font lacks a glyph: the
/// caller-supplied fallback font (D-004).
///
/// Parallel to the compiler's `default` id (D-017): a caller supplies its
/// open-licensed fallback under this id, and the font manager covers characters
/// the chosen font does not.
pub const FALLBACK_FONT_ID: &str = "fallback";

/// A text node names a font the render model does not carry in a usable form.
///
/// A missing font is refused at compile time; at this seam the compiler has
/// already succeeded, so the node is omitted with this warning rather than
/// failing the export (C-003).
pub const FONT_MISSING: DiagnosticCode = DiagnosticCode::new("W_FONT_MISSING");

/// No available font covers a character, so its glyph is omitted (FEAT-024).
pub const MISSING_GLYPH: DiagnosticCode = DiagnosticCode::new("W_MISSING_GLYPH");

/// The glyph outline of a text run, in its node's local coordinates, and the
/// findings recorded while shaping it.
///
/// The path is anchored where the compiler baked the element's anchor into the
/// node's transform: the first line's baseline sits at the origin and later
/// lines descend by the run's line height.
#[derive(Debug, Clone, PartialEq)]
pub struct OutlinedText {
    /// The glyph contours. Empty when the run draws nothing.
    pub path: Path,
    /// Warnings for content that could not be outlined.
    pub diagnostics: Diagnostics,
}

/// The fonts a render model carries, parsed once and reused across its text
/// nodes (FEAT-024).
pub struct FontLibrary<'a> {
    faces: Vec<LoadedFace<'a>>,
    fallback: Option<usize>,
}

/// A parsed font face, borrowing the model's font bytes.
struct LoadedFace<'a> {
    id: &'a str,
    face: rustybuzz::Face<'a>,
}

/// One shaped glyph, its offsets and advance already in scene units.
struct ShapedGlyph {
    face: usize,
    glyph_id: u16,
    x_offset: f64,
    y_offset: f64,
    x_advance: f64,
}

impl<'a> FontLibrary<'a> {
    /// Parses every usable font the model carries.
    ///
    /// A font whose bytes cannot be read is left out; a text node that names it
    /// reports a missing font when it is outlined, so the failure is located to
    /// the node that needs it. The model's fallback font, if any, is the one
    /// named [`FALLBACK_FONT_ID`].
    pub fn new(fonts: &'a [ResolvedFont]) -> Self {
        let mut faces = Vec::with_capacity(fonts.len());
        for font in fonts {
            if let Some(face) = rustybuzz::Face::from_slice(&font.data, 0) {
                faces.push(LoadedFace {
                    id: font.id.as_str(),
                    face,
                });
            }
        }
        let fallback = faces
            .iter()
            .position(|loaded| loaded.id == FALLBACK_FONT_ID);
        Self { faces, fallback }
    }

    /// Outlines a text run in its node's local coordinates (FEAT-024).
    ///
    /// A run naming a font the model does not carry is omitted with a warning;
    /// so is a character no available font covers. The run's own font is
    /// preferred, with the model's fallback covering characters it lacks.
    pub fn outline(&self, run: &TextRun, element_id: &str) -> OutlinedText {
        let mut diagnostics = Diagnostics::new();

        let Some(primary) = self.faces.iter().position(|face| face.id == run.font_id) else {
            diagnostics.push(
                Diagnostic::warning(
                    FONT_MISSING,
                    format!(
                        "text element `{element_id}` names font `{}`, which the render model does not carry",
                        run.font_id
                    ),
                )
                .with_location(Location::element_at(element_id, "/fontId")),
            );
            return OutlinedText::empty(diagnostics);
        };

        if !run.font_size.is_finite() || run.font_size <= 0.0 {
            diagnostics.push(
                Diagnostic::warning(
                    FONT_MISSING,
                    format!(
                        "text element `{element_id}` has a font size that is not a positive number"
                    ),
                )
                .with_location(Location::element_at(element_id, "/geometry/fontSize")),
            );
            return OutlinedText::empty(diagnostics);
        }

        let fallback = self.fallback.filter(|&index| index != primary);
        let spacing = finite_or(run.letter_spacing, 0.0);
        let line_height = finite_or(run.line_height, run.font_size);

        let lines = self.wrap(run, primary, fallback, spacing);
        let mut subpaths: Vec<SubPath> = Vec::new();
        let mut missing: Vec<char> = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let (mut line_paths, width, line_missing) =
                self.shape_line(primary, fallback, line, run.font_size, spacing);
            for ch in line_missing {
                if !missing.contains(&ch) {
                    missing.push(ch);
                }
            }
            let origin = match run.align {
                TextAlign::Start => 0.0,
                TextAlign::Center => -width / 2.0,
                TextAlign::End => -width,
            };
            let baseline = index as f64 * line_height;
            for subpath in line_paths.drain(..) {
                subpaths.push(translate(subpath, origin, baseline));
            }
        }

        for ch in &missing {
            diagnostics.push(
                Diagnostic::warning(
                    MISSING_GLYPH,
                    format!(
                        "text element `{element_id}` has no glyph for `{ch}` (U+{:04X}) in font `{}` and no fallback covers it",
                        *ch as u32,
                        run.font_id
                    ),
                )
                .with_location(Location::element_at(element_id, "/geometry/text")),
            );
        }

        OutlinedText {
            path: Path { subpaths },
            diagnostics,
        }
    }

    /// Splits a run's string into the lines it lays out: an explicit newline
    /// starts a line, and a declared width wraps each paragraph to it.
    fn wrap(
        &self,
        run: &TextRun,
        primary: usize,
        fallback: Option<usize>,
        spacing: f64,
    ) -> Vec<String> {
        let width = run.width.filter(|width| width.is_finite() && *width > 0.0);
        let mut lines = Vec::new();
        for paragraph in run.value.split('\n') {
            match width {
                Some(width) => {
                    lines.extend(self.wrap_paragraph(
                        paragraph,
                        width,
                        primary,
                        fallback,
                        run.font_size,
                        spacing,
                    ));
                }
                None => lines.push(paragraph.to_string()),
            }
        }
        lines
    }

    /// Wraps one paragraph greedily at whitespace so a word is never split.
    ///
    /// A word wider than the wrap width is placed on its own line and allowed to
    /// overflow, so a long token is never dropped.
    fn wrap_paragraph(
        &self,
        text: &str,
        width: f64,
        primary: usize,
        fallback: Option<usize>,
        size: f64,
        spacing: f64,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let mut current = String::new();
        for token in tokens(text) {
            if current.is_empty() {
                if token.chars().all(char::is_whitespace) {
                    continue;
                }
                current.push_str(token);
                continue;
            }
            let mut candidate = current.clone();
            candidate.push_str(token);
            if self.measure(primary, fallback, &candidate, size, spacing) <= width {
                current = candidate;
            } else {
                lines.push(std::mem::take(&mut current));
                if !token.chars().all(char::is_whitespace) {
                    current.push_str(token);
                }
            }
        }
        lines.push(current);
        lines
    }

    /// The advance width of a string in scene units, used for wrapping.
    fn measure(
        &self,
        primary: usize,
        fallback: Option<usize>,
        text: &str,
        size: f64,
        spacing: f64,
    ) -> f64 {
        let (glyphs, _) = self.shape(primary, fallback, text, size);
        advance_width(&glyphs, spacing)
    }

    /// Shapes one line and outlines its glyphs in line-local coordinates.
    ///
    /// The returned subpaths sit at the first glyph's pen origin with the
    /// baseline at `y = 0`; the caller translates them to the line's alignment
    /// and baseline.
    fn shape_line(
        &self,
        primary: usize,
        fallback: Option<usize>,
        text: &str,
        size: f64,
        spacing: f64,
    ) -> (Vec<SubPath>, f64, Vec<char>) {
        let (glyphs, missing) = self.shape(primary, fallback, text, size);
        let width = advance_width(&glyphs, spacing);
        let mut subpaths = Vec::new();
        let mut pen = 0.0;
        for glyph in &glyphs {
            let face = &self.faces[glyph.face];
            let mut outline = GlyphOutline::new(
                glyph_scale(face, size),
                pen + glyph.x_offset,
                -glyph.y_offset,
            );
            face.face
                .outline_glyph(ttf_parser::GlyphId(glyph.glyph_id), &mut outline);
            subpaths.extend(outline.subpaths);
            pen += glyph.x_advance + spacing;
        }
        (subpaths, width, missing)
    }

    /// Shapes a string into placed glyphs, splitting it into runs by coverage so
    /// a character the chosen font lacks is shaped with the fallback instead.
    fn shape(
        &self,
        primary: usize,
        fallback: Option<usize>,
        text: &str,
        size: f64,
    ) -> (Vec<ShapedGlyph>, Vec<char>) {
        let (runs, missing) = self.runs(primary, fallback, text);
        let mut glyphs = Vec::new();
        for (face_index, run_text) in runs {
            let face = &self.faces[face_index];
            let scale = glyph_scale(face, size);
            if scale == 0.0 {
                continue;
            }
            let mut buffer = rustybuzz::UnicodeBuffer::new();
            buffer.push_str(run_text);
            let output = rustybuzz::shape(&face.face, &[], buffer);
            for (info, position) in output.glyph_infos().iter().zip(output.glyph_positions()) {
                glyphs.push(ShapedGlyph {
                    face: face_index,
                    glyph_id: info.glyph_id as u16,
                    x_offset: f64::from(position.x_offset) * scale,
                    y_offset: f64::from(position.y_offset) * scale,
                    x_advance: f64::from(position.x_advance) * scale,
                });
            }
        }
        (glyphs, missing)
    }

    /// Splits a string into runs of characters the same font covers, collecting
    /// the characters no available font covers.
    fn runs<'b>(
        &self,
        primary: usize,
        fallback: Option<usize>,
        text: &'b str,
    ) -> (Vec<(usize, &'b str)>, Vec<char>) {
        let mut runs: Vec<(usize, &str)> = Vec::new();
        let mut missing = Vec::new();
        let mut current: Option<usize> = None;
        let mut start = 0;
        for (index, ch) in text.char_indices() {
            let face = if self.faces[primary].face.glyph_index(ch).is_some() {
                Some(primary)
            } else {
                fallback.filter(|&other| self.faces[other].face.glyph_index(ch).is_some())
            };
            match face {
                Some(face) => {
                    if current != Some(face) {
                        if let Some(previous) = current {
                            runs.push((previous, &text[start..index]));
                        }
                        current = Some(face);
                        start = index;
                    }
                }
                None => {
                    if let Some(previous) = current {
                        runs.push((previous, &text[start..index]));
                        current = None;
                    }
                    missing.push(ch);
                }
            }
        }
        if let Some(previous) = current {
            runs.push((previous, &text[start..]));
        }
        (runs, missing)
    }
}

impl OutlinedText {
    fn empty(diagnostics: Diagnostics) -> Self {
        Self {
            path: Path {
                subpaths: Vec::new(),
            },
            diagnostics,
        }
    }
}

/// Outlines a text run against the fonts a render model carries (FEAT-024).
///
/// A convenience over [`FontLibrary`] for a caller that has a single run; an
/// exporter that walks many text nodes builds one [`FontLibrary`] and reuses it.
pub fn outline_text(run: &TextRun, element_id: &str, fonts: &[ResolvedFont]) -> OutlinedText {
    FontLibrary::new(fonts).outline(run, element_id)
}

/// The scene-unit scale of a font: the run's em size over the font's design
/// units, or zero when the font reports no usable em.
fn glyph_scale(face: &LoadedFace<'_>, size: f64) -> f64 {
    let units = face.face.units_per_em();
    if units <= 0 {
        0.0
    } else {
        size / f64::from(units)
    }
}

/// The advance width of shaped glyphs, adding the inter-glyph letter spacing
/// (but not a trailing one).
fn advance_width(glyphs: &[ShapedGlyph], spacing: f64) -> f64 {
    if glyphs.is_empty() {
        0.0
    } else {
        glyphs.iter().map(|glyph| glyph.x_advance).sum::<f64>()
            + spacing * (glyphs.len() as f64 - 1.0)
    }
}

/// Splits text into alternating runs of whitespace and non-whitespace, so
/// wrapping never breaks inside a word.
fn tokens(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0;
    let mut previous: Option<bool> = None;
    for (index, ch) in text.char_indices() {
        let whitespace = ch.is_whitespace();
        if previous.is_some_and(|previous| previous != whitespace) {
            tokens.push(&text[start..index]);
            start = index;
        }
        previous = Some(whitespace);
    }
    if start < text.len() {
        tokens.push(&text[start..]);
    }
    tokens
}

/// Translates a subpath by a scene-unit offset.
fn translate(mut subpath: SubPath, dx: f64, dy: f64) -> SubPath {
    subpath.start[0] += dx;
    subpath.start[1] += dy;
    for segment in &mut subpath.segments {
        match segment {
            Segment::Line { to } => shift(to, dx, dy),
            Segment::Quadratic { ctrl, to } => {
                shift(ctrl, dx, dy);
                shift(to, dx, dy);
            }
            Segment::Cubic { ctrl1, ctrl2, to } => {
                shift(ctrl1, dx, dy);
                shift(ctrl2, dx, dy);
                shift(to, dx, dy);
            }
            Segment::Arc { to, .. } => shift(to, dx, dy),
        }
    }
    subpath
}

fn shift(point: &mut [f64; 2], dx: f64, dy: f64) {
    point[0] += dx;
    point[1] += dy;
}

/// A finite value, or the given fallback for a non-finite one.
fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a bundled open-licensed font so the tests shape real glyphs.
    fn font_bytes(file: &str) -> Vec<u8> {
        let path = format!("{}/../../assets/fonts/{file}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(&path).unwrap_or_else(|error| panic!("could not read {path}: {error}"))
    }

    fn inter() -> ResolvedFont {
        ResolvedFont {
            id: "body".to_string(),
            name: "Inter".to_string(),
            data: font_bytes("Inter.ttf"),
        }
    }

    fn noto() -> ResolvedFont {
        ResolvedFont {
            id: FALLBACK_FONT_ID.to_string(),
            name: "Noto Sans".to_string(),
            data: font_bytes("NotoSans.ttf"),
        }
    }

    fn run(value: &str) -> TextRun {
        TextRun {
            value: value.to_string(),
            font_id: "body".to_string(),
            font_size: 100.0,
            align: TextAlign::Start,
            line_height: 100.0,
            letter_spacing: 0.0,
            width: None,
        }
    }

    /// Every point a path references, including curve controls.
    fn points(path: &Path) -> Vec<[f64; 2]> {
        let mut points = Vec::new();
        for subpath in &path.subpaths {
            points.push(subpath.start);
            for segment in &subpath.segments {
                match segment {
                    Segment::Line { to } => points.push(*to),
                    Segment::Quadratic { ctrl, to } => {
                        points.push(*ctrl);
                        points.push(*to);
                    }
                    Segment::Cubic { ctrl1, ctrl2, to } => {
                        points.push(*ctrl1);
                        points.push(*ctrl2);
                        points.push(*to);
                    }
                    Segment::Arc { to, .. } => points.push(*to),
                }
            }
        }
        points
    }

    /// The bounding box of a path as `(min_x, min_y, max_x, max_y)`.
    fn bounds(path: &Path) -> (f64, f64, f64, f64) {
        let points = points(path);
        assert!(!points.is_empty(), "expected geometry");
        let mut box_ = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for point in points {
            box_.0 = box_.0.min(point[0]);
            box_.1 = box_.1.min(point[1]);
            box_.2 = box_.2.max(point[0]);
            box_.3 = box_.3.max(point[1]);
        }
        box_
    }

    #[test]
    fn a_run_outlines_to_concrete_geometry_above_the_baseline() {
        let fonts = [inter()];
        let outlined = outline_text(&run("Hi"), "t1", &fonts);
        assert!(
            outlined.diagnostics.is_empty(),
            "{:?}",
            outlined.diagnostics
        );
        assert!(!outlined.path.is_empty());
        let (_, min_y, _, max_y) = bounds(&outlined.path);
        // Glyphs sit above the baseline, in the y-down axis, so y is negative
        // and no glyph here descends below the baseline.
        assert!(min_y < -1.0, "min_y = {min_y}");
        assert!(max_y < 1.0, "max_y = {max_y}");
        for point in points(&outlined.path) {
            assert!(point[0].is_finite() && point[1].is_finite(), "{point:?}");
        }
    }

    #[test]
    fn a_font_the_model_does_not_carry_is_reported_naming_it() {
        let outlined = outline_text(&run("Hi"), "t1", &[]);
        assert!(outlined.path.is_empty());
        let warning = outlined
            .diagnostics
            .warnings()
            .find(|warning| warning.code == FONT_MISSING)
            .expect("a missing-font warning");
        assert!(warning.message.contains("body"), "{}", warning.message);
        assert_eq!(
            warning
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/fontId")
        );
    }

    #[test]
    fn a_character_the_chosen_font_lacks_is_drawn_from_the_fallback() {
        // U+0149 is carried by Noto Sans but not by Inter, so only the fallback
        // can cover it.
        let with_fallback = outline_text(&run("\u{149}"), "t1", &[inter(), noto()]);
        assert!(!with_fallback.path.is_empty());
        assert!(
            !with_fallback
                .diagnostics
                .warnings()
                .any(|warning| warning.code == MISSING_GLYPH),
            "{:?}",
            with_fallback.diagnostics
        );

        // Without a fallback the same character is reported, not drawn blank.
        let without_fallback = outline_text(&run("\u{149}"), "t1", &[inter()]);
        assert!(without_fallback.path.is_empty());
        assert!(without_fallback
            .diagnostics
            .warnings()
            .any(|warning| warning.code == MISSING_GLYPH));
    }

    #[test]
    fn a_character_no_font_covers_is_reported_rather_than_drawn() {
        // Neither bundled font carries CJK, so the glyph is reported.
        let outlined = outline_text(&run("\u{4e2d}"), "t1", &[inter(), noto()]);
        assert!(outlined.path.is_empty());
        let warning = outlined
            .diagnostics
            .warnings()
            .find(|warning| warning.code == MISSING_GLYPH)
            .expect("a missing-glyph warning");
        assert!(warning.message.contains("U+4E2D"), "{}", warning.message);
    }

    #[test]
    fn lines_descend_by_the_line_height() {
        let fonts = [inter()];
        let outlined = outline_text(&run("H\nH"), "t1", &fonts);
        let (_, min_y, _, max_y) = bounds(&outlined.path);
        // Two lines, one line height apart, span more than a single em.
        assert!(max_y - min_y > 150.0, "span = {}", max_y - min_y);
        assert!(
            max_y > 50.0,
            "the second line sits below the first: {max_y}"
        );
    }

    #[test]
    fn alignment_offsets_the_run_about_the_anchor() {
        let fonts = [inter()];
        let mut centered = run("HH");
        centered.align = TextAlign::Center;
        let (min_x, _, max_x, _) = bounds(&outline_text(&centered, "t1", &fonts).path);
        assert!(min_x < -1.0 && max_x > 1.0, "centered: {min_x}..{max_x}");

        let mut ended = run("HH");
        ended.align = TextAlign::End;
        let (_, _, max_x, _) = bounds(&outline_text(&ended, "t1", &fonts).path);
        assert!(max_x < 1.0, "ended: {max_x}");

        let (min_x, _, _, _) = bounds(&outline_text(&run("HH"), "t1", &fonts).path);
        assert!(min_x > -1.0, "started: {min_x}");
    }

    #[test]
    fn a_declared_width_wraps_the_run_onto_several_lines() {
        let fonts = [inter()];
        let mut wrapped = run("aaa aaa aaa");
        wrapped.width = Some(150.0);
        let (_, min_y, _, max_y) = bounds(&outline_text(&wrapped, "t1", &fonts).path);
        assert!(max_y - min_y > 150.0, "span = {}", max_y - min_y);

        let (_, min_y, _, max_y) = bounds(&outline_text(&run("aaa aaa aaa"), "t1", &fonts).path);
        assert!(max_y - min_y < 150.0, "unwrapped span = {}", max_y - min_y);
    }

    #[test]
    fn outlining_is_deterministic() {
        let fonts = [inter(), noto()];
        let first = outline_text(&run("Hello \u{149}"), "t1", &fonts);
        let second = outline_text(&run("Hello \u{149}"), "t1", &fonts);
        assert_eq!(first, second, "shaping must be stable (NFR-010)");
    }
}
