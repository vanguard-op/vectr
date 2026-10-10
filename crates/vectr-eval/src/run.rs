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
use crate::provider::{self, AuthoringModel, Judge, Judgement, ModelSpec};
use crate::record::{Comparison, ModelInfo, PromptResult, RunRecord, ShardInfo};
use crate::score;
use crate::toolchain::{self, CompileOutcome};

/// A slice of the corpus a `--shard <index>/<count>` run covers.
///
/// The prompt at corpus position `p` belongs to the shard with index
/// `p mod count`, so the shards partition the corpus deterministically and a
/// merge can rebuild the whole-corpus order from the partial records
/// (FEAT-023, D-051).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shard {
    /// Zero-based index of this shard.
    pub index: usize,
    /// How many shards the corpus is split into.
    pub count: usize,
}

impl Shard {
    /// Parses `<index>/<count>`, refusing a count below one or an index outside
    /// the shard range.
    pub fn parse(text: &str) -> Result<Self, String> {
        let (index, count) = text
            .split_once('/')
            .ok_or_else(|| format!("`{text}` is not a `<index>/<count>` shard"))?;
        let index: usize = index
            .parse()
            .map_err(|_| format!("`{text}` names a shard index that is not a number"))?;
        let count: usize = count
            .parse()
            .map_err(|_| format!("`{text}` names a shard count that is not a number"))?;
        if count == 0 {
            return Err(format!("`{text}` names a shard count of zero"));
        }
        if index >= count {
            return Err(format!("`{text}` names a shard index outside `0..{count}`"));
        }
        Ok(Self { index, count })
    }

    /// Whether this shard runs the prompt at corpus position `position`.
    pub fn selects(self, position: usize) -> bool {
        position % self.count == self.index
    }
}

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
///
/// The whole corpus is loaded and its coverage checked even when only one shard
/// runs, so a shard is always a slice of a corpus that meets its coverage
/// requirement. With `shard`, only the prompts the shard selects run and the
/// record carries the shard descriptor; a merge combines the partial records
/// into the whole-corpus record (FEAT-023, D-051). With `log_dir`, each prompt's
/// authoring input, authored scene, judge verdict and diagnostics are written
/// for inspection (C-006).
pub fn execute(
    corpus_dir: &Path,
    spec: &ModelSpec,
    shard: Option<Shard>,
    log_dir: Option<&Path>,
) -> Result<RunRecord, RunFailure> {
    let corpus = corpus::load(corpus_dir).map_err(|error| RunFailure::Usage(error.0))?;
    let coverage = corpus::coverage(&corpus);
    if !coverage.satisfied() {
        return Err(RunFailure::Unscorable(format!(
            "the corpus does not meet its coverage requirement:\n{}",
            coverage.omissions.join("\n")
        )));
    }

    if let Some(dir) = log_dir {
        std::fs::create_dir_all(dir).map_err(|error| {
            RunFailure::Usage(format!(
                "cannot create the log directory `{}`: {error}",
                dir.display()
            ))
        })?;
    }

    let (author, judge) = provider::build(spec).map_err(|error| RunFailure::Usage(error.0))?;
    let selected: Vec<&Prompt> = match shard {
        Some(shard) => corpus
            .prompts
            .iter()
            .enumerate()
            .filter(|(position, _)| shard.selects(*position))
            .map(|(_, prompt)| prompt)
            .collect(),
        None => corpus.prompts.iter().collect(),
    };

    let mut results = Vec::with_capacity(selected.len());
    for prompt in selected {
        results.push(run_prompt(
            prompt,
            author.as_ref(),
            judge.as_ref(),
            log_dir,
        )?);
    }

    // A whole run with nothing scorable cannot be reported; an empty shard has
    // nothing to score, so it is not a failure (FEAT-023).
    if !results.is_empty() && results.iter().all(|result| !result.scored) {
        return Err(RunFailure::Unscorable(
            "no prompt in the corpus could be scored".to_string(),
        ));
    }

    let model = ModelInfo {
        id: spec.id(),
        name: spec.model.clone(),
        version: spec.model.clone(),
    };
    let mut record = score::summarize(&corpus, &coverage, model, results);
    record.shard = shard.map(|shard| ShardInfo {
        index: shard.index,
        count: shard.count,
        total: corpus.prompts.len(),
    });
    Ok(record)
}

fn run_prompt(
    prompt: &Prompt,
    author: &dyn AuthoringModel,
    judge: &dyn Judge,
    log_dir: Option<&Path>,
) -> Result<PromptResult, RunFailure> {
    log(log_dir, prompt, "input.json", &input_log(prompt))?;
    let authored = author.author(prompt).map_err(|error| {
        RunFailure::Usage(format!(
            "the authoring model `{}` could not be reached: {}",
            author.label(),
            error.0
        ))
    })?;
    log(log_dir, prompt, "scene.json", &authored.text)?;
    let outcome = toolchain::evaluate(&authored);
    log(
        log_dir,
        prompt,
        "diagnostics.json",
        &diagnostics_log(&outcome),
    )?;
    if let Some(tokens) = authored.tokens {
        log(log_dir, prompt, "usage.json", &usage_log(tokens))?;
    }

    // A judge that returns a verdict scores the prompt even when the scene did
    // not compile; the fidelity it gives reflects the authored output. Only a
    // judge that cannot return a verdict makes the prompt unscorable, and an
    // unscorable prompt is counted as a failure (FEAT-023).
    let judged = judge.judge(prompt, &authored);
    log(
        log_dir,
        prompt,
        "judge.json",
        &match &judged {
            Ok(judgement) => verdict_log(judgement),
            Err(error) => error_log(&error.0),
        },
    )?;
    let (scored, fidelity, qualities, failure) = match judged {
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

/// Writes one prompt's log artifact under `log_dir`, when logging is requested.
///
/// The artifacts use the `replay` provider's file names (`<id>.scene.json`,
/// `<id>.judge.json`, `<id>.usage.json`), so a log directory is also a recorded
/// run that can be replayed; `<id>.input.json` and `<id>.diagnostics.json` are
/// the harness's own inspection files (D-051). A write that fails is reported,
/// never silent (NFR-011).
fn log(
    log_dir: Option<&Path>,
    prompt: &Prompt,
    suffix: &str,
    text: &str,
) -> Result<(), RunFailure> {
    let Some(dir) = log_dir else {
        return Ok(());
    };
    let path = dir.join(format!("{}.{suffix}", prompt.id));
    std::fs::write(&path, text).map_err(|error| {
        RunFailure::Usage(format!(
            "cannot write the log for prompt `{}` at `{}`: {error}",
            prompt.id,
            path.display()
        ))
    })
}

/// The authoring input the harness supplied: the prompt's identity and text.
fn input_log(prompt: &Prompt) -> String {
    pretty(&serde_json::json!({
        "id": prompt.id,
        "intent": prompt.intent,
        "text": prompt.text,
    }))
}

/// The toolchain outcome for the authored scene: whether it compiled, why it
/// did not when it did not, and the diagnostics it produced.
fn diagnostics_log(outcome: &CompileOutcome) -> String {
    pretty(&serde_json::json!({
        "compiled": outcome.compiled,
        "failure": outcome.failure,
        "diagnostics": outcome.diagnostics,
    }))
}

/// The judge's verdict: the judged fidelity and the per-quality marks.
fn verdict_log(judgement: &Judgement) -> String {
    pretty(&serde_json::json!({
        "fidelity": judgement.fidelity,
        "qualities": judgement.qualities,
    }))
}

/// A judge that returned no verdict, recorded so the artifact is never missing.
fn error_log(message: &str) -> String {
    pretty(&serde_json::json!({ "error": message }))
}

/// The reported token usage, in the `replay` provider's recording shape.
fn usage_log(tokens: u64) -> String {
    pretty(&serde_json::json!({ "tokens": tokens }))
}

fn pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
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

        let first = execute(&corpus, &spec, None, None).expect("runs");
        let second = execute(&corpus, &spec, None, None).expect("runs again");
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
        let failure = execute(&corpus_dir(), &spec, None, None).expect_err("fails");
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
        let record = execute(&corpus_dir(), &spec, None, None).expect("runs");
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
        let failure = execute(dir.path(), &spec, None, None).expect_err("fails");
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
        let record = execute(&corpus_dir(), &spec, None, None).expect("runs");
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

        let record = execute(&corpus_dir(), &spec, None, None).expect("runs");
        assert_eq!(record.median_tokens, Some(2_000));
        assert!(record.thresholds.tokens);
        let value = serde_json::to_value(&record).expect("serializes");
        assert_eq!(value["medianTokens"], serde_json::json!(2_000));
    }

    #[test]
    fn a_run_whose_provider_reports_no_usage_reports_no_median() {
        let dir = two_prompt_replies();
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");

        let record = execute(&corpus_dir(), &spec, None, None).expect("runs");
        assert_eq!(record.median_tokens, None);
        let value = serde_json::to_value(&record).expect("serializes");
        assert!(
            value.get("medianTokens").is_none(),
            "the median is absent, not fabricated"
        );
    }

    #[test]
    fn parses_a_shard_reference_and_refuses_a_malformed_one() {
        assert_eq!(
            Shard::parse("1/5").expect("parses"),
            Shard { index: 1, count: 5 }
        );
        assert!(
            Shard::parse("5/5").is_err(),
            "the index is outside the range"
        );
        assert!(Shard::parse("0/0").is_err(), "a shard count of zero");
        assert!(Shard::parse("1").is_err(), "no slash");
        assert!(Shard::parse("a/2").is_err(), "not a number");
    }

    #[test]
    fn a_sharded_run_runs_only_its_prompts_and_carries_the_shard() {
        let dir = two_prompt_replies();
        let corpus = corpus_dir();
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");

        let first =
            execute(&corpus, &spec, Some(Shard { index: 0, count: 2 }), None).expect("runs");
        assert_eq!(
            first
                .prompts
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["p1"]
        );
        assert_eq!(
            first.shard,
            Some(crate::record::ShardInfo {
                index: 0,
                count: 2,
                total: 2
            })
        );

        let second =
            execute(&corpus, &spec, Some(Shard { index: 1, count: 2 }), None).expect("runs");
        assert_eq!(
            second
                .prompts
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["p2"]
        );
        assert_eq!(
            second.shard,
            Some(crate::record::ShardInfo {
                index: 1,
                count: 2,
                total: 2
            })
        );
    }

    #[test]
    fn a_log_directory_writes_each_prompts_artifacts() {
        let dir = two_prompt_replies();
        write_usage(dir.path(), "p1", 1_500);
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");
        let logs = crate::testing::TempDir::new("logs");
        let log_dir = logs.path().join("run");

        execute(&corpus_dir(), &spec, None, Some(&log_dir)).expect("runs");

        for id in ["p1", "p2"] {
            for suffix in ["input.json", "scene.json", "judge.json", "diagnostics.json"] {
                let path = log_dir.join(format!("{id}.{suffix}"));
                assert!(path.is_file(), "{} is written", path.display());
            }
        }
        let input = std::fs::read_to_string(log_dir.join("p1.input.json")).expect("input");
        assert!(input.contains("draw a dot"), "{input}");
        let verdict = std::fs::read_to_string(log_dir.join("p1.judge.json")).expect("judge");
        assert!(verdict.contains("fidelity"), "{verdict}");
        let diagnostics =
            std::fs::read_to_string(log_dir.join("p1.diagnostics.json")).expect("diagnostics");
        assert!(diagnostics.contains("compiled"), "{diagnostics}");
        // The usage is recorded only where the provider reported it.
        assert!(log_dir.join("p1.usage.json").is_file());
        assert!(!log_dir.join("p2.usage.json").exists());
    }

    #[test]
    fn a_log_directory_replays_to_the_same_prompts() {
        let dir = two_prompt_replies();
        write_usage(dir.path(), "p1", 1_000);
        write_usage(dir.path(), "p2", 3_000);
        let spec = parse_model_spec(&format!("replay:{}", dir.path().display())).expect("spec");
        let logs = crate::testing::TempDir::new("logs-replay");
        let log_dir = logs.path().join("run");
        let original = execute(&corpus_dir(), &spec, None, Some(&log_dir)).expect("runs");

        let replay_spec = parse_model_spec(&format!("replay:{}", log_dir.display())).expect("spec");
        let replayed = execute(&corpus_dir(), &replay_spec, None, None).expect("replays");
        assert_eq!(
            replayed.prompts, original.prompts,
            "the log directory records the run"
        );
        assert_eq!(replayed.median_tokens, original.median_tokens);
    }
}
