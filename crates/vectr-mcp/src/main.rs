//! `vectr-mcp` MCP server entry point (FEAT-019, C-005).
//!
//! Runs the Vectr toolchain as an agent's discoverable tools. The default
//! transport is stdio, which an MCP host launches as a subprocess and drives
//! with newline-delimited JSON-RPC; `--bind <address>` serves the same tools
//! over HTTP on a loopback address. The filesystem scope starts at the working
//! directory and widens only with `--allow <dir>` (NFR-024).

mod http;
mod output;
mod protocol;
mod scope;
mod tools;

use std::ffi::OsString;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use protocol::Server;
use scope::Scope;

/// The usage block shared by `--help` and every usage error.
const USAGE: &str = "\
Usage:
  vectr-mcp [--allow <dir>]... [--bind <address>]

Runs the Vectr MCP server. The default transport is stdio; `--bind <address>`
serves the same tools over HTTP on <address>, which should be a loopback
address such as 127.0.0.1:8765. `--allow <dir>` widens the filesystem scope
beyond the working directory.";

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            eprint!("{}", failure.message);
            ExitCode::from(failure.code)
        }
    }
}

/// One parsed command line.
#[derive(Debug, PartialEq)]
enum Command {
    /// Print the usage and exit zero.
    Help,
    /// Print the version and exit zero.
    Version,
    /// Run the server.
    Serve {
        /// Extra roots the filesystem scope is widened to.
        allow: Vec<PathBuf>,
        /// The HTTP address to bind, when HTTP rather than stdio is requested.
        bind: Option<String>,
    },
}

/// A failure with the process exit status the command reports.
struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    fn usage(message: &str) -> Self {
        Self {
            code: 2,
            message: format!("error: {message}\n\n{USAGE}\n"),
        }
    }

    fn runtime(error: io::Error) -> Self {
        Self {
            code: 1,
            message: format!("error: {error}\n"),
        }
    }
}

fn run(args: Vec<OsString>) -> Result<(), Failure> {
    match parse(args).map_err(|message| Failure::usage(&message))? {
        Command::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Command::Version => {
            println!("vectr-mcp {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Serve { allow, bind } => {
            let mut roots = vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))];
            roots.extend(allow);
            let scope = Scope::new(roots);

            match bind {
                Some(address) => http::serve(&scope, &address).map_err(Failure::runtime),
                None => {
                    let stdin = io::stdin();
                    let mut stdout = io::stdout().lock();
                    serve_stream(&scope, stdin.lock(), &mut stdout).map_err(Failure::runtime)
                }
            }
        }
    }
}

/// Drives the server over a byte stream, one JSON-RPC message per line.
fn serve_stream<R: BufRead, W: Write>(scope: &Scope, reader: R, writer: &mut W) -> io::Result<()> {
    let server = Server::new(scope);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle_line(&line) {
            serde_json::to_writer(&mut *writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }
    Ok(())
}

fn parse(args: Vec<OsString>) -> Result<Command, String> {
    let mut allow = Vec::new();
    let mut bind: Option<String> = None;

    let mut index = 0;
    while index < args.len() {
        let text = args[index].to_string_lossy().into_owned();
        match text.as_str() {
            "-h" | "--help" | "help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--stdio" => {}
            "--bind" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--bind` needs an address".to_string())?;
                bind = Some(value.to_string_lossy().into_owned());
            }
            "--allow" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "`--allow` needs a directory".to_string())?;
                allow.push(PathBuf::from(value));
            }
            other if other.starts_with("--bind=") => {
                bind = Some(other["--bind=".len()..].to_string());
            }
            other if other.starts_with("--allow=") => {
                allow.push(PathBuf::from(&other["--allow=".len()..]));
            }
            other => return Err(format!("unknown option `{other}`")),
        }
        index += 1;
    }

    Ok(Command::Serve { allow, bind })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(OsString::from).collect())
    }

    fn tempdir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vectr-mcp-main-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("creates the temp dir");
        dir
    }

    #[test]
    fn no_arguments_serves_over_stdio() {
        assert_eq!(
            parse_args(&[]).unwrap(),
            Command::Serve {
                allow: Vec::new(),
                bind: None
            }
        );
    }

    #[test]
    fn allow_and_bind_are_parsed_in_both_forms() {
        assert_eq!(
            parse_args(&["--allow", "/tmp", "--bind", "127.0.0.1:8765"]).unwrap(),
            parse_args(&["--allow=/tmp", "--bind=127.0.0.1:8765"]).unwrap()
        );
        match parse_args(&["--allow", "/tmp", "--bind=127.0.0.1:1"]).unwrap() {
            Command::Serve { allow, bind } => {
                assert_eq!(allow, vec![PathBuf::from("/tmp")]);
                assert_eq!(bind.as_deref(), Some("127.0.0.1:1"));
            }
            other => panic!("expected serve, got {other:?}"),
        }
    }

    #[test]
    fn help_version_and_unknown_options_are_classified() {
        assert_eq!(parse_args(&["--help"]).unwrap(), Command::Help);
        assert_eq!(parse_args(&["--version"]).unwrap(), Command::Version);
        assert!(parse_args(&["--nope"]).is_err());
        assert!(parse_args(&["--bind"]).is_err());
        assert!(parse_args(&["--allow"]).is_err());
    }

    #[test]
    fn the_stdio_loop_answers_requests_and_ignores_notifications() {
        let dir = tempdir("stdio");
        let scope = Scope::new(vec![dir]);
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n",
        );
        let mut output = Vec::new();
        serve_stream(&scope, input.as_bytes(), &mut output).expect("serves");

        let text = String::from_utf8(output).expect("utf-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "one response per request: {text}");
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("json");
        let second: serde_json::Value = serde_json::from_str(lines[1]).expect("json");
        assert_eq!(first["id"], 1);
        assert_eq!(second["id"], 2);
        assert_eq!(second["result"]["tools"].as_array().map(Vec::len), Some(4));
    }

    #[test]
    fn every_emitted_message_is_one_line() {
        let dir = tempdir("single-line");
        let scope = Scope::new(vec![dir]);
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n";
        let mut output = Vec::new();
        serve_stream(&scope, input.as_bytes(), &mut output).expect("serves");
        let text = String::from_utf8(output).expect("utf-8");
        assert_eq!(text.matches('\n').count(), 1, "{text}");
    }
}
