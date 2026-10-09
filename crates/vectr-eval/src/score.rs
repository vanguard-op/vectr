//! Scoring a run and comparing two runs (FEAT-023, NFR-030, NFR-031).
//!
//! Compile success is the share of prompts whose authored scene compiled.
//! Fidelity is the mean judged score; an unscorable prompt contributes zero and
//! is counted as a failure, never dropped. The hard-end sub-scores are the
//! per-quality pass rates and the share of complex-end prompts that met the
//! whole rubric. The bars are NFR-030's compile success and fidelity and
//! NFR-031's complex-end rubric.

use std::collections::BTreeMap;

use crate::corpus::{Complexity, Corpus, Coverage, Quality};
use crate::record::{
    Comparison, CoverageReport, HardEndScores, ModelInfo, PromptResult, QualityScore, Regression,
    RunRecord, Thresholds, CORPUS_MISMATCH,
};

/// NFR-030's compile-success bar.
pub const COMPILE_SUCCESS_BAR: f64 = 0.95;
/// NFR-030's judged-fidelity bar.
pub const FIDELITY_BAR: f64 = 0.80;
/// NFR-031's complex-end rubric bar.
pub const HARD_END_BAR: f64 = 0.80;

/// The tolerance below which two scores are the same measurement.
const EPSILON: f64 = 1e-9;

/// Builds a run record from the scored prompt results.
pub fn summarize(
    corpus: &Corpus,
    coverage: &Coverage,
    model: ModelInfo,
    results: Vec<PromptResult>,
) -> RunRecord {
    let total = results.len();
    let compiled = results.iter().filter(|result| result.compiled).count();
    let compile_success_rate = ratio(compiled, total);

    let fidelity_total: f64 = results.iter().map(|result| result.fidelity).sum();
    let fidelity_score = if total == 0 {
        0.0
    } else {
        fidelity_total / total as f64
    };

    let hard_end = hard_end_scores(&results);
    let thresholds = Thresholds {
        compile_success: compile_success_rate + EPSILON >= COMPILE_SUCCESS_BAR,
        fidelity: fidelity_score + EPSILON >= FIDELITY_BAR,
        hard_end: hard_end.rubric_pass_rate + EPSILON >= HARD_END_BAR,
    };

    let model_id = model.id.clone();
    RunRecord {
        id: format!(
            "{}@{}#{}",
            corpus.manifest.id, corpus.manifest.version, model_id
        ),
        corpus_id: corpus.manifest.id.clone(),
        corpus_version: corpus.manifest.version.clone(),
        authoring_model_id: model_id,
        model,
        compile_success_rate,
        fidelity_score,
        regression: false,
        notes: None,
        hard_end,
        thresholds,
        coverage: CoverageReport {
            satisfied: coverage.satisfied(),
            omissions: coverage.omissions.clone(),
        },
        prompts: results,
    }
}

fn hard_end_scores(results: &[PromptResult]) -> HardEndScores {
    let complex: Vec<&PromptResult> = results
        .iter()
        .filter(|result| result.complexity == Complexity::Complex)
        .collect();

    let mut qualities = BTreeMap::new();
    for quality in Quality::HARD_END {
        let mut applicable = 0usize;
        let mut passed = 0usize;
        for result in &complex {
            if result.qualities.contains_key(quality.key()) {
                applicable += 1;
                if result.quality(quality) {
                    passed += 1;
                }
            }
        }
        qualities.insert(
            quality.key().to_string(),
            QualityScore {
                applicable,
                passed,
                pass_rate: ratio(passed, applicable),
            },
        );
    }

    let met = complex.iter().filter(|result| met_rubric(result)).count();

    HardEndScores {
        complex_prompts: complex.len(),
        rubric_pass_rate: ratio(met, complex.len()),
        qualities,
    }
}

/// Whether a complex-end prompt met the whole rubric: it exercises at least
/// one hard-end quality and every one it exercises passed.
fn met_rubric(result: &PromptResult) -> bool {
    let declared: Vec<Quality> = Quality::HARD_END
        .into_iter()
        .filter(|quality| result.qualities.contains_key(quality.key()))
        .collect();
    !declared.is_empty() && declared.into_iter().all(|quality| result.quality(quality))
}

fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// Compares a baseline run with a candidate run of the same corpus.
///
/// The runs must measure the same corpus and version, or the comparison is
/// refused. Every prompt the candidate compiled less, scored lower, or dropped
/// a quality on is named (FEAT-023).
pub fn compare(baseline: &RunRecord, candidate: &RunRecord) -> Result<Comparison, String> {
    if baseline.corpus_id != candidate.corpus_id
        || baseline.corpus_version != candidate.corpus_version
    {
        return Err(format!(
            "{CORPUS_MISMATCH}: `{}` measures {}@{} and `{}` measures {}@{}",
            baseline.id,
            baseline.corpus_id,
            baseline.corpus_version,
            candidate.id,
            candidate.corpus_id,
            candidate.corpus_version
        ));
    }

    let candidate_by_id: BTreeMap<&str, &PromptResult> = candidate
        .prompts
        .iter()
        .map(|result| (result.id.as_str(), result))
        .collect();

    let mut regressions = Vec::new();
    for before in &baseline.prompts {
        let Some(after) = candidate_by_id.get(before.id.as_str()) else {
            regressions.push(Regression {
                prompt_id: before.id.clone(),
                reason: "the prompt is absent from the candidate run".to_string(),
            });
            continue;
        };
        if before.compiled && !after.compiled {
            regressions.push(Regression {
                prompt_id: before.id.clone(),
                reason: "compile success regressed".to_string(),
            });
        }
        if after.fidelity + EPSILON < before.fidelity {
            regressions.push(Regression {
                prompt_id: before.id.clone(),
                reason: format!(
                    "fidelity dropped from {:.4} to {:.4}",
                    before.fidelity, after.fidelity
                ),
            });
        }
        for quality in Quality::HARD_END {
            if before.quality(quality) && !after.quality(quality) {
                regressions.push(Regression {
                    prompt_id: before.id.clone(),
                    reason: format!("the `{}` quality regressed", quality.key()),
                });
            }
        }
    }

    Ok(Comparison {
        corpus_id: baseline.corpus_id.clone(),
        baseline: baseline.id.clone(),
        candidate: candidate.id.clone(),
        regression: !regressions.is_empty(),
        regressions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{CorpusManifest, Prompt};

    fn result(id: &str, compiled: bool, fidelity: f64, qualities: &[Quality]) -> PromptResult {
        let mut map = BTreeMap::new();
        for quality in qualities {
            map.insert(quality.key().to_string(), compiled);
        }
        PromptResult {
            id: id.to_string(),
            intent: "i".to_string(),
            complexity: Complexity::Complex,
            compiled,
            scored: true,
            fidelity,
            qualities: map,
            diagnostics: Vec::new(),
            failure: None,
            tokens: None,
        }
    }

    fn corpus() -> Corpus {
        Corpus {
            manifest: CorpusManifest {
                id: "c".to_string(),
                version: "1.0.0".to_string(),
                description: "d".to_string(),
            },
            prompts: vec![Prompt {
                id: "p".to_string(),
                corpus_id: "c".to_string(),
                intent: "i".to_string(),
                text: "t".to_string(),
                order: 0,
                complexity: Complexity::Complex,
                qualities: Quality::HARD_END.to_vec(),
            }],
        }
    }

    #[test]
    fn summarizes_the_rates_and_bars() {
        let mut results = vec![
            result("a", true, 1.0, &Quality::HARD_END),
            result("b", true, 0.9, &Quality::HARD_END),
            result("c", false, 0.0, &[]),
        ];
        // Nineteen compile, so compile success is exactly the NFR-030 bar; the
        // passing prompts score 0.85, keeping fidelity above its bar too.
        for index in 0..17 {
            results.push(result(&format!("p{index}"), true, 0.85, &Quality::HARD_END));
        }
        let coverage = Coverage::default();
        let model = ModelInfo {
            id: "replay:x".to_string(),
            name: "x".to_string(),
            version: "x".to_string(),
        };
        let record = summarize(&corpus(), &coverage, model, results);
        assert_eq!(record.compile_success_rate, 19.0 / 20.0);
        assert!(record.thresholds.compile_success);
        assert!(record.thresholds.fidelity);
        assert!(record.thresholds.hard_end);
    }

    #[test]
    fn an_unscorable_prompt_counts_as_a_failure() {
        let mut unscorable = result("c", false, 0.0, &[]);
        unscorable.scored = false;
        unscorable.failure = Some("no JSON".to_string());
        let record = summarize(
            &corpus(),
            &Coverage::default(),
            ModelInfo {
                id: "replay:x".to_string(),
                name: "x".to_string(),
                version: "x".to_string(),
            },
            vec![unscorable],
        );
        assert_eq!(record.compile_success_rate, 0.0);
        assert_eq!(record.fidelity_score, 0.0);
    }

    #[test]
    fn compare_flags_compile_fidelity_and_quality_regressions() {
        let mut baseline = summarize(
            &corpus(),
            &Coverage::default(),
            ModelInfo {
                id: "replay:x".to_string(),
                name: "x".to_string(),
                version: "x".to_string(),
            },
            vec![result("a", true, 0.9, &[Quality::Depth])],
        );
        baseline.id = "run-a".to_string();
        let mut candidate = baseline.clone();
        candidate.id = "run-b".to_string();
        candidate.prompts = vec![result("a", false, 0.5, &[Quality::Depth])];
        // The candidate's quality is tied to `compiled`, so it is false now.

        let comparison = compare(&baseline, &candidate).expect("compares");
        assert!(comparison.regression);
        let reasons: Vec<&str> = comparison
            .regressions
            .iter()
            .map(|regression| regression.reason.as_str())
            .collect();
        assert!(
            reasons.contains(&"compile success regressed"),
            "{reasons:?}"
        );
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("fidelity dropped")),
            "{reasons:?}"
        );
        assert!(
            reasons.iter().any(|reason| reason.contains("`depth`")),
            "{reasons:?}"
        );
    }

    #[test]
    fn comparing_different_corpora_is_refused() {
        let mut a = summarize(
            &corpus(),
            &Coverage::default(),
            ModelInfo {
                id: "replay:x".to_string(),
                name: "x".to_string(),
                version: "x".to_string(),
            },
            vec![result("a", true, 1.0, &[])],
        );
        let mut b = a.clone();
        b.corpus_version = "2.0.0".to_string();
        a.regression = false;
        let error = compare(&a, &b).expect_err("refused");
        assert!(error.contains(CORPUS_MISMATCH), "{error}");
    }
}
