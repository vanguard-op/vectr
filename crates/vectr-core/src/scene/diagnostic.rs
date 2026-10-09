//! Structured, located findings shared by every engine stage (C-002).
//!
//! A [`Diagnostic`] carries a severity, a stable [`DiagnosticCode`], a
//! human-readable message, and — where it applies — a [`Location`] naming the
//! element and JSON path a scene author must edit (NFR-011). [`Diagnostics`]
//! is the collection the library returns.

use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Whether a finding blocks the pipeline or merely warns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The stage failed; nothing downstream runs and no output is written.
    Error,
    /// The stage succeeded but something is worth reporting.
    Warning,
}

impl Severity {
    /// The lowercase word used in rendered output.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A stable, machine-readable code such as `E_SCHEMA`.
///
/// Codes are intentionally open: later stages add their own without changing
/// this type, and callers can compare against the associated constants. A code
/// built from a literal borrows it; one read back from JSON owns its text, so
/// the type stays open without leaking or capping the vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiagnosticCode(Cow<'static, str>);

impl DiagnosticCode {
    /// The document is not valid JSON, or its top level is not a scene.
    pub const PARSE: Self = Self::new("E_PARSE");
    /// A property is unknown, a required field is missing, or a value is invalid.
    pub const SCHEMA: Self = Self::new("E_SCHEMA");
    /// The declared format version is well-formed but not supported.
    pub const FORMAT_VERSION: Self = Self::new("E_FORMAT_VERSION");
    /// Two elements share an identifier.
    pub const DUPLICATE_ID: Self = Self::new("E_DUPLICATE_ID");
    /// Two elements share an accessible name (FEAT-026).
    pub const DUPLICATE_NAME: Self = Self::new("E_DUPLICATE_NAME");
    /// The document exceeds the parser's defined input size limit.
    pub const SIZE_LIMIT: Self = Self::new("E_SIZE_LIMIT");

    /// Builds a code from a literal; intended for stage owners defining new
    /// codes next to their logic.
    pub const fn new(code: &'static str) -> Self {
        Self(Cow::Borrowed(code))
    }

    /// The code's text, e.g. `E_SCHEMA`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for DiagnosticCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DiagnosticCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(Cow::Owned(String::deserialize(deserializer)?)))
    }
}

/// Where a finding applies.
///
/// A parse failure knows its line and column; a semantic failure knows the
/// element and the JSON path a human would edit. All fields are optional
/// because a stage may only have some of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    /// JSON pointer to the offending value, e.g. `/elements/2/opacity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_path: Option<String>,
    /// Stable identifier of the offending element, when one is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    /// One-based line in the source document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// One-based column in the source document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

impl Location {
    /// A location at a JSON pointer.
    pub fn path(path: impl Into<String>) -> Self {
        Self {
            json_path: Some(path.into()),
            ..Self::default()
        }
    }

    /// A location at an element, addressed by its stable identifier.
    pub fn element(id: impl Into<String>) -> Self {
        Self {
            element_id: Some(id.into()),
            ..Self::default()
        }
    }

    /// A location at an element and a JSON pointer within it.
    pub fn element_at(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            element_id: Some(id.into()),
            json_path: Some(path.into()),
            ..Self::default()
        }
    }

    /// A location at a line and column in the source text.
    pub fn line_column(line: usize, column: usize) -> Self {
        Self {
            line: Some(line),
            column: Some(column),
            ..Self::default()
        }
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(id) = &self.element_id {
            parts.push(format!("element `{id}`"));
        }
        if let Some(path) = &self.json_path {
            parts.push(format!("at {path}"));
        }
        if let (Some(line), Some(column)) = (self.line, self.column) {
            parts.push(format!("at line {line} column {column}"));
        } else if let Some(line) = self.line {
            parts.push(format!("at line {line}"));
        }
        parts.join(" ")
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// One structured finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    /// Whether the finding blocks the pipeline.
    pub severity: Severity,
    /// Stable machine-readable code.
    pub code: DiagnosticCode,
    /// Human-readable explanation.
    pub message: String,
    /// Where the finding applies, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

impl Diagnostic {
    /// An error with the given code and message.
    pub fn error(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            location: None,
        }
    }

    /// A warning with the given code and message.
    pub fn warning(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            code,
            message: message.into(),
            location: None,
        }
    }

    /// Attaches a location.
    pub fn with_location(mut self, location: Location) -> Self {
        self.location = Some(location);
        self
    }

    /// Attaches a JSON-pointer location.
    pub fn at_path(self, path: impl Into<String>) -> Self {
        self.with_location(Location::path(path))
    }

    /// Attaches an element-identifier location.
    pub fn for_element(self, id: impl Into<String>) -> Self {
        self.with_location(Location::element(id))
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]: {}", self.severity, self.code, self.message)?;
        if let Some(location) = &self.location {
            write!(f, " at {location}")?;
        }
        Ok(())
    }
}

/// An ordered collection of findings.
///
/// Order is the order findings were produced, so a given input always yields
/// identical diagnostics (NFR-010).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Diagnostics(Vec<Diagnostic>);

impl Diagnostics {
    /// An empty collection.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Whether the collection holds nothing.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many findings the collection holds.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether any finding is an error.
    pub fn has_errors(&self) -> bool {
        self.0.iter().any(|d| d.severity == Severity::Error)
    }

    /// All findings, in order.
    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.0.iter()
    }

    /// Errors only, in order.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter().filter(|d| d.severity == Severity::Error)
    }

    /// Warnings only, in order.
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter().filter(|d| d.severity == Severity::Warning)
    }

    /// Appends a finding.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.0.push(diagnostic);
    }

    /// Appends every finding from another collection.
    pub fn extend(&mut self, other: Diagnostics) {
        self.0.extend(other.0);
    }
}

impl From<Diagnostic> for Diagnostics {
    fn from(diagnostic: Diagnostic) -> Self {
        Self(vec![diagnostic])
    }
}

impl From<Vec<Diagnostic>> for Diagnostics {
    fn from(diagnostics: Vec<Diagnostic>) -> Self {
        Self(diagnostics)
    }
}

impl FromIterator<Diagnostic> for Diagnostics {
    fn from_iter<T: IntoIterator<Item = Diagnostic>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a Diagnostics {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, diagnostic) in self.0.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{diagnostic}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_serializes_as_its_text() {
        assert_eq!(
            serde_json::to_string(&DiagnosticCode::SCHEMA).unwrap(),
            "\"E_SCHEMA\""
        );
    }

    #[test]
    fn an_open_code_round_trips_without_a_registry() {
        let code: DiagnosticCode =
            serde_json::from_str("\"E_FROM_A_LATER_STAGE\"").expect("open codes are accepted");
        assert_eq!(code.as_str(), "E_FROM_A_LATER_STAGE");
        assert_eq!(
            serde_json::to_string(&code).unwrap(),
            "\"E_FROM_A_LATER_STAGE\""
        );
    }

    #[test]
    fn a_diagnostic_round_trips_with_its_location() {
        let diagnostic = Diagnostic::warning(DiagnosticCode::new("W_X"), "careful")
            .with_location(Location::element_at("e1", "/fill"));
        let text = serde_json::to_string(&diagnostic).unwrap();
        assert!(text.contains("\"severity\":\"warning\""), "{text}");
        assert!(text.contains("\"elementId\":\"e1\""), "{text}");
        assert!(text.contains("\"jsonPath\":\"/fill\""), "{text}");
        let reparsed: Diagnostic = serde_json::from_str(&text).unwrap();
        assert_eq!(diagnostic, reparsed);
    }
}
