//! The run record and the comparison of two runs (FEAT-023, schema.md).
//!
//! A run record extends the schema's `EvaluationRun` with the per-prompt
//! results, the hard-end sub-scores, the coverage findings and the threshold
//! outcomes. It is deterministic: the same corpus and the same model output
//! produce byte-identical JSON, so repeated runs are reproducible and a
//! comparison is meaningful (FEAT-023, NFR-010). No wall-clock time or random
//! identifier enters the record.
//!
//! `compileSuccessRate` and `fidelityScore` are the schema's fields; the rest
//! are the harness's own. `regression` is false on a fresh run and is decided
//! by [`crate::score::compare`].

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use vectr_core::Diagnostic;

use crate::corpus::{Complexity, Quality};

/// The result of running one corpus through one authoring model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    /// Stable identifier for the run: the corpus, its version and the model.
    pub id: String,
    /// The corpus that was run.
    pub corpus_id: String,
    /// The corpus version the run measured.
    pub corpus_version: String,
    /// The authoring model, as `provider:model`.
    pub authoring_model_id: String,
    /// The model's name and pinned version.
    pub model: ModelInfo,
    /// Share of authored scenes that compiled without error.
    pub compile_success_rate: f64,
    /// Judged visual fidelity score for the run.
    pub fidelity_score: f64,
    /// The median authoring-model token usage per prompt across the run, over
    /// the prompts whose provider reported usage; absent when no prompt
    /// reported usage (FEAT-023, schema.md "EvaluationRun").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub median_tokens: Option<u64>,
    /// Whether the run regressed against the previous run of the same corpus
    /// and model. False on a fresh run.
    pub regression: bool,
    /// Anything notable about the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// The hard-end sub-scores NFR-031 measures.
    pub hard_end: HardEndScores,
    /// Whether the run meets the NFR-030 and NFR-031 bars.
    pub thresholds: Thresholds,
    /// Whether the corpus met its coverage requirements.
    pub coverage: CoverageReport,
    /// One result per prompt, in corpus order.
    pub prompts: Vec<PromptResult>,
}

/// The model a run used, with the pinned version the score applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// Stable identifier for the model entry, `provider:model`.
    pub id: String,
    /// Model name.
    pub name: String,
    /// Pinned model version the score applies to.
    pub version: String,
}

/// The hard-end sub-scores: how many complex-end prompts met the rubric, and
/// each quality's pass rate (NFR-031).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardEndScores {
    /// How many prompts sit on the complex end of the range.
    pub complex_prompts: usize,
    /// Share of complex-end prompts that met the whole rubric (every quality
    /// they exercise passed).
    pub rubric_pass_rate: f64,
    /// Per-quality pass rates, keyed by the quality's stable key.
    pub qualities: BTreeMap<String, QualityScore>,
}

/// One hard-end quality's pass rate over the prompts that exercise it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityScore {
    /// How many prompts exercise the quality.
    pub applicable: usize,
    /// How many of them passed it.
    pub passed: usize,
    /// `passed / applicable`, or zero when none exercise it.
    pub pass_rate: f64,
}

/// Whether the run clears the judged bars (NFR-030, NFR-031).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thresholds {
    /// Compile success is at or above the NFR-030 bar.
    pub compile_success: bool,
    /// Judged fidelity is at or above the NFR-030 bar.
    pub fidelity: bool,
    /// The complex-end rubric pass rate is at or above the NFR-031 bar.
    pub hard_end: bool,
    /// The median per-prompt token count is within NFR-030's budget. True when
    /// no prompt reported usage, because no measured median can exceed it.
    pub tokens: bool,
}

/// The corpus coverage findings carried into the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageReport {
    /// Whether the corpus met every coverage requirement.
    pub satisfied: bool,
    /// The requirements it did not meet.
    pub omissions: Vec<String>,
}

/// One prompt's result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResult {
    /// The prompt's identifier.
    pub id: String,
    /// The graphic the prompt asked for.
    pub intent: String,
    /// Where the prompt sits in the complexity range.
    pub complexity: Complexity,
    /// Whether the authored scene compiled without error.
    pub compiled: bool,
    /// Whether the prompt could be scored. An unscorable prompt is counted as
    /// a failure, never skipped (FEAT-023).
    pub scored: bool,
    /// The judged fidelity, or zero when the prompt was unscorable.
    pub fidelity: f64,
    /// The hard-end qualities the judge marked true, keyed by quality.
    pub qualities: BTreeMap<String, bool>,
    /// The diagnostics the toolchain produced, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
    /// Why the prompt was unscorable or the scene failed, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// The authoring model's token usage for the prompt, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
}

impl PromptResult {
    /// The prompt's judged score for one quality, defaulting to false when the
    /// quality was not scored.
    pub fn quality(&self, quality: Quality) -> bool {
        self.qualities.get(quality.key()).copied().unwrap_or(false)
    }
}

/// A comparison of two runs of the same corpus (FEAT-023).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    /// The corpus both runs measured.
    pub corpus_id: String,
    /// The baseline run's identifier.
    pub baseline: String,
    /// The candidate run's identifier.
    pub candidate: String,
    /// Whether any prompt or aggregate metric regressed.
    pub regression: bool,
    /// The prompts that regressed, each with the reason.
    pub regressions: Vec<Regression>,
}

/// One prompt's regression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Regression {
    /// The prompt that regressed.
    pub prompt_id: String,
    /// What regressed.
    pub reason: String,
}

/// The corpus version a comparison requires both runs to share.
pub const CORPUS_MISMATCH: &str = "the two runs measure different corpora";
