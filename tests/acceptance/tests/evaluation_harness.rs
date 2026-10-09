//! Acceptance tests for the evaluation harness's published contract (FEAT-023,
//! C-006).
//!
//! FEAT-023 requires a completed run to report the median per-prompt token
//! count alongside compile success and fidelity, and C-006 fixes that as the
//! run record's `medianTokens`. The `schema` command prints the published
//! language contract (`docs/Vectr/schema.md`, "EvaluationRun"), so a build whose
//! contract omits the field the run record must carry has drifted from the
//! documented record. This pins the contract surface; the harness's own tests
//! cover the judged half.

mod common;

use common::{code, run_vectr, stderr, stdout, TempDir};
use serde_json::Value;

#[test]
fn the_published_contract_declares_the_run_records_median_token_field() {
    let dir = TempDir::new("eval-run-contract");
    let output = run_vectr(dir.path(), &["schema", "--type", "EvaluationRun"]);
    assert_eq!(code(&output), 0, "the schema prints: {}", stderr(&output));

    let schema: Value = serde_json::from_str(&stdout(&output)).expect("the schema is JSON");
    let properties = &schema["$defs"]["EvaluationRun"]["properties"];
    assert!(
        properties.get("medianTokens").is_some(),
        "the published contract omits the run record's documented `medianTokens` field: {properties}"
    );
}
