//! Constraint and geometry resolution (FEAT-004).
//!
//! A scene may state relationships between elements — equal spacing, alignment,
//! attachment, containment, grid snapping — and this module resolves them into
//! concrete placements before rendering. Each element's resolved translation is
//! reported alongside any attachments a connector forms, so the compiler can
//! emit concrete geometry with no unresolved references (FEAT-004).
//!
//! Resolution is deterministic (NFR-010): constraints are applied in document
//! order, ties break on scene order, and every comparison uses a fixed
//! tolerance. An unsatisfiable or over-constrained system is refused with an
//! error naming the conflicting constraint, never a partial layout (FEAT-004).
//!
//! # Under-constrained defaults
//!
//! A constraint that does not pin a coordinate is left to a documented default:
//!
//! - `equalSpacing` without a value keeps the first and last element fixed and
//!   distributes the rest so the gaps between neighbours are equal.
//! - `align` without a value aligns to the first referenced element's centre.
//! - `snapToGrid` without a positive value does nothing.
//! - `contain` moves only the elements that fall outside their container.
//! - `attach` takes each anchor's frame centre as its anchor point.
//! - An element this layer cannot bound contributes a degenerate frame at its
//!   transformed origin.

mod frame;

pub use frame::{element_frame, Frame, Point};

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::composition::{self, Affine};
use crate::scene::{
    Axis, Constraint, ConstraintKind, Diagnostic, DiagnosticCode, Diagnostics, Scene,
};

/// A constraint cannot be satisfied as stated.
pub const CONSTRAINT: DiagnosticCode = DiagnosticCode::new("E_CONSTRAINT");

/// Two constraints pin the same element to different positions.
pub const CONFLICT: DiagnosticCode = DiagnosticCode::new("E_CONSTRAINT_CONFLICT");

/// The tolerance below which two coordinates are the same position.
const EPSILON: f64 = 1e-9;

/// One element's resolved placement: where its origin sits after constraints.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// The element's stable identifier.
    pub id: String,
    /// The element's resolved translation in scene units.
    pub translation: Point,
}

/// A connector's resolved endpoints for an `attach` constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    /// The constraint that produced the attachment.
    pub constraint_id: String,
    /// The connector element.
    pub connector_id: String,
    /// The first anchor's resolved point.
    pub from: Point,
    /// The second anchor's resolved point.
    pub to: Point,
}

/// The concrete placements a scene's constraints resolve to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resolution {
    placements: Vec<Placement>,
    attachments: Vec<Attachment>,
}

impl Resolution {
    /// The resolved translation of an element, or `None` when it is not in the
    /// scene.
    pub fn translation(&self, id: &str) -> Option<Point> {
        self.placements
            .iter()
            .find(|placement| placement.id == id)
            .map(|placement| placement.translation)
    }

    /// Every element's resolved placement, in scene order.
    pub fn placements(&self) -> &[Placement] {
        &self.placements
    }

    /// The attachments the scene's connectors form, in constraint order.
    pub fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }
}

/// Resolves a scene's constraints into concrete placements.
///
/// Returns `Err` when a constraint cannot be satisfied or two constraints
/// conflict, with a diagnostic naming the constraint and its location; on
/// success every element carries a resolved translation and every attachment
/// carries its resolved endpoints.
pub fn resolve(scene: &Scene) -> Result<Resolution, Diagnostics> {
    let mut solver = Solver::new(scene);
    for index in 0..solver.constraints.len() {
        let constraint = solver.constraints[index].clone();
        solver.apply(&constraint, index);
    }
    if solver.diagnostics.has_errors() {
        return Err(solver.diagnostics);
    }
    Ok(solver.into_resolution())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Axis1 {
    X,
    Y,
}

impl Axis1 {
    fn index(self) -> usize {
        match self {
            Axis1::X => 0,
            Axis1::Y => 1,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Axis1::X => "x",
            Axis1::Y => "y",
        }
    }
}

struct ElementState {
    id: String,
    base_frame: Frame,
    base_translation: Point,
    translation: Point,
}

impl ElementState {
    fn frame(&self) -> Frame {
        self.base_frame.translated([
            self.translation[0] - self.base_translation[0],
            self.translation[1] - self.base_translation[1],
        ])
    }

    fn center(&self) -> Point {
        self.frame().center()
    }
}

struct Solver<'a> {
    constraints: &'a [Constraint],
    elements: Vec<ElementState>,
    index_of: HashMap<String, usize>,
    pins: HashMap<(usize, Axis1), (f64, usize)>,
    attachments: Vec<Attachment>,
    diagnostics: Diagnostics,
}

impl<'a> Solver<'a> {
    fn new(scene: &'a Scene) -> Self {
        let constraints = scene.constraints.as_deref().unwrap_or(&[]);
        let mut diagnostics = Diagnostics::new();
        let mut elements = Vec::with_capacity(scene.elements.len());
        let mut index_of = HashMap::new();

        for element in &scene.elements {
            let affine = composition::resolve_transform(element, &mut diagnostics)
                .unwrap_or(Affine::IDENTITY);
            let base_translation = [element.transform.translate_x, element.transform.translate_y];
            index_of.insert(element.id.clone(), elements.len());
            elements.push(ElementState {
                id: element.id.clone(),
                base_frame: element_frame(element, affine),
                base_translation,
                translation: base_translation,
            });
        }

        Self {
            constraints,
            elements,
            index_of,
            pins: HashMap::new(),
            attachments: Vec::new(),
            diagnostics,
        }
    }

    fn into_resolution(self) -> Resolution {
        Resolution {
            placements: self
                .elements
                .into_iter()
                .map(|element| Placement {
                    id: element.id,
                    translation: element.translation,
                })
                .collect(),
            attachments: self.attachments,
        }
    }

    fn apply(&mut self, constraint: &Constraint, index: usize) {
        let Some(indices) = self.resolve_ids(constraint, index) else {
            return;
        };
        match constraint.kind {
            ConstraintKind::EqualSpacing => self.equal_spacing(&indices, constraint, index),
            ConstraintKind::Align => self.align(&indices, constraint, index),
            ConstraintKind::Attach => self.attach(&indices, constraint, index),
            ConstraintKind::Contain => self.contain(&indices, constraint, index),
            ConstraintKind::SnapToGrid => self.snap_to_grid(&indices, constraint, index),
        }
    }

    fn resolve_ids(&mut self, constraint: &Constraint, index: usize) -> Option<Vec<usize>> {
        let mut indices = Vec::with_capacity(constraint.element_ids.len());
        let mut missing = None;
        for id in &constraint.element_ids {
            match self.index_of.get(id) {
                Some(&element) => indices.push(element),
                None => {
                    missing = Some(id.clone());
                    break;
                }
            }
        }
        if let Some(id) = missing {
            self.reject(
                index,
                constraint,
                format!("references unknown element `{id}`"),
            );
            return None;
        }
        if indices.len() < 2 {
            self.reject(
                index,
                constraint,
                "must reference at least two elements".to_string(),
            );
            return None;
        }
        Some(indices)
    }

    /// Pins an element's centre on one axis, detecting a conflicting pin.
    fn pin(&mut self, element: usize, axis: Axis1, coordinate: f64, constraint: usize) {
        match self.pins.get(&(element, axis)).copied() {
            Some((existing, other)) if (existing - coordinate).abs() > EPSILON => {
                let element_id = self.elements[element].id.clone();
                let first = self.constraints[other].id.clone();
                let second = self.constraints[constraint].id.clone();
                self.diagnostics.push(
                    Diagnostic::error(
                        CONFLICT,
                        format!(
                            "constraints `{first}` and `{second}` conflict: element `{element_id}` is pinned to different positions on the {} axis",
                            axis.name()
                        ),
                    )
                    .at_path(format!("/constraints/{constraint}")),
                );
            }
            Some(_) => {}
            None => {
                self.pins.insert((element, axis), (coordinate, constraint));
                let center = self.elements[element].center()[axis.index()];
                self.elements[element].translation[axis.index()] += coordinate - center;
            }
        }
    }

    fn equal_spacing(&mut self, indices: &[usize], constraint: &Constraint, index: usize) {
        let axis = axis1_of(constraint.axis, Axis1::X);

        let mut order = indices.to_vec();
        order.sort_by(|&a, &b| {
            let left = self.elements[a].center()[axis.index()];
            let right = self.elements[b].center()[axis.index()];
            left.partial_cmp(&right)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.cmp(&b))
        });

        let count = order.len();
        let gap = match constraint.value {
            Some(value) if !value.is_finite() || value < 0.0 => {
                self.reject(
                    index,
                    constraint,
                    format!("has an invalid spacing `{value}`"),
                );
                return;
            }
            Some(value) => value,
            None => {
                let first = self.elements[order[0]].frame();
                let last = self.elements[order[count - 1]].frame();
                let span = last.max[axis.index()] - first.min[axis.index()];
                let extents: f64 = order
                    .iter()
                    .map(|&element| self.elements[element].frame().extent(axis.index()))
                    .sum();
                (span - extents) / (count as f64 - 1.0)
            }
        };

        for position in 1..count {
            let previous = order[position - 1];
            let current = order[position];
            let previous_max = self.elements[previous].frame().max[axis.index()];
            let current_extent = self.elements[current].frame().extent(axis.index());
            let target_center = previous_max + gap + current_extent / 2.0;
            self.pin(current, axis, target_center, index);
        }
    }

    fn align(&mut self, indices: &[usize], constraint: &Constraint, index: usize) {
        let axes = axes_of(constraint.axis, Axis::Both);
        let reference = self.elements[indices[0]].center();
        for &element in &indices[1..] {
            for &axis in &axes {
                let target = constraint.value.unwrap_or(reference[axis.index()]);
                self.pin(element, axis, target, index);
            }
        }
    }

    fn attach(&mut self, indices: &[usize], constraint: &Constraint, index: usize) {
        let connector = indices[0];
        let anchors = &indices[1..];
        if anchors.len() < 2 {
            self.reject(
                index,
                constraint,
                format!(
                    "attaches `{}` but needs two anchor elements",
                    self.elements[connector].id
                ),
            );
            return;
        }
        let from = self.elements[anchors[0]].center();
        let to = self.elements[anchors[1]].center();
        self.attachments.push(Attachment {
            constraint_id: constraint.id.clone(),
            connector_id: self.elements[connector].id.clone(),
            from,
            to,
        });
        for axis in axes_of(constraint.axis, Axis::Both) {
            self.pin(connector, axis, from[axis.index()], index);
        }
    }

    fn contain(&mut self, indices: &[usize], constraint: &Constraint, index: usize) {
        let container = indices[0];
        let contained = &indices[1..];
        if contained.is_empty() {
            self.reject(
                index,
                constraint,
                "must name a container and at least one contained element".to_string(),
            );
            return;
        }
        let container_frame = self.elements[container].frame();
        for axis in axes_of(constraint.axis, Axis::Both) {
            let minimum = container_frame.min[axis.index()];
            let maximum = container_frame.max[axis.index()];
            for &element in contained {
                let frame = self.elements[element].frame();
                let extent = frame.extent(axis.index());
                if extent > (maximum - minimum) + EPSILON {
                    self.reject(
                        index,
                        constraint,
                        format!(
                            "cannot contain element `{}`: it is larger than the container",
                            self.elements[element].id
                        ),
                    );
                    continue;
                }
                let center = frame.center()[axis.index()];
                let target = center.clamp(minimum + extent / 2.0, maximum - extent / 2.0);
                if (target - center).abs() > EPSILON {
                    self.pin(element, axis, target, index);
                }
            }
        }
    }

    fn snap_to_grid(&mut self, indices: &[usize], constraint: &Constraint, index: usize) {
        let Some(grid) = constraint.value else {
            return;
        };
        if !grid.is_finite() || grid <= 0.0 {
            return;
        }
        for &element in indices {
            for axis in axes_of(constraint.axis, Axis::Both) {
                let coordinate = self.elements[element].center()[axis.index()];
                let target = (coordinate / grid).round() * grid;
                self.pin(element, axis, target, index);
            }
        }
    }

    fn reject(&mut self, index: usize, constraint: &Constraint, reason: String) {
        self.diagnostics.push(
            Diagnostic::error(
                CONSTRAINT,
                format!("constraint `{}` {reason}", constraint.id),
            )
            .at_path(format!("/constraints/{index}")),
        );
    }
}

fn axes_of(axis: Option<Axis>, default: Axis) -> Vec<Axis1> {
    match axis.unwrap_or(default) {
        Axis::X => vec![Axis1::X],
        Axis::Y => vec![Axis1::Y],
        Axis::Both => vec![Axis1::X, Axis1::Y],
    }
}

fn axis1_of(axis: Option<Axis>, default: Axis1) -> Axis1 {
    match axis {
        Some(Axis::X) => Axis1::X,
        Some(Axis::Y) => Axis1::Y,
        _ => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::parse as parse_scene;

    fn rect(id: &str, order: usize, x: f64, y: f64, width: f64, height: f64) -> String {
        format!(
            r#"{{"id":"{id}","sceneId":"s","order":{order},"kind":"rect","geometry":{{"x":0,"y":0,"width":{width},"height":{height}}},"transform":{{"translateX":{x},"translateY":{y},"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}"#
        )
    }

    fn line(id: &str, order: usize, points: &str) -> String {
        format!(
            r#"{{"id":"{id}","sceneId":"s","order":{order},"kind":"line","geometry":{{"points":{points}}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}"#
        )
    }

    fn scene(elements: &[String], constraints: &str) -> Scene {
        let constraints = if constraints.is_empty() {
            String::new()
        } else {
            format!(r#","constraints":{constraints}"#)
        };
        parse_scene(&format!(
            r#"{{"id":"s","projectId":"p","name":"S","formatVersion":"0.1","canvas":{{"width":400,"height":400,"background":"transparent"}},"elements":[{}]{constraints}}}"#,
            elements.join(",")
        ))
        .expect("valid scene")
    }

    fn close(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-6
    }

    #[test]
    fn equal_spacing_distributes_equal_gaps() {
        let scene = scene(
            &[
                rect("a", 0, 0.0, 0.0, 10.0, 10.0),
                rect("b", 1, 50.0, 0.0, 10.0, 10.0),
                rect("c", 2, 200.0, 0.0, 10.0, 10.0),
            ],
            r#"[{"id":"sp","sceneId":"s","kind":"equalSpacing","elementIds":["a","b","c"],"axis":"x"}]"#,
        );
        let resolution = resolve(&scene).expect("resolved");

        let a = resolution.translation("a").unwrap()[0];
        let b = resolution.translation("b").unwrap()[0];
        let c = resolution.translation("c").unwrap()[0];
        assert!(close(a, 0.0), "first stays put: {a}");
        assert!(close(c, 200.0), "last stays put: {c}");
        let first_gap = b - (a + 10.0);
        let second_gap = c - (b + 10.0);
        assert!(close(first_gap, second_gap), "{first_gap} vs {second_gap}");
    }

    #[test]
    fn equal_spacing_honours_an_explicit_value() {
        let scene = scene(
            &[
                rect("a", 0, 0.0, 0.0, 10.0, 10.0),
                rect("b", 1, 50.0, 0.0, 10.0, 10.0),
                rect("c", 2, 200.0, 0.0, 10.0, 10.0),
            ],
            r#"[{"id":"sp","sceneId":"s","kind":"equalSpacing","elementIds":["a","b","c"],"axis":"x","value":20}]"#,
        );
        let resolution = resolve(&scene).expect("resolved");
        assert!(close(resolution.translation("b").unwrap()[0], 30.0));
        assert!(close(resolution.translation("c").unwrap()[0], 60.0));
    }

    #[test]
    fn a_connector_attaches_to_two_anchors() {
        let scene = scene(
            &[
                line("wire", 0, "[[0,0],[10,0]]"),
                rect("left", 1, 0.0, 0.0, 20.0, 20.0),
                rect("right", 2, 100.0, 40.0, 20.0, 20.0),
            ],
            r#"[{"id":"link","sceneId":"s","kind":"attach","elementIds":["wire","left","right"]}]"#,
        );
        let resolution = resolve(&scene).expect("resolved");

        let attachment = resolution.attachments().first().expect("an attachment");
        assert_eq!(attachment.connector_id, "wire");
        assert_eq!(attachment.from, [10.0, 10.0]);
        assert_eq!(attachment.to, [110.0, 50.0]);
        // The connector's centre is placed on its first anchor.
        let wire = resolution.translation("wire").unwrap();
        assert!(close(wire[0], 5.0) && close(wire[1], 10.0), "{wire:?}");
    }

    #[test]
    fn conflicting_pins_fail_naming_both_constraints() {
        let scene = scene(
            &[
                rect("a", 0, 0.0, 0.0, 10.0, 10.0),
                rect("b", 1, 50.0, 0.0, 10.0, 10.0),
            ],
            r#"[{"id":"c1","sceneId":"s","kind":"align","elementIds":["a","b"],"axis":"x","value":0},{"id":"c2","sceneId":"s","kind":"align","elementIds":["a","b"],"axis":"x","value":50}]"#,
        );
        let diagnostics = resolve(&scene).expect_err("conflict");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, CONFLICT);
        assert!(
            error.message.contains("c1") && error.message.contains("c2"),
            "{}",
            error.message
        );
    }

    #[test]
    fn pinning_within_tolerance_does_not_conflict() {
        let scene = scene(
            &[
                rect("a", 0, 0.0, 0.0, 10.0, 10.0),
                rect("b", 1, 50.0, 0.0, 10.0, 10.0),
            ],
            r#"[{"id":"c1","sceneId":"s","kind":"align","elementIds":["a","b"],"axis":"x","value":0},{"id":"c2","sceneId":"s","kind":"align","elementIds":["a","b"],"axis":"x","value":0.0000000001}]"#,
        );
        assert!(resolve(&scene).is_ok());
    }

    #[test]
    fn an_unknown_element_is_refused() {
        let scene = scene(
            &[
                rect("a", 0, 0.0, 0.0, 10.0, 10.0),
                rect("b", 1, 50.0, 0.0, 10.0, 10.0),
            ],
            r#"[{"id":"c1","sceneId":"s","kind":"align","elementIds":["a","zzz"],"axis":"x"}]"#,
        );
        let diagnostics = resolve(&scene).expect_err("unknown element");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, CONSTRAINT);
        assert!(error.message.contains("zzz"));
        assert!(error.message.contains("c1"));
    }

    #[test]
    fn contain_moves_an_element_inside() {
        let scene = scene(
            &[
                rect("box", 0, 0.0, 0.0, 100.0, 100.0),
                rect("dot", 1, 200.0, 0.0, 10.0, 10.0),
            ],
            r#"[{"id":"in","sceneId":"s","kind":"contain","elementIds":["box","dot"]}]"#,
        );
        let resolution = resolve(&scene).expect("resolved");
        assert!(close(resolution.translation("dot").unwrap()[0], 90.0));
    }

    #[test]
    fn contain_rejects_an_element_larger_than_its_container() {
        let scene = scene(
            &[
                rect("box", 0, 0.0, 0.0, 100.0, 100.0),
                rect("dot", 1, 0.0, 0.0, 200.0, 10.0),
            ],
            r#"[{"id":"in","sceneId":"s","kind":"contain","elementIds":["box","dot"]}]"#,
        );
        let diagnostics = resolve(&scene).expect_err("unsatisfiable");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, CONSTRAINT);
        assert!(error.message.contains("in"));
    }

    #[test]
    fn snap_to_grid_snaps_a_centre() {
        let scene = scene(
            &[rect("a", 0, 13.0, 0.0, 10.0, 10.0)],
            r#"[{"id":"snap","sceneId":"s","kind":"snapToGrid","elementIds":["a","a"],"axis":"x","value":10}]"#,
        );
        let resolution = resolve(&scene).expect("resolved");
        assert!(close(resolution.translation("a").unwrap()[0], 15.0));
    }

    #[test]
    fn snap_without_a_value_is_a_no_op() {
        let scene = scene(
            &[rect("a", 0, 13.0, 0.0, 10.0, 10.0)],
            r#"[{"id":"snap","sceneId":"s","kind":"snapToGrid","elementIds":["a","a"],"axis":"x"}]"#,
        );
        let resolution = resolve(&scene).expect("resolved");
        assert!(close(resolution.translation("a").unwrap()[0], 13.0));
    }

    #[test]
    fn resolution_is_deterministic() {
        let scene = scene(
            &[
                rect("a", 0, 0.0, 0.0, 10.0, 10.0),
                rect("b", 1, 50.0, 0.0, 10.0, 10.0),
                rect("c", 2, 200.0, 0.0, 10.0, 10.0),
            ],
            r#"[{"id":"sp","sceneId":"s","kind":"equalSpacing","elementIds":["a","b","c"],"axis":"x"}]"#,
        );
        assert_eq!(resolve(&scene).unwrap(), resolve(&scene).unwrap());
    }

    #[test]
    fn a_scene_without_constraints_keeps_its_translations() {
        let scene = scene(&[rect("a", 0, 12.0, 34.0, 10.0, 10.0)], "");
        let resolution = resolve(&scene).expect("resolved");
        assert_eq!(resolution.translation("a"), Some([12.0, 34.0]));
        assert!(resolution.attachments().is_empty());
    }
}
