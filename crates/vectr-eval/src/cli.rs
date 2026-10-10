//! `vectr-eval` command parsing and dispatch (C-006).
//!
//! The grammar and exit codes are frozen by the contract: `run` loads a corpus
//! and scores it against a model — over the whole corpus or one shard of it,
//! optionally logging each prompt's artifacts and resuming from them — `merge`
//! combines the partial records of one corpus and model into one record, and
//! `compare` flags regressions between two runs, with 0 success, 1 a run that
//! cannot be scored, and 2 usage, an unreadable corpus, a merge of mismatched or
//! incomplete records, or a model that cannot be reached. The harness is gated
//! by `enable_eval_harness` (off by default, maintainer-only), read from
//! `VECTR_ENABLE_EVAL_HARNESS`; without it the command reports the harness as
//! disabled and exits 2 (release.md, "Rollout Phases & Feature Flags").

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::merge;
use crate::provider;
use crate::run::{self, Shard};

/// The command succeeded.
pub const EXIT_SUCCESS: i32 = 0;
/// The run cannot be scored: the corpus omits coverage, or no prompt scored.
pub const EXIT_UNSCORABLE: i32 = 1;
/// Usage, an unreadable corpus, or a model that cannot be reached.
pub const EXIT_USAGE: i32 = 2;

/// The environment variable that enables the maintainer-only harness.
pub const ENABLE_ENV: &str = "VECTR_ENABLE_EVAL_HARNESS";

/// The usage block shared by the help text and every usage error.
const USAGE: &str = "\
Usage:
  vectr-eval run --corpus <dir> --model <provider:model> [--shard <index>/<count>] [--log-dir <dir>] [--resume] [--out <file>]
  vectr-eval merge <record...> --out <file>
  vectr-eval compare <run-a> <run-b>

<provider:model> names the authoring model: openai, anthropic, google or
opencode with a pinned model, or replay with a directory of recorded replies.
`--shard <index>/<count>` runs only the prompts at corpus positions where
`position mod count == index` and writes a partial record for that shard;
`merge` combines the partials of one corpus and model into one record.
`--resume` scores a prompt from the artifacts already recorded in `--log-dir`
instead of calling the model again, so a re-run resumes a partial shard; it
requires `--log-dir`.
The harness is maintainer-only and off by default; set VECTR_ENABLE_EVAL_HARNESS=1
to enable it.";

/// One parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Print the usage and exit zero.
    Help,
    /// Print the version and exit zero.
    Version,
    /// Run a corpus through a model.
    Run {
        /// The corpus directory.
        corpus: PathBuf,
        /// The `provider:model` reference.
        model: String,
        /// The shard to run, when the run is sharded.
        shard: Option<Shard>,
        /// Where to write each prompt's artifacts, when logging is requested.
        log_dir: Option<PathBuf>,
        /// Whether to score a prompt from its recorded artifacts instead of
        /// calling the model again (requires `log_dir`).
        resume: bool,
        /// Where to write the run record; stdout when absent.
        out: Option<PathBuf>,
    },
    /// Combine partial shard records into one whole-corpus record.
    Merge {
        /// The partial run records.
        records: Vec<PathBuf>,
        /// Where to write the merged record.
        out: PathBuf,
    },
    /// Compare two run records of the same corpus.
    Compare {
        /// The baseline run record.
        baseline: PathBuf,
        /// The candidate run record.
        candidate: PathBuf,
    },
}

/// The text to print and the status to exit with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The process exit code.
    pub code: i32,
    /// Text for standard output.
    pub stdout: String,
    /// Text for standard error.
    pub stderr: String,
}

/// Parses the command line.
pub fn parse(args: Vec<OsString>) -> Result<Command, String> {
    let mut iter = args.into_iter();
    let Some(first) = iter.next() else {
        return Ok(Command::Help);
    };
    let first = first.to_string_lossy().into_owned();
    match first.as_str() {
        "help" | "--help" | "-h" => Ok(Command::Help),
        "version" | "--version" | "-V" => Ok(Command::Version),
        "run" => parse_run(iter.collect()),
        "merge" => parse_merge(iter.collect()),
        "compare" => parse_compare(iter.collect()),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn parse_run(args: Vec<OsString>) -> Result<Command, String> {
    let mut corpus = None;
    let mut model = None;
    let mut shard = None;
    let mut log_dir = None;
    let mut resume = false;
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].to_string_lossy().into_owned();
        match flag.as_str() {
            "--corpus" => corpus = Some(take_value(&args, &mut index, "--corpus")?),
            "--model" => model = Some(take_value(&args, &mut index, "--model")?),
            "--shard" => shard = Some(Shard::parse(&take_value(&args, &mut index, "--shard")?)?),
            "--log-dir" => {
                log_dir = Some(PathBuf::from(take_value(&args, &mut index, "--log-dir")?))
            }
            "--resume" => resume = true,
            "--out" => out = Some(take_value(&args, &mut index, "--out")?),
            other => return Err(format!("unexpected argument `{other}` for `run`")),
        }
        index += 1;
    }
    let corpus = corpus.ok_or("`run` needs `--corpus <dir>`")?;
    let model = model.ok_or("`run` needs `--model <provider:model>`")?;
    if resume && log_dir.is_none() {
        return Err("`--resume` requires `--log-dir`".to_string());
    }
    Ok(Command::Run {
        corpus: PathBuf::from(corpus),
        model,
        shard,
        log_dir,
        resume,
        out: out.map(PathBuf::from),
    })
}

fn parse_merge(args: Vec<OsString>) -> Result<Command, String> {
    let mut records = Vec::new();
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].to_string_lossy().into_owned();
        if flag == "--out" {
            out = Some(PathBuf::from(take_value(&args, &mut index, "--out")?));
        } else if flag.starts_with("--") {
            return Err(format!("unexpected argument `{flag}` for `merge`"));
        } else {
            records.push(PathBuf::from(flag));
        }
        index += 1;
    }
    if records.is_empty() {
        return Err("`merge` needs at least one partial run record".to_string());
    }
    let out = out.ok_or("`merge` needs `--out <file>`")?;
    Ok(Command::Merge { records, out })
}

fn parse_compare(args: Vec<OsString>) -> Result<Command, String> {
    if args.len() != 2 {
        return Err("`compare` needs two run records".to_string());
    }
    let mut paths = args.into_iter().map(PathBuf::from);
    let baseline = paths.next().expect("two arguments");
    let candidate = paths.next().expect("two arguments");
    Ok(Command::Compare {
        baseline,
        candidate,
    })
}

fn take_value(args: &[OsString], index: &mut usize, flag: &str) -> Result<String, String> {
    *index += 1;
    args.get(*index)
        .map(|value| value.to_string_lossy().into_owned())
        .ok_or_else(|| format!("`{flag}` needs a value"))
}

/// Runs one command.
pub fn run(command: Command) -> Report {
    match command {
        Command::Help => Report::ok(USAGE),
        Command::Version => Report::ok(concat!("vectr-eval ", env!("CARGO_PKG_VERSION"))),
        Command::Run {
            corpus,
            model,
            shard,
            log_dir,
            resume,
            out,
        } => {
            if let Some(report) = disabled() {
                return report;
            }
            run_corpus(
                &corpus,
                &model,
                shard,
                log_dir.as_deref(),
                resume,
                out.as_deref(),
            )
        }
        Command::Merge { records, out } => {
            if let Some(report) = disabled() {
                return report;
            }
            match merge::combine(&records) {
                Ok(record) => write_record(&record, &out, "merged"),
                Err(failure) => Report::failure(failure.code(), failure.message()),
            }
        }
        Command::Compare {
            baseline,
            candidate,
        } => {
            if let Some(report) = disabled() {
                return report;
            }
            match run::compare(&baseline, &candidate) {
                Ok(comparison) => match serde_json::to_string_pretty(&comparison) {
                    Ok(text) => Report::ok(&text),
                    Err(error) => Report::failure(
                        EXIT_USAGE,
                        format!("cannot serialize the comparison: {error}"),
                    ),
                },
                Err(failure) => Report::failure(failure.code(), failure.message()),
            }
        }
    }
}

fn run_corpus(
    corpus: &Path,
    model: &str,
    shard: Option<Shard>,
    log_dir: Option<&Path>,
    resume: bool,
    out: Option<&Path>,
) -> Report {
    let spec = match provider::parse_model_spec(model) {
        Ok(spec) => spec,
        Err(message) => return usage_report(&message),
    };
    let record = match run::execute(corpus, &spec, shard, log_dir, resume) {
        Ok(record) => record,
        Err(failure) => return Report::failure(failure.code(), failure.message()),
    };
    match out {
        Some(path) => write_record(&record, path, "run"),
        None => match serde_json::to_string_pretty(&record) {
            Ok(json) => Report::ok(&json),
            Err(error) => Report::failure(
                EXIT_USAGE,
                format!("cannot serialize the run record: {error}"),
            ),
        },
    }
}

/// Writes a record to `path`, or prints it when `path` is absent, and prints
/// the summary line for it. `kind` names the record in the confirmation
/// ("run" or "merged") (NFR-011, no partial output).
fn write_record(record: &crate::record::RunRecord, path: &Path, kind: &str) -> Report {
    let json = match serde_json::to_string_pretty(record) {
        Ok(json) => json,
        Err(error) => {
            return Report::failure(
                EXIT_USAGE,
                format!("cannot serialize the run record: {error}"),
            )
        }
    };
    if let Err(error) = write_atomic(path, json.as_bytes()) {
        return Report::failure(
            EXIT_USAGE,
            format!("cannot write run record `{}`: {error}", path.display()),
        );
    }
    let mut stdout = summary(record);
    stdout.push_str(&format!("\nwrote {kind} record to {}\n", path.display()));
    Report {
        code: EXIT_SUCCESS,
        stdout,
        stderr: String::new(),
    }
}

fn summary(record: &crate::record::RunRecord) -> String {
    let total = record.prompts.len();
    let compiled = record
        .prompts
        .iter()
        .filter(|result| result.compiled)
        .count();
    let tokens = match record.median_tokens {
        Some(median) => format!(
            "median tokens {median} (budget {})",
            crate::score::TOKEN_BUDGET
        ),
        None => "median tokens none reported".to_string(),
    };
    format!(
        "compile success {:.1}% ({compiled}/{total}), fidelity {:.1}%, hard-end rubric {:.1}%, {tokens}",
        record.compile_success_rate * 100.0,
        record.fidelity_score * 100.0,
        record.hard_end.rubric_pass_rate * 100.0,
    )
}

/// The report for a disabled harness, when it is not enabled.
fn disabled() -> Option<Report> {
    if enabled_value(std::env::var(ENABLE_ENV).ok().as_deref()) {
        None
    } else {
        Some(usage_report(&format!(
            "the evaluation harness is disabled; set {ENABLE_ENV}=1 to enable it"
        )))
    }
}

/// Whether the flag value enables the harness.
pub fn enabled_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("on")
    )
}

/// The usage report for a parse or usage failure.
pub fn usage_report(message: &str) -> Report {
    Report {
        code: EXIT_USAGE,
        stdout: String::new(),
        stderr: format!("error: {message}\n\n{USAGE}\n"),
    }
}

/// Writes a file without leaving a partial result behind (NFR-011).
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

impl Report {
    fn ok(text: &str) -> Self {
        Self {
            code: EXIT_SUCCESS,
            stdout: format!("{text}\n"),
            stderr: String::new(),
        }
    }

    fn failure(code: i32, message: impl Into<String>) -> Self {
        let message: String = message.into();
        Self {
            code,
            stdout: String::new(),
            stderr: format!("error: {message}\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_a_run_command() {
        let command = parse(args(&[
            "run",
            "--corpus",
            "corpus",
            "--model",
            "openai:gpt",
            "--shard",
            "2/5",
            "--log-dir",
            "logs",
            "--resume",
            "--out",
            "run.json",
        ]))
        .expect("parses");
        assert_eq!(
            command,
            Command::Run {
                corpus: PathBuf::from("corpus"),
                model: "openai:gpt".to_string(),
                shard: Some(Shard { index: 2, count: 5 }),
                log_dir: Some(PathBuf::from("logs")),
                resume: true,
                out: Some(PathBuf::from("run.json")),
            }
        );
    }

    #[test]
    fn parses_a_run_command_without_a_shard_or_log_directory() {
        let command =
            parse(args(&["run", "--corpus", "c", "--model", "openai:gpt"])).expect("parses");
        assert_eq!(
            command,
            Command::Run {
                corpus: PathBuf::from("c"),
                model: "openai:gpt".to_string(),
                shard: None,
                log_dir: None,
                resume: false,
                out: None,
            }
        );
    }

    #[test]
    fn refuses_a_resume_without_a_log_directory() {
        let error = parse(args(&[
            "run",
            "--corpus",
            "c",
            "--model",
            "openai:gpt",
            "--resume",
        ]))
        .expect_err("refused");
        assert!(error.contains("--log-dir"), "{error}");
    }

    #[test]
    fn parses_a_merge_command() {
        assert_eq!(
            parse(args(&["merge", "0.json", "1.json", "--out", "all.json"])).expect("parses"),
            Command::Merge {
                records: vec![PathBuf::from("0.json"), PathBuf::from("1.json")],
                out: PathBuf::from("all.json"),
            }
        );
    }

    #[test]
    fn rejects_a_malformed_shard_or_merge() {
        let shard = parse(args(&[
            "run", "--corpus", "c", "--model", "m", "--shard", "3/2",
        ]))
        .expect_err("refused");
        assert!(shard.contains("shard"), "{shard}");
        assert!(parse(args(&["merge", "a.json"])).is_err(), "needs --out");
        assert!(
            parse(args(&["merge", "--out", "a.json"])).is_err(),
            "needs a record"
        );
        assert!(parse(args(&["merge", "a.json", "--nope"])).is_err());
    }

    #[test]
    fn parses_a_compare_command_and_help() {
        assert_eq!(
            parse(args(&["compare", "a.json", "b.json"])).expect("parses"),
            Command::Compare {
                baseline: PathBuf::from("a.json"),
                candidate: PathBuf::from("b.json"),
            }
        );
        assert_eq!(parse(Vec::new()).expect("parses"), Command::Help);
    }

    #[test]
    fn rejects_a_run_missing_its_required_flags() {
        assert!(parse(args(&["run", "--model", "openai:gpt"])).is_err());
        assert!(parse(args(&["run", "--corpus", "c"])).is_err());
        assert!(parse(args(&["run", "--corpus"])).is_err());
    }

    #[test]
    fn rejects_an_unknown_command() {
        let error = parse(args(&["frobnicate"])).expect_err("refused");
        assert!(error.contains("unknown command"), "{error}");
    }

    #[test]
    fn the_harness_flag_is_off_unless_explicitly_enabled() {
        assert!(!enabled_value(None));
        assert!(!enabled_value(Some("0")));
        assert!(!enabled_value(Some("false")));
        assert!(enabled_value(Some("1")));
        assert!(enabled_value(Some("true")));
    }

    #[test]
    fn the_summary_reports_the_median_token_count_against_the_budget() {
        let text = summary(&sample_record(Some(12_345)));
        assert!(text.contains("median tokens 12345"), "{text}");
        assert!(text.contains("20000"), "names the budget: {text}");
    }

    #[test]
    fn the_summary_reports_a_missing_median_as_unreported() {
        let text = summary(&sample_record(None));
        assert!(text.contains("median tokens none reported"), "{text}");
    }

    fn sample_record(median_tokens: Option<u64>) -> crate::record::RunRecord {
        crate::record::RunRecord {
            id: "c@1.0.0#replay:x".to_string(),
            corpus_id: "c".to_string(),
            corpus_version: "1.0.0".to_string(),
            authoring_model_id: "replay:x".to_string(),
            model: crate::record::ModelInfo {
                id: "replay:x".to_string(),
                name: "x".to_string(),
                version: "x".to_string(),
            },
            compile_success_rate: 1.0,
            fidelity_score: 1.0,
            median_tokens,
            regression: false,
            notes: None,
            hard_end: crate::record::HardEndScores {
                complex_prompts: 0,
                rubric_pass_rate: 0.0,
                qualities: std::collections::BTreeMap::new(),
            },
            thresholds: crate::record::Thresholds {
                compile_success: true,
                fidelity: true,
                hard_end: true,
                tokens: true,
            },
            coverage: crate::record::CoverageReport {
                satisfied: true,
                omissions: Vec::new(),
            },
            shard: None,
            prompts: Vec::new(),
        }
    }
}
