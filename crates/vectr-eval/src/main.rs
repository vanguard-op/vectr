//! `vectr-eval` evaluation-harness entry point (FEAT-023, C-006).
//!
//! Parses the command line, runs exactly one command, prints its output and
//! exits with the command's status. The harness is maintainer-only and gated by
//! `enable_eval_harness`; the command grammar, the run record and the exit
//! codes are the contract [`cli`] owns. The corpus is [`corpus`], a model is
//! reached through [`provider`], a scene runs through [`toolchain`], and
//! [`score`] turns the results into the run record and the regression
//! comparison.

mod cli;
mod corpus;
mod json;
mod provider;
mod record;
mod run;
mod score;
mod toolchain;

#[cfg(test)]
mod testing;

use std::ffi::OsString;

fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let report = match cli::parse(args) {
        Ok(command) => cli::run(command),
        Err(message) => cli::usage_report(&message),
    };
    print!("{}", report.stdout);
    eprint!("{}", report.stderr);
    std::process::exit(report.code);
}
