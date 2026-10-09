//! `vectr-eval` command parsing and dispatch (C-006).
//!
//! The grammar and exit codes are frozen by the contract: `run` loads a corpus
//! and scores it against a model, `compare` flags regressions between two runs,
//! with 0 success, 1 a run that cannot be scored, and 2 usage, an unreadable
//! corpus, or a model that cannot be reached. The harness is gated by
//! `enable_eval_harness` (off by default, maintainer-only), read from
//! `VECTR_ENABLE_EVAL_HARNESS`; without it the command reports the harness as
//! disabled and exits 2 (release.md, "Rollout Phases & Feature Flags").

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::provider;
use crate::run;

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
  vectr-eval run --corpus <dir> --model <provider:model> [--out <file>]
  vectr-eval compare <run-a> <run-b>

<provider:model> names the authoring model: openai, anthropic, google or
opencode with a pinned model, or replay with a directory of recorded replies.
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
        /// Where to write the run record; stdout when absent.
        out: Option<PathBuf>,
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
        "compare" => parse_compare(iter.collect()),
        other => Err(format!("unknown command `{other}`")),
    }
}

fn parse_run(args: Vec<OsString>) -> Result<Command, String> {
    let mut corpus = None;
    let mut model = None;
    let mut out = None;
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].to_string_lossy().into_owned();
        match flag.as_str() {
            "--corpus" => corpus = Some(take_value(&args, &mut index, "--corpus")?),
            "--model" => model = Some(take_value(&args, &mut index, "--model")?),
            "--out" => out = Some(take_value(&args, &mut index, "--out")?),
            other => return Err(format!("unexpected argument `{other}` for `run`")),
        }
        index += 1;
    }
    let corpus = corpus.ok_or("`run` needs `--corpus <dir>`")?;
    let model = model.ok_or("`run` needs `--model <provider:model>`")?;
    Ok(Command::Run {
        corpus: PathBuf::from(corpus),
        model,
        out: out.map(PathBuf::from),
    })
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
        Command::Run { corpus, model, out } => {
            if let Some(report) = disabled() {
                return report;
            }
            run_corpus(&corpus, &model, out.as_deref())
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

fn run_corpus(corpus: &Path, model: &str, out: Option<&Path>) -> Report {
    let spec = match provider::parse_model_spec(model) {
        Ok(spec) => spec,
        Err(message) => return usage_report(&message),
    };
    let record = match run::execute(corpus, &spec) {
        Ok(record) => record,
        Err(failure) => return Report::failure(failure.code(), failure.message()),
    };
    let json = match serde_json::to_string_pretty(&record) {
        Ok(json) => json,
        Err(error) => {
            return Report::failure(
                EXIT_USAGE,
                format!("cannot serialize the run record: {error}"),
            )
        }
    };
    match out {
        Some(path) => {
            if let Err(error) = write_atomic(path, json.as_bytes()) {
                return Report::failure(
                    EXIT_USAGE,
                    format!("cannot write run record `{}`: {error}", path.display()),
                );
            }
            let mut stdout = summary(&record);
            stdout.push_str(&format!("\nwrote run record to {}\n", path.display()));
            Report {
                code: EXIT_SUCCESS,
                stdout,
                stderr: String::new(),
            }
        }
        None => Report::ok(&json),
    }
}

fn summary(record: &crate::record::RunRecord) -> String {
    let total = record.prompts.len();
    let compiled = record
        .prompts
        .iter()
        .filter(|result| result.compiled)
        .count();
    format!(
        "compile success {:.1}% ({compiled}/{total}), fidelity {:.1}%, hard-end rubric {:.1}%",
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
            "--out",
            "run.json",
        ]))
        .expect("parses");
        assert_eq!(
            command,
            Command::Run {
                corpus: PathBuf::from("corpus"),
                model: "openai:gpt".to_string(),
                out: Some(PathBuf::from("run.json")),
            }
        );
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
}
