//! The render model: the compiler's output and the single input every exporter
//! reads (C-003).
//!
//! A [`RenderModel`] is a flat, fully resolved list of drawing nodes — concrete
//! geometry, a resolved world transform, paint, and paint order — with no
//! reference left for an exporter to chase (FEAT-011). Compiling the same scene
//! twice yields an identical model, because every value is concrete and the node
//! order is fixed (NFR-010).
//!
//! Geometry reuses [`Shape`], the concrete type the primitive and composition
//! layers already produce, and [`Affine`] carries the resolved world transform.
//! A node's `kind` names the scene element that produced it; where a composition
//! expands one element into several nodes (a repeat), each node carries a stable
//! identifier derived from the element it came from.

use crate::composition::Affine;
use crate::primitives::Shape;
use crate::scene::Diagnostics;
use crate::style::{StrokeCap, StrokeJoin};

/// The compiled result of a scene, ready for any exporter (C-003).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderModel {
    /// The drawing surface.
    pub canvas: RenderCanvas,
    /// The drawing nodes, in paint order.
    pub nodes: Vec<ResolvedNode>,
    /// Accessible metadata carried from the scene.
    pub meta: RenderMeta,
    /// Findings recorded while compiling: warnings only on success.
    pub diagnostics: Diagnostics,
}

impl RenderModel {
    /// Finds a node by its resolved identifier.
    pub fn node(&self, id: &str) -> Option<&ResolvedNode> {
        self.nodes.iter().find(|node| node.id == id)
    }
}

/// The drawing surface carried into output.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderCanvas {
    /// Canvas width in scene units.
    pub width: f64,
    /// Canvas height in scene units.
    pub height: f64,
    /// Canvas background color, or `transparent`.
    pub background: String,
}

/// Accessible metadata carried into exported output.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderMeta {
    /// Accessible title, when the scene declares one.
    pub title: Option<String>,
    /// Accessible description, when the scene declares one.
    pub description: Option<String>,
}

/// One concrete drawing node.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedNode {
    /// Stable identifier; unique across the model.
    pub id: String,
    /// Element name, preserved for the exporter.
    pub name: Option<String>,
    /// Position in the model's paint order; lower values paint first.
    pub order: usize,
    /// The scene-language kind that produced the node.
    pub kind: String,
    /// The node's concrete geometry, in its own local coordinates.
    pub geometry: Shape,
    /// The resolved world transform applied to the geometry.
    pub transform: Affine,
    /// The node's fill and stroke.
    pub paint: Paint,
    /// The node's effective opacity, from 0 to 1, composed down the element tree.
    pub opacity: f64,
    /// Whether the node is visible, composed down the element tree.
    pub visible: bool,
}

/// The paint applied to a node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paint {
    /// The resolved fill color, or the scene's declared token name when the
    /// model was compiled without a palette; absent for no fill.
    pub fill: Option<String>,
    /// The resolved stroke, or absent for no stroke.
    pub stroke: Option<NodeStroke>,
}

/// A resolved stroke.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeStroke {
    /// The stroke paint value.
    pub value: String,
    /// Stroke width in scene units.
    pub width: f64,
    /// Stroke line cap.
    pub cap: StrokeCap,
    /// Stroke line join.
    pub join: StrokeJoin,
}
