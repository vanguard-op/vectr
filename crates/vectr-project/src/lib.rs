//! Project loading shared by the Vectr front ends (FEAT-016, FEAT-019).
//!
//! The command line and the MCP server both compile a scene against the assets
//! its project provides: the palette the scene selects, the style recipe it
//! renders in, the stroke profiles and gradients its elements reference, and the
//! fonts its text names (D-009, D-013). Keeping that loader here, rather than in
//! each front end, means the two resolve a project identically — the same
//! documents read in the same order, with the same diagnostics (NFR-010,
//! NFR-011).
//!
//! The [`authoring_guide`] the scaffold writes into a new project (FEAT-020)
//! lives here too, embedded from the agent skill's reference. Both front ends
//! can hand a coding agent the one procedure without shipping a second copy of
//! it.

mod assets;
mod guide;

pub use assets::{project_root, ProjectAssets, STYLE_ASSET};
pub use guide::{authoring_guide, AUTHORING_GUIDE_FILE};
