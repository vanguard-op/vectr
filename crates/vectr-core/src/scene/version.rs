//! Scene format versioning and the format-version gate.
//!
//! A scene declares its format as `major.minor` (docs/Vectr/schema.md). The
//! tool accepts the versions it knows and refuses anything else, naming both
//! the declared version and the supported range (FEAT-001, user-flow.md
//! "Global Flows → Format-Version Gate").

/// The format version this build writes and reads by default.
pub const CURRENT_FORMAT_VERSION: &str = "0.2";

/// Lowest format version this build accepts, inclusive.
pub const MIN_SUPPORTED_FORMAT_VERSION: &str = "0.2";

/// Highest format version this build accepts, inclusive.
pub const MAX_SUPPORTED_FORMAT_VERSION: &str = "0.2";

/// Splits a `major.minor` version string into its numeric parts.
///
/// Returns `None` when the text does not match the schema pattern
/// `^[0-9]+\.[0-9]+$`; callers report that as an invalid value rather than an
/// unsupported one.
pub fn parse_version(version: &str) -> Option<(u32, u32)> {
    let (major, minor) = version.split_once('.')?;
    if major.is_empty() || minor.is_empty() {
        return None;
    }
    if !major.bytes().all(|b| b.is_ascii_digit()) || !minor.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Whether a well-formed `major.minor` version is inside the supported range.
pub fn is_supported_version(version: (u32, u32)) -> bool {
    match (
        parse_version(MIN_SUPPORTED_FORMAT_VERSION),
        parse_version(MAX_SUPPORTED_FORMAT_VERSION),
    ) {
        (Some(min), Some(max)) => version >= min && version <= max,
        _ => false,
    }
}

/// Convenience test for a raw version string: well-formed and supported.
pub fn is_supported(version: &str) -> bool {
    parse_version(version).is_some_and(is_supported_version)
}

/// The supported range, phrased for a diagnostic ("`0.1`" for a single
/// version, "`0.1` through `0.4`" for a span).
pub fn supported_range() -> String {
    if MIN_SUPPORTED_FORMAT_VERSION == MAX_SUPPORTED_FORMAT_VERSION {
        format!("`{MIN_SUPPORTED_FORMAT_VERSION}`")
    } else {
        format!("`{MIN_SUPPORTED_FORMAT_VERSION}` through `{MAX_SUPPORTED_FORMAT_VERSION}`")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_well_formed_versions() {
        assert_eq!(parse_version("0.1"), Some((0, 1)));
        assert_eq!(parse_version("12.34"), Some((12, 34)));
    }

    #[test]
    fn rejects_malformed_versions() {
        for bad in ["", "0", "0.", ".1", "v1.0", "1.2.3", "-1.0", "1.0 "] {
            assert_eq!(parse_version(bad), None, "expected {bad:?} to be malformed");
        }
    }

    #[test]
    fn current_version_is_supported() {
        assert!(is_supported(CURRENT_FORMAT_VERSION));
    }

    #[test]
    fn versions_outside_the_range_are_unsupported() {
        assert!(is_supported("0.2"));
        assert!(!is_supported("0.1"));
        assert!(!is_supported("1.0"));
        assert!(!is_supported("9.9"));
    }
}
