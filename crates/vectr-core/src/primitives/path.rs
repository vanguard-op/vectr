//! SVG path-data parsing for the freeform `path` primitive (FEAT-002).
//!
//! A scene's `geometry.pathData` is SVG path syntax. Resolving a path needs
//! two facts from it: which subpaths are actually drawn, and which of those are
//! closed, so that an empty path renders nothing and a stroke applies caps only
//! at the open ends (FEAT-002).
//!
//! The parser normalises every command to absolute coordinates in one pass and
//! is pure and deterministic: the same text always yields the same subpaths
//! (NFR-010). Curves and arcs are carried as written rather than flattened, so
//! the exporters can emit them losslessly.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A concrete path: its subpaths in document order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Path {
    /// The subpaths, in the order they appear in the path data.
    pub subpaths: Vec<SubPath>,
}

/// A single subpath: where it starts, its segments, and whether it is closed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubPath {
    /// The subpath's start point, in scene units.
    pub start: [f64; 2],
    /// The subpath's segments, in order.
    pub segments: Vec<Segment>,
    /// Whether a close command ended the subpath.
    pub closed: bool,
}

/// One drawing segment, with absolute endpoints and controls.
///
/// Serializes as a `kind`-tagged object — `{"kind":"cubic", ...}` — so a path's
/// concrete segments survive the render model's JSON round trip (D-014).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Segment {
    /// A straight line to `to`.
    Line {
        /// The segment's end point.
        to: [f64; 2],
    },
    /// A cubic Bézier from the current point to `to`.
    Cubic {
        /// The first control point.
        ctrl1: [f64; 2],
        /// The second control point.
        ctrl2: [f64; 2],
        /// The segment's end point.
        to: [f64; 2],
    },
    /// A quadratic Bézier from the current point to `to`.
    Quadratic {
        /// The control point.
        ctrl: [f64; 2],
        /// The segment's end point.
        to: [f64; 2],
    },
    /// An elliptical arc from the current point to `to`.
    Arc {
        /// The ellipse's X radius.
        rx: f64,
        /// The ellipse's Y radius.
        ry: f64,
        /// The ellipse's X-axis rotation in degrees.
        x_rotation: f64,
        /// Whether the arc is the larger of the two candidates.
        large_arc: bool,
        /// Whether the arc sweeps in the positive angle direction.
        sweep: bool,
        /// The segment's end point.
        to: [f64; 2],
    },
}

impl Path {
    /// Whether the path draws nothing: no subpath carries a segment.
    ///
    /// A path that only moves the pen draws no geometry, so it renders nothing
    /// (FEAT-002).
    pub fn is_empty(&self) -> bool {
        self.subpaths
            .iter()
            .all(|subpath| subpath.segments.is_empty())
    }

    /// Whether every subpath is closed.
    ///
    /// A stroke applies caps at the ends of any subpath that is not closed
    /// (FEAT-002).
    pub fn is_closed(&self) -> bool {
        !self.subpaths.is_empty() && self.subpaths.iter().all(|subpath| subpath.closed)
    }
}

/// A malformed path-data string, with the reason it was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathError {
    /// A human-readable explanation of the first parse failure.
    pub message: String,
}

impl PathError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PathError {}

/// Parses SVG path data into concrete subpaths.
///
/// Every command is accepted except unsupported ones; coordinates are made
/// absolute, and `M`/`m` runs are treated as `L`/`l` as the syntax specifies.
/// The first command must be a move, and every parameter must be present.
pub fn parse(data: &str) -> Result<Path, PathError> {
    Parser::new(data).parse()
}

struct Parser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
    subpaths: Vec<SubPath>,
    /// Index of the subpath currently being drawn, if any.
    current: Option<usize>,
    /// The current point in absolute scene units.
    cur: [f64; 2],
    /// The start of the current subpath, restored by a close command.
    start: [f64; 2],
    /// The last cubic control point, for a smooth `S`/`s` command.
    last_cubic_ctrl: Option<[f64; 2]>,
    /// The last quadratic control point, for a smooth `T`/`t` command.
    last_quad_ctrl: Option<[f64; 2]>,
    /// The last command, replayed for implicit repeated coordinates.
    last_cmd: Option<u8>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            pos: 0,
            subpaths: Vec::new(),
            current: None,
            cur: [0.0, 0.0],
            start: [0.0, 0.0],
            last_cubic_ctrl: None,
            last_quad_ctrl: None,
            last_cmd: None,
        }
    }

    fn parse(mut self) -> Result<Path, PathError> {
        self.skip_separators();
        match self.peek() {
            Some(b'M') | Some(b'm') => {}
            _ => return Err(PathError::new("path data must begin with a move command")),
        }

        loop {
            self.skip_separators();
            if self.pos >= self.bytes.len() {
                break;
            }

            let command = match self.peek() {
                Some(byte) if byte.is_ascii_alphabetic() => {
                    self.pos += 1;
                    if !is_command(byte) {
                        return Err(PathError::new(format!(
                            "unknown path command `{}`",
                            byte as char
                        )));
                    }
                    self.last_cmd = Some(byte);
                    byte
                }
                _ => match self.last_cmd {
                    Some(b'M') => b'L',
                    Some(b'm') => b'l',
                    Some(byte) => byte,
                    None => return Err(PathError::new("expected a path command")),
                },
            };

            self.execute(command)?;
        }

        Ok(Path {
            subpaths: self.subpaths,
        })
    }

    fn execute(&mut self, command: u8) -> Result<(), PathError> {
        match command {
            b'M' => self.moveto(false),
            b'm' => self.moveto(true),
            b'L' => self.line(false),
            b'l' => self.line(true),
            b'H' => self.horizontal(false),
            b'h' => self.horizontal(true),
            b'V' => self.vertical(false),
            b'v' => self.vertical(true),
            b'C' => self.cubic(false),
            b'c' => self.cubic(true),
            b'S' => self.smooth_cubic(false),
            b's' => self.smooth_cubic(true),
            b'Q' => self.quadratic(false),
            b'q' => self.quadratic(true),
            b'T' => self.smooth_quadratic(false),
            b't' => self.smooth_quadratic(true),
            b'A' => self.arc(false),
            b'a' => self.arc(true),
            b'Z' | b'z' => {
                self.close();
                Ok(())
            }
            other => Err(PathError::new(format!(
                "unknown path command `{}`",
                other as char
            ))),
        }
    }

    fn moveto(&mut self, relative: bool) -> Result<(), PathError> {
        let point = self.read_point(relative)?;
        let index = self.subpaths.len();
        self.subpaths.push(SubPath {
            start: point,
            segments: Vec::new(),
            closed: false,
        });
        self.current = Some(index);
        self.cur = point;
        self.start = point;
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;
        Ok(())
    }

    fn line(&mut self, relative: bool) -> Result<(), PathError> {
        let to = self.read_point(relative)?;
        self.segment(Segment::Line { to });
        self.cur = to;
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;
        Ok(())
    }

    fn horizontal(&mut self, relative: bool) -> Result<(), PathError> {
        let value = self.read_number()?;
        let x = if relative { self.cur[0] + value } else { value };
        let to = [x, self.cur[1]];
        self.segment(Segment::Line { to });
        self.cur = to;
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;
        Ok(())
    }

    fn vertical(&mut self, relative: bool) -> Result<(), PathError> {
        let value = self.read_number()?;
        let y = if relative { self.cur[1] + value } else { value };
        let to = [self.cur[0], y];
        self.segment(Segment::Line { to });
        self.cur = to;
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;
        Ok(())
    }

    fn cubic(&mut self, relative: bool) -> Result<(), PathError> {
        let ctrl1 = self.read_point(relative)?;
        let ctrl2 = self.read_point(relative)?;
        let to = self.read_point(relative)?;
        self.segment(Segment::Cubic { ctrl1, ctrl2, to });
        self.cur = to;
        self.last_cubic_ctrl = Some(ctrl2);
        self.last_quad_ctrl = None;
        Ok(())
    }

    fn smooth_cubic(&mut self, relative: bool) -> Result<(), PathError> {
        let ctrl1 = self
            .last_cubic_ctrl
            .map_or(self.cur, |ctrl| reflect(ctrl, self.cur));
        let ctrl2 = self.read_point(relative)?;
        let to = self.read_point(relative)?;
        self.segment(Segment::Cubic { ctrl1, ctrl2, to });
        self.cur = to;
        self.last_cubic_ctrl = Some(ctrl2);
        self.last_quad_ctrl = None;
        Ok(())
    }

    fn quadratic(&mut self, relative: bool) -> Result<(), PathError> {
        let ctrl = self.read_point(relative)?;
        let to = self.read_point(relative)?;
        self.segment(Segment::Quadratic { ctrl, to });
        self.cur = to;
        self.last_quad_ctrl = Some(ctrl);
        self.last_cubic_ctrl = None;
        Ok(())
    }

    fn smooth_quadratic(&mut self, relative: bool) -> Result<(), PathError> {
        let ctrl = self
            .last_quad_ctrl
            .map_or(self.cur, |point| reflect(point, self.cur));
        let to = self.read_point(relative)?;
        self.segment(Segment::Quadratic { ctrl, to });
        self.cur = to;
        self.last_quad_ctrl = Some(ctrl);
        self.last_cubic_ctrl = None;
        Ok(())
    }

    fn arc(&mut self, relative: bool) -> Result<(), PathError> {
        let rx = self.read_number()?.abs();
        let ry = self.read_number()?.abs();
        let x_rotation = self.read_number()?;
        let large_arc = self.read_flag()?;
        let sweep = self.read_flag()?;
        let to = self.read_point(relative)?;
        self.segment(Segment::Arc {
            rx,
            ry,
            x_rotation,
            large_arc,
            sweep,
            to,
        });
        self.cur = to;
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;
        Ok(())
    }

    fn close(&mut self) {
        if let Some(index) = self.current {
            self.subpaths[index].closed = true;
        }
        self.cur = self.start;
        self.current = None;
        self.last_cubic_ctrl = None;
        self.last_quad_ctrl = None;
    }

    fn segment(&mut self, segment: Segment) {
        let index = self.current.unwrap_or_else(|| {
            let index = self.subpaths.len();
            self.subpaths.push(SubPath {
                start: self.cur,
                segments: Vec::new(),
                closed: false,
            });
            self.current = Some(index);
            index
        });
        self.subpaths[index].segments.push(segment);
    }

    fn read_point(&mut self, relative: bool) -> Result<[f64; 2], PathError> {
        let x = self.read_number()?;
        let y = self.read_number()?;
        Ok(if relative {
            [self.cur[0] + x, self.cur[1] + y]
        } else {
            [x, y]
        })
    }

    fn read_flag(&mut self) -> Result<bool, PathError> {
        self.skip_separators();
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.pos += 1;
                Ok(true)
            }
            _ => Err(PathError::new("expected an arc flag of `0` or `1`")),
        }
    }

    fn read_number(&mut self) -> Result<f64, PathError> {
        self.skip_separators();
        let start = self.pos;
        let bytes = self.bytes;
        let mut index = self.pos;

        if matches!(bytes.get(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }

        let mut has_digits = false;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
            has_digits = true;
        }
        if bytes.get(index) == Some(&b'.') {
            index += 1;
            while bytes.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
                has_digits = true;
            }
        }
        if !has_digits {
            return Err(PathError::new("expected a number"));
        }

        // A malformed exponent must not be swallowed as part of the number.
        if matches!(bytes.get(index), Some(b'e') | Some(b'E')) {
            let mut exponent = index + 1;
            if matches!(bytes.get(exponent), Some(b'+') | Some(b'-')) {
                exponent += 1;
            }
            let mut exponent_digits = false;
            while bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
                exponent += 1;
                exponent_digits = true;
            }
            if !exponent_digits {
                return Err(PathError::new("invalid number: missing exponent digits"));
            }
            index = exponent;
        }

        let value: f64 = self.input[start..index]
            .parse()
            .map_err(|_| PathError::new("invalid number"))?;
        if !value.is_finite() {
            return Err(PathError::new("coordinates must be finite numbers"));
        }
        self.pos = index;
        Ok(value)
    }

    fn skip_separators(&mut self) {
        while self
            .peek()
            .is_some_and(|byte| byte.is_ascii_whitespace() || byte == b',')
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
}

/// The reflection of `control` through `pivot`, used by smooth commands.
fn reflect(control: [f64; 2], pivot: [f64; 2]) -> [f64; 2] {
    [2.0 * pivot[0] - control[0], 2.0 * pivot[1] - control[1]]
}

fn is_command(byte: u8) -> bool {
    matches!(
        byte,
        b'M' | b'm'
            | b'L'
            | b'l'
            | b'H'
            | b'h'
            | b'V'
            | b'v'
            | b'C'
            | b'c'
            | b'S'
            | b's'
            | b'Q'
            | b'q'
            | b'T'
            | b't'
            | b'A'
            | b'a'
            | b'Z'
            | b'z'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_to(to: [f64; 2]) -> Segment {
        Segment::Line { to }
    }

    #[test]
    fn parses_absolute_lines_and_close() {
        let path = parse("M 0 0 L 10 0 L 10 10 Z").expect("valid path");
        assert_eq!(path.subpaths.len(), 1);
        let subpath = &path.subpaths[0];
        assert_eq!(subpath.start, [0.0, 0.0]);
        assert_eq!(
            subpath.segments,
            vec![line_to([10.0, 0.0]), line_to([10.0, 10.0])]
        );
        assert!(subpath.closed);
        assert!(path.is_closed());
    }

    #[test]
    fn relative_commands_accumulate_from_the_current_point() {
        let path = parse("m 5 5 l 10 0 l 0 10 z").expect("valid path");
        let subpath = &path.subpaths[0];
        assert_eq!(subpath.start, [5.0, 5.0]);
        assert_eq!(
            subpath.segments,
            vec![line_to([15.0, 5.0]), line_to([15.0, 15.0])]
        );
    }

    #[test]
    fn horizontal_and_vertical_commands_hold_the_other_axis() {
        let path = parse("M0 0 H10 v5").expect("valid path");
        assert_eq!(
            path.subpaths[0].segments,
            vec![line_to([10.0, 0.0]), line_to([10.0, 5.0])]
        );
    }

    #[test]
    fn an_open_path_is_not_closed() {
        let path = parse("M0 0 L10 0 L10 10").expect("valid path");
        assert!(!path.is_closed());
        assert!(!path.is_empty());
    }

    #[test]
    fn a_move_only_path_is_empty() {
        let path = parse("M 10 10").expect("valid path");
        assert!(path.is_empty());
        assert!(!path.is_closed());
    }

    #[test]
    fn repeated_coordinates_after_a_move_are_lines() {
        let path = parse("M0 0 10 0 10 10").expect("valid path");
        assert_eq!(
            path.subpaths[0].segments,
            vec![line_to([10.0, 0.0]), line_to([10.0, 10.0])]
        );
    }

    #[test]
    fn adjacent_numbers_without_separators_are_read_as_one_number_each() {
        let path = parse("M.5.5L1-1").expect("valid path");
        assert_eq!(path.subpaths[0].start, [0.5, 0.5]);
        assert_eq!(path.subpaths[0].segments, vec![line_to([1.0, -1.0])]);
    }

    #[test]
    fn exponents_are_accepted() {
        let path = parse("M0 0 L1e1 1.5e0").expect("valid path");
        assert_eq!(path.subpaths[0].segments, vec![line_to([10.0, 1.5])]);
    }

    #[test]
    fn smooth_cubic_reflects_the_previous_control() {
        let path = parse("M0 0 C0 10 10 10 10 0 S20 -10 20 0").expect("valid path");
        let Segment::Cubic { ctrl1, .. } = path.subpaths[0].segments[1] else {
            panic!("expected a cubic segment");
        };
        assert_eq!(ctrl1, [10.0, -10.0]);
    }

    #[test]
    fn smooth_quadratic_reflects_the_previous_control() {
        let path = parse("M0 0 Q5 5 10 0 T20 0").expect("valid path");
        let Segment::Quadratic { ctrl, .. } = path.subpaths[0].segments[1] else {
            panic!("expected a quadratic segment");
        };
        assert_eq!(ctrl, [15.0, -5.0]);
    }

    #[test]
    fn arcs_capture_their_parameters_and_take_positive_radii() {
        let path = parse("M0 0 A5 10 0 0 1 10 0").expect("valid path");
        let Segment::Arc {
            rx,
            ry,
            large_arc,
            sweep,
            to,
            ..
        } = path.subpaths[0].segments[0]
        else {
            panic!("expected an arc segment");
        };
        assert_eq!((rx, ry), (5.0, 10.0));
        assert!(!large_arc);
        assert!(sweep);
        assert_eq!(to, [10.0, 0.0]);
    }

    #[test]
    fn multiple_move_commands_produce_multiple_subpaths() {
        let path = parse("M0 0 L1 1 M5 5 L6 6").expect("valid path");
        assert_eq!(path.subpaths.len(), 2);
        assert_eq!(path.subpaths[1].start, [5.0, 5.0]);
    }

    #[test]
    fn a_path_that_does_not_begin_with_a_move_is_refused() {
        let error = parse("L10 10").expect_err("must be refused");
        assert!(error.message.contains("move command"), "{}", error.message);
    }

    #[test]
    fn an_unknown_command_is_refused() {
        let error = parse("M0 0 X10").expect_err("must be refused");
        assert!(error.message.contains('X'), "{}", error.message);
    }

    #[test]
    fn a_missing_parameter_is_refused() {
        assert!(parse("M0 0 L10").is_err());
    }

    #[test]
    fn a_non_finite_number_is_refused() {
        assert!(parse("M0 0 L1e999 0").is_err());
    }

    #[test]
    fn parsing_is_deterministic() {
        let data = "M0 0 C0 10 10 10 10 0 S20 -10 20 0 A5 5 0 1 0 30 0 Z";
        assert_eq!(parse(data), parse(data));
    }
}
