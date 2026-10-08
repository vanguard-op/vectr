//! `vectr` command-line entry point (C-004).
//!
//! Parses the command line, runs exactly one command, prints its output, and
//! exits with the command's status so a pipeline can gate on the result. The
//! command grammar and the exit codes are the interface the contract freezes;
//! [`cli`] owns both, [`init`] scaffolds a project, and [`output`] writes files
//! without leaving a partial result behind (NFR-011).

mod cli;
mod init;
mod output;

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
