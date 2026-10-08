//! The scene compiler: resolves a validated scene into one render model
//! (FEAT-011).
//!
//! [`compile`] is the library entry point (C-002). It runs the pipeline the
//! architecture names — validate, resolve, style, compile — and returns a
//! [`RenderModel`] whose geometry is concrete and whose references between
//! elements are gone (C-003).
//!
//! Compilation is deterministic (NFR-010): the element tree is walked in a fixed
//! document order, constraints are resolved by the resolver's fixed rules, and
//! no unseeded randomness or unordered collection feeds the output. A failure is
//! reported as a located diagnostic and nothing is returned, never a partial
//! model (NFR-011).
//!
//! # Element tree and composition
//!
//! Elements form a tree through `parentId`; a group or composition element acts
//! on its children. A `group` passes its transform down to its children, a
//! `repeat` emits one subtree per copy, an `alongPath` places its children along
//! a guide, a `projection` maps its children onto an axis, a `boolean` combines
//! its children, and an `offset` grows or shrinks its children. Every other
//! element contributes one node. A `group` is a container that contributes no
//! node of its own: its identity and name travel on every descendant node as an
//! ancestor chain, outermost first, so an exporter can rebuild the named-group
//! nesting (FEAT-011, FEAT-012). The one feature the compiler does not yet
//! accept — a plain `raster` layer, which ships behind the raster-layers flag
//! (FEAT-015) — is refused by name.
//!
//! # Paint
//!
//! An element's fill and stroke each carry a paint that names a palette token or
//! a gradient (D-021). [`compile`] runs without style assets, so a token paint
//! carries its declared name and a gradient paint is dropped with a warning;
//! callers that hold a project's palette, gradients and stroke profiles use
//! [`compile_with_style`] to resolve every paint to a concrete colour or a
//! gradient whose stop colours are concrete, and to report a reference that does
//! not resolve. No unresolved colour or gradient reference survives compilation
//! (C-003, FEAT-005, FEAT-027).
//!
//! # Recipes
//!
//! A scene may render in a style recipe; when the context supplies one,
//! [`compile_with_style`] applies it and names it in the model's `meta.recipe`.
//! The flat recipe (FEAT-007) draws even, solid palette fills and no texture,
//! honoring an element's explicit gradient; a recipe that asks for a look the
//! language cannot express is reported rather than silently approximated.
//! The line-art recipe (FEAT-008) fixes a consistent stroke weight — its
//! declared `strokeWeight` fills a stroke whose profile leaves the weight
//! unset, while a profile that states a width is honored as an explicit varying
//! weight — takes each stroke's paint from the palette, clamps a weight below
//! the minimum renderable unit, and reports a scene that draws no strokes.
//! The geometric recipe (FEAT-009) aligns elements to a grid: each element's
//! resolved position snaps to the nearest grid intersection, a polygon's
//! vertices snap with it so a set of polygons is constructed on shared grid
//! points, and an element that sat off the grid is reported. A freeform curve
//! is kept as an explicit exception and reported, and a grid finer than the
//! renderable resolution is reported as a performance concern.
//!
//! # Fonts
//!
//! A text element's font reference resolves against the caller's font assets and
//! travels in the model's font table, so an exporter shapes glyphs with no
//! external state (FEAT-024). A text element that names no font resolves to
//! [`DEFAULT_FONT_ID`]; a font the context does not carry is a located error
//! naming it. The caller's fallback asset, supplied under `fallback`, is carried
//! into the table too, so the font manager substitutes a glyph the resolved font
//! lacks rather than drawing a blank box (D-018, FEAT-024).

use std::collections::{HashMap, HashSet};

use crate::composition::{
    self, along_path_placements, combine, flatten_shape, offset_shape,
    placements as repeat_placements, Affine,
};
use crate::constraints::{self, Point, Resolution};
use crate::fonts::FALLBACK_FONT_ID;
use crate::primitives::{
    self, parse as parse_path, Line, Path as PathGeometry, Polygon, Segment, Shape, SubPath,
};
use crate::render::{
    NodeGroup, NodePaint, NodeStroke, RenderCanvas, RenderMeta, RenderModel, ResolvedFont,
    ResolvedNode, TextRun,
};
use crate::scene::{
    validate, BooleanOperation, Diagnostic, DiagnosticCode, Diagnostics, ElementKind, Location,
    Scene, TextAlign,
};
use crate::style::{self, Gradient, Palette, StrokeProfile, StyleRecipe, UNDEFINED_STROKE};

/// Two elements reference each other, directly or through a chain.
pub const CYCLE: DiagnosticCode = DiagnosticCode::new("E_CYCLE");

/// An element references a parent the scene does not define.
pub const REFERENCE: DiagnosticCode = DiagnosticCode::new("E_REFERENCE");

/// An element uses a feature the compiler does not implement.
pub const UNSUPPORTED: DiagnosticCode = DiagnosticCode::new("E_UNSUPPORTED");

/// A stroke profile reference could not be resolved for lack of style assets.
pub const UNRESOLVED_STROKE: DiagnosticCode = DiagnosticCode::new("W_UNRESOLVED_STROKE");

/// A font reference could not be resolved: the font is missing from the style
/// context, or a text element names none and no default is available
/// (FEAT-024).
pub const FONT: DiagnosticCode = DiagnosticCode::new("E_FONT");

/// A font reference could not be resolved for lack of style assets.
pub const UNRESOLVED_FONT: DiagnosticCode = DiagnosticCode::new("W_UNRESOLVED_FONT");

/// The font asset id a text element resolves to when it names none: the
/// caller's open-licensed default font (FEAT-024).
///
/// A text element's `fontId` is `null` to select the default; the compiler
/// resolves that to the font asset the caller supplies under this id, so a
/// caller with style assets always provides its default font explicitly.
pub const DEFAULT_FONT_ID: &str = "default";

/// A scene above the documented large-scene element count.
pub const LARGE_SCENE: DiagnosticCode = DiagnosticCode::new("W_LARGE_SCENE");

/// The element count above which a scene is processed with a warning (NFR-002).
pub const LARGE_SCENE_ELEMENTS: usize = 50_000;

/// The most render nodes one compilation may emit; beyond it the scene is
/// refused with a defined size limit rather than allowed to grow without bound
/// (NFR-021). A composition that expands (a large repeat) is bounded here.
pub const MAX_RENDER_NODES: usize = 1_000_000;

/// The palette and stroke profiles a scene's style references resolve against.
///
/// A caller without style assets compiles without a context, and paint carries
/// the scene's declared references instead of resolved values.
#[derive(Debug, Clone, Copy, Default)]
pub struct StyleContext<'a> {
    /// The palette the scene's fill tokens resolve against.
    pub palette: Option<&'a Palette>,
    /// The stroke profiles the scene's elements resolve against.
    pub strokes: &'a [StrokeProfile],
    /// The gradients the scene's gradient paints resolve against (FEAT-027).
    pub gradients: &'a [Gradient],
    /// The font assets the scene's text elements resolve against, each an id, a
    /// name and its bytes. The default open-licensed font is supplied under
    /// [`DEFAULT_FONT_ID`], since a text element that names no font resolves to
    /// it (FEAT-024).
    pub fonts: &'a [FontAsset],
    /// The style recipe the scene renders in, when one is supplied (C-002).
    ///
    /// A flat recipe resolves to even, solid palette fills with no texture,
    /// honoring an element's explicit gradient as its one exception (FEAT-007).
    pub recipe: Option<&'a StyleRecipe>,
}

/// A font asset a caller supplies so the compiler can resolve a text element's
/// `fontId` without filesystem access (C-002, FEAT-024).
///
/// The caller loads the font file (the bundled open-licensed default or a font
/// the user supplied) and passes its bytes here; the compiler carries the
/// resolved font into the render model so an exporter finalizes glyph geometry
/// with no external state.
#[derive(Debug, Clone, PartialEq)]
pub struct FontAsset {
    /// The font's stable identifier, matched against a text element's `fontId`.
    pub id: String,
    /// The font's human-readable name.
    pub name: String,
    /// The font file's bytes.
    pub data: Vec<u8>,
}

impl FontAsset {
    /// Builds a font asset from its parts.
    pub fn new(id: impl Into<String>, name: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            data,
        }
    }

    /// The font as the render model carries it.
    pub fn resolved(&self) -> ResolvedFont {
        ResolvedFont {
            id: self.id.clone(),
            name: self.name.clone(),
            data: self.data.clone(),
        }
    }
}

/// Compiles a validated scene into its render model (C-002).
///
/// Runs without style assets, so paint carries the scene's declared references;
/// use [`compile_with_style`] to resolve them against a project's palette and
/// stroke profiles.
pub fn compile(scene: &Scene) -> Result<RenderModel, Diagnostics> {
    compile_with_style(scene, &StyleContext::default())
}

/// Compiles a scene, resolving its style references against the given context.
///
/// A fill token or stroke profile the context does not provide is a located
/// error naming it; a scene with neither is unaffected.
pub fn compile_with_style<'s>(
    scene: &Scene,
    style: &'s StyleContext<'s>,
) -> Result<RenderModel, Diagnostics> {
    let mut diagnostics = validate(scene);
    if diagnostics.has_errors() {
        return Err(diagnostics);
    }

    if scene.elements.len() > LARGE_SCENE_ELEMENTS {
        diagnostics.push(Diagnostic::warning(
            LARGE_SCENE,
            format!(
                "scene has {} elements, above the {LARGE_SCENE_ELEMENTS}-element limit; compilation continues",
                scene.elements.len()
            ),
        ));
    }

    let resolution = match constraints::resolve(scene) {
        Ok(resolution) => resolution,
        Err(errors) => {
            diagnostics.extend(errors);
            return Err(diagnostics);
        }
    };

    let attachments = resolution
        .attachments()
        .iter()
        .map(|attachment| {
            (
                attachment.connector_id.clone(),
                (attachment.from, attachment.to),
            )
        })
        .collect();

    let mut compiler = Compiler {
        scene,
        style,
        resolution,
        attachments,
        diagnostics,
        nodes: Vec::new(),
        used_ids: HashSet::new(),
        fonts: Vec::new(),
        font_ids: HashSet::new(),
        order: 0,
        limit_hit: false,
        parent: Vec::new(),
        children: Vec::new(),
        snapped: HashMap::new(),
    };

    compiler.prepare();
    if compiler.diagnostics.has_errors() {
        return Err(compiler.diagnostics);
    }
    compiler.plan_geometric_snap();
    compiler.run();
    compiler.carry_fallback();

    // A gradient no element references is a warning, not an error (FEAT-027).
    if !style.gradients.is_empty() {
        compiler
            .diagnostics
            .extend(style::validate_gradient_usage(scene, style.gradients));
    }

    // A recipe that asks for a look the language cannot express is reported,
    // never silently approximated (FEAT-007, NFR-011).
    if let Some(recipe) = style.recipe {
        compiler
            .diagnostics
            .extend(style::check_recipe_expressible(recipe));
        // A grid finer than the renderable resolution is a performance concern,
        // reported rather than silently snapped (FEAT-009).
        compiler
            .diagnostics
            .extend(style::check_recipe_grid(recipe));
        // A line-art scene with no stroke has no line work in it; the emptiness
        // is reported rather than passed off as a successful render (FEAT-008).
        let has_strokes = compiler
            .nodes
            .iter()
            .any(|node| node.paint.stroke.is_some());
        compiler
            .diagnostics
            .extend(style::check_recipe_line_art(recipe, has_strokes));
    }

    let model = compiler.into_model();
    if model.diagnostics.has_errors() {
        Err(model.diagnostics)
    } else {
        Ok(model)
    }
}

/// A stable suffix identifying one copy within a composition expansion.
struct CopyTag {
    path: String,
}

impl CopyTag {
    fn child(parent: Option<&CopyTag>, owner: &str, index: usize) -> Self {
        let path = match parent {
            Some(parent) => format!("{}/{}#{}", parent.path, owner, index),
            None => format!("{owner}#{index}"),
        };
        Self { path }
    }
}

struct Compiler<'a, 's> {
    scene: &'a Scene,
    style: &'s StyleContext<'s>,
    resolution: Resolution,
    attachments: HashMap<String, (Point, Point)>,
    diagnostics: Diagnostics,
    nodes: Vec<ResolvedNode>,
    used_ids: HashSet<String>,
    fonts: Vec<ResolvedFont>,
    font_ids: HashSet<String>,
    order: usize,
    limit_hit: bool,
    parent: Vec<Option<usize>>,
    children: Vec<Vec<usize>>,
    /// The geometric recipe's planned grid positions, one per element, keyed by
    /// element id; empty when no grid applies (FEAT-009).
    snapped: HashMap<String, Point>,
}

impl Compiler<'_, '_> {
    /// Builds the parent/child tree and refuses a dangling parent or a cycle.
    fn prepare(&mut self) {
        let count = self.scene.elements.len();
        let mut index_of: HashMap<&str, usize> = HashMap::with_capacity(count);
        for (index, element) in self.scene.elements.iter().enumerate() {
            index_of.insert(element.id.as_str(), index);
        }

        let mut parent = vec![None; count];
        let mut children = vec![Vec::new(); count];
        for (index, element) in self.scene.elements.iter().enumerate() {
            let Some(parent_id) = element.parent_id.as_deref() else {
                continue;
            };
            match index_of.get(parent_id).copied() {
                Some(ancestor) => {
                    parent[index] = Some(ancestor);
                    children[ancestor].push(index);
                }
                None => self.push_reference_error(index, parent_id),
            }
        }

        if let Some(cycle) = detect_cycle(&parent) {
            self.push_cycle_error(&cycle);
        }

        for list in children.iter_mut() {
            list.sort_by(|&left, &right| {
                self.scene.elements[left]
                    .order
                    .cmp(&self.scene.elements[right].order)
                    .then(left.cmp(&right))
            });
        }

        self.parent = parent;
        self.children = children;
    }

    fn run(&mut self) {
        let mut roots: Vec<usize> = (0..self.scene.elements.len())
            .filter(|&index| self.parent[index].is_none())
            .collect();
        roots.sort_by(|&left, &right| {
            self.scene.elements[left]
                .order
                .cmp(&self.scene.elements[right].order)
                .then(left.cmp(&right))
        });
        for root in roots {
            if self.limit_hit {
                break;
            }
            self.emit(root, Affine::IDENTITY, 1.0, true, None, &[]);
        }
    }

    /// Emits the subtree rooted at one element.
    ///
    /// `groups` is the ancestor group chain of the element being emitted,
    /// outermost first; a `group` element extends it for its own descendants so
    /// every node below it carries the group's identity (C-003, FEAT-011).
    fn emit(
        &mut self,
        index: usize,
        parent_world: Affine,
        parent_opacity: f64,
        parent_visible: bool,
        copy: Option<&CopyTag>,
        groups: &[NodeGroup],
    ) {
        if self.limit_hit {
            return;
        }

        let kind = self.scene.elements[index].kind;
        let Some(local) = self.local_affine(index) else {
            return;
        };
        let world = parent_world.then(local);
        let opacity = parent_opacity * self.scene.elements[index].opacity;
        let visible = parent_visible && self.scene.elements[index].visible;

        // A group contributes no node; it only deepens the chain its descendants
        // carry. Every other kind passes the chain through unchanged.
        let mut descendant_groups = groups.to_vec();
        if kind == ElementKind::Group {
            let element = &self.scene.elements[index];
            descendant_groups.push(NodeGroup {
                id: element.id.clone(),
                name: element.name.clone(),
            });
        }

        match kind {
            ElementKind::Raster => self.reject_unsupported(index, "raster"),
            ElementKind::Rect
            | ElementKind::Ellipse
            | ElementKind::Polygon
            | ElementKind::Line
            | ElementKind::Path => {
                if let Some((from, to)) = self.connector(index) {
                    let geometry = Shape::Line(Line {
                        points: vec![from, to],
                    });
                    self.push_node(
                        index,
                        geometry,
                        Affine::IDENTITY,
                        opacity,
                        visible,
                        copy,
                        groups,
                    );
                    return;
                }
                let mut findings = Diagnostics::new();
                let shape = primitives::resolve(&self.scene.elements[index], &mut findings);
                let failed = findings.has_errors();
                self.diagnostics.extend(findings);
                if !failed {
                    if let Some(shape) = shape {
                        self.push_node(index, shape, world, opacity, visible, copy, groups);
                    }
                }
                self.emit_children(index, world, opacity, visible, copy, &descendant_groups);
            }
            ElementKind::Group => {
                self.emit_children(index, world, opacity, visible, copy, &descendant_groups)
            }
            ElementKind::Text => {
                self.emit_text(index, world, opacity, visible, copy, groups);
            }
            ElementKind::Repeat => {
                let mut findings = Diagnostics::new();
                let count = self.scene.elements[index].geometry.count.unwrap_or(0);
                if !self.guard_count(index, count) {
                    return;
                }
                let placements = repeat_placements(&self.scene.elements[index], &mut findings);
                self.diagnostics.extend(findings);
                let owner = self.scene.elements[index].id.clone();
                for (copy_index, placement) in placements.iter().enumerate() {
                    if self.limit_hit {
                        break;
                    }
                    let tag = CopyTag::child(copy, &owner, copy_index);
                    self.emit_children(
                        index,
                        world.then(*placement),
                        opacity,
                        visible,
                        Some(&tag),
                        &descendant_groups,
                    );
                }
            }
            ElementKind::AlongPath => {
                let Some(guide) = self.guide_path(index) else {
                    return;
                };
                let count = self.scene.elements[index].geometry.count.unwrap_or(0);
                if !self.guard_count(index, count) {
                    return;
                }
                let mut findings = Diagnostics::new();
                let placements = along_path_placements(
                    &guide,
                    count,
                    &self.scene.elements[index],
                    &mut findings,
                );
                self.diagnostics.extend(findings);
                let owner = self.scene.elements[index].id.clone();
                for (copy_index, placement) in placements.iter().enumerate() {
                    if self.limit_hit {
                        break;
                    }
                    let tag = CopyTag::child(copy, &owner, copy_index);
                    self.emit_children(
                        index,
                        world.then(*placement),
                        opacity,
                        visible,
                        Some(&tag),
                        &descendant_groups,
                    );
                }
            }
            ElementKind::Projection => {
                let Some(axis) = self.scene.elements[index].geometry.axis else {
                    self.reject_composition(index, "must declare a projection axis");
                    return;
                };
                let projected = world.then(composition::projection_for(axis));
                self.emit_children(index, projected, opacity, visible, copy, &descendant_groups);
            }
            ElementKind::Boolean => {
                let Some(operation) = self.scene.elements[index].geometry.operation else {
                    self.reject_composition(index, "must declare a boolean operation");
                    return;
                };
                let shapes = self.lower_children(index);
                let mut findings = Diagnostics::new();
                let combined = combine(
                    operation,
                    &shapes,
                    &self.scene.elements[index],
                    &mut findings,
                );
                self.diagnostics.extend(findings);
                if let Some(shape) = combined {
                    self.push_node(index, shape, world, opacity, visible, copy, groups);
                }
            }
            ElementKind::Offset => {
                let distance = self.scene.elements[index].geometry.distance.unwrap_or(0.0);
                let children = self.children[index].clone();
                for child in children {
                    if self.limit_hit {
                        break;
                    }
                    let Some(shape) = self.lower(child) else {
                        continue;
                    };
                    let mut findings = Diagnostics::new();
                    let offset =
                        offset_shape(&shape, distance, &self.scene.elements[index], &mut findings);
                    self.diagnostics.extend(findings);
                    if let Some(shape) = offset {
                        self.push_node(index, shape, world, opacity, visible, copy, groups);
                    }
                }
            }
        }
    }

    /// Lowers a composition element to a single concrete shape in its parent's
    /// coordinates, used where an element is an operand rather than a drawing.
    fn lower(&mut self, index: usize) -> Option<Shape> {
        if self.limit_hit {
            return None;
        }
        let kind = self.scene.elements[index].kind;
        let local = self.local_affine(index)?;
        match kind {
            ElementKind::Raster => {
                self.reject_unsupported(index, "raster");
                None
            }
            ElementKind::Text => {
                self.reject_unsupported(index, "text as a composition operand");
                None
            }
            ElementKind::Rect
            | ElementKind::Ellipse
            | ElementKind::Polygon
            | ElementKind::Line
            | ElementKind::Path => {
                let mut findings = Diagnostics::new();
                let shape = primitives::resolve(&self.scene.elements[index], &mut findings);
                let failed = findings.has_errors();
                self.diagnostics.extend(findings);
                if failed {
                    return None;
                }
                self.bake(shape?, local)
            }
            ElementKind::Group => {
                let shapes = self.lower_children(index);
                let combined = self.union_all(shapes, index)?;
                self.bake(combined, local)
            }
            ElementKind::Projection => {
                let Some(axis) = self.scene.elements[index].geometry.axis else {
                    self.reject_composition(index, "must declare a projection axis");
                    return None;
                };
                let shapes = self.lower_children(index);
                let combined = self.union_all(shapes, index)?;
                let projection = composition::projection_for(axis);
                self.bake(combined, local.then(projection))
            }
            ElementKind::Repeat => {
                let mut findings = Diagnostics::new();
                let count = self.scene.elements[index].geometry.count.unwrap_or(0);
                if !self.guard_count(index, count) {
                    return None;
                }
                let placements = repeat_placements(&self.scene.elements[index], &mut findings);
                self.diagnostics.extend(findings);
                let children = self.children[index].clone();
                let mut shapes = Vec::new();
                for placement in &placements {
                    for &child in &children {
                        if let Some(shape) = self.lower(child) {
                            if let Some(baked) = self.bake(shape, *placement) {
                                shapes.push(baked);
                            }
                        }
                    }
                }
                let combined = self.union_all(shapes, index)?;
                self.bake(combined, local)
            }
            ElementKind::AlongPath => {
                let guide = self.guide_path(index)?;
                let count = self.scene.elements[index].geometry.count.unwrap_or(0);
                if !self.guard_count(index, count) {
                    return None;
                }
                let mut findings = Diagnostics::new();
                let placements = along_path_placements(
                    &guide,
                    count,
                    &self.scene.elements[index],
                    &mut findings,
                );
                self.diagnostics.extend(findings);
                let children = self.children[index].clone();
                let mut shapes = Vec::new();
                for placement in &placements {
                    for &child in &children {
                        if let Some(shape) = self.lower(child) {
                            if let Some(baked) = self.bake(shape, *placement) {
                                shapes.push(baked);
                            }
                        }
                    }
                }
                let combined = self.union_all(shapes, index)?;
                self.bake(combined, local)
            }
            ElementKind::Offset => {
                let distance = self.scene.elements[index].geometry.distance.unwrap_or(0.0);
                let children = self.children[index].clone();
                let mut shapes = Vec::new();
                for child in children {
                    let Some(shape) = self.lower(child) else {
                        continue;
                    };
                    let mut findings = Diagnostics::new();
                    let offset =
                        offset_shape(&shape, distance, &self.scene.elements[index], &mut findings);
                    self.diagnostics.extend(findings);
                    if let Some(shape) = offset {
                        if let Some(baked) = self.bake(shape, Affine::IDENTITY) {
                            shapes.push(baked);
                        }
                    }
                }
                let combined = self.union_all(shapes, index)?;
                self.bake(combined, local)
            }
            ElementKind::Boolean => {
                let Some(operation) = self.scene.elements[index].geometry.operation else {
                    self.reject_composition(index, "must declare a boolean operation");
                    return None;
                };
                let shapes = self.lower_children(index);
                let mut findings = Diagnostics::new();
                let combined = combine(
                    operation,
                    &shapes,
                    &self.scene.elements[index],
                    &mut findings,
                );
                self.diagnostics.extend(findings);
                self.bake(combined?, local)
            }
        }
    }

    /// Emits each child of an element with the given resolved world transform.
    fn emit_children(
        &mut self,
        index: usize,
        world: Affine,
        opacity: f64,
        visible: bool,
        copy: Option<&CopyTag>,
        groups: &[NodeGroup],
    ) {
        let children = self.children[index].clone();
        for child in children {
            if self.limit_hit {
                break;
            }
            self.emit(child, world, opacity, visible, copy, groups);
        }
    }

    /// Lowers each child of an element to a shape.
    fn lower_children(&mut self, index: usize) -> Vec<Shape> {
        let children = self.children[index].clone();
        let mut shapes = Vec::with_capacity(children.len());
        for child in children {
            if self.limit_hit {
                break;
            }
            if let Some(shape) = self.lower(child) {
                shapes.push(shape);
            }
        }
        shapes
    }

    /// Unions a list of shapes, treating a single shape as itself and an empty
    /// list as nothing.
    fn union_all(&mut self, mut shapes: Vec<Shape>, index: usize) -> Option<Shape> {
        match shapes.len() {
            0 => None,
            1 => shapes.pop(),
            _ => {
                let mut findings = Diagnostics::new();
                let combined = combine(
                    BooleanOperation::Union,
                    &shapes,
                    &self.scene.elements[index],
                    &mut findings,
                );
                self.diagnostics.extend(findings);
                combined
            }
        }
    }

    /// Bakes a transform into a shape, flattening it to concrete contours.
    ///
    /// Used only for operand math (booleans, offsets), where the result is
    /// flattened again anyway; the identity transform keeps the exact shape.
    fn bake(&self, shape: Shape, affine: Affine) -> Option<Shape> {
        if affine == Affine::IDENTITY {
            return Some(shape);
        }
        let mut subpaths = Vec::new();
        for contour in flatten_shape(&shape) {
            if contour.len() < 3 {
                continue;
            }
            let points: Vec<[f64; 2]> = contour.iter().map(|point| affine.apply(*point)).collect();
            let start = points[0];
            let segments = points
                .windows(2)
                .map(|pair| Segment::Line { to: pair[1] })
                .collect();
            subpaths.push(SubPath {
                start,
                segments,
                closed: true,
            });
        }
        if subpaths.is_empty() {
            None
        } else {
            Some(Shape::Path(PathGeometry { subpaths }))
        }
    }

    /// The element's resolved local transform: its declared transform with the
    /// constraint-resolved translation.
    fn local_affine(&mut self, index: usize) -> Option<Affine> {
        let (id, mut transform, declared) = {
            let element = &self.scene.elements[index];
            (
                element.id.clone(),
                element.transform.clone(),
                [element.transform.translate_x, element.transform.translate_y],
            )
        };
        let translation = self
            .snapped
            .get(&id)
            .copied()
            .or_else(|| self.resolution.translation(&id))
            .unwrap_or(declared);
        transform.translate_x = translation[0];
        transform.translate_y = translation[1];
        match Affine::from_scene(&transform) {
            Ok(affine) if affine.is_finite() => Some(affine),
            _ => {
                self.diagnostics.push(
                    Diagnostic::error(
                        composition::TRANSFORM,
                        format!("element `{id}` has a malformed transform"),
                    )
                    .with_location(Location::element(id)),
                );
                None
            }
        }
    }

    /// Plans the geometric recipe's grid alignment for every element (FEAT-009).
    ///
    /// The recipe aligns elements to its grid: each element's resolved position
    /// snaps to the nearest grid intersection, and an element that sat off the
    /// grid is reported. The plan is computed once, before emission, so a
    /// composition operand that is lowered several times snaps identically and
    /// is reported once.
    fn plan_geometric_snap(&mut self) {
        let Some(recipe) = self.style.recipe else {
            return;
        };
        if !recipe.snaps_to_grid() {
            return;
        }

        let plan: Vec<(String, [f64; 2], bool)> = self
            .scene
            .elements
            .iter()
            .map(|element| {
                let declared = [element.transform.translate_x, element.transform.translate_y];
                let current = self.resolution.translation(&element.id).unwrap_or(declared);
                let (x, moved_x) = recipe.snap_coordinate(current[0]);
                let (y, moved_y) = recipe.snap_coordinate(current[1]);
                (element.id.clone(), [x, y], moved_x || moved_y)
            })
            .collect();

        for (id, translation, moved) in plan {
            if moved {
                self.diagnostics.push(
                    Diagnostic::warning(
                        style::GRID_SNAPPED,
                        format!(
                            "element `{id}` sits off the geometric grid; it was snapped to the nearest grid intersection"
                        ),
                    )
                    .with_location(Location::element_at(id.clone(), "/transform")),
                );
            }
            self.snapped.insert(id, translation);
        }
    }

    /// Snaps constructed geometry to the scene's grid (FEAT-009).
    ///
    /// Under the geometric recipe a polygon's vertices snap to grid
    /// intersections, so a set of polygons is constructed on shared grid points
    /// and their edges line up. Every other shape and every other recipe is
    /// left untouched.
    fn snap_geometry(&self, geometry: Shape) -> Shape {
        let Some(recipe) = self.style.recipe else {
            return geometry;
        };
        if !recipe.snaps_to_grid() {
            return geometry;
        }
        match geometry {
            Shape::Polygon(polygon) => Shape::Polygon(Polygon {
                points: polygon
                    .points
                    .iter()
                    .map(|point| {
                        [
                            recipe.snap_coordinate(point[0]).0,
                            recipe.snap_coordinate(point[1]).0,
                        ]
                    })
                    .collect(),
            }),
            other => other,
        }
    }

    /// Reports a freeform curve kept in a geometric scene (FEAT-009).
    ///
    /// The geometric recipe constructs from straight edges and circles, so a
    /// path that declares a curve is avoided by the look; because the curve is
    /// written explicitly it is kept as an exception and reported, rather than
    /// silently drawn or silently replaced.
    fn check_freeform(&mut self, element_id: &str, geometry: &Shape) {
        let Some(recipe) = self.style.recipe else {
            return;
        };
        if !recipe.is_geometric() || !has_freeform_curve(geometry) {
            return;
        }
        self.diagnostics.push(
            Diagnostic::warning(
                style::FREEFORM_CURVE,
                format!(
                    "element `{element_id}` draws a freeform curve, which the geometric recipe avoids; it is kept as an explicit exception"
                ),
            )
            .with_location(Location::element(element_id)),
        );
    }

    /// Parses an `alongPath` element's guide, or reports why it cannot.
    fn guide_path(&mut self, index: usize) -> Option<PathGeometry> {
        let Some(data) = self.scene.elements[index].geometry.path_data.clone() else {
            self.reject_composition(index, "must declare a guide pathData");
            return None;
        };
        match parse_path(&data) {
            Ok(path) => Some(path),
            Err(error) => {
                self.reject_composition(index, &format!("has invalid guide path data: {error}"));
                None
            }
        }
    }

    /// The connector endpoints for an element, when a constraint makes it one.
    fn connector(&self, index: usize) -> Option<(Point, Point)> {
        self.attachments
            .get(self.scene.elements[index].id.as_str())
            .copied()
    }

    /// Appends one drawing node for an element.
    #[allow(clippy::too_many_arguments)]
    fn push_node(
        &mut self,
        index: usize,
        geometry: Shape,
        transform: Affine,
        opacity: f64,
        visible: bool,
        copy: Option<&CopyTag>,
        groups: &[NodeGroup],
    ) {
        if self.limit_hit {
            return;
        }
        if self.nodes.len() >= MAX_RENDER_NODES {
            self.limit_hit = true;
            self.diagnostics.push(Diagnostic::error(
                DiagnosticCode::SIZE_LIMIT,
                format!("compilation produced more than {MAX_RENDER_NODES} nodes"),
            ));
            return;
        }

        let (base_id, name, kind) = {
            let element = &self.scene.elements[index];
            (
                element.id.clone(),
                element.name.clone(),
                kind_name(element.kind).to_string(),
            )
        };
        let id = self.unique_id(&base_id, copy);
        let geometry = self.snap_geometry(geometry);
        self.check_freeform(&base_id, &geometry);
        let paint = self.paint_for(index);
        let order = self.order;
        self.order += 1;
        self.nodes.push(ResolvedNode {
            id,
            name,
            order,
            kind,
            groups: groups.to_vec(),
            geometry: Some(geometry),
            text: None,
            transform,
            paint,
            opacity,
            visible,
        });
    }

    /// Appends one text node for a text element (C-003, FEAT-011).
    ///
    /// A text node carries its string and resolved layout and no geometry: the
    /// element's anchor is baked into the resolved transform, and the font is
    /// carried into the model so an exporter finalizes glyph geometry with no
    /// external state (FEAT-024).
    #[allow(clippy::too_many_arguments)]
    fn push_text_node(
        &mut self,
        index: usize,
        transform: Affine,
        opacity: f64,
        visible: bool,
        copy: Option<&CopyTag>,
        groups: &[NodeGroup],
    ) {
        if self.limit_hit {
            return;
        }
        if self.nodes.len() >= MAX_RENDER_NODES {
            self.limit_hit = true;
            self.diagnostics.push(Diagnostic::error(
                DiagnosticCode::SIZE_LIMIT,
                format!("compilation produced more than {MAX_RENDER_NODES} nodes"),
            ));
            return;
        }

        let Some(run) = self.text_run(index) else {
            return;
        };
        let (base_id, name) = {
            let element = &self.scene.elements[index];
            (element.id.clone(), element.name.clone())
        };
        let id = self.unique_id(&base_id, copy);
        let paint = self.paint_for(index);
        let order = self.order;
        self.order += 1;
        self.nodes.push(ResolvedNode {
            id,
            name,
            order,
            kind: "text".to_string(),
            groups: groups.to_vec(),
            geometry: None,
            text: Some(run),
            transform,
            paint,
            opacity,
            visible,
        });
    }

    /// Builds a text element's run, resolving its font and layout (C-003).
    fn text_run(&mut self, index: usize) -> Option<TextRun> {
        let element = &self.scene.elements[index];
        let geometry = &element.geometry;
        let value = geometry.text.clone().unwrap_or_default();
        let font_size = geometry.font_size.unwrap_or(0.0);
        let align = geometry.align.unwrap_or(TextAlign::Start);
        let line_height = geometry.line_height.unwrap_or(font_size);
        let letter_spacing = geometry.letter_spacing.unwrap_or(0.0);
        let width = geometry.width;
        let declared = element.font_id.clone();
        let element_id = element.id.clone();

        let font_id = self.resolve_font(&element_id, declared.as_deref());
        Some(TextRun {
            value,
            font_id,
            font_size,
            align,
            line_height,
            letter_spacing,
            width,
        })
    }

    /// Resolves a text element's font against the style context, carrying the
    /// resolved font into the model and returning the id the run names.
    ///
    /// A declared font that the context does not provide is a located error
    /// naming the font (FEAT-024). Without any font assets the declared
    /// reference passes through, mirroring the no-style path, with a warning;
    /// a text element that names none resolves to [`DEFAULT_FONT_ID`].
    fn resolve_font(&mut self, element_id: &str, declared: Option<&str>) -> String {
        let requested = declared.unwrap_or(DEFAULT_FONT_ID);
        match self.style.fonts.iter().find(|font| font.id == requested) {
            Some(font) => {
                self.add_font(font.resolved());
                font.id.clone()
            }
            None if self.style.fonts.is_empty() => {
                if declared.is_some() {
                    self.diagnostics.push(
                        Diagnostic::warning(
                            UNRESOLVED_FONT,
                            format!(
                                "font `{requested}` for element `{element_id}` was not resolved; compile with a style context to apply it"
                            ),
                        )
                        .with_location(Location::element_at(element_id, "/fontId")),
                    );
                }
                requested.to_string()
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error(
                        FONT,
                        format!(
                            "text element `{element_id}` references undefined font `{requested}`"
                        ),
                    )
                    .with_location(Location::element_at(element_id, "/fontId")),
                );
                requested.to_string()
            }
        }
    }

    /// Records a resolved font once, in first-seen order (NFR-010).
    fn add_font(&mut self, font: ResolvedFont) {
        if self.font_ids.insert(font.id.clone()) {
            self.fonts.push(font);
        }
    }

    /// Carries the caller's fallback font into the model's font table when one
    /// is supplied (D-018, FEAT-024).
    ///
    /// The fallback is appended after the fonts the text nodes name and recorded
    /// once, so a glyph the resolved font lacks can be substituted by the font
    /// manager while the table stays deterministic and duplicate-free.
    fn carry_fallback(&mut self) {
        let Some(font) = self
            .style
            .fonts
            .iter()
            .find(|font| font.id == FALLBACK_FONT_ID)
        else {
            return;
        };
        self.add_font(font.resolved());
    }

    /// Emits a text element's node, baking its anchor into the transform.
    #[allow(clippy::too_many_arguments)]
    fn emit_text(
        &mut self,
        index: usize,
        world: Affine,
        opacity: f64,
        visible: bool,
        copy: Option<&CopyTag>,
        groups: &[NodeGroup],
    ) {
        let anchor = {
            let geometry = &self.scene.elements[index].geometry;
            Affine::translate(geometry.x.unwrap_or(0.0), geometry.y.unwrap_or(0.0))
        };
        self.push_text_node(index, world.then(anchor), opacity, visible, copy, groups);
    }

    /// Resolves the paint an element declares (C-002, FEAT-005, FEAT-027).
    ///
    /// A fill and a stroke paint each resolve against the palette and gradients
    /// the context carries; a stroke also needs its profile for width, cap and
    /// join. Without style assets the declared token name passes through and a
    /// gradient is dropped with a warning, mirroring the no-style path; with
    /// assets, a reference that does not resolve is an error and no fallback is
    /// substituted.
    fn paint_for(&mut self, index: usize) -> NodePaint {
        let palette = self.style.palette;
        let gradients = self.style.gradients;
        let fill = {
            let element = &self.scene.elements[index];
            style::resolve_fill(element, palette, gradients, &mut self.diagnostics)
        };
        let stroke = self.stroke_for(index);
        NodePaint { fill, stroke }
    }

    /// Resolves the stroke geometry and paint an element declares.
    fn stroke_for(&mut self, index: usize) -> Option<NodeStroke> {
        let (element_id, profile_id) = {
            let element = &self.scene.elements[index];
            (
                element.id.clone(),
                element
                    .stroke
                    .as_ref()
                    .map(|stroke| stroke.profile_id.clone()),
            )
        };
        let profile_id = profile_id?;

        let geometry = match self
            .style
            .strokes
            .iter()
            .find(|profile| profile.id == profile_id)
        {
            Some(profile) => profile.resolved(),
            None if self.style.strokes.is_empty() => {
                self.diagnostics.push(
                    Diagnostic::warning(
                        UNRESOLVED_STROKE,
                        format!(
                            "stroke profile `{profile_id}` for element `{element_id}` was not resolved; compile with a style context to apply it"
                        ),
                    )
                    .with_location(Location::element_at(&element_id, "/stroke/profileId")),
                );
                return None;
            }
            None => {
                self.diagnostics.push(
                    Diagnostic::error(
                        UNDEFINED_STROKE,
                        format!(
                            "element `{element_id}` references undefined stroke profile `{profile_id}`"
                        ),
                    )
                    .with_location(Location::element_at(&element_id, "/stroke/profileId")),
                );
                return None;
            }
        };

        let palette = self.style.palette;
        let gradients = self.style.gradients;
        let paint = {
            let element = &self.scene.elements[index];
            style::resolve_stroke_paint(element, palette, gradients, &mut self.diagnostics)
        }?;
        let width = self.apply_stroke_weight(&element_id, geometry.width);
        Some(NodeStroke {
            paint,
            width,
            cap: geometry.cap,
            join: geometry.join,
        })
    }

    /// The stroke's effective width under the scene's recipe (FEAT-008).
    ///
    /// A line-art recipe fixes a consistent weight: its declared `strokeWeight`
    /// fills a stroke whose profile leaves the weight unset, while a profile
    /// that states a positive width is honored as an explicit varying weight. A
    /// weight below the minimum renderable unit is clamped up and reported,
    /// since a thinner line does not survive rasterization. Every other recipe,
    /// and the no-recipe path, leaves the profile's width untouched.
    fn apply_stroke_weight(&mut self, element_id: &str, profile_width: f64) -> f64 {
        let Some(recipe) = self.style.recipe else {
            return profile_width;
        };
        let (width, clamped) = recipe.resolve_stroke_weight(profile_width);
        if clamped {
            self.diagnostics.push(
                Diagnostic::warning(
                    style::STROKE_WEIGHT_CLAMPED,
                    format!(
                        "stroke weight {profile_width} for element `{element_id}` is below the minimum renderable unit {}; it was clamped",
                        style::MIN_STROKE_WEIGHT
                    ),
                )
                .with_location(Location::element_at(element_id, "/stroke/profileId")),
            );
        }
        width
    }

    /// A node identifier unique across the model.
    fn unique_id(&mut self, base: &str, copy: Option<&CopyTag>) -> String {
        let candidate = match copy {
            Some(tag) => format!("{base}~{}", tag.path),
            None => base.to_string(),
        };
        if self.used_ids.insert(candidate.clone()) {
            return candidate;
        }
        let mut suffix = 1usize;
        loop {
            let alternative = format!("{candidate}~{suffix}");
            if self.used_ids.insert(alternative.clone()) {
                return alternative;
            }
            suffix += 1;
        }
    }

    fn reject_unsupported(&mut self, index: usize, feature: &str) {
        let id = self.scene.elements[index].id.clone();
        self.diagnostics.push(
            Diagnostic::error(
                UNSUPPORTED,
                format!("element `{id}` uses unsupported feature `{feature}`"),
            )
            .with_location(Location::element(id)),
        );
    }

    /// Refuses a copy count that would expand past the node limit before the
    /// expansion is materialised, so an untrusted scene cannot demand unbounded
    /// memory (NFR-021).
    fn guard_count(&mut self, index: usize, count: u32) -> bool {
        if u64::from(count) <= MAX_RENDER_NODES as u64 {
            return true;
        }
        let id = self.scene.elements[index].id.clone();
        self.diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::SIZE_LIMIT,
                format!(
                    "element `{id}` expands to {count} copies, above the {MAX_RENDER_NODES}-copy limit"
                ),
            )
            .with_location(Location::element(id)),
        );
        false
    }

    fn reject_composition(&mut self, index: usize, reason: &str) {
        let id = self.scene.elements[index].id.clone();
        self.diagnostics.push(
            Diagnostic::error(
                composition::COMPOSITION,
                format!("composition `{id}` {reason}"),
            )
            .with_location(Location::element(id)),
        );
    }

    fn push_reference_error(&mut self, index: usize, parent_id: &str) {
        let id = self.scene.elements[index].id.clone();
        self.diagnostics.push(
            Diagnostic::error(
                REFERENCE,
                format!("element `{id}` references unknown parent `{parent_id}`"),
            )
            .with_location(Location::element_at(id, "/parentId")),
        );
    }

    fn push_cycle_error(&mut self, cycle: &[usize]) {
        let names: Vec<&str> = cycle
            .iter()
            .map(|&index| self.scene.elements[index].id.as_str())
            .collect();
        let first = names.first().copied().unwrap_or_default().to_string();
        let path = format!("{} -> {}", names.join(" -> "), first);
        self.diagnostics.push(
            Diagnostic::error(CYCLE, format!("circular reference: {path}"))
                .with_location(Location::element(first)),
        );
    }

    fn into_model(self) -> RenderModel {
        RenderModel {
            canvas: RenderCanvas {
                width: self.scene.canvas.width,
                height: self.scene.canvas.height,
                background: self.scene.canvas.background.clone(),
            },
            nodes: self.nodes,
            meta: RenderMeta {
                title: self.scene.title.clone(),
                description: self.scene.description.clone(),
                recipe: self
                    .style
                    .recipe
                    .map(|recipe| recipe.name_str().to_string()),
            },
            diagnostics: self.diagnostics,
            fonts: self.fonts,
        }
    }
}

/// Finds one cycle in a parent-pointer forest, or `None` when it is acyclic.
///
/// Iterative so a deep, healthy tree of fifty thousand elements cannot exhaust
/// the stack. Each element has at most one parent, so following parents from any
/// node either reaches a root or re-enters a cycle.
fn detect_cycle(parent: &[Option<usize>]) -> Option<Vec<usize>> {
    const UNVISITED: u8 = 0;
    const ON_PATH: u8 = 1;
    const DONE: u8 = 2;

    let mut state = vec![UNVISITED; parent.len()];
    for start in 0..parent.len() {
        if state[start] != UNVISITED {
            continue;
        }
        let mut path = Vec::new();
        let mut current = Some(start);
        while let Some(node) = current {
            match state[node] {
                ON_PATH => {
                    let position = path
                        .iter()
                        .position(|&visited| visited == node)
                        .unwrap_or_default();
                    return Some(path[position..].to_vec());
                }
                DONE => break,
                _ => {}
            }
            state[node] = ON_PATH;
            path.push(node);
            current = parent[node];
        }
        for &node in &path {
            state[node] = DONE;
        }
    }
    None
}

fn kind_name(kind: ElementKind) -> &'static str {
    kind.as_str()
}

/// Whether a shape draws a segment that is not a straight line.
///
/// A rectangle with rounded corners and an ellipse are constructed curves the
/// geometric recipe keeps; only a path's Bézier or arc segments are the
/// freeform curves it avoids (FEAT-009).
fn has_freeform_curve(shape: &Shape) -> bool {
    let Shape::Path(path) = shape else {
        return false;
    };
    path.subpaths.iter().any(|subpath| {
        subpath
            .segments
            .iter()
            .any(|segment| !matches!(segment, Segment::Line { .. }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{Ellipse, Rect as PrimRect};
    use crate::style::{
        parse_gradient, parse_palette, parse_stroke_profile, parse_style_recipe, FREEFORM_CURVE,
        GRID_SNAPPED, GRID_TOO_FINE, LINE_ART_EMPTY, MIN_GRID_SIZE, MIN_STROKE_WEIGHT,
        STROKE_WEIGHT_CLAMPED, TEXTURE_UNSUPPORTED, UNDEFINED_TOKEN,
    };
    use serde_json::{json, Value};

    fn base(id: &str, order: u64, kind: &str, geometry: Value) -> Value {
        json!({
            "id": id,
            "sceneId": "s",
            "order": order,
            "kind": kind,
            "geometry": geometry,
            "transform": {
                "translateX": 0.0,
                "translateY": 0.0,
                "rotate": 0.0,
                "scaleX": 1.0,
                "scaleY": 1.0
            },
            "opacity": 1.0,
            "visible": true
        })
    }

    fn rect(id: &str, order: u64, width: f64, height: f64) -> Value {
        base(
            id,
            order,
            "rect",
            json!({ "x": 0.0, "y": 0.0, "width": width, "height": height }),
        )
    }

    /// A token paint reference as the scene language carries it.
    fn token_paint(token: &str) -> Value {
        json!({ "kind": "token", "ref": token })
    }

    /// A stroke pairing a profile with a token paint.
    fn stroke_json(profile: &str, token: &str) -> Value {
        json!({ "profileId": profile, "paint": token_paint(token) })
    }

    /// The colour a resolved paint carries, or `None` for a gradient.
    fn color(paint: &crate::render::Paint) -> Option<&str> {
        match paint {
            crate::render::Paint::Color { value } => Some(value),
            crate::render::Paint::Gradient(_) => None,
        }
    }

    fn scene_of(elements: Value, constraints: Option<Value>) -> Scene {
        let mut document = json!({
            "id": "s",
            "projectId": "p",
            "name": "S",
            "formatVersion": "0.2",
            "canvas": { "width": 400.0, "height": 400.0, "background": "#ffffff" },
            "elements": elements
        });
        if let Some(constraints) = constraints {
            document["constraints"] = constraints;
        }
        crate::scene::parse(&document.to_string()).expect("a valid scene")
    }

    fn compiled(elements: Value) -> RenderModel {
        compile(&scene_of(elements, None)).expect("compiles")
    }

    #[test]
    fn a_rect_compiles_to_one_concrete_node() {
        let model = compiled(json!([rect("e1", 0, 30.0, 40.0)]));
        assert_eq!(model.nodes.len(), 1);
        let node = &model.nodes[0];
        assert_eq!(node.id, "e1");
        assert_eq!(node.kind, "rect");
        assert_eq!(node.order, 0);
        assert_eq!(
            node.geometry,
            Some(Shape::Rect(PrimRect {
                x: 0.0,
                y: 0.0,
                width: 30.0,
                height: 40.0,
                rx: 0.0,
                ry: 0.0,
            }))
        );
        assert_eq!(model.canvas.width, 400.0);
        assert_eq!(model.canvas.background, "#ffffff");
    }

    #[test]
    fn an_element_transform_becomes_the_resolved_world_transform() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["transform"]["translateX"] = json!(5.0);
        element["transform"]["translateY"] = json!(6.0);
        let model = compiled(json!([element]));
        assert_eq!(model.nodes[0].transform.apply([0.0, 0.0]), [5.0, 6.0]);
    }

    #[test]
    fn compilation_is_deterministic() {
        let elements = json!([rect("a", 0, 10.0, 10.0), rect("b", 1, 10.0, 10.0)]);
        let first = compiled(elements.clone());
        let second = compiled(elements);
        assert_eq!(
            first, second,
            "repeated runs must be byte-identical (NFR-010)"
        );
    }

    #[test]
    fn an_invalid_scene_fails_with_a_location() {
        let mut scene = scene_of(json!([rect("e1", 0, 10.0, 10.0)]), None);
        scene.elements[0].opacity = 1.5;
        let diagnostics = compile(&scene).expect_err("an invalid scene is refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::SCHEMA);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/elements/0/opacity")
        );
    }

    #[test]
    fn constraints_resolve_into_concrete_translations() {
        let elements = json!([
            rect("a", 0, 10.0, 10.0),
            rect("b", 1, 10.0, 10.0),
            rect("c", 2, 10.0, 10.0),
        ]);
        let mut b = elements[1].clone();
        b["transform"]["translateX"] = json!(50.0);
        let mut c = elements[2].clone();
        c["transform"]["translateX"] = json!(200.0);
        let scene = scene_of(
            json!([elements[0].clone(), b, c]),
            Some(json!([{
                "id": "sp",
                "sceneId": "s",
                "kind": "equalSpacing",
                "elementIds": ["a", "b", "c"],
                "axis": "x",
                "value": 20.0
            }])),
        );
        let model = compile(&scene).expect("compiles");
        assert_eq!(
            model.node("b").unwrap().transform.apply([0.0, 0.0]),
            [30.0, 0.0]
        );
        assert_eq!(
            model.node("c").unwrap().transform.apply([0.0, 0.0]),
            [60.0, 0.0]
        );
    }

    #[test]
    fn an_attached_connector_meets_both_anchors() {
        let wire = base(
            "wire",
            0,
            "line",
            json!({ "points": [[0.0, 0.0], [10.0, 0.0]] }),
        );
        let mut left = rect("left", 1, 20.0, 20.0);
        left["transform"]["translateX"] = json!(0.0);
        let mut right = rect("right", 2, 20.0, 20.0);
        right["transform"]["translateX"] = json!(100.0);
        right["transform"]["translateY"] = json!(40.0);
        let scene = scene_of(
            json!([wire, left, right]),
            Some(json!([{
                "id": "link",
                "sceneId": "s",
                "kind": "attach",
                "elementIds": ["wire", "left", "right"]
            }])),
        );
        let model = compile(&scene).expect("compiles");
        let node = model.node("wire").expect("the connector");
        assert_eq!(
            node.geometry,
            Some(Shape::Line(Line {
                points: vec![[10.0, 10.0], [110.0, 50.0]],
            }))
        );
        assert_eq!(node.transform, Affine::IDENTITY);
    }

    #[test]
    fn a_boolean_compiles_to_one_concrete_path() {
        let boolean = base("b1", 0, "boolean", json!({ "operation": "subtract" }));
        let mut first = rect("c1", 0, 10.0, 10.0);
        first["parentId"] = json!("b1");
        let mut second = rect("c2", 1, 3.0, 3.0);
        second["parentId"] = json!("b1");
        second["geometry"]["x"] = json!(2.0);
        second["geometry"]["y"] = json!(2.0);
        let model = compiled(json!([boolean, first, second]));
        assert_eq!(model.nodes.len(), 1);
        let node = &model.nodes[0];
        assert_eq!(node.kind, "boolean");
        assert!(matches!(node.geometry, Some(Shape::Path(_))));
    }

    #[test]
    fn a_repeat_emits_one_node_per_copy_with_unique_ids() {
        let repeat = base("r1", 0, "repeat", json!({ "count": 3, "spacing": 10.0 }));
        let mut child = rect("c1", 0, 5.0, 5.0);
        child["parentId"] = json!("r1");
        let model = compiled(json!([repeat, child]));
        assert_eq!(model.nodes.len(), 3);
        let mut ids: Vec<&str> = model.nodes.iter().map(|node| node.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3, "copy identifiers are unique");
        let positions: Vec<f64> = model
            .nodes
            .iter()
            .map(|node| node.transform.apply([0.0, 0.0])[0])
            .collect();
        assert_eq!(positions, vec![0.0, 10.0, 20.0]);
    }

    #[test]
    fn an_oversized_copy_count_is_refused_with_a_size_limit() {
        let repeat = base(
            "r1",
            0,
            "repeat",
            json!({ "count": u32::MAX, "spacing": 1.0 }),
        );
        let mut child = rect("c1", 0, 5.0, 5.0);
        child["parentId"] = json!("r1");
        let scene = scene_of(json!([repeat, child]), None);
        let diagnostics = compile(&scene).expect_err("an unbounded expansion is refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, DiagnosticCode::SIZE_LIMIT);
    }

    #[test]
    fn a_projection_maps_its_children_onto_its_axis() {
        let projection = base("p1", 0, "projection", json!({ "axis": "x" }));
        let mut child = rect("c1", 0, 5.0, 5.0);
        child["parentId"] = json!("p1");
        child["transform"]["translateX"] = json!(3.0);
        child["transform"]["translateY"] = json!(4.0);
        let model = compiled(json!([projection, child]));
        let node = model.node("c1").expect("the child");
        assert_eq!(node.transform.apply([0.0, 0.0]), [3.0, 0.0]);
    }

    #[test]
    fn an_along_path_element_places_its_children_along_the_guide() {
        let along = base(
            "a1",
            0,
            "alongPath",
            json!({ "pathData": "M0 0 L30 0", "count": 3 }),
        );
        let mut child = rect("c1", 0, 4.0, 4.0);
        child["parentId"] = json!("a1");
        let model = compiled(json!([along, child]));
        assert_eq!(model.nodes.len(), 3);
        let positions: Vec<f64> = model
            .nodes
            .iter()
            .map(|node| node.transform.apply([0.0, 0.0])[0])
            .collect();
        assert_eq!(positions, vec![0.0, 15.0, 30.0]);
    }

    #[test]
    fn an_offset_compiles_to_concrete_path_geometry() {
        let offset = base("o1", 0, "offset", json!({ "distance": 1.0 }));
        let mut child = rect("c1", 0, 10.0, 10.0);
        child["parentId"] = json!("o1");
        let model = compiled(json!([offset, child]));
        assert_eq!(model.nodes.len(), 1);
        assert_eq!(model.nodes[0].kind, "offset");
        assert!(matches!(model.nodes[0].geometry, Some(Shape::Path(_))));
    }

    #[test]
    fn a_group_transform_is_inherited_by_its_children() {
        let mut group = base("g1", 0, "group", json!({}));
        group["transform"]["translateX"] = json!(100.0);
        group["transform"]["rotate"] = json!(90.0);
        let mut child = rect("c1", 0, 5.0, 5.0);
        child["parentId"] = json!("g1");
        child["transform"]["translateX"] = json!(10.0);
        let model = compiled(json!([group, child]));
        let node = model.node("c1").expect("the child");
        let origin = node.transform.apply([0.0, 0.0]);
        assert!((origin[0] - 100.0).abs() < 1e-9, "{origin:?}");
        assert!((origin[1] - 10.0).abs() < 1e-9, "{origin:?}");
    }

    #[test]
    fn a_root_element_carries_no_group_chain() {
        let model = compiled(json!([rect("e1", 0, 10.0, 10.0)]));
        assert!(model.nodes[0].groups.is_empty());
    }

    #[test]
    fn a_named_group_chain_is_carried_by_its_descendants() {
        let mut group = base("g1", 0, "group", json!({}));
        group["name"] = json!("Outer");
        let mut child = rect("c1", 0, 5.0, 5.0);
        child["parentId"] = json!("g1");
        let model = compiled(json!([group, child]));
        assert_eq!(
            model.node("c1").expect("the child").groups,
            vec![NodeGroup {
                id: "g1".to_string(),
                name: Some("Outer".to_string()),
            }]
        );
    }

    #[test]
    fn nested_group_chains_run_outermost_first() {
        let mut outer = base("outer", 0, "group", json!({}));
        outer["name"] = json!("Outer");
        let mut inner = base("inner", 1, "group", json!({}));
        inner["name"] = json!("Inner");
        inner["parentId"] = json!("outer");
        let mut child = rect("c1", 0, 5.0, 5.0);
        child["parentId"] = json!("inner");
        let model = compiled(json!([outer, inner, child]));
        let groups = &model.node("c1").expect("the child").groups;
        assert_eq!(
            groups
                .iter()
                .map(|group| group.id.as_str())
                .collect::<Vec<_>>(),
            vec!["outer", "inner"]
        );
        assert_eq!(groups[0].name.as_deref(), Some("Outer"));
        assert_eq!(groups[1].name.as_deref(), Some("Inner"));
    }

    #[test]
    fn an_unnamed_group_is_still_carried_in_the_chain() {
        let group = base("g1", 0, "group", json!({}));
        let mut child = rect("c1", 0, 5.0, 5.0);
        child["parentId"] = json!("g1");
        let model = compiled(json!([group, child]));
        assert_eq!(
            model.node("c1").expect("the child").groups,
            vec![NodeGroup {
                id: "g1".to_string(),
                name: None,
            }]
        );
    }

    #[test]
    fn a_composition_inside_a_group_keeps_the_group_chain() {
        let mut group = base("g1", 0, "group", json!({}));
        group["name"] = json!("Outer");
        let mut repeat = base("r1", 1, "repeat", json!({ "count": 2, "spacing": 10.0 }));
        repeat["parentId"] = json!("g1");
        let mut child = rect("c1", 0, 5.0, 5.0);
        child["parentId"] = json!("r1");
        let model = compiled(json!([group, repeat, child]));
        assert_eq!(model.nodes.len(), 2);
        let expected = vec![NodeGroup {
            id: "g1".to_string(),
            name: Some("Outer".to_string()),
        }];
        for node in &model.nodes {
            assert_eq!(node.groups, expected);
        }
    }

    #[test]
    fn a_circular_reference_is_refused_naming_the_cycle() {
        let mut a = rect("a", 0, 10.0, 10.0);
        a["parentId"] = json!("b");
        let mut b = rect("b", 1, 10.0, 10.0);
        b["parentId"] = json!("a");
        let scene = scene_of(json!([a, b]), None);
        let diagnostics = compile(&scene).expect_err("a cycle is refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, CYCLE);
        assert!(error.message.contains('a') && error.message.contains('b'));
    }

    #[test]
    fn an_unknown_parent_is_refused() {
        let mut child = rect("c1", 0, 10.0, 10.0);
        child["parentId"] = json!("ghost");
        let scene = scene_of(json!([child]), None);
        let diagnostics = compile(&scene).expect_err("a dangling parent is refused");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, REFERENCE);
        assert!(error.message.contains("ghost"));
    }

    #[test]
    fn a_raster_element_is_refused_as_unsupported() {
        let raster = base("r1", 0, "raster", json!({}));
        let scene = scene_of(json!([raster]), None);
        let diagnostics = compile(&scene).expect_err("raster is unsupported");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNSUPPORTED);
        assert!(error.message.contains("raster"));
    }

    #[test]
    fn an_empty_path_emits_no_node_and_keeps_its_warning() {
        let path = base("p1", 0, "path", json!({}));
        let model = compiled(json!([path]));
        assert!(model.nodes.is_empty());
        assert!(!model.diagnostics.has_errors());
        assert!(model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == primitives::EMPTY_PATH));
    }

    #[test]
    fn style_resolves_against_a_supplied_context() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["fill"] = token_paint("accent");
        element["stroke"] = stroke_json("stroke-1", "accent");
        let scene = scene_of(json!([element]), None);

        let palette = parse_palette(
            r##"{"id":"pal","projectId":"p","name":"P","tokens":[{"name":"accent","value":"#ff0000"}]}"##,
        )
        .expect("a palette");
        let profile = parse_stroke_profile(
            r#"{"id":"stroke-1","projectId":"p","name":"O","width":3,"cap":"butt","join":"miter"}"#,
        )
        .expect("a profile");
        let style = StyleContext {
            palette: Some(&palette),
            strokes: std::slice::from_ref(&profile),
            gradients: &[],
            fonts: &[],
            recipe: None,
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        let paint = &model.nodes[0].paint;
        assert_eq!(paint.fill.as_ref().and_then(color), Some("#ff0000"));
        let stroke = paint.stroke.as_ref().expect("a resolved stroke");
        assert_eq!(color(&stroke.paint), Some("#ff0000"));
        assert_eq!(stroke.width, 3.0);
        assert_eq!(stroke.cap, crate::style::StrokeCap::Butt);
    }

    /// A flat recipe document naming the flat look with the given shading.
    fn flat_recipe(shading: &str) -> crate::style::StyleRecipe {
        parse_style_recipe(&format!(
            r#"{{"id":"recipe-1","projectId":"p","name":"flat","parameters":{{"shading":"{shading}"}}}}"#
        ))
        .expect("a flat recipe")
    }

    /// A palette carrying a solid colour, an ink, and a transparent fill.
    fn flat_palette() -> crate::style::Palette {
        parse_palette(
            r##"{"id":"pal","projectId":"p","name":"P","tokens":[{"name":"accent","value":"#ff0000"},{"name":"ink","value":"#111111"},{"name":"clear","value":"transparent"}]}"##,
        )
        .expect("a palette")
    }

    #[test]
    fn a_flat_recipe_is_named_in_the_model_and_keeps_fills_solid() {
        let mut solid = rect("solid", 0, 10.0, 10.0);
        solid["fill"] = token_paint("accent");
        let mut blended = rect("blended", 1, 10.0, 10.0);
        blended["fill"] = json!({ "kind": "gradient", "ref": "fade" });
        let scene = scene_of(json!([solid, blended]), None);

        let palette = flat_palette();
        let gradients = [parse_gradient(
            r##"{"id":"fade","projectId":"p","name":"F","type":"linear","stops":[{"offset":0,"token":"accent"},{"offset":1,"token":"ink"}]}"##,
        )
        .expect("a gradient")];
        let recipe = flat_recipe("none");
        let style = StyleContext {
            palette: Some(&palette),
            strokes: &[],
            gradients: &gradients,
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert_eq!(model.meta.recipe.as_deref(), Some("flat"));
        assert_eq!(
            model
                .node("solid")
                .unwrap()
                .paint
                .fill
                .as_ref()
                .and_then(color),
            Some("#ff0000"),
            "a flat fill is the solid palette colour it names"
        );
        let crate::render::Paint::Gradient(fill) = model
            .node("blended")
            .unwrap()
            .paint
            .fill
            .as_ref()
            .expect("a fill")
        else {
            panic!("a requested gradient is honored under the flat recipe");
        };
        assert_eq!(fill.stops[0].color, "#ff0000");
        assert_eq!(fill.stops[1].color, "#111111");

        // The applied recipe is named in the model and survives its camelCase
        // JSON round trip (C-003, D-014).
        let text = model.to_json_string().expect("serializable");
        assert!(text.contains("\"recipe\":\"flat\""), "{text}");
        let reparsed = crate::render::parse(&text).expect("deserializable");
        assert_eq!(reparsed.meta.recipe.as_deref(), Some("flat"));
    }

    #[test]
    fn a_transparent_fill_resolves_to_transparency() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["fill"] = token_paint("clear");
        let scene = scene_of(json!([element]), None);
        let palette = flat_palette();
        let style = StyleContext {
            palette: Some(&palette),
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: None,
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert_eq!(
            model.nodes[0].paint.fill.as_ref().and_then(color),
            Some("transparent")
        );
    }

    #[test]
    fn a_flat_recipe_that_asks_for_texture_warns_and_draws_none() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["fill"] = token_paint("accent");
        let scene = scene_of(json!([element]), None);
        let palette = flat_palette();
        let recipe = flat_recipe("raster");
        let style = StyleContext {
            palette: Some(&palette),
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("the scene still compiles");
        assert!(
            model
                .diagnostics
                .warnings()
                .any(|warning| warning.code == TEXTURE_UNSUPPORTED),
            "texture is reported, not silently drawn: {:?}",
            model.diagnostics
        );
        assert_eq!(
            model.nodes[0].paint.fill.as_ref().and_then(color),
            Some("#ff0000"),
            "the fill stays a solid palette colour"
        );
    }

    #[test]
    fn compiling_without_a_recipe_names_none_in_the_model() {
        let model = compiled(json!([rect("e1", 0, 10.0, 10.0)]));
        assert_eq!(model.meta.recipe, None);
    }

    /// A line-art recipe with the given default stroke weight.
    fn line_art_recipe(weight: Option<f64>) -> crate::style::StyleRecipe {
        let parameters = match weight {
            Some(weight) => format!(r#"{{"strokeWeight":{weight}}}"#),
            None => "{}".to_string(),
        };
        parse_style_recipe(&format!(
            r#"{{"id":"recipe-l","projectId":"p","name":"line-art","parameters":{parameters}}}"#
        ))
        .expect("a line-art recipe")
    }

    /// A stroke profile with the given identifier and width.
    fn profile(id: &str, width: f64) -> crate::style::StrokeProfile {
        parse_stroke_profile(&format!(
            r#"{{"id":"{id}","projectId":"p","name":"O","width":{width},"cap":"round","join":"miter"}}"#
        ))
        .expect("a stroke profile")
    }

    #[test]
    fn a_line_art_recipe_fixes_the_stroke_weight_and_paint() {
        let mut element = rect("e1", 0, 20.0, 20.0);
        element["stroke"] = stroke_json("outline", "accent");
        let scene = scene_of(json!([element]), None);

        let palette = flat_palette();
        let outline = profile("outline", 0.0);
        let recipe = line_art_recipe(Some(2.5));
        let style = StyleContext {
            palette: Some(&palette),
            strokes: std::slice::from_ref(&outline),
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert_eq!(model.meta.recipe.as_deref(), Some("line-art"));
        let stroke = model
            .node("e1")
            .expect("the node")
            .paint
            .stroke
            .as_ref()
            .expect("a resolved stroke");
        assert_eq!(
            stroke.width, 2.5,
            "the recipe's weight fills a profile that leaves the weight unset"
        );
        assert_eq!(
            color(&stroke.paint),
            Some("#ff0000"),
            "the stroke takes its paint from the palette"
        );
    }

    #[test]
    fn varying_stroke_weights_are_honored_under_line_art() {
        let mut thin = rect("thin", 0, 10.0, 10.0);
        thin["stroke"] = stroke_json("hairline", "accent");
        let mut bold = rect("bold", 1, 10.0, 10.0);
        bold["stroke"] = stroke_json("heavy", "accent");
        let scene = scene_of(json!([thin, bold]), None);

        let palette = flat_palette();
        let profiles = [profile("hairline", 1.0), profile("heavy", 6.0)];
        let recipe = line_art_recipe(Some(2.5));
        let style = StyleContext {
            palette: Some(&palette),
            strokes: &profiles,
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        let width = |id: &str| model.node(id).unwrap().paint.stroke.as_ref().unwrap().width;
        assert_eq!(width("thin"), 1.0, "an explicit light weight is honored");
        assert_eq!(width("bold"), 6.0, "an explicit heavy weight is honored");
        assert!(
            !model
                .diagnostics
                .warnings()
                .any(|warning| warning.code == STROKE_WEIGHT_CLAMPED),
            "neither weight is below the minimum"
        );
    }

    #[test]
    fn a_fill_disabled_shape_renders_only_its_outline() {
        let mut element = rect("e1", 0, 20.0, 20.0);
        element["fill"] = json!(null);
        element["stroke"] = stroke_json("outline", "accent");
        let scene = scene_of(json!([element]), None);

        let palette = flat_palette();
        let outline = profile("outline", 2.0);
        let recipe = line_art_recipe(Some(2.0));
        let style = StyleContext {
            palette: Some(&palette),
            strokes: std::slice::from_ref(&outline),
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        let node = model.node("e1").expect("the node");
        assert!(node.paint.fill.is_none(), "no fill is drawn");
        assert!(node.paint.stroke.is_some(), "the outline is drawn");
    }

    #[test]
    fn a_sub_minimum_stroke_weight_is_clamped_and_reported() {
        let mut element = rect("e1", 0, 20.0, 20.0);
        element["stroke"] = stroke_json("hairline", "accent");
        let scene = scene_of(json!([element]), None);

        let palette = flat_palette();
        let outline = profile("hairline", 0.001);
        let recipe = line_art_recipe(Some(2.5));
        let style = StyleContext {
            palette: Some(&palette),
            strokes: std::slice::from_ref(&outline),
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        let stroke = model
            .node("e1")
            .expect("the node")
            .paint
            .stroke
            .as_ref()
            .expect("a resolved stroke");
        assert_eq!(stroke.width, MIN_STROKE_WEIGHT, "the weight is clamped up");
        let warning = model
            .diagnostics
            .warnings()
            .find(|warning| warning.code == STROKE_WEIGHT_CLAMPED)
            .expect("a clamp warning");
        assert!(warning.message.contains("0.001"), "{}", warning.message);
    }

    #[test]
    fn a_line_art_scene_with_no_strokes_reports_empty_output() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["fill"] = token_paint("accent");
        let scene = scene_of(json!([element]), None);

        let palette = flat_palette();
        let recipe = line_art_recipe(Some(2.5));
        let style = StyleContext {
            palette: Some(&palette),
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert!(
            model
                .diagnostics
                .warnings()
                .any(|warning| warning.code == LINE_ART_EMPTY),
            "a line-art scene with no strokes reports empty line work: {:?}",
            model.diagnostics
        );
    }

    #[test]
    fn a_self_intersecting_stroke_path_renders_without_failing() {
        let mut path = base(
            "p1",
            0,
            "path",
            json!({ "pathData": "M0 0 L10 10 L10 0 L0 10 Z" }),
        );
        path["stroke"] = stroke_json("outline", "accent");
        let scene = scene_of(json!([path]), None);

        let palette = flat_palette();
        let outline = profile("outline", 2.0);
        let recipe = line_art_recipe(Some(2.0));
        let style = StyleContext {
            palette: Some(&palette),
            strokes: std::slice::from_ref(&outline),
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model =
            compile_with_style(&scene, &style).expect("a self-intersecting stroke compiles");
        let node = model.node("p1").expect("the node");
        assert!(node.paint.stroke.is_some(), "the stroke rule draws it");
        assert!(!model.diagnostics.has_errors());
    }

    #[test]
    fn a_changed_token_restyles_both_fill_and_stroke_in_one_recompile() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["fill"] = token_paint("accent");
        element["stroke"] = stroke_json("stroke-1", "accent");
        let scene = scene_of(json!([element]), None);
        let profile = parse_stroke_profile(
            r#"{"id":"stroke-1","projectId":"p","name":"O","width":3,"cap":"butt","join":"miter"}"#,
        )
        .expect("a profile");

        let before = parse_palette(
            r##"{"id":"pal","projectId":"p","name":"P","tokens":[{"name":"accent","value":"#ff0000"}]}"##,
        )
        .expect("a palette");
        let after = parse_palette(
            r##"{"id":"pal","projectId":"p","name":"P","tokens":[{"name":"accent","value":"#0000ff"}]}"##,
        )
        .expect("a palette");

        let first = compile_with_style(
            &scene,
            &StyleContext {
                palette: Some(&before),
                strokes: std::slice::from_ref(&profile),
                gradients: &[],
                fonts: &[],
                recipe: None,
            },
        )
        .expect("compiles");
        let second = compile_with_style(
            &scene,
            &StyleContext {
                palette: Some(&after),
                strokes: std::slice::from_ref(&profile),
                gradients: &[],
                fonts: &[],
                recipe: None,
            },
        )
        .expect("compiles");

        assert_eq!(
            first.nodes[0].paint.fill.as_ref().and_then(color),
            Some("#ff0000")
        );
        assert_eq!(
            first.nodes[0]
                .paint
                .stroke
                .as_ref()
                .and_then(|s| color(&s.paint)),
            Some("#ff0000")
        );
        assert_eq!(
            second.nodes[0].paint.fill.as_ref().and_then(color),
            Some("#0000ff")
        );
        assert_eq!(
            second.nodes[0]
                .paint
                .stroke
                .as_ref()
                .and_then(|s| color(&s.paint)),
            Some("#0000ff")
        );
    }

    #[test]
    fn an_undefined_stroke_token_is_an_error_naming_the_token() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["stroke"] = stroke_json("stroke-1", "missing");
        let scene = scene_of(json!([element]), None);
        let palette = parse_palette(
            r##"{"id":"pal","projectId":"p","name":"P","tokens":[{"name":"accent","value":"#ff0000"}]}"##,
        )
        .expect("a palette");
        let profile = parse_stroke_profile(
            r#"{"id":"stroke-1","projectId":"p","name":"O","width":3,"cap":"butt","join":"miter"}"#,
        )
        .expect("a profile");
        let style = StyleContext {
            palette: Some(&palette),
            strokes: std::slice::from_ref(&profile),
            gradients: &[],
            fonts: &[],
            recipe: None,
        };

        let diagnostics =
            compile_with_style(&scene, &style).expect_err("an undefined stroke token is refused");
        let error = diagnostics
            .errors()
            .find(|error| error.code == UNDEFINED_TOKEN)
            .expect("an undefined-token error");
        assert!(error.message.contains("missing"));
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/stroke/paint")
        );
    }

    #[test]
    fn fill_and_stroke_references_pass_through_without_a_palette() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["fill"] = token_paint("accent");
        element["stroke"] = stroke_json("stroke-1", "accent");
        let model = compiled(json!([element]));
        assert_eq!(
            model.nodes[0].paint.fill.as_ref().and_then(color),
            Some("accent")
        );
        assert!(model.nodes[0].paint.stroke.is_none());
        assert!(model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == UNRESOLVED_STROKE));
    }

    #[test]
    fn visibility_and_opacity_are_composed_down_the_tree() {
        let mut group = base("g1", 0, "group", json!({}));
        group["opacity"] = json!(0.5);
        group["visible"] = json!(false);
        let mut child = rect("c1", 0, 5.0, 5.0);
        child["parentId"] = json!("g1");
        child["opacity"] = json!(0.5);
        let model = compiled(json!([group, child]));
        let node = model.node("c1").expect("the child");
        assert!((node.opacity - 0.25).abs() < 1e-12, "{}", node.opacity);
        assert!(!node.visible);
    }

    #[test]
    fn an_ellipse_keeps_its_concrete_geometry() {
        let element = base(
            "e1",
            0,
            "ellipse",
            json!({ "x": 0.0, "y": 0.0, "width": 80.0, "height": 40.0 }),
        );
        let model = compiled(json!([element]));
        assert_eq!(
            model.nodes[0].geometry,
            Some(Shape::Ellipse(Ellipse {
                cx: 40.0,
                cy: 20.0,
                rx: 40.0,
                ry: 20.0,
            }))
        );
    }

    fn text_element(geometry: Value) -> Value {
        base("t1", 0, "text", geometry)
    }

    #[test]
    fn a_text_element_compiles_to_a_text_node_with_no_geometry() {
        let mut element = text_element(json!({
            "x": 10.0, "y": 20.0, "text": "Hi", "fontSize": 12.0
        }));
        element["fontId"] = json!("body");
        let scene = scene_of(json!([element]), None);
        let model = compile(&scene).expect("compiles");

        assert_eq!(model.nodes.len(), 1);
        let node = &model.nodes[0];
        assert_eq!(node.kind, "text");
        assert!(node.geometry.is_none(), "a text node carries no geometry");
        let run = node.text.as_ref().expect("a text run");
        assert_eq!(run.value, "Hi");
        assert_eq!(run.font_size, 12.0);
        assert_eq!(run.align, TextAlign::Start);
        assert_eq!(run.line_height, 12.0);
        assert_eq!(run.letter_spacing, 0.0);
        assert_eq!(run.font_id, "body");
        // The anchor is baked into the resolved transform.
        assert_eq!(node.transform.apply([0.0, 0.0]), [10.0, 20.0]);
        // Without a style context the declared reference passes through with a
        // warning, mirroring the no-style paint path.
        assert!(model.fonts.is_empty());
        assert!(model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == UNRESOLVED_FONT));
    }

    #[test]
    fn a_text_run_carries_its_declared_layout_and_fill() {
        let mut element = text_element(json!({
            "text": "Hi", "fontSize": 20.0, "align": "center",
            "lineHeight": 26.0, "letterSpacing": 2.0, "width": 80.0
        }));
        element["fill"] = token_paint("ink");
        let model = compiled(json!([element]));
        let run = model.nodes[0].text.as_ref().expect("a text run");
        assert_eq!(run.align, TextAlign::Center);
        assert_eq!(run.line_height, 26.0);
        assert_eq!(run.letter_spacing, 2.0);
        assert_eq!(run.width, Some(80.0));
        assert_eq!(
            model.nodes[0].paint.fill.as_ref().and_then(color),
            Some("ink")
        );
    }

    #[test]
    fn a_declared_font_resolves_and_travels_in_the_model() {
        let mut element = text_element(json!({ "text": "Hi", "fontSize": 12.0 }));
        element["fontId"] = json!("body");
        let scene = scene_of(json!([element]), None);
        let fonts = [FontAsset::new("body", "Body", vec![1, 2, 3, 4])];
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &fonts,
            recipe: None,
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert_eq!(model.nodes[0].text.as_ref().unwrap().font_id, "body");
        assert_eq!(model.fonts.len(), 1);
        assert_eq!(model.fonts[0].id, "body");
        assert_eq!(model.fonts[0].name, "Body");
        assert_eq!(model.fonts[0].data, vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_text_element_with_no_font_resolves_to_the_default() {
        let scene = scene_of(
            json!([text_element(json!({ "text": "Hi", "fontSize": 12.0 }))]),
            None,
        );
        let fonts = [FontAsset::new(DEFAULT_FONT_ID, "Inter", vec![9])];
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &fonts,
            recipe: None,
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        let run = model.nodes[0].text.as_ref().expect("a text run");
        assert_eq!(run.font_id, DEFAULT_FONT_ID);
        assert_eq!(model.fonts.len(), 1);
        assert_eq!(model.fonts[0].name, "Inter");
    }

    #[test]
    fn a_missing_font_is_refused_naming_it() {
        let mut element = text_element(json!({ "text": "Hi", "fontSize": 12.0 }));
        element["fontId"] = json!("ghost");
        let scene = scene_of(json!([element]), None);
        let fonts = [FontAsset::new("body", "Body", vec![1])];
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &fonts,
            recipe: None,
        };

        let diagnostics =
            compile_with_style(&scene, &style).expect_err("a missing font is refused");
        let error = diagnostics
            .errors()
            .find(|error| error.code == FONT)
            .expect("a font error");
        assert!(error.message.contains("ghost"), "{}", error.message);
        assert_eq!(
            error
                .location
                .as_ref()
                .and_then(|location| location.json_path.as_deref()),
            Some("/fontId")
        );
    }

    /// Reads a bundled open-licensed font so a test can shape real glyphs.
    fn bundled_font(file: &str) -> Vec<u8> {
        let path = format!("{}/../../assets/fonts/{file}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(&path).unwrap_or_else(|error| panic!("could not read {path}: {error}"))
    }

    #[test]
    fn the_fallback_font_is_carried_into_the_model() {
        let scene = scene_of(
            json!([text_element(json!({ "text": "Hi", "fontSize": 12.0 }))]),
            None,
        );
        let fonts = [
            FontAsset::new(DEFAULT_FONT_ID, "Inter", vec![1, 2]),
            FontAsset::new(FALLBACK_FONT_ID, "Noto Sans", vec![3, 4]),
        ];
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &fonts,
            recipe: None,
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        let ids: Vec<&str> = model.fonts.iter().map(|font| font.id.as_str()).collect();
        assert_eq!(ids, vec![DEFAULT_FONT_ID, FALLBACK_FONT_ID]);
        assert_eq!(model.fonts[1].name, "Noto Sans");
        assert_eq!(model.fonts[1].data, vec![3, 4]);
    }

    #[test]
    fn the_carried_fallback_covers_a_glyph_the_named_font_lacks() {
        // U+0149 is carried by Noto Sans but not by Inter, so only the fallback
        // can draw it (FEAT-024).
        let scene = scene_of(
            json!([text_element(
                json!({ "text": "\u{149}", "fontSize": 100.0 })
            )]),
            None,
        );
        let fonts = [
            FontAsset::new(DEFAULT_FONT_ID, "Inter", bundled_font("Inter.ttf")),
            FontAsset::new(FALLBACK_FONT_ID, "Noto Sans", bundled_font("NotoSans.ttf")),
        ];
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &fonts,
            recipe: None,
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        let run = model.nodes[0].text.as_ref().expect("a text run");
        let outlined = crate::fonts::outline_text(run, "t1", &model.fonts);
        assert!(!outlined.path.is_empty(), "the fallback draws the glyph");
        assert!(
            !outlined
                .diagnostics
                .warnings()
                .any(|warning| warning.code == crate::fonts::MISSING_GLYPH),
            "{:?}",
            outlined.diagnostics
        );
    }

    #[test]
    fn a_fallback_named_by_a_text_node_is_carried_once() {
        let mut element = text_element(json!({ "text": "Hi", "fontSize": 12.0 }));
        element["fontId"] = json!(FALLBACK_FONT_ID);
        let scene = scene_of(json!([element]), None);
        let fonts = [FontAsset::new(FALLBACK_FONT_ID, "Noto Sans", vec![7])];
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &fonts,
            recipe: None,
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert_eq!(model.fonts.len(), 1);
        assert_eq!(model.fonts[0].id, FALLBACK_FONT_ID);
    }

    #[test]
    fn text_compilation_is_deterministic() {
        let scene = scene_of(
            json!([text_element(json!({ "text": "Hi", "fontSize": 12.0 }))]),
            None,
        );
        let first = compile(&scene).expect("compiles");
        let second = compile(&scene).expect("compiles");
        assert_eq!(first, second, "repeated runs must be identical (NFR-010)");
    }

    #[test]
    fn a_text_element_inside_a_group_carries_the_group_chain() {
        let mut group = base("g1", 0, "group", json!({}));
        group["name"] = json!("Caption");
        let mut text = text_element(json!({ "text": "Hi", "fontSize": 12.0 }));
        text["parentId"] = json!("g1");
        let model = compiled(json!([group, text]));
        let node = model.node("t1").expect("the text node");
        assert_eq!(
            node.groups,
            vec![NodeGroup {
                id: "g1".to_string(),
                name: Some("Caption".to_string()),
            }]
        );
    }

    #[test]
    fn a_text_element_lowered_into_a_boolean_is_refused_as_unsupported() {
        let boolean = base("b1", 0, "boolean", json!({ "operation": "union" }));
        let mut group = base("g1", 0, "group", json!({}));
        group["parentId"] = json!("b1");
        let mut text = text_element(json!({ "text": "Hi", "fontSize": 12.0 }));
        text["parentId"] = json!("g1");
        let scene = scene_of(json!([boolean, group, text]), None);

        let diagnostics = compile(&scene).expect_err("a lowered text is refused");
        let error = diagnostics
            .errors()
            .find(|error| error.code == UNSUPPORTED)
            .expect("an unsupported-feature error");
        assert!(error.message.contains("t1"), "{}", error.message);
    }

    /// A geometric recipe constructing on the given grid.
    fn geometric_recipe(grid: f64) -> crate::style::StyleRecipe {
        parse_style_recipe(&format!(
            r#"{{"id":"recipe-g","projectId":"p","name":"geometric","parameters":{{"gridSize":{grid}}}}}"#
        ))
        .expect("a geometric recipe")
    }

    #[test]
    fn a_geometric_recipe_snaps_off_grid_elements_and_warns() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["transform"]["translateX"] = json!(13.0);
        element["transform"]["translateY"] = json!(27.0);
        let scene = scene_of(json!([element]), None);
        let recipe = geometric_recipe(10.0);
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert_eq!(model.meta.recipe.as_deref(), Some("geometric"));
        assert_eq!(
            model
                .node("e1")
                .expect("the node")
                .transform
                .apply([0.0, 0.0]),
            [10.0, 30.0],
            "an off-grid element snaps to the nearest grid intersection"
        );
        let warning = model
            .diagnostics
            .warnings()
            .find(|warning| warning.code == GRID_SNAPPED)
            .expect("an off-grid element is reported");
        assert!(warning.message.contains("e1"), "{}", warning.message);
    }

    #[test]
    fn an_element_already_on_the_grid_is_left_alone() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["transform"]["translateX"] = json!(20.0);
        element["transform"]["translateY"] = json!(30.0);
        let scene = scene_of(json!([element]), None);
        let recipe = geometric_recipe(10.0);
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert_eq!(
            model
                .node("e1")
                .expect("the node")
                .transform
                .apply([0.0, 0.0]),
            [20.0, 30.0]
        );
        assert!(
            !model
                .diagnostics
                .warnings()
                .any(|warning| warning.code == GRID_SNAPPED),
            "an element on the grid is not reported"
        );
    }

    #[test]
    fn a_geometric_recipe_constructs_polygons_on_the_grid() {
        let first = base(
            "p1",
            0,
            "polygon",
            json!({ "points": [[3.0, 4.0], [23.0, 4.0], [23.0, 24.0]] }),
        );
        let second = base(
            "p2",
            1,
            "polygon",
            json!({ "points": [[1.0, 1.0], [21.0, 1.0], [21.0, 21.0]] }),
        );
        let scene = scene_of(json!([first, second]), None);
        let recipe = geometric_recipe(10.0);
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        let expected = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0]];
        for id in ["p1", "p2"] {
            let Some(Shape::Polygon(polygon)) = &model.node(id).expect("the node").geometry else {
                panic!("a polygon node");
            };
            assert_eq!(
                polygon.points, expected,
                "polygon `{id}` is constructed on the grid"
            );
        }
    }

    #[test]
    fn a_grid_finer_than_the_renderable_resolution_is_reported() {
        let scene = scene_of(json!([rect("e1", 0, 10.0, 10.0)]), None);
        let recipe = geometric_recipe(MIN_GRID_SIZE / 10.0);
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert!(
            model
                .diagnostics
                .warnings()
                .any(|warning| warning.code == GRID_TOO_FINE),
            "a sub-resolution grid is a performance concern: {:?}",
            model.diagnostics
        );
    }

    #[test]
    fn a_freeform_curve_in_a_geometric_scene_is_kept_and_reported() {
        let curved = base(
            "c1",
            0,
            "path",
            json!({ "pathData": "M0 0 C0 10 10 10 10 0" }),
        );
        let straight = base("s1", 1, "path", json!({ "pathData": "M0 0 L10 0 L10 10" }));
        let scene = scene_of(json!([curved, straight]), None);
        let recipe = geometric_recipe(10.0);
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert!(
            matches!(
                model.node("c1").expect("the curve").geometry,
                Some(Shape::Path(_))
            ),
            "the explicitly requested curve is kept"
        );
        let warning = model
            .diagnostics
            .warnings()
            .find(|warning| warning.code == FREEFORM_CURVE)
            .expect("a freeform curve is reported");
        assert!(warning.message.contains("c1"), "{}", warning.message);
        assert!(
            !model
                .diagnostics
                .warnings()
                .any(|warning| warning.code == FREEFORM_CURVE && warning.message.contains("s1")),
            "a straight path is not a freeform curve"
        );
    }

    #[test]
    fn another_recipe_leaves_positions_untouched() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["transform"]["translateX"] = json!(13.0);
        let scene = scene_of(json!([element]), None);
        let recipe = flat_recipe("none");
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };

        let model = compile_with_style(&scene, &style).expect("compiles");
        assert_eq!(
            model
                .node("e1")
                .expect("the node")
                .transform
                .apply([0.0, 0.0])[0],
            13.0
        );
        assert!(!model
            .diagnostics
            .warnings()
            .any(|warning| warning.code == GRID_SNAPPED));
    }

    #[test]
    fn geometric_compilation_is_deterministic() {
        let mut element = rect("e1", 0, 10.0, 10.0);
        element["transform"]["translateX"] = json!(13.0);
        let scene = scene_of(json!([element]), None);
        let recipe = geometric_recipe(10.0);
        let style = StyleContext {
            palette: None,
            strokes: &[],
            gradients: &[],
            fonts: &[],
            recipe: Some(&recipe),
        };
        assert_eq!(
            compile_with_style(&scene, &style).expect("compiles"),
            compile_with_style(&scene, &style).expect("compiles"),
            "repeated runs must be identical (NFR-010)"
        );
    }
}
