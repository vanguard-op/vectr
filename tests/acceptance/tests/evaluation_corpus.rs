//! Acceptance tests for the evaluation corpus (FEAT-023).
//!
//! The harness's judged half — compile success, fidelity, and the hard-end
//! rubric — is measured by running the corpus against a model. These checks
//! pin the deterministic half the harness depends on: the corpus is versioned,
//! its prompts conform to the corpus contract the harness loads, the coverage
//! classification marks every prompt and none is unclassified, and the corpus
//! spans the complexity range including the complex end built from reusable
//! parts and the four hard-end qualities the guidance directs (FEAT-032). A
//! corpus that omits part of the range or a hard-end quality does not meet its
//! coverage requirement, and the harness reports the omission; this test fails
//! first so the omission never ships.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use common::workspace_root;
use serde_json::Value;

/// The corpus directory shipped with the repository.
fn corpus_dir() -> PathBuf {
    workspace_root().join("corpus")
}

/// The manifest file name the harness loads (`crates/vectr-eval`).
const MANIFEST_FILE: &str = "corpus.json";

/// The prompt directory the harness loads.
const PROMPT_DIR: &str = "prompts";

/// The `complexity` value that marks the complex end of the range.
const COMPLEX: &str = "complex";

/// The hard-end qualities NFR-031 scores, as the harness names them.
const HARD_END: [&str; 4] = [
    "depth",
    "relative_placement",
    "shared_anchors",
    "subject_accuracy",
];

/// The FEAT-023 coverage that the complex end is built from reusable parts.
const REUSABLE_PARTS: &str = "reusable_parts";

/// Every allowed `complexity` value.
const COMPLEXITIES: [&str; 3] = ["simple", "moderate", COMPLEX];

/// Every allowed `qualities` value: the four hard-end qualities plus the
/// reusable-parts coverage.
const QUALITIES: [&str; 5] = [
    "depth",
    "relative_placement",
    "shared_anchors",
    "subject_accuracy",
    REUSABLE_PARTS,
];

fn read_json(path: &PathBuf) -> Value {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not JSON: {error}", path.display()))
}

/// The manifest as an object, with its required string fields asserted.
fn manifest() -> Value {
    let value = read_json(&corpus_dir().join(MANIFEST_FILE));
    for field in ["id", "version", "description"] {
        assert!(
            value[field].as_str().is_some_and(|text| !text.is_empty()),
            "the manifest is missing a non-empty `{field}`"
        );
    }
    value
}

/// Every prompt document, keyed by its identifier.
fn prompts() -> BTreeMap<String, Value> {
    let dir = corpus_dir().join(PROMPT_DIR);
    let mut prompts = BTreeMap::new();
    let entries = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let value = read_json(&path);
        let id = value["id"]
            .as_str()
            .unwrap_or_else(|| panic!("{} has no string id", path.display()))
            .to_string();
        assert!(
            prompts.insert(id.clone(), value).is_none(),
            "duplicate prompt id `{id}`"
        );
    }
    prompts
}

/// The `qualities` array of a prompt or a coverage entry, as a set.
fn qualities(value: &Value) -> BTreeSet<&str> {
    value["qualities"]
        .as_array()
        .unwrap_or_else(|| panic!("`{}` needs a qualities array", value["id"]))
        .iter()
        .filter_map(Value::as_str)
        .collect()
}

#[test]
fn the_corpus_is_versioned() {
    let manifest = manifest();
    assert!(
        manifest["version"].as_str().is_some_and(|v| !v.is_empty()),
        "a corpus is versioned so comparisons stay meaningful"
    );
}

#[test]
fn every_prompt_conforms_to_the_corpus_contract() {
    let manifest = manifest();
    let corpus_id = manifest["id"].as_str().expect("a corpus id");
    let prompts = prompts();
    assert!(!prompts.is_empty(), "the corpus holds at least one prompt");

    let mut orders: BTreeSet<i64> = BTreeSet::new();
    for (id, prompt) in &prompts {
        // The published Prompt fields (`docs/Vectr/schema.md`), including the
        // coverage classification (`complexity`, `qualities`). No other field
        // is expected.
        let fields: BTreeSet<&str> = prompt
            .as_object()
            .expect("a prompt is an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            fields,
            BTreeSet::from([
                "id",
                "corpusId",
                "intent",
                "text",
                "order",
                "complexity",
                "qualities"
            ]),
            "prompt `{id}` must carry the corpus contract's fields"
        );
        assert_eq!(prompt["corpusId"], corpus_id, "prompt `{id}` corpusId");
        assert!(
            prompt["intent"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "prompt `{id}` needs a non-empty intent"
        );
        assert!(
            prompt["text"].as_str().is_some_and(|text| !text.is_empty()),
            "prompt `{id}` needs a non-empty text"
        );
        let order = prompt["order"]
            .as_i64()
            .unwrap_or_else(|| panic!("prompt `{id}` needs an integer order"));
        assert!(orders.insert(order), "two prompts share order {order}");

        let complexity = prompt["complexity"]
            .as_str()
            .unwrap_or_else(|| panic!("prompt `{id}` needs a complexity"));
        assert!(
            COMPLEXITIES.contains(&complexity),
            "prompt `{id}` names an unknown complexity `{complexity}`"
        );
        for quality in qualities(prompt) {
            assert!(
                QUALITIES.contains(&quality),
                "prompt `{id}` names an unknown quality `{quality}`"
            );
        }
    }

    // Orders are contiguous from zero, so processing in order visits every
    // prompt and none is silently skipped.
    let expected: BTreeSet<i64> = (0..prompts.len() as i64).collect();
    assert_eq!(
        orders, expected,
        "prompt orders must be contiguous from zero"
    );
}

#[test]
fn the_corpus_spans_the_complexity_range() {
    let prompts = prompts();

    // The full range is present: the simple end, the complex end, and the
    // tier between them.
    let complexities: BTreeSet<&str> = prompts
        .values()
        .filter_map(|prompt| prompt["complexity"].as_str())
        .collect();
    for expected in COMPLEXITIES {
        assert!(
            complexities.contains(expected),
            "the corpus omits the `{expected}` part of the range"
        );
    }

    let complex: Vec<&Value> = prompts
        .values()
        .filter(|prompt| prompt["complexity"].as_str() == Some(COMPLEX))
        .collect();
    assert!(
        !complex.is_empty(),
        "the corpus omits the complex end of the range"
    );

    // The complex end is built up from reusable parts (FEAT-023, FEAT-030).
    for prompt in &complex {
        let id = prompt["id"].as_str().expect("a prompt id");
        assert!(
            qualities(prompt).contains(REUSABLE_PARTS),
            "complex-end prompt `{id}` does not cover `{REUSABLE_PARTS}`"
        );
    }

    // The corpus is large enough to span the range.
    assert!(
        prompts.len() >= 90,
        "the corpus holds {} prompts, below the ~100 the bar is measured over",
        prompts.len()
    );
}

#[test]
fn the_complex_end_covers_every_hard_end_quality() {
    let prompts = prompts();
    for quality in HARD_END {
        let covered = prompts.values().any(|prompt| {
            prompt["complexity"].as_str() == Some(COMPLEX) && qualities(prompt).contains(quality)
        });
        assert!(
            covered,
            "the complex end omits the hard-end quality `{quality}`, so the corpus does not meet its coverage requirement"
        );
    }
}

#[test]
fn a_prompt_carries_its_coverage_classification() {
    let prompts = prompts();
    // No prompt is silently unclassified: every prompt names a complexity and
    // every complex-end prompt exercises at least one quality.
    for (id, prompt) in &prompts {
        assert!(
            prompt["complexity"].as_str().is_some(),
            "prompt `{id}` is unclassified"
        );
        if prompt["complexity"].as_str() == Some(COMPLEX) {
            assert!(
                !qualities(prompt).is_empty(),
                "complex-end prompt `{id}` exercises no hard-end quality"
            );
        }
    }
}
