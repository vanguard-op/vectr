//! Running a corpus through a model and the toolchain (FEAT-023, C-006).
//!
//! The harness loads the versioned corpus, checks its coverage, then for each
//! prompt asks the authoring model for a scene, runs it through the toolchain,
//! and asks the judge for a fidelity score and the hard-end sub-scores. A model
//! that cannot be reached fails the whole run with no partial results; a
//! prompt whose reply cannot be scored is counted as a failure, never skipped
//! (FEAT-023, NFR-011).

use std::collections::BTreeMap;
use std::path::Path;

use crate::corpus::{self, Prompt, Quality};
use crate::provider::{self, AuthoringModel, Judge, ModelSpec};
use crate::record::{Comparison, ModelInfo, PromptResult, RunRecord};
use crate::score;
use crate::toolchain;

/// Why a run could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunFailure {
    /// The command line, the corpus, or the model could not be reached. Exit 2.
    Usage(String),
    /// The run cannot be scored: the corpus omits coverage, or no prompt could
    /// be scored. Exit 1.
    Unscorable(String),
}

impl RunFailure {
    /// The process exit code the failure maps to (C-006).
    pub fn code(&self) -> i32 {
        match self {
            RunFailure::Usage(_) => crate::cli::EXIT_USAGE,
            RunFailure::Unscorable(_) => crate::cli::EXIT_UNSCORABLE,
        }
    }

    /// The failure's message.
    pub fn message(&self) -> &str {
        match self {
            RunFailure::Usage(message) | RunFailure::Unscorable(message) => message,
        }
    }
}

/// Runs a corpus through a model and returns its record.
pub fn execute(corpus_dir: &Path, spec: &ModelSpec) -> Result<RunRecord, RunFailure> {
    let corpus = corpus::load(corpus_dir).map_err(|error| RunFailure::Usage(error.0))?;
    let coverage = corpus::coverage(&corpus);
    if !coverage.satisfied() {
        return Err(RunFailure::Unscorable(format!(
            "the corpus does not meet its coverage requirement:\n{}",
            coverage.omissions.join("\n")
        )));
    }

    let (author, judge) = provider::build(spec).map_err(|error| RunFailure::Usage(error.0))?;
    let mut results = Vec::with_capacity(corpus.prompts.len());
    for prompt in &corpus.prompts {
        results.push(run_prompt(prompt, author.as_ref(), judge.as_ref())?);
    }

    if results.iter().all(|result| !result.scored) {
        return Err(RunFailure::Unscorable(
            "no prompt in the corpus could be scored".to_string(),
        ));
    }

    let model = ModelInfo {
        id: spec.id(),
        name: spec.model.clone(),
        version: spec.model.clone(),
    };
    Ok(score::summarize(&corpus, &coverage, model, results))
}

fn run_prompt(
    prompt: &Prompt,
    author: &dyn AuthoringModel,
    judge: &dyn Judge,
) -> Result<PromptResult, RunFailure> {
    let authored = author.author(prompt).map_err(|error| {
        RunFailure::Usage(format!(
            "the authoring model `{}` could not be reached: {}",
            author.label(),
            error.0
        ))
    })?;
    let outcome = toolchain::evaluate(&authored);

    // A judge that returns a verdict scores the prompt even when the scene did
    // not compile; the fidelity it gives reflects the authored output. Only a
    // judge that cannot return a verdict makes the prompt unscorable, and an
    // unscorable prompt is counted as a failure (FEAT-023).
    let (scored, fidelity, qualities, failure) = match judge.judge(prompt, &authored) {
        Ok(judgement) => {
            // Only the hard-end qualities the prompt declares are applicable;
            // the judge scores all four, but a prompt is measured on the ones
            // it exercises (FEAT-023, NFR-031).
            let mut qualities = BTreeMap::new();
            for quality in Quality::HARD_END {
                if prompt.qualities.contains(&quality) {
                    qualities.insert(
                        quality.key().to_string(),
                        judgement
                            .qualities
                            .get(quality.key())
                            .copied()
                            .unwrap_or(false),
                    );
                }
            }
            (true, judgement.fidelity, qualities, outcome.failure.clone())
        }
        Err(error) => (
            false,
            0.0,
            BTreeMap::new(),
            Some(format!("could not be scored: {}", error.0)),
        ),
    };

    Ok(PromptResult {
        id: prompt.id.clone(),
        intent: prompt.intent.clone(),
        complexity: prompt.complexity,
        compiled: outcome.compiled,
        scored,
        fidelity,
        qualities,
        diagnostics: outcome.diagnostics,
        failure,
        tokens: authored.tokens,
    })
}

/// Reads a run record from a file.
pub fn read_record(path: &Path) -> Result<RunRecord, RunFailure> {
    let source = std::fs::read_to_string(path).map_err(|error| {
        RunFailure::Usage(format!(
            "cannot read run record `{}`: {error}",
            path.display()
        ))
    })?;
    serde_json::from_str(&source).map_err(|error| {
        RunFailure::Usage(format!(
            "run record `{}` is not valid: {error}",
            path.display()
        ))
    })
}

/// Compares two run records of the same corpus.
pub fn compare(baseline: &Path, candidate: &Path) -> Result<Comparison, RunFailure> {
    let baseline = read_record(baseline)?;
    let candidate = read_record(candidate)?;
    score::compare(&baseline, &candidate).map_err(RunFailure::Usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::parse_model_spec;
    use crate::testing::{corpus_dir, replay_dir, write_usage};

    #[test]
    fn a_replay_run_produces_a_record_and_is_reproducible() {
        let dir = replay_dir(&[
            (
                "p1",
                r##"{"scene": {"id":"s","projectId":"eval","name":"S","formatVersion":"0.2","canvas":{"width":100,"height":100,"background":"#ffffff"},"elements":[]}}"##,
                r#"{"fidelity": 0.9, "qualities": {"depth": true, "relative_placement": true, "shared_anchors": true, "subject_accuracy": true}}"#,
            ),
            (
                "p2",
                r##"{"scene": {"id":"s2","projectId":"eval","name":"S","formatVersion":"0.2","canvas":{"width":100,"height":100,"background":"#ffffff"},"elements":[]}}"##,
                r#"{"fidelity": 0.7, "qualities": {"depth": false, "relative_placement": true, "shared_anchors": false, "subject_accuracy": true}}"#,
            ),
        ]);
        let corpus = corpus_dir();
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");

        let first = execute(&corpus, &spec).expect("runs");
        let second = execute(&corpus, &spec).expect("runs again");
        assert_eq!(first, second, "repeated runs are reproducible");
        assert_eq!(first.prompts.len(), 2);
        assert!(first.prompts.iter().all(|result| result.compiled));
        assert_eq!(first.fidelity_score, 0.8);
        assert!(first.thresholds.compile_success);
        assert!(first.thresholds.fidelity);
    }

    #[test]
    fn a_replay_run_reports_a_missing_recording_as_unreachable() {
        let dir = replay_dir(&[]);
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");
        let failure = execute(&corpus_dir(), &spec).expect_err("fails");
        assert_eq!(failure.code(), 2, "{}", failure.message());
    }

    #[test]
    fn an_unscorable_reply_is_counted_as_a_failure() {
        let dir = replay_dir(&[
            (
                "p1",
                "I could not draw that.",
                r#"{"fidelity": 0.0, "qualities": {}}"#,
            ),
            (
                "p2",
                r##"{"scene": {"id":"s","projectId":"eval","name":"S","formatVersion":"0.2","canvas":{"width":100,"height":100,"background":"#ffffff"},"elements":[]}}"##,
                r#"{"fidelity": 0.9, "qualities": {"depth": true, "relative_placement": true, "shared_anchors": true, "subject_accuracy": true}}"#,
            ),
        ]);
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");
        let record = execute(&corpus_dir(), &spec).expect("runs");
        assert!(!record.prompts[0].compiled);
        assert!(record.prompts[0].scored);
        assert_eq!(record.compile_success_rate, 0.5);
    }

    #[test]
    fn a_corpus_that_omits_coverage_is_not_scored() {
        let dir = crate::testing::TempDir::new("run-coverage");
        std::fs::create_dir_all(dir.path().join("prompts")).expect("creates");
        std::fs::write(
            dir.path().join("corpus.json"),
            r#"{"id":"c","version":"1.0.0","description":"d"}"#,
        )
        .expect("writes");
        std::fs::write(
            dir.path().join("prompts/a.json"),
            r#"{"id":"a","corpusId":"c","intent":"i","text":"t","order":0}"#,
        )
        .expect("writes");

        let spec = parse_model_spec("replay:/does/not/exist").expect("spec");
        let failure = execute(dir.path(), &spec).expect_err("fails");
        assert_eq!(failure.code(), 1);
        assert!(
            failure.message().contains("coverage"),
            "{}",
            failure.message()
        );
    }

    #[test]
    fn the_run_record_carries_the_schemas_required_fields() {
        let dir = replay_dir(&[
            (
                "p1",
                r##"{"scene": {"id":"s","projectId":"eval","name":"S","formatVersion":"0.2","canvas":{"width":100,"height":100,"background":"#ffffff"},"elements":[]}}"##,
                r#"{"fidelity": 0.9, "qualities": {}}"#,
            ),
            (
                "p2",
                r##"{"scene": {"id":"s","projectId":"eval","name":"S","formatVersion":"0.2","canvas":{"width":100,"height":100,"background":"#ffffff"},"elements":[]}}"##,
                r#"{"fidelity": 0.9, "qualities": {"depth": true, "relative_placement": true, "shared_anchors": true, "subject_accuracy": true}}"#,
            ),
        ]);
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");
        let record = execute(&corpus_dir(), &spec).expect("runs");
        let value = serde_json::to_value(&record).expect("serializes");

        // The schema's EvaluationRun required fields (schema.md).
        for field in [
            "id",
            "corpusId",
            "authoringModelId",
            "compileSuccessRate",
            "fidelityScore",
            "regression",
        ] {
            assert!(value.get(field).is_some(), "the record carries `{field}`");
        }
        // The harness's own sub-scores and per-prompt results.
        assert!(value.get("hardEnd").is_some());
        assert!(value.get("thresholds").is_some());
        assert_eq!(
            value["prompts"].as_array().map(Vec::len),
            Some(2),
            "one result per prompt"
        );
    }

    fn two_prompt_replies() -> crate::testing::TempDir {
        replay_dir(&[
            (
                "p1",
                r##"{"scene": {"id":"s","projectId":"eval","name":"S","formatVersion":"0.2","canvas":{"width":100,"height":100,"background":"#ffffff"},"elements":[]}}"##,
                r#"{"fidelity": 0.9, "qualities": {}}"#,
            ),
            (
                "p2",
                r##"{"scene": {"id":"s2","projectId":"eval","name":"S","formatVersion":"0.2","canvas":{"width":100,"height":100,"background":"#ffffff"},"elements":[]}}"##,
                r#"{"fidelity": 0.9, "qualities": {"depth": true, "relative_placement": true, "shared_anchors": true, "subject_accuracy": true}}"#,
            ),
        ])
    }

    #[test]
    fn a_run_reports_the_median_of_the_reported_token_usage() {
        let dir = two_prompt_replies();
        write_usage(dir.path(), "p1", 1_000);
        write_usage(dir.path(), "p2", 3_000);
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");

        let record = execute(&corpus_dir(), &spec).expect("runs");
        assert_eq!(record.median_tokens, Some(2_000));
        assert!(record.thresholds.tokens);
        let value = serde_json::to_value(&record).expect("serializes");
        assert_eq!(value["medianTokens"], serde_json::json!(2_000));
    }

    #[test]
    fn a_run_whose_provider_reports_no_usage_reports_no_median() {
        let dir = two_prompt_replies();
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");

        let record = execute(&corpus_dir(), &spec).expect("runs");
        assert_eq!(record.median_tokens, None);
        let value = serde_json::to_value(&record).expect("serializes");
        assert!(
            value.get("medianTokens").is_none(),
            "the median is absent, not fabricated"
        );
    }
}
