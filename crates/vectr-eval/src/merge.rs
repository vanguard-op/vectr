//! Merging sharded run records (FEAT-023, C-006, D-051).
//!
//! A sharded run writes one partial record per shard; a merge combines the
//! partials of one corpus and one model into one record over the whole corpus.
//! It recomputes the rates, the median and the hard-end scores from the
//! per-prompt results rather than averaging the shard aggregates, and it refuses
//! records of different corpora, versions or models, or a set that omits or
//! duplicates a prompt (C-006).
//!
//! The partition is derived, not read back from disk: the prompt at corpus
//! position `p` belongs to the shard with index `p mod count`, so a shard's
//! `k`-th result sits at position `index + k * count`. Each partial record
//! carries its shard's index, count and the corpus's total prompt count, so a
//! merge can rebuild the whole-corpus order and prove that the set covers every
//! prompt exactly once without reloading the corpus.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::record::{PromptResult, RunRecord};
use crate::run::{self, RunFailure};
use crate::score;

/// Combines the partial records of one corpus and one model into a single
/// whole-corpus record.
///
/// The result has the same shape as an unsharded run's record, with no shard
/// descriptor, so a comparison treats a merged run like any other (FEAT-023).
pub fn combine(paths: &[PathBuf]) -> Result<RunRecord, RunFailure> {
    if paths.is_empty() {
        return Err(RunFailure::Usage(
            "`merge` needs at least one partial run record".to_string(),
        ));
    }

    let mut records = Vec::with_capacity(paths.len());
    for path in paths {
        records.push((path, run::read_record(path)?));
    }
    let first = &records[0].1;

    for (path, record) in &records {
        if record.corpus_id != first.corpus_id || record.corpus_version != first.corpus_version {
            return Err(RunFailure::Usage(format!(
                "cannot merge `{}`: it measures {}/{} while `{}` measures {}/{}",
                path.display(),
                record.corpus_id,
                record.corpus_version,
                paths[0].display(),
                first.corpus_id,
                first.corpus_version
            )));
        }
        if record.authoring_model_id != first.authoring_model_id {
            return Err(RunFailure::Usage(format!(
                "cannot merge `{}`: it measures model `{}` while `{}` measures `{}`",
                path.display(),
                record.authoring_model_id,
                paths[0].display(),
                first.authoring_model_id
            )));
        }
        if record.coverage != first.coverage {
            return Err(RunFailure::Usage(format!(
                "cannot merge `{}`: its coverage findings differ from `{}`",
                path.display(),
                paths[0].display()
            )));
        }
    }

    let mut shards: BTreeMap<usize, &RunRecord> = BTreeMap::new();
    let mut count: Option<usize> = None;
    let mut total: Option<usize> = None;
    for (path, record) in &records {
        let Some(shard) = record.shard else {
            return Err(RunFailure::Usage(format!(
                "cannot merge `{}`: it is a whole-corpus record, not a shard",
                path.display()
            )));
        };
        if shard.count == 0 || shard.index >= shard.count {
            return Err(RunFailure::Usage(format!(
                "cannot merge `{}`: it names an invalid shard {}/{}",
                path.display(),
                shard.index,
                shard.count
            )));
        }
        check_consistent(&mut count, shard.count, path)?;
        check_consistent(&mut total, shard.total, path)?;
        if shards.insert(shard.index, record).is_some() {
            return Err(RunFailure::Usage(format!(
                "cannot merge: more than one record covers shard {} of {}",
                shard.index, shard.count
            )));
        }
    }
    let count = count.expect("at least one record");
    let total = total.expect("at least one record");

    for index in 0..count {
        if !shards.contains_key(&index) {
            return Err(RunFailure::Usage(format!(
                "cannot merge: no record covers shard {index} of {count}; the merged record would omit its prompts"
            )));
        }
    }

    let mut placed: Vec<(usize, &PromptResult)> = Vec::with_capacity(total);
    let mut seen = BTreeSet::new();
    for (index, record) in &shards {
        let expected = shard_size(total, count, *index);
        if record.prompts.len() != expected {
            return Err(RunFailure::Usage(format!(
                "cannot merge: shard {index}/{count} holds {} of its {expected} prompts; the merged record would omit a prompt",
                record.prompts.len()
            )));
        }
        for (offset, result) in record.prompts.iter().enumerate() {
            let position = index + offset * count;
            if position >= total {
                return Err(RunFailure::Usage(format!(
                    "cannot merge: shard {index}/{count} holds a prompt past the corpus's {total} positions"
                )));
            }
            if !seen.insert(result.id.clone()) {
                return Err(RunFailure::Usage(format!(
                    "cannot merge: prompt `{}` appears in more than one record",
                    result.id
                )));
            }
            placed.push((position, result));
        }
    }

    placed.sort_by_key(|(position, _)| *position);
    let mut results = Vec::with_capacity(placed.len());
    for (expected, (position, result)) in placed.into_iter().enumerate() {
        if position != expected {
            return Err(RunFailure::Usage(
                "cannot merge: the records do not cover every prompt exactly once".to_string(),
            ));
        }
        results.push(result.clone());
    }
    if results.len() != total {
        return Err(RunFailure::Usage(
            "cannot merge: the records do not cover every prompt exactly once".to_string(),
        ));
    }

    Ok(score::summarize_parts(
        &first.corpus_id,
        &first.corpus_version,
        first.coverage.clone(),
        first.model.clone(),
        results,
    ))
}

fn check_consistent(seen: &mut Option<usize>, value: usize, path: &Path) -> Result<(), RunFailure> {
    match seen {
        Some(known) if *known != value => Err(RunFailure::Usage(format!(
            "cannot merge `{}`: it covers a different shard layout than the other records",
            path.display()
        ))),
        _ => {
            *seen = Some(value);
            Ok(())
        }
    }
}

/// How many corpus positions the shard `index` of `count` selects out of
/// `total`.
fn shard_size(total: usize, count: usize, index: usize) -> usize {
    if index >= total {
        0
    } else {
        (total - 1 - index) / count + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::parse_model_spec;
    use crate::run::Shard;
    use crate::testing::{corpus_dir, replay_dir, write_usage, TempDir};

    const SCENE: &str = r##"{"scene":{"id":"s","projectId":"eval","name":"S","formatVersion":"0.2","canvas":{"width":100,"height":100,"background":"#ffffff"},"elements":[]}}"##;
    const ALL_QUALITIES: &str = r#"{"fidelity":0.8,"qualities":{"depth":true,"relative_placement":true,"shared_anchors":true,"subject_accuracy":true}}"#;
    const NONE_QUALITIES: &str = r#"{"fidelity":0.4,"qualities":{}}"#;

    fn write_record(dir: &TempDir, name: &str, record: &RunRecord) -> PathBuf {
        let path = dir.path().join(name);
        let text = serde_json::to_string_pretty(record).expect("serializes");
        std::fs::write(&path, text).expect("writes");
        path
    }

    fn mutate(path: &Path, field: &str, value: serde_json::Value) {
        let text = std::fs::read_to_string(path).expect("reads");
        let mut root: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        root[field] = value;
        std::fs::write(
            path,
            serde_json::to_string_pretty(&root).expect("serializes"),
        )
        .expect("writes");
    }

    /// A whole-corpus run and its two partial shard records, all of one corpus
    /// and one model, with the partials written to files.
    struct Fixture {
        _records: TempDir,
        whole: RunRecord,
        a: PathBuf,
        b: PathBuf,
    }

    fn fixture() -> Fixture {
        let replies = replay_dir(&[("p1", SCENE, NONE_QUALITIES), ("p2", SCENE, ALL_QUALITIES)]);
        write_usage(replies.path(), "p1", 1_000);
        write_usage(replies.path(), "p2", 3_000);
        let spec = parse_model_spec(&format!("replay:{}", replies.path().display())).expect("spec");
        let corpus = corpus_dir();
        let whole = run::execute(&corpus, &spec, None, None).expect("whole run");
        let first = run::execute(&corpus, &spec, Some(Shard { index: 0, count: 2 }), None)
            .expect("shard 0");
        let second = run::execute(&corpus, &spec, Some(Shard { index: 1, count: 2 }), None)
            .expect("shard 1");

        let records = TempDir::new("merge");
        let a = write_record(&records, "0.json", &first);
        let b = write_record(&records, "1.json", &second);
        Fixture {
            _records: records,
            whole,
            a,
            b,
        }
    }

    #[test]
    fn merges_partials_into_the_whole_corpus_record() {
        let fixture = fixture();
        let merged = combine(&[fixture.a, fixture.b]).expect("merges");

        assert_eq!(merged.prompts, fixture.whole.prompts);
        assert_eq!(
            merged.compile_success_rate,
            fixture.whole.compile_success_rate
        );
        assert_eq!(merged.fidelity_score, fixture.whole.fidelity_score);
        assert_eq!(merged.median_tokens, fixture.whole.median_tokens);
        assert_eq!(merged.hard_end, fixture.whole.hard_end);
        assert_eq!(merged.thresholds, fixture.whole.thresholds);
        assert_eq!(
            merged.id, fixture.whole.id,
            "a merged run has the corpus id"
        );
        assert_eq!(
            merged.shard, None,
            "a merged record covers the whole corpus"
        );
    }

    #[test]
    fn refuses_a_mismatched_corpus_version() {
        let fixture = fixture();
        mutate(&fixture.b, "corpusVersion", serde_json::json!("2.0.0"));
        let error = combine(&[fixture.a, fixture.b]).expect_err("refused");
        assert_eq!(error.code(), 2);
        assert!(error.message().contains("measures"), "{}", error.message());
    }

    #[test]
    fn refuses_a_mismatched_model() {
        let fixture = fixture();
        mutate(
            &fixture.b,
            "authoringModelId",
            serde_json::json!("replay:other"),
        );
        let error = combine(&[fixture.a, fixture.b]).expect_err("refused");
        assert!(
            error.message().contains("model `replay:other`"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn refuses_a_duplicated_shard() {
        let fixture = fixture();
        let error = combine(&[fixture.a.clone(), fixture.b, fixture.a]).expect_err("refused");
        assert!(
            error
                .message()
                .contains("more than one record covers shard"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn refuses_a_set_that_omits_a_shard() {
        let fixture = fixture();
        let error = combine(&[fixture.a]).expect_err("refused");
        assert!(
            error.message().contains("no record covers shard 1"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn refuses_a_shard_that_omits_one_of_its_prompts() {
        let fixture = fixture();
        // Empty the first shard: it now holds none of its one prompt.
        mutate(&fixture.a, "prompts", serde_json::json!([]));
        let error = combine(&[fixture.a, fixture.b]).expect_err("refused");
        assert!(error.message().contains("omit"), "{}", error.message());
    }

    #[test]
    fn refuses_a_whole_corpus_record() {
        let fixture = fixture();
        let dir = TempDir::new("merge-whole");
        let path = write_record(&dir, "whole.json", &fixture.whole);
        let error = combine(&[path]).expect_err("refused");
        assert!(
            error.message().contains("whole-corpus"),
            "{}",
            error.message()
        );
    }

    #[test]
    fn refuses_no_records_at_all() {
        let error = combine(&[]).expect_err("refused");
        assert_eq!(error.code(), 2);
    }
}
