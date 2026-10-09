//! Acceptance tests for the evaluation harness's shipped contract (FEAT-023,
//! C-006).
//!
//! FEAT-023 requires a completed run to report the median per-prompt token
//! count alongside compile success and fidelity, and C-006 fixes that as the
//! run record's `medianTokens`. The `schema` command prints the published
//! language contract (`docs/Vectr/schema.md`, "EvaluationRun"), so a build whose
//! contract omits the field the run record must carry has drifted from the
//! documented record. The `run` and `compare` commands are driven as
//! subprocesses over the shipped versioned corpus with the offline `replay`
//! provider, so the whole contract — the run record, the median over the
//! prompts that reported usage, the gate, and the regression comparison — is
//! exercised end to end without a network call (C-006).

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{code, run_vectr, run_vectr_eval_env, stderr, stdout, workspace_root, TempDir};
use serde_json::Value;

/// The environment variable that enables the maintainer-only harness (C-006).
const ENABLE: &str = "VECTR_ENABLE_EVAL_HARNESS";

/// The corpus directory shipped with the repository.
fn corpus_dir() -> PathBuf {
    workspace_root().join("corpus")
}

/// Every prompt identifier in the shipped corpus, in file order.
fn prompt_ids() -> Vec<String> {
    let dir = corpus_dir().join("prompts");
    let mut ids: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", dir.display()))
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .map(|path| {
            let text = fs::read_to_string(&path).expect("a prompt document");
            let value: Value = serde_json::from_str(&text).expect("a prompt is JSON");
            value["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{} has no string id", path.display()))
                .to_string()
        })
        .collect();
    ids.sort();
    ids
}

/// A scene every replay recording returns: a minimal valid document that
/// compiles through the toolchain's evaluation project.
const REPLAY_SCENE: &str = r##"{"scene":{"id":"s","projectId":"eval","name":"S","formatVersion":"0.2","canvas":{"width":100,"height":100,"background":"#ffffff"},"elements":[]}}"##;

/// A judge verdict marking every hard-end quality true at the given fidelity.
fn judge_reply(fidelity: f64) -> String {
    format!(
        r#"{{"fidelity": {fidelity}, "qualities": {{"depth": true, "relative_placement": true, "shared_anchors": true, "subject_accuracy": true}}}}"#
    )
}

/// A directory of recorded replies covering every prompt in the shipped corpus.
///
/// When `tokens` is given, every prompt records that usage; when it is `None`,
/// no prompt records any usage, so the run reports no median (FEAT-023).
fn replay_recordings(tag: &str, fidelity: f64, tokens: Option<u64>) -> TempDir {
    let dir = TempDir::new(tag);
    for id in prompt_ids() {
        dir.write(&format!("{id}.scene.json"), REPLAY_SCENE);
        dir.write(&format!("{id}.judge.json"), &judge_reply(fidelity));
        if let Some(tokens) = tokens {
            dir.write(
                &format!("{id}.usage.json"),
                &format!("{{\"tokens\": {tokens}}}"),
            );
        }
    }
    dir
}

/// Runs the harness over the shipped corpus with the replay provider.
fn run_harness(corpus: &Path, replay: &Path, out: &str) -> std::process::Output {
    run_vectr_eval_env(
        &workspace_root(),
        &[
            "run",
            "--corpus",
            &corpus.display().to_string(),
            "--model",
            &format!("replay:{}", replay.display()),
            "--out",
            out,
        ],
        &[],
        &[(ENABLE, "1")],
    )
}

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

#[test]
fn the_harness_is_gated_off_by_default_and_writes_no_record() {
    let replay = replay_recordings("eval-gated-replay", 0.9, Some(1_000));
    let out = replay.path().join("run.json");
    let output = run_vectr_eval_env(
        &workspace_root(),
        &[
            "run",
            "--corpus",
            &corpus_dir().display().to_string(),
            "--model",
            &format!("replay:{}", replay.path().display()),
            "--out",
            &out.display().to_string(),
        ],
        &[ENABLE],
        &[],
    );
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("disabled"),
        "the gate is reported: {}",
        stderr(&output)
    );
    assert!(!out.exists(), "nothing is written while the harness is off");
}

#[test]
fn a_completed_run_reports_the_median_token_count_over_the_shipped_corpus() {
    let corpus = corpus_dir();
    let ids = prompt_ids();
    let replay = replay_recordings("eval-median", 0.9, Some(9_000));
    let out = replay.path().join("run.json");

    let output = run_harness(&corpus, replay.path(), &out.display().to_string());
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let record: Value =
        serde_json::from_str(&fs::read_to_string(&out).expect("the run record")).expect("JSON");
    assert_eq!(
        record["medianTokens"], 9_000,
        "the median of the reported usage is carried: {record}"
    );
    assert_eq!(record["compileSuccessRate"], 1.0, "{record}");
    assert!(
        (record["fidelityScore"].as_f64().unwrap_or(0.0) - 0.9).abs() < 1e-9,
        "the judged fidelity is averaged over every prompt: {record}"
    );
    assert_eq!(
        record["prompts"].as_array().map(Vec::len),
        Some(ids.len()),
        "every prompt is scored, none skipped (FEAT-023)"
    );
    assert!(
        record["thresholds"]["compileSuccess"]
            .as_bool()
            .unwrap_or(false),
        "compile success clears its bar: {record}"
    );
    assert!(
        record["thresholds"]["tokens"].as_bool().unwrap_or(false),
        "the median is within the token budget: {record}"
    );
    assert!(
        record["coverage"]["satisfied"].as_bool().unwrap_or(false),
        "the corpus meets its coverage requirement: {record}"
    );

    // The summary names the median against the documented budget.
    assert!(
        stdout(&output).contains("median tokens 9000"),
        "{}",
        stdout(&output)
    );
    assert!(stdout(&output).contains("20000"), "{}", stdout(&output));
}

#[test]
fn a_repeated_run_of_the_same_corpus_is_byte_identical() {
    let corpus = corpus_dir();
    let replay = replay_recordings("eval-determinism", 0.9, Some(4_000));
    let first = replay.path().join("first.json");
    let second = replay.path().join("second.json");

    let one = run_harness(&corpus, replay.path(), &first.display().to_string());
    assert_eq!(code(&one), 0, "{}", stderr(&one));
    let two = run_harness(&corpus, replay.path(), &second.display().to_string());
    assert_eq!(code(&two), 0, "{}", stderr(&two));

    assert_eq!(
        fs::read(&first).expect("the first record"),
        fs::read(&second).expect("the second record"),
        "the same corpus and the same model output produce byte-identical records (FEAT-023, NFR-010)"
    );
}

#[test]
fn a_run_whose_provider_reports_no_usage_reports_no_median() {
    let corpus = corpus_dir();
    let replay = replay_recordings("eval-no-usage", 0.9, None);
    let out = replay.path().join("run.json");

    let output = run_harness(&corpus, replay.path(), &out.display().to_string());
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let record: Value =
        serde_json::from_str(&fs::read_to_string(&out).expect("the run record")).expect("JSON");
    assert!(
        record.get("medianTokens").is_none(),
        "no reported usage means no median, not a fabricated one (FEAT-023): {record}"
    );
    assert!(
        stdout(&output).contains("none reported"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn comparing_two_runs_flags_a_regressed_prompt() {
    let corpus = corpus_dir();
    let ids = prompt_ids();
    let replay = replay_recordings("eval-compare", 0.9, Some(2_000));
    let baseline = replay.path().join("baseline.json");
    let candidate = replay.path().join("candidate.json");

    let first = run_harness(&corpus, replay.path(), &baseline.display().to_string());
    assert_eq!(code(&first), 0, "{}", stderr(&first));

    // The candidate drops one prompt's fidelity, so the comparison names it.
    let regressed = &ids[0];
    replay.write(&format!("{regressed}.judge.json"), &judge_reply(0.2));
    let second = run_harness(&corpus, replay.path(), &candidate.display().to_string());
    assert_eq!(code(&second), 0, "{}", stderr(&second));

    let comparison = run_vectr_eval_env(
        &workspace_root(),
        &[
            "compare",
            &baseline.display().to_string(),
            &candidate.display().to_string(),
        ],
        &[],
        &[(ENABLE, "1")],
    );
    assert_eq!(code(&comparison), 0, "{}", stderr(&comparison));
    let value: Value = serde_json::from_str(&stdout(&comparison)).expect("the comparison is JSON");
    assert!(
        value["regression"].as_bool().unwrap_or(false),
        "the regression is flagged: {value}"
    );
    assert!(
        value["regressions"].as_array().is_some_and(|regressions| {
            regressions
                .iter()
                .any(|regression| regression["promptId"] == *regressed)
        }),
        "the regressed prompt is named: {value}"
    );
}
