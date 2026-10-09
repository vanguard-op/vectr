//! The language contract: the complete scene schema and per-type discovery
//! (FEAT-017, C-002).
//!
//! [`schema`] prints the whole machine-readable contract — the Scene document
//! and every entity it references — in a full or compact form; [`schema_for`]
//! prints one named type's properties and allowed values. The contract is
//! carried inside the crate at `schema/vectr.schema.json` and embedded into the
//! binary at build time, so discovery works with no filesystem access (NFR-021)
//! and is byte-identical run to run (NFR-010). The copy travels with the crate,
//! so a packaged or standalone build never reaches outside the package; the
//! repository's `schema/vectr.schema.json` remains the published artifact, and a
//! test keeps the two byte-identical.
//!
//! An unknown type is a located error naming the types closest to it; a
//! published artifact whose declared format version does not match this build
//! is refused rather than served stale, so a model never authors against a
//! contract the tool will reject.

use std::sync::OnceLock;

use serde_json::{Map, Value};

use crate::scene::{Diagnostic, DiagnosticCode, Diagnostics, CURRENT_FORMAT_VERSION};

/// The language contract carried inside the crate, embedded at build time.
///
/// The copy lives under the crate so `cargo package` and a standalone build
/// never reach outside the package; the repository's `schema/vectr.schema.json`
/// is the published artifact, and a test keeps the two byte-identical.
const CONTRACT: &str = include_str!("../../schema/vectr.schema.json");

/// The complete contract, or a single type, as full or compact JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaForm {
    /// Indented JSON with every description, for a person or a model to read.
    Full,
    /// Minified JSON with no insignificant whitespace, for machine consumption.
    Compact,
}

/// The requested type is not part of the language contract.
pub const UNKNOWN_SCHEMA_TYPE: DiagnosticCode = DiagnosticCode::new("E_SCHEMA_TYPE");

/// The published schema declares a format version this build does not support.
pub const SCHEMA_VERSION: DiagnosticCode = DiagnosticCode::new("E_SCHEMA_VERSION");

/// The whole language contract in the requested form.
pub fn schema(form: SchemaForm) -> Result<String, Diagnostics> {
    render(document()?, form)
}

/// One named type's schema, with every description, properties, and allowed
/// values, wrapped so it stands alone with its `$defs`.
///
/// The name is matched case-insensitively; an unknown name is refused with the
/// closest type names.
pub fn schema_for(name: &str) -> Result<String, Diagnostics> {
    let document = document()?;
    let canonical = resolve(name, document)?;
    let definitions = document
        .get("$defs")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));

    let mut output = Map::new();
    output.insert(
        "$schema".to_string(),
        Value::String("https://json-schema.org/draft/2020-12/schema".to_string()),
    );
    output.insert(
        "$id".to_string(),
        Value::String(format!(
            "https://vectr.dev/schema/{}.schema.json",
            kebab(canonical)
        )),
    );
    output.insert("title".to_string(), Value::String(canonical.to_string()));
    if let Some(description) = definitions
        .get(canonical)
        .and_then(|definition| definition.get("description"))
        .cloned()
    {
        output.insert("description".to_string(), description);
    }
    output.insert(
        "x-vectr-formatVersion".to_string(),
        Value::String(CURRENT_FORMAT_VERSION.to_string()),
    );
    output.insert(
        "$ref".to_string(),
        Value::String(format!("#/$defs/{canonical}")),
    );
    output.insert("$defs".to_string(), definitions);

    render(&Value::Object(output), SchemaForm::Full)
}

/// The complete, embedded contract, parsed once and verified against this
/// build's format version.
fn document() -> Result<&'static Value, Diagnostics> {
    static DOCUMENT: OnceLock<Result<Value, Diagnostics>> = OnceLock::new();
    match DOCUMENT.get_or_init(|| {
        let value: Value = serde_json::from_str(CONTRACT).map_err(|error| {
            Diagnostics::from(Diagnostic::error(
                DiagnosticCode::SCHEMA,
                format!("the published schema is not valid JSON: {error}"),
            ))
        })?;
        check_version(&value)?;
        Ok(value)
    }) {
        Ok(value) => Ok(value),
        Err(diagnostics) => Err(diagnostics.clone()),
    }
}

/// Refuses a contract whose declared format version is not this build's.
fn check_version(document: &Value) -> Result<(), Diagnostics> {
    let declared = document
        .get("x-vectr-formatVersion")
        .and_then(Value::as_str)
        .unwrap_or("<none>");
    if declared != CURRENT_FORMAT_VERSION {
        return Err(Diagnostics::from(Diagnostic::error(
            SCHEMA_VERSION,
            format!(
                "the published schema declares format version `{declared}`, but this build supports `{CURRENT_FORMAT_VERSION}`"
            ),
        )));
    }
    Ok(())
}

/// Resolves a requested type name to its canonical spelling, or reports the
/// closest names.
fn resolve<'a>(name: &str, document: &'a Value) -> Result<&'a str, Diagnostics> {
    let types = type_names(document);
    if let Some(canonical) = types
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
    {
        return Ok(canonical);
    }

    let suggestions = similar_types(name, &types);
    let listed = match suggestions.as_slice() {
        [] => "no types are available".to_string(),
        [only] => format!("`{only}`"),
        [head @ .., last] => {
            let head = head
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{head}, or `{last}`")
        }
    };
    Err(Diagnostics::from(Diagnostic::error(
        UNKNOWN_SCHEMA_TYPE,
        format!("unknown schema type `{name}`; similar types: {listed}"),
    )))
}

/// The canonical type names, in the order the contract declares them.
fn type_names(document: &Value) -> Vec<&str> {
    document
        .get("x-vectr-types")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The types closest to a requested name, nearest first.
fn similar_types(name: &str, types: &[&str]) -> Vec<String> {
    let lowered = name.to_lowercase();
    let mut ranked: Vec<(usize, &str)> = types
        .iter()
        .map(|candidate| {
            (
                edit_distance(&lowered, &candidate.to_lowercase()),
                *candidate,
            )
        })
        .collect();
    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)));

    let close: Vec<&str> = {
        let threshold = (name.chars().count() / 3).max(1);
        ranked
            .iter()
            .filter(|(distance, _)| *distance <= threshold)
            .map(|(_, candidate)| *candidate)
            .collect()
    };
    let chosen: Vec<&str> = if close.is_empty() {
        ranked.iter().take(3).map(|(_, c)| *c).collect()
    } else {
        close
    };
    chosen.into_iter().take(3).map(str::to_string).collect()
}

/// The Levenshtein edit distance between two strings.
fn edit_distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0usize; right.len() + 1];

    for (i, left_char) in left.chars().enumerate() {
        current[0] = i + 1;
        for (j, right_char) in right.iter().enumerate() {
            let substitution = previous[j] + usize::from(left_char != *right_char);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

/// `ElementKind` becomes `element-kind`, the published file stem.
fn kebab(name: &str) -> String {
    let mut output = String::with_capacity(name.len() + 4);
    for (index, character) in name.chars().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            output.push('-');
        }
        output.push(character.to_ascii_lowercase());
    }
    output
}

/// Serializes a value as full or compact JSON.
fn render(value: &Value, form: SchemaForm) -> Result<String, Diagnostics> {
    let serialized = match form {
        SchemaForm::Full => serde_json::to_string_pretty(value),
        SchemaForm::Compact => serde_json::to_string(value),
    };
    serialized.map_err(|error| {
        Diagnostics::from(Diagnostic::error(
            DiagnosticCode::SCHEMA,
            format!("could not serialize the schema: {error}"),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).expect("the schema is valid JSON")
    }

    #[test]
    fn the_full_contract_describes_a_scene_and_every_entity() {
        let schema = parsed(&schema(SchemaForm::Full).expect("the contract renders"));
        assert_eq!(schema["$ref"], "#/$defs/Scene");
        let definitions = schema["$defs"].as_object().expect("definitions");
        for entity in [
            "Scene",
            "Element",
            "Definition",
            "Parameter",
            "Canvas",
            "Transform",
            "Paint",
            "Stroke",
            "Palette",
            "StrokeProfile",
            "StyleRecipe",
            "Gradient",
            "Constraint",
            "Asset",
        ] {
            assert!(definitions.contains_key(entity), "missing `{entity}`");
        }
    }

    #[test]
    fn the_element_kinds_match_the_language() {
        let schema = parsed(&schema(SchemaForm::Full).unwrap());
        let kinds: Vec<&str> = schema["$defs"]["ElementKind"]["enum"]
            .as_array()
            .expect("an enum")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            kinds,
            vec![
                "rect",
                "ellipse",
                "polygon",
                "line",
                "path",
                "text",
                "group",
                "repeat",
                "boolean",
                "alongPath",
                "offset",
                "projection",
                "raster",
                "instance"
            ]
        );
    }

    #[test]
    fn the_compact_form_is_the_full_contract_minified() {
        let full = schema(SchemaForm::Full).unwrap();
        let compact = schema(SchemaForm::Compact).unwrap();
        assert!(!compact.contains('\n'), "compact form has no line breaks");
        assert_eq!(parsed(&full), parsed(&compact));
    }

    #[test]
    fn a_type_request_carries_its_properties_and_allowed_values() {
        let element = parsed(&schema_for("Element").unwrap());
        assert_eq!(element["title"], "Element");
        assert_eq!(element["$ref"], "#/$defs/Element");
        assert!(element["$defs"]["Element"]["properties"]["geometry"].is_object());
        assert_eq!(element["$defs"]["ElementKind"]["enum"][0], "rect");

        let palette = parsed(&schema_for("Palette").unwrap());
        assert!(palette["$defs"]["PaletteToken"]["properties"]["value"].is_object());
    }

    #[test]
    fn a_type_name_matches_case_insensitively() {
        assert_eq!(parsed(&schema_for("element").unwrap())["title"], "Element");
        assert_eq!(parsed(&schema_for("PALETTE").unwrap())["title"], "Palette");
    }

    #[test]
    fn every_declared_type_resolves() {
        let document = document().expect("the contract loads");
        for name in type_names(document) {
            assert!(
                schema_for(name).is_ok(),
                "the declared type `{name}` does not resolve"
            );
        }
    }

    #[test]
    fn an_unknown_type_lists_similar_names() {
        let diagnostics = schema_for("Palete").expect_err("an unknown type");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, UNKNOWN_SCHEMA_TYPE);
        assert!(error.message.contains("Palette"), "{}", error.message);
        assert!(error.message.contains("Palete"), "{}", error.message);
    }

    #[test]
    fn a_schema_version_that_does_not_match_is_reported() {
        let mismatched = serde_json::json!({ "x-vectr-formatVersion": "9.9" });
        let diagnostics = check_version(&mismatched).expect_err("a mismatch");
        let error = diagnostics.errors().next().expect("an error");
        assert_eq!(error.code, SCHEMA_VERSION);
        assert!(error.message.contains("9.9"), "{}", error.message);
        assert!(
            error.message.contains(CURRENT_FORMAT_VERSION),
            "{}",
            error.message
        );
    }

    #[test]
    fn a_minimal_scene_built_from_the_schema_parses() {
        // The required fields the contract names are exactly the ones the parser
        // demands of an empty scene, so a model with only the schema can author
        // one that compiles (FEAT-017).
        let source = format!(
            r##"{{"id":"s","projectId":"p","name":"Minimal","formatVersion":"{CURRENT_FORMAT_VERSION}","canvas":{{"width":10,"height":10,"background":"#000000"}},"elements":[]}}"##
        );
        crate::scene::parse(&source).expect("a minimal scene parses");
    }

    #[test]
    fn a_minimal_element_built_from_the_schema_parses() {
        let source = format!(
            r#"{{"id":"s","projectId":"p","name":"Minimal","formatVersion":"{CURRENT_FORMAT_VERSION}","canvas":{{"width":10,"height":10,"background":"transparent"}},"elements":[{{"id":"e1","sceneId":"s","order":0,"kind":"ellipse","geometry":{{}},"transform":{{"translateX":0,"translateY":0,"rotate":0,"scaleX":1,"scaleY":1}},"opacity":1,"visible":true}}]}}"#
        );
        crate::scene::parse(&source).expect("a minimal element parses");
    }

    #[test]
    fn kebab_case_splits_a_pascal_name() {
        assert_eq!(kebab("Element"), "element");
        assert_eq!(kebab("StrokeProfile"), "stroke-profile");
        assert_eq!(kebab("EvaluationRun"), "evaluation-run");
    }

    #[test]
    fn the_embedded_contract_matches_the_published_artifact() {
        // The crate carries its own copy so it packages standalone; this keeps
        // the copy from drifting from the published `schema/vectr.schema.json`.
        let published =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/vectr.schema.json");
        let Ok(published) = std::fs::read_to_string(&published) else {
            // A packaged crate has no repository around it; nothing to compare.
            return;
        };
        assert_eq!(CONTRACT, published, "the embedded schema has drifted");
    }
}
