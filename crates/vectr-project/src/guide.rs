//! The minimal agent guide a scaffold writes into a new project (FEAT-020).
//!
//! Coding agents load a project's `AGENTS.md` without bespoke setup and keep it
//! across the authoring loop, so the scaffold writes a small, always-read guide
//! there: orientation, the project's local facts, and a pointer to the Vectr
//! skill's on-demand references. It is deliberately *not* the authoring
//! procedure — the procedure, the worked examples, the rules the schema does not
//! state, the one method, the inspect-and-correct loop, the failure
//! catalogue, and the defaults live in the skill's on-demand references and its
//! `examples/`, and are read only when a step needs them (FEAT-020, D-043, D-047).
//!
//! The guide is carried inside the crate at `references/authoring-guide.md` and
//! embedded verbatim at build time rather than restated in Rust, so the text
//! stays editable and reviewable. Embedding rather than reading from disk keeps
//! the scaffold honest: `vectr init` writes the same guide whether it runs from
//! a checkout or an installed binary, and it never depends on the skill package
//! being present. The copy lives under the crate so `cargo package` and a
//! standalone build never reach outside the package.
//!
//! A project with no skill available still works: the guide points at the
//! published schema as the contract to author against and states no requirement
//! that the skill be present (FEAT-020).

/// The file the scaffold writes the guide to, at the project root.
pub const AUTHORING_GUIDE_FILE: &str = "AGENTS.md";

/// The documented size budget for the scaffolded agent guide.
///
/// The guide is always-read material: a coding agent loads it in full and keeps
/// it across the authoring loop, so it is a permanent tax on the context budget.
/// It stays well under this bound — the depth is in the skill's on-demand
/// references, not here (FEAT-020, D-042, D-043).
pub const AGENT_GUIDE_BUDGET_BYTES: usize = 3072;

/// The documented size budget for the skill's always-read entry point
/// (`skills/vectr/SKILL.md`).
///
/// The entry point carries orientation, the workflow, the tool surface, the
/// version check, and pointers to its references; the depth is loaded on demand.
/// Bounding it keeps the authoring loop from paying for material it does not yet
/// need (FEAT-020, D-042).
pub const SKILL_ENTRY_BUDGET_BYTES: usize = 8192;

/// The minimal agent guide the scaffold writes, embedded from the copy the
/// crate carries.
///
/// The guide names the tool and format versions it targets, so a guide embedded
/// into a project always tells an agent which contract it was written for
/// (FEAT-020).
pub fn authoring_guide() -> &'static str {
    include_str!("../references/authoring-guide.md")
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectr_core::scene::CURRENT_FORMAT_VERSION;

    #[test]
    fn the_guide_orients_the_agent_and_states_the_projects_local_facts() {
        let guide = authoring_guide();
        // Orientation: what a scene is and what the toolchain does with it.
        assert!(guide.contains("Vectr scenes"), "orients the agent");
        assert!(guide.contains("published schema"), "points at the contract");
        // The project's local facts, as `vectr init` scaffolds them.
        for fact in ["`project`", "`example`", "`brand`", "`flat`"] {
            assert!(guide.contains(fact), "states the local fact {fact}");
        }
        assert!(
            guide.contains("`scenes/<id>.json`"),
            "states the scene-addressing rule"
        );
        // The tool surface, including the command the scaffold's own contract
        // checks for.
        assert!(guide.contains("vectr validate"), "names `vectr validate`");
        // A pointer to the skill's on-demand references (plural) and to the
        // schema; the depth is not embedded here (FEAT-020, D-042, D-043).
        assert!(
            guide.contains("on-demand references"),
            "points at the skill's on-demand references"
        );
        assert!(
            guide.contains("references/authoring-guide.md"),
            "names the procedure reference to start from"
        );
        assert!(guide.contains("vectr schema"), "points at the schema");
    }

    #[test]
    fn the_guide_names_the_running_tool_and_format_versions() {
        let guide = authoring_guide();
        assert!(
            guide.contains(env!("CARGO_PKG_VERSION")),
            "the guide names the tool version it targets"
        );
        assert!(
            guide.contains(CURRENT_FORMAT_VERSION),
            "the guide names the target format version"
        );
    }

    #[test]
    fn the_guide_carries_no_procedure_or_worked_examples() {
        // The always-read guide must not embed the depth: the procedure, the
        // worked examples, the error catalogue, and a condensed procedure all
        // live in the skill's references (FEAT-020).
        let guide = authoring_guide();
        for forbidden in [
            "Inspect and correct",
            "Retry once",
            "Defaults for an ambiguous request",
            "Licensing",
            "E_SCHEMA",
            "E_PARSE",
            "E_FORMAT_VERSION",
            "```json",
        ] {
            assert!(
                !guide.contains(forbidden),
                "the minimal guide must not carry `{forbidden}`"
            );
        }
    }

    #[test]
    fn the_guide_stays_within_the_always_read_budget() {
        let guide = authoring_guide();
        assert!(
            guide.len() <= AGENT_GUIDE_BUDGET_BYTES,
            "the agent guide is {} bytes, over the {AGENT_GUIDE_BUDGET_BYTES}-byte budget",
            guide.len()
        );
    }

    #[test]
    fn the_skill_entry_point_stays_within_its_budget() {
        // The skill's always-read entry point is the other half of the
        // always-loaded material; its depth lives in the references it points
        // at (FEAT-020, D-042).
        let skill =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/vectr/SKILL.md");
        let Ok(skill) = std::fs::read_to_string(&skill) else {
            // A packaged crate has no repository around it; nothing to measure.
            return;
        };
        assert!(
            skill.len() <= SKILL_ENTRY_BUDGET_BYTES,
            "the skill entry point is {} bytes, over the {SKILL_ENTRY_BUDGET_BYTES}-byte budget",
            skill.len()
        );
    }
}
