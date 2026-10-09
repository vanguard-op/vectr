//! The authoring guide a scaffold writes into a new project (FEAT-020).
//!
//! Coding agents load a project's `AGENTS.md` without bespoke setup, so the
//! scaffold writes the authoring procedure there. The procedure is the agent
//! skill's reference, carried inside the crate at
//! `references/authoring-guide.md` and embedded verbatim at build time rather
//! than restated in Rust: the skill remains the single source of the guide, and
//! a test keeps the embedded copy byte-identical to it (FEAT-020).
//!
//! Embedding rather than reading from disk keeps the scaffold honest: `vectr
//! init` writes the same guide whether it runs from a checkout or an installed
//! binary, and it never depends on the skill package being present. The copy
//! lives under the crate so `cargo package` and a standalone build never reach
//! outside the package.

/// The file the scaffold writes the guide to, at the project root.
pub const AUTHORING_GUIDE_FILE: &str = "AGENTS.md";

/// The authoring guide the scaffold writes, embedded from the copy the crate
/// carries.
///
/// The skill's reference names the tool and format versions it targets, so a
/// guide embedded into a project always tells an agent which contract it was
/// written for (FEAT-020).
pub fn authoring_guide() -> &'static str {
    include_str!("../references/authoring-guide.md")
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

    #[test]
    fn the_embedded_guide_matches_the_skill_reference() {
        // The crate carries its own copy so it packages standalone; this keeps
        // the copy from drifting from the skill's reference.
        let reference = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../skills/vectr/references/authoring-guide.md");
        let Ok(reference) = std::fs::read_to_string(&reference) else {
            // A packaged crate has no repository around it; nothing to compare.
            return;
        };
        assert_eq!(
            authoring_guide(),
            reference,
            "the embedded guide has drifted"
        );
    }
}
