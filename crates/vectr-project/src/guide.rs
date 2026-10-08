//! The authoring guide a scaffold writes into a new project (FEAT-020).
//!
//! Coding agents load a project's `AGENTS.md` without bespoke setup, so the
//! scaffold writes the authoring procedure there. The procedure is the agent
//! skill's reference, embedded verbatim at build time rather than restated here:
//! the skill is the single source of the guide, so an edit to it reaches the
//! scaffold without a second copy drifting (FEAT-020).
//!
//! Embedding rather than reading from disk keeps the scaffold honest: `vectr
//! init` writes the same guide whether it runs from a checkout or an installed
//! binary, and it never depends on the skill package being present.

/// The file the scaffold writes the guide to, at the project root.
pub const AUTHORING_GUIDE_FILE: &str = "AGENTS.md";

/// The authoring guide the scaffold writes, embedded from the agent skill.
///
/// The skill's reference names the tool and format versions it targets, so a
/// guide embedded into a project always tells an agent which contract it was
/// written for (FEAT-020).
pub fn authoring_guide() -> &'static str {
    include_str!("../../../skills/vectr/references/authoring-guide.md")
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectr_core::scene::CURRENT_FORMAT_VERSION;

    #[test]
    fn the_guide_is_the_full_authoring_procedure() {
        let guide = authoring_guide();
        for command in [
            "vectr schema",
            "vectr validate",
            "vectr compile",
            "vectr export",
        ] {
            assert!(guide.contains(command), "the guide teaches `{command}`");
        }
        // The inspect-and-correct loop and the bounded retry are the behaviour
        // FEAT-020's acceptance criteria and edge cases turn on.
        assert!(guide.contains("Inspect and correct"), "teaches inspection");
        assert!(guide.contains("Retry once"), "bounds the retry");
        assert!(guide.contains("Defaults for an ambiguous request"));
        assert!(guide.contains("Licensing"));
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
}
