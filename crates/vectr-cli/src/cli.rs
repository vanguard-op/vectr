//! `vectr` command parsing and dispatch (C-004).
//!
//! The grammar and exit codes are frozen by the contract: `init`, `validate`,
//! `compile` and `export`, with 0 success, 1 invalid scene, 2 usage or unreadable
//! input, 3 compilation failure, 4 missing export dependency, and 5 output I/O
//! failure. Parsing is hand-rolled rather than pulled from a CLI crate so the
//! binary depends only on the engine and can control the exit code of every
//! path, including "no arguments prints usage and exits zero".
//!
//! Every command returns a [`Report`] holding the text to print and the status
//! to exit with; only [`main`](crate::main) touches the process. Diagnostics are
//! the engine's structured findings, so a failure names its code, message and
//! location (NFR-011).
//!
//! `validate`, `compile` and `export` load the assets the scene's project
//! provides — its palette, stroke profiles, and fonts — and compile against
//! them, so a scene's style and font references resolve to concrete values
//! before anything is written (FEAT-005, FEAT-024).

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use vectr_core::compiler::FONT;
use vectr_core::export::png as png_export;
use vectr_core::export::svg as svg_export;
use vectr_core::{
    compile_with_style, export_png_reporting, export_svg_reporting, parse as parse_scene_source,
    validate as validate_scene_model, validate_gradient_usage, validate_palette_usage, Diagnostic,
    Diagnostics, RasterOptions, SvgOptions,
};

use crate::init;
use crate::output::write_atomic;
use crate::project::ProjectAssets;

/// The command succeeded.
pub const EXIT_SUCCESS: i32 = 0;
/// The scene is invalid: it did not parse or did not validate.
pub const EXIT_INVALID_SCENE: i32 = 1;
/// The command line is wrong, or the input is missing or unreadable.
pub const EXIT_USAGE: i32 = 2;
/// Compilation failed: a constraint conflict, a cycle, or a defined size limit.
pub const EXIT_COMPILE: i32 = 3;
/// An export dependency is missing: the rasterizer or a required font.
pub const EXIT_DEPENDENCY: i32 = 4;
/// The output path could not be written.
pub const EXIT_OUTPUT: i32 = 5;

/// The usage block shared by the help text and every usage error.
const USAGE: &str = "\
Usage:
  vectr init [dir]
  vectr validate <scene> [--json]
  vectr compile <scene> [--out <file>] [--check]
  vectr export <scene> --format svg|png [--out <file>] [--width <n>] [--height <n>] [--density <n>] [--background <color|transparent>]";

/// One parsed command line.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Print the usage and exit zero.
    Help,
    /// Print the version and exit zero.
    Version,
    /// Scaffold a project in a directory.
    Init {
        /// The target directory; the current directory when absent.
        dir: PathBuf,
    },
    /// Check a scene against the language contract.
    Validate {
        /// The scene file to read.
        scene: PathBuf,
        /// Emit the findings as JSON rather than as text.
        json: bool,
    },
    /// Compile a scene into its render model.
    Compile {
        /// The scene file to read.
        scene: PathBuf,
        /// Where to write the model; the default `dist/` path when absent.
        out: Option<PathBuf>,
        /// Compile without writing anything.
        check: bool,
    },
    /// Export a scene to a raster or vector target.
    Export {
        /// The scene file to read.
        scene: PathBuf,
        /// The output format.
        format: Format,
        /// Where to write the output; the default `dist/` path when absent.
        out: Option<PathBuf>,
        /// Output width override.
        width: Option<f64>,
        /// Output height override.
        height: Option<f64>,
        /// Pixel density multiplier; PNG only.
        density: Option<f64>,
        /// Background override, or `transparent`.
        background: Option<String>,
    },
}

/// The export targets the contract names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A portable SVG document.
    Svg,
    /// A rasterized PNG image.
    Png,
}

impl Format {
    /// The file extension the format writes.
    fn extension(self) -> &'static str {
        match self {
            Format::Svg => "svg",
            Format::Png => "png",
        }
    }
}

/// The outcome of a command: what to print and the status to exit with.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// The process exit status.
    pub code: i32,
    /// Text to write to standard output.
    pub stdout: String,
    /// Text to write to standard error.
    pub stderr: String,
}

impl Report {
    pub(crate) fn success(stdout: impl Into<String>) -> Self {
        Self {
            code: EXIT_SUCCESS,
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub(crate) fn failure(code: i32, stderr: impl Into<String>) -> Self {
        Self {
            code,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }
}

/// The help text, printed on `--help` and on no arguments.
pub fn help_text() -> String {
    format!(
        "Vectr — describe a vector graphic as a scene and build it.\n\n\
         {USAGE}\n\n\
         Commands:\n\
         \x20 init      Create a project scaffold in a directory.\n\
         \x20 validate  Check a scene against the language contract.\n\
         \x20 compile   Compile a scene into its render model.\n\
         \x20 export    Export a scene as SVG or PNG.\n\n\
         Exit codes:\n\
         \x20 0 success   1 invalid scene   2 usage or unreadable input\n\
         \x20 3 compilation failure   4 export dependency missing   5 output I/O failure\n"
    )
}

/// A usage error: the short usage plus the reason.
pub fn usage_report(message: &str) -> Report {
    Report {
        code: EXIT_USAGE,
        stdout: String::new(),
        stderr: format!("error: {message}\n\n{USAGE}\n"),
    }
}

/// Parses a command line (without the program name).
pub fn parse(args: Vec<OsString>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Ok(Command::Help);
    };
    let rest: Vec<OsString> = args.collect();
    let Some(name) = first.to_str() else {
        return Err("the command name is not valid UTF-8".to_string());
    };

    match name {
        "-h" | "--help" | "help" => Ok(Command::Help),
        "-V" | "--version" => Ok(Command::Version),
        "init" => parse_init(rest),
        "validate" => parse_validate(rest),
        "compile" => parse_compile(rest),
        "export" => parse_export(rest),
        "render" | "inspect" => Err(format!("`{name}` is reserved for a later release")),
        "schema" => Err("`schema` is reserved for a later release".to_string()),
        _ if name.starts_with('-') => Err(format!("unknown option `{name}`")),
        _ => Err(format!("unknown command `{name}`")),
    }
}

/// Runs a parsed command.
pub fn run(command: Command) -> Report {
    match command {
        Command::Help => Report::success(help_text()),
        Command::Version => Report::success(format!("vectr {}\n", env!("CARGO_PKG_VERSION"))),
        Command::Init { dir } => init::scaffold(&dir),
        Command::Validate { scene, json } => validate_scene(&scene, json),
        Command::Compile { scene, out, check } => compile_scene(&scene, out.as_deref(), check),
        Command::Export {
            scene,
            format,
            out,
            width,
            height,
            density,
            background,
        } => export_scene(
            &scene,
            format,
            out.as_deref(),
            width,
            height,
            density,
            background.as_deref(),
        ),
    }
}

fn parse_init(args: Vec<OsString>) -> Result<Command, String> {
    let mut dir: Option<PathBuf> = None;
    for arg in args {
        let is_flag = arg
            .to_str()
            .is_some_and(|text| text.starts_with('-') && text != "-");
        if is_flag {
            let text = arg.to_str().expect("checked above");
            match text {
                "-h" | "--help" => return Ok(Command::Help),
                _ => return Err(format!("unknown option `{text}` for `init`")),
            }
        }
        set_scene(&mut dir, arg, "`init` accepts at most one directory")?;
    }
    Ok(Command::Init {
        dir: dir.unwrap_or_else(|| PathBuf::from(".")),
    })
}

fn parse_validate(args: Vec<OsString>) -> Result<Command, String> {
    let mut scene: Option<PathBuf> = None;
    let mut json = false;
    for arg in args {
        let is_flag = arg
            .to_str()
            .is_some_and(|text| text.starts_with('-') && text != "-");
        if is_flag {
            let text = arg.to_str().expect("checked above");
            match text {
                "-h" | "--help" => return Ok(Command::Help),
                "--json" => {
                    json = true;
                    continue;
                }
                _ => return Err(format!("unknown option `{text}` for `validate`")),
            }
        }
        set_scene(&mut scene, arg, "`validate` accepts exactly one scene path")?;
    }
    let scene = scene.ok_or_else(|| "`validate` needs a scene path".to_string())?;
    Ok(Command::Validate { scene, json })
}

fn parse_compile(args: Vec<OsString>) -> Result<Command, String> {
    let mut scene: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut check = false;
    let mut i = 0;
    while i < args.len() {
        let raw = args[i].clone();
        let text = raw.to_str().map(str::to_owned);
        match text.as_deref() {
            None => set_scene(&mut scene, raw, "`compile` accepts exactly one scene path")?,
            Some("-h") | Some("--help") => return Ok(Command::Help),
            Some("--check") => check = true,
            Some("--out") => out = Some(PathBuf::from(take_value(&args, &mut i, "--out")?)),
            Some(text) => {
                if let Some(value) = text.strip_prefix("--out=") {
                    out = Some(PathBuf::from(value));
                } else if text.starts_with('-') && text != "-" {
                    return Err(format!("unknown option `{text}` for `compile`"));
                } else {
                    set_scene(&mut scene, raw, "`compile` accepts exactly one scene path")?;
                }
            }
        }
        i += 1;
    }
    let scene = scene.ok_or_else(|| "`compile` needs a scene path".to_string())?;
    Ok(Command::Compile { scene, out, check })
}

fn parse_export(args: Vec<OsString>) -> Result<Command, String> {
    let mut scene: Option<PathBuf> = None;
    let mut format: Option<Format> = None;
    let mut out: Option<PathBuf> = None;
    let mut width: Option<f64> = None;
    let mut height: Option<f64> = None;
    let mut density: Option<f64> = None;
    let mut background: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        let raw = args[i].clone();
        let text = raw.to_str().map(str::to_owned);
        match text.as_deref() {
            None => set_scene(&mut scene, raw, "`export` accepts exactly one scene path")?,
            Some("-h") | Some("--help") => return Ok(Command::Help),
            Some(text) => {
                if let Some(value) = text.strip_prefix("--format=") {
                    format = Some(parse_format(value)?);
                } else if let Some(value) = text.strip_prefix("--out=") {
                    out = Some(PathBuf::from(value));
                } else if let Some(value) = text.strip_prefix("--width=") {
                    width = Some(parse_number(value, "width")?);
                } else if let Some(value) = text.strip_prefix("--height=") {
                    height = Some(parse_number(value, "height")?);
                } else if let Some(value) = text.strip_prefix("--density=") {
                    density = Some(parse_number(value, "density")?);
                } else if let Some(value) = text.strip_prefix("--background=") {
                    background = Some(value.to_string());
                } else {
                    match text {
                        "--format" => {
                            format = Some(parse_format(
                                &take_value(&args, &mut i, "--format")?.to_string_lossy(),
                            )?)
                        }
                        "--out" => out = Some(PathBuf::from(take_value(&args, &mut i, "--out")?)),
                        "--width" => {
                            width = Some(parse_number(
                                &take_value(&args, &mut i, "--width")?.to_string_lossy(),
                                "width",
                            )?)
                        }
                        "--height" => {
                            height = Some(parse_number(
                                &take_value(&args, &mut i, "--height")?.to_string_lossy(),
                                "height",
                            )?)
                        }
                        "--density" => {
                            density = Some(parse_number(
                                &take_value(&args, &mut i, "--density")?.to_string_lossy(),
                                "density",
                            )?)
                        }
                        "--background" => {
                            background = Some(
                                take_value(&args, &mut i, "--background")?
                                    .to_string_lossy()
                                    .into_owned(),
                            )
                        }
                        t if t.starts_with('-') && t != "-" => {
                            return Err(format!("unknown option `{t}` for `export`"))
                        }
                        _ => set_scene(&mut scene, raw, "`export` accepts exactly one scene path")?,
                    }
                }
            }
        }
        i += 1;
    }

    let scene = scene.ok_or_else(|| "`export` needs a scene path".to_string())?;
    let format = format.ok_or_else(|| "`export` needs `--format svg|png`".to_string())?;
    Ok(Command::Export {
        scene,
        format,
        out,
        width,
        height,
        density,
        background,
    })
}

/// Consumes the next argument as a flag's value.
fn take_value(args: &[OsString], index: &mut usize, flag: &str) -> Result<OsString, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("`{flag}` needs a value"))
}

/// Records the single positional argument a subcommand accepts.
fn set_scene(slot: &mut Option<PathBuf>, value: OsString, message: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(message.to_string());
    }
    *slot = Some(PathBuf::from(value));
    Ok(())
}

fn parse_format(value: &str) -> Result<Format, String> {
    match value {
        "svg" => Ok(Format::Svg),
        "png" => Ok(Format::Png),
        _ => Err(format!("`--format` must be `svg` or `png`, got `{value}`")),
    }
}

fn parse_number(value: &str, flag: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("`--{flag}` needs a number, got `{value}`"))
}

fn validate_scene(scene: &Path, json: bool) -> Report {
    let source = match read_scene(scene) {
        Ok(source) => source,
        Err(report) => return report,
    };

    let parsed = match parse_scene_source(&source) {
        Ok(parsed) => parsed,
        Err(diagnostics) => return report_findings(EXIT_INVALID_SCENE, diagnostics, json),
    };

    let assets = match ProjectAssets::load(scene, &parsed) {
        Ok(assets) => assets,
        Err(diagnostics) => {
            return report_findings(asset_exit_code(&diagnostics), diagnostics, json)
        }
    };

    // Validation is the gate a project's references pass: a palette token the
    // palette no longer defines, a stroke profile, or a font that does not
    // resolve is an error naming it, so a restyle that broke a scene is caught
    // before anything is compiled (FEAT-005, FEAT-024).
    let mut findings = validate_scene_model(&parsed);
    if let Some(palette) = assets.palette() {
        findings.extend(validate_palette_usage(&parsed, palette, assets.gradients()));
    }
    findings.extend(validate_gradient_usage(&parsed, assets.gradients()));
    findings.extend(assets.check_references(&parsed));

    let code = if findings.has_errors() {
        dependency_exit_code(&findings, EXIT_INVALID_SCENE)
    } else {
        EXIT_SUCCESS
    };
    report_findings(code, findings, json)
}

fn compile_scene(scene: &Path, out: Option<&Path>, check: bool) -> Report {
    let source = match read_scene(scene) {
        Ok(source) => source,
        Err(report) => return report,
    };
    let parsed = match parse_scene_source(&source) {
        Ok(parsed) => parsed,
        Err(diagnostics) => {
            return Report::failure(EXIT_INVALID_SCENE, diagnostics_text(&diagnostics))
        }
    };
    let assets = match ProjectAssets::load(scene, &parsed) {
        Ok(assets) => assets,
        Err(diagnostics) => {
            return Report::failure(
                asset_exit_code(&diagnostics),
                diagnostics_text(&diagnostics),
            )
        }
    };
    let references = assets.check_references(&parsed);
    if references.has_errors() {
        return Report::failure(
            dependency_exit_code(&references, EXIT_COMPILE),
            diagnostics_text(&references),
        );
    }

    let style = assets.style_context();
    let model = match compile_with_style(&parsed, &style) {
        Ok(model) => model,
        Err(diagnostics) => {
            return Report::failure(
                dependency_exit_code(&diagnostics, EXIT_COMPILE),
                diagnostics_text(&diagnostics),
            )
        }
    };
    let warnings = diagnostics_text(&model.diagnostics);

    if check {
        return Report {
            code: EXIT_SUCCESS,
            stdout: String::new(),
            stderr: warnings,
        };
    }

    let target = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_output(scene, "json"));
    let text = match model.to_json_pretty() {
        Ok(text) => text,
        Err(diagnostics) => return Report::failure(EXIT_COMPILE, diagnostics_text(&diagnostics)),
    };
    write_report(&target, text.as_bytes(), &warnings)
}

fn export_scene(
    scene: &Path,
    format: Format,
    out: Option<&Path>,
    width: Option<f64>,
    height: Option<f64>,
    density: Option<f64>,
    background: Option<&str>,
) -> Report {
    if format == Format::Svg && density.is_some() {
        return Report::failure(
            EXIT_USAGE,
            "error: `--density` applies only to PNG output\n".to_string(),
        );
    }

    let source = match read_scene(scene) {
        Ok(source) => source,
        Err(report) => return report,
    };
    let parsed = match parse_scene_source(&source) {
        Ok(parsed) => parsed,
        Err(diagnostics) => {
            return Report::failure(EXIT_INVALID_SCENE, diagnostics_text(&diagnostics))
        }
    };
    let assets = match ProjectAssets::load(scene, &parsed) {
        Ok(assets) => assets,
        Err(diagnostics) => {
            return Report::failure(
                asset_exit_code(&diagnostics),
                diagnostics_text(&diagnostics),
            )
        }
    };
    let references = assets.check_references(&parsed);
    if references.has_errors() {
        return Report::failure(
            dependency_exit_code(&references, EXIT_COMPILE),
            diagnostics_text(&references),
        );
    }

    let style = assets.style_context();
    let model = match compile_with_style(&parsed, &style) {
        Ok(model) => model,
        Err(diagnostics) => {
            return Report::failure(
                dependency_exit_code(&diagnostics, EXIT_COMPILE),
                diagnostics_text(&diagnostics),
            )
        }
    };
    let mut warnings = diagnostics_text(&model.diagnostics);

    let bytes = match format {
        Format::Svg => {
            let options = SvgOptions {
                width,
                height,
                background: background.map(str::to_string),
            };
            match export_svg_reporting(&model, &options) {
                Ok(export) => {
                    warnings.push_str(&diagnostics_text(&export.diagnostics));
                    export.svg.into_bytes()
                }
                Err(diagnostics) => {
                    return Report::failure(
                        export_exit_code(&diagnostics),
                        diagnostics_text(&diagnostics),
                    )
                }
            }
        }
        Format::Png => {
            let options = RasterOptions {
                width,
                height,
                density,
                background: background.map(str::to_string),
            };
            match export_png_reporting(&model, &options) {
                Ok(export) => {
                    warnings.push_str(&diagnostics_text(&export.diagnostics));
                    export.png
                }
                Err(diagnostics) => {
                    return Report::failure(
                        export_exit_code(&diagnostics),
                        diagnostics_text(&diagnostics),
                    )
                }
            }
        }
    };

    let target = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_output(scene, format.extension()));
    write_report(&target, &bytes, &warnings)
}

/// Writes a completed result, mapping an I/O failure to exit 5.
fn write_report(target: &Path, bytes: &[u8], warnings: &str) -> Report {
    match write_atomic(target, bytes) {
        Ok(()) => Report {
            code: EXIT_SUCCESS,
            stdout: format!("wrote {}\n", target.display()),
            stderr: warnings.to_string(),
        },
        Err(error) => Report::failure(
            EXIT_OUTPUT,
            format!(
                "{warnings}error: cannot write output `{}`: {error}\n",
                target.display()
            ),
        ),
    }
}

/// Reads a scene file, mapping a missing or unreadable file to exit 2.
fn read_scene(scene: &Path) -> Result<String, Report> {
    fs::read_to_string(scene).map_err(|error| {
        Report::failure(
            EXIT_USAGE,
            format!("error: cannot read scene `{}`: {error}\n", scene.display()),
        )
    })
}

/// The default output path for a scene: `dist/<stem>.<extension>`.
fn default_output(scene: &Path, extension: &str) -> PathBuf {
    let stem = scene
        .file_stem()
        .map(|stem| stem.to_os_string())
        .unwrap_or_else(|| OsString::from("scene"));
    let mut name = stem;
    name.push(".");
    name.push(extension);
    Path::new("dist").join(name)
}

/// Classifies an export failure: a missing dependency outranks a usage error,
/// which outranks a defined size limit.
fn export_exit_code(diagnostics: &Diagnostics) -> i32 {
    let mut code = EXIT_COMPILE;
    for error in diagnostics.errors() {
        if error.code == png_export::RASTERIZER {
            return EXIT_DEPENDENCY;
        }
        if error.code == svg_export::OPTIONS || error.code == png_export::OPTIONS {
            code = EXIT_USAGE;
        }
    }
    code
}

/// Classifies a project-asset loading failure: a missing font is a dependency,
/// any other unreadable project asset is missing input.
fn asset_exit_code(diagnostics: &Diagnostics) -> i32 {
    dependency_exit_code(diagnostics, EXIT_USAGE)
}

/// Elevates a missing-font error to the dependency exit, keeping the caller's
/// class for every other finding (C-004).
fn dependency_exit_code(diagnostics: &Diagnostics, fallback: i32) -> i32 {
    if diagnostics.errors().any(|error| error.code == FONT) {
        EXIT_DEPENDENCY
    } else {
        fallback
    }
}

/// Renders findings for `validate`, to stdout as JSON or to stderr as text.
fn report_findings(code: i32, findings: Diagnostics, json: bool) -> Report {
    if json {
        Report {
            code,
            stdout: json_diagnostics(&findings),
            stderr: String::new(),
        }
    } else if code == EXIT_SUCCESS {
        Report {
            code,
            stdout: String::new(),
            stderr: diagnostics_text(&findings),
        }
    } else {
        Report::failure(code, diagnostics_text(&findings))
    }
}

/// Renders findings as newline-terminated text, empty when there are none.
pub(crate) fn diagnostics_text(diagnostics: &Diagnostics) -> String {
    if diagnostics.is_empty() {
        return String::new();
    }
    let mut text = String::new();
    for diagnostic in diagnostics.iter() {
        text.push_str(&render_diagnostic(diagnostic));
        text.push('\n');
    }
    text
}

/// Renders one finding as `severity[code]: message at location`.
///
/// The CLI formats its own output rather than deferring to the engine's
/// `Display`, which repeats the word "at" before a bare JSON path.
fn render_diagnostic(diagnostic: &Diagnostic) -> String {
    let mut text = format!(
        "{}[{}]: {}",
        diagnostic.severity, diagnostic.code, diagnostic.message
    );
    if let Some(location) = &diagnostic.location {
        let mut parts: Vec<String> = Vec::new();
        if let Some(id) = &location.element_id {
            parts.push(format!("element `{id}`"));
        }
        if let Some(path) = &location.json_path {
            if location.element_id.is_some() {
                parts.push(format!("at {path}"));
            } else {
                parts.push(path.clone());
            }
        }
        if let (Some(line), Some(column)) = (location.line, location.column) {
            parts.push(format!("line {line} column {column}"));
        } else if let Some(line) = location.line {
            parts.push(format!("line {line}"));
        }
        if !parts.is_empty() {
            text.push_str(" at ");
            text.push_str(&parts.join(" "));
        }
    }
    text
}

/// Renders findings as a JSON array for machine consumption.
fn json_diagnostics(diagnostics: &Diagnostics) -> String {
    let text = serde_json::to_string(diagnostics).unwrap_or_else(|_| "[]".to_string());
    format!("{text}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    const VALID_SCENE: &str = r##"{
      "id": "scene-1",
      "projectId": "project",
      "name": "Example",
      "formatVersion": "0.2",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "e1", "sceneId": "scene-1", "order": 0, "kind": "rect",
          "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    /// A scene whose elements reference each other, so it parses but the
    /// compiler reports a cycle.
    const CYCLIC_SCENE: &str = r##"{
      "id": "scene-1",
      "projectId": "project",
      "name": "Cycle",
      "formatVersion": "0.2",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "a", "sceneId": "scene-1", "parentId": "b", "order": 0, "kind": "rect",
          "geometry": { "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        },
        {
          "id": "b", "sceneId": "scene-1", "parentId": "a", "order": 1, "kind": "rect",
          "geometry": { "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    const INVALID_SCENE: &str = r##"{
      "id": "scene-1",
      "projectId": "project",
      "name": "Example",
      "formatVersion": "0.2",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "e1", "sceneId": "scene-1", "order": 0, "kind": "rect",
          "geometry": { "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 2, "visible": true
        }
      ]
    }"##;

    fn parse_args(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(OsString::from).collect())
    }

    #[test]
    fn a_diagnostic_renders_its_location_once() {
        use vectr_core::{DiagnosticCode, Location};

        let path = Diagnostic::error(DiagnosticCode::SCHEMA, "bad").at_path("/elements/0/opacity");
        assert_eq!(
            render_diagnostic(&path),
            "error[E_SCHEMA]: bad at /elements/0/opacity"
        );

        let element = Diagnostic::error(DiagnosticCode::new("E_CYCLE"), "loop").for_element("a");
        assert_eq!(
            render_diagnostic(&element),
            "error[E_CYCLE]: loop at element `a`"
        );

        let line = Diagnostic::error(DiagnosticCode::PARSE, "syntax")
            .with_location(Location::line_column(2, 5));
        assert_eq!(
            render_diagnostic(&line),
            "error[E_PARSE]: syntax at line 2 column 5"
        );
    }

    fn write_scene(dir: &TempDir, name: &str, text: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, text).expect("writes the scene");
        path
    }

    #[test]
    fn no_arguments_prints_help_with_a_zero_exit() {
        assert_eq!(parse_args(&[]).unwrap(), Command::Help);
        assert_eq!(run(Command::Help).code, EXIT_SUCCESS);
    }

    #[test]
    fn an_unknown_command_is_a_usage_error() {
        assert!(parse_args(&["frobnicate"]).is_err());
    }

    #[test]
    fn a_reserved_command_is_reported() {
        for command in ["render", "inspect", "schema"] {
            let error = parse_args(&[command]).expect_err("reserved");
            assert!(error.contains("reserved"), "{error}");
        }
    }

    #[test]
    fn init_defaults_to_the_current_directory() {
        assert_eq!(
            parse_args(&["init"]).unwrap(),
            Command::Init {
                dir: PathBuf::from(".")
            }
        );
        assert_eq!(
            parse_args(&["init", "habit"]).unwrap(),
            Command::Init {
                dir: PathBuf::from("habit")
            }
        );
    }

    #[test]
    fn validate_parses_the_scene_and_the_json_flag() {
        assert_eq!(
            parse_args(&["validate", "scene.json"]).unwrap(),
            Command::Validate {
                scene: PathBuf::from("scene.json"),
                json: false
            }
        );
        assert_eq!(
            parse_args(&["validate", "--json", "scene.json"]).unwrap(),
            Command::Validate {
                scene: PathBuf::from("scene.json"),
                json: true
            }
        );
    }

    #[test]
    fn compile_parses_out_and_check_in_both_flag_forms() {
        assert_eq!(
            parse_args(&["compile", "scene.json", "--check"]).unwrap(),
            Command::Compile {
                scene: PathBuf::from("scene.json"),
                out: None,
                check: true
            }
        );
        assert_eq!(
            parse_args(&["compile", "scene.json", "--out", "dist/a.json"]).unwrap(),
            parse_args(&["compile", "scene.json", "--out=dist/a.json"]).unwrap()
        );
    }

    #[test]
    fn export_parses_every_option() {
        let command = parse_args(&[
            "export",
            "scene.json",
            "--format",
            "png",
            "--out",
            "dist/a.png",
            "--width",
            "64",
            "--height",
            "64",
            "--density",
            "2",
            "--background",
            "transparent",
        ])
        .unwrap();
        assert_eq!(
            command,
            Command::Export {
                scene: PathBuf::from("scene.json"),
                format: Format::Png,
                out: Some(PathBuf::from("dist/a.png")),
                width: Some(64.0),
                height: Some(64.0),
                density: Some(2.0),
                background: Some("transparent".to_string()),
            }
        );
    }

    #[test]
    fn export_requires_a_known_format() {
        assert!(parse_args(&["export", "scene.json"]).is_err());
        assert!(parse_args(&["export", "scene.json", "--format", "pdf"]).is_err());
    }

    #[test]
    fn unknown_options_and_extra_positionals_are_refused() {
        assert!(parse_args(&["compile", "scene.json", "--nope"]).is_err());
        assert!(parse_args(&["validate", "a.json", "b.json"]).is_err());
        assert!(parse_args(&["compile", "scene.json", "--width", "1"]).is_err());
    }

    #[test]
    fn the_default_output_is_under_dist() {
        assert_eq!(
            default_output(Path::new("scenes/logo.json"), "svg"),
            PathBuf::from("dist").join("logo.svg")
        );
        assert_eq!(
            default_output(Path::new("logo.json"), "json"),
            PathBuf::from("dist").join("logo.json")
        );
    }

    #[test]
    fn export_failures_are_classified() {
        let missing = Diagnostics::from(vectr_core::scene::Diagnostic::error(
            png_export::RASTERIZER,
            "no rasterizer",
        ));
        assert_eq!(export_exit_code(&missing), EXIT_DEPENDENCY);

        let options = Diagnostics::from(vectr_core::scene::Diagnostic::error(
            png_export::OPTIONS,
            "bad size",
        ));
        assert_eq!(export_exit_code(&options), EXIT_USAGE);

        let limit = Diagnostics::from(vectr_core::scene::Diagnostic::error(
            png_export::SIZE_LIMIT,
            "too big",
        ));
        assert_eq!(export_exit_code(&limit), EXIT_COMPILE);
    }

    #[test]
    fn validate_reports_a_valid_scene_with_a_zero_exit() {
        let dir = TempDir::new("validate-valid");
        let scene = write_scene(&dir, "scene.json", VALID_SCENE);
        let report = run(Command::Validate { scene, json: false });
        assert_eq!(report.code, EXIT_SUCCESS);
        assert!(report.stderr.is_empty(), "{}", report.stderr);
    }

    #[test]
    fn validate_reports_an_invalid_scene_with_exit_one() {
        let dir = TempDir::new("validate-invalid");
        let scene = write_scene(&dir, "scene.json", INVALID_SCENE);
        let report = run(Command::Validate { scene, json: false });
        assert_eq!(report.code, EXIT_INVALID_SCENE);
        assert!(report.stderr.contains("E_SCHEMA"), "{}", report.stderr);
    }

    #[test]
    fn validate_json_emits_a_machine_readable_array() {
        let dir = TempDir::new("validate-json");
        let valid = write_scene(&dir, "valid.json", VALID_SCENE);
        let report = run(Command::Validate {
            scene: valid,
            json: true,
        });
        assert_eq!(report.code, EXIT_SUCCESS);
        assert_eq!(report.stdout.trim(), "[]");

        let invalid = write_scene(&dir, "invalid.json", INVALID_SCENE);
        let report = run(Command::Validate {
            scene: invalid,
            json: true,
        });
        assert_eq!(report.code, EXIT_INVALID_SCENE);
        let parsed: serde_json::Value =
            serde_json::from_str(report.stdout.trim()).expect("valid JSON");
        assert!(parsed.as_array().is_some_and(|items| !items.is_empty()));
    }

    #[test]
    fn a_missing_scene_is_a_usage_error() {
        let dir = TempDir::new("missing");
        let report = run(Command::Compile {
            scene: dir.path().join("absent.json"),
            out: None,
            check: false,
        });
        assert_eq!(report.code, EXIT_USAGE);
    }

    #[test]
    fn a_valid_scene_compiles_to_a_render_model() {
        let dir = TempDir::new("compile-valid");
        let scene = write_scene(&dir, "scene.json", VALID_SCENE);
        let out = dir.path().join("model.json");
        let report = run(Command::Compile {
            scene,
            out: Some(out.clone()),
            check: false,
        });
        assert_eq!(report.code, EXIT_SUCCESS);
        let text = fs::read_to_string(&out).expect("reads the model");
        vectr_core::render::parse(&text).expect("the output is a render model");
    }

    #[test]
    fn a_compilation_failure_exits_three() {
        let dir = TempDir::new("compile-cycle");
        let scene = write_scene(&dir, "scene.json", CYCLIC_SCENE);
        let report = run(Command::Compile {
            scene,
            out: None,
            check: true,
        });
        assert_eq!(report.code, EXIT_COMPILE);
        assert!(report.stderr.contains("E_CYCLE"), "{}", report.stderr);
    }

    #[test]
    fn check_writes_nothing() {
        let dir = TempDir::new("compile-check");
        let scene = write_scene(&dir, "scene.json", VALID_SCENE);
        let out = dir.path().join("model.json");
        let report = run(Command::Compile {
            scene,
            out: Some(out.clone()),
            check: true,
        });
        assert_eq!(report.code, EXIT_SUCCESS);
        assert!(!out.exists(), "no output under --check");
    }

    #[test]
    fn a_failed_compile_leaves_an_existing_output_untouched() {
        let dir = TempDir::new("compile-untouched");
        let scene = write_scene(&dir, "scene.json", INVALID_SCENE);
        let out = dir.path().join("model.json");
        fs::write(&out, "previous").expect("seeds the output");
        let report = run(Command::Compile {
            scene,
            out: Some(out.clone()),
            check: false,
        });
        assert_eq!(report.code, EXIT_INVALID_SCENE);
        assert_eq!(
            fs::read_to_string(&out).expect("reads"),
            "previous",
            "existing output is untouched"
        );
    }

    #[test]
    fn an_unwritable_output_exits_five() {
        let dir = TempDir::new("compile-unwritable");
        let scene = write_scene(&dir, "scene.json", VALID_SCENE);
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "not a directory").expect("seeds the blocker");
        let report = run(Command::Compile {
            scene,
            out: Some(blocker.join("model.json")),
            check: false,
        });
        assert_eq!(report.code, EXIT_OUTPUT);
    }

    #[test]
    fn a_valid_scene_exports_svg() {
        let dir = TempDir::new("export-svg");
        let scene = write_scene(&dir, "scene.json", VALID_SCENE);
        let out = dir.path().join("out.svg");
        let report = run(Command::Export {
            scene,
            format: Format::Svg,
            out: Some(out.clone()),
            width: None,
            height: None,
            density: None,
            background: None,
        });
        assert_eq!(report.code, EXIT_SUCCESS);
        let svg = fs::read_to_string(&out).expect("reads the svg");
        assert!(svg.contains("<svg"), "{svg}");
        assert!(svg.contains("</svg>"), "{svg}");
    }

    #[test]
    fn a_valid_scene_exports_png() {
        let dir = TempDir::new("export-png");
        let scene = write_scene(&dir, "scene.json", VALID_SCENE);
        let out = dir.path().join("out.png");
        let report = run(Command::Export {
            scene,
            format: Format::Png,
            out: Some(out.clone()),
            width: None,
            height: None,
            density: None,
            background: None,
        });
        assert_eq!(report.code, EXIT_SUCCESS);
        let bytes = fs::read(&out).expect("reads the png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    }

    #[test]
    fn density_on_svg_is_a_usage_error() {
        let dir = TempDir::new("export-density");
        let scene = write_scene(&dir, "scene.json", VALID_SCENE);
        let report = run(Command::Export {
            scene,
            format: Format::Svg,
            out: None,
            width: None,
            height: None,
            density: Some(2.0),
            background: None,
        });
        assert_eq!(report.code, EXIT_USAGE);
    }

    const PROJECT_TEXT_SCENE: &str = r##"{
      "id": "s",
      "projectId": "project",
      "name": "Text",
      "formatVersion": "0.2",
      "canvas": { "width": 200, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "t1", "sceneId": "s", "order": 0, "kind": "text",
          "geometry": { "text": "Hi", "fontSize": 32, "x": 10, "y": 60 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    const PALETTE: &str = r##"{"id":"brand","projectId":"project","name":"Brand","tokens":[{"name":"accent","value":"#ff0000"}]}"##;

    const PALETTE_SCENE: &str = r##"{
      "id": "s",
      "projectId": "project",
      "name": "Brand",
      "formatVersion": "0.2",
      "paletteId": "brand",
      "canvas": { "width": 100, "height": 100, "background": "#ffffff" },
      "elements": [
        {
          "id": "r1", "sceneId": "s", "order": 0, "kind": "rect",
          "geometry": { "x": 0, "y": 0, "width": 10, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "fill": { "kind": "token", "ref": "accent" }, "opacity": 1, "visible": true
        }
      ]
    }"##;

    /// Writes a file, creating the directories its path names.
    fn write_at(dir: &TempDir, name: &str, text: &str) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("creates the parent");
        }
        fs::write(&path, text).expect("writes the file");
        path
    }

    #[test]
    fn a_project_scene_exports_text_as_outlines() {
        let dir = TempDir::new("project-text");
        write_at(&dir, "vectr.project.json", "{}");
        let scene = write_at(&dir, "scenes/logo.json", PROJECT_TEXT_SCENE);
        let out = dir.path().join("logo.svg");

        let report = run(Command::Export {
            scene,
            format: Format::Svg,
            out: Some(out.clone()),
            width: None,
            height: None,
            density: None,
            background: None,
        });
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        let svg = fs::read_to_string(&out).expect("reads the svg");
        assert!(svg.contains("<desc>Hi</desc>"), "{svg}");
        assert!(svg.contains("<path"), "{svg}");
        assert!(
            !svg.contains("<text"),
            "glyphs are outlined, not left font-dependent (FEAT-024): {svg}"
        );
    }

    #[test]
    fn a_missing_font_exits_four() {
        let dir = TempDir::new("project-missing-font");
        write_at(&dir, "vectr.project.json", "{}");
        let scene_text = PROJECT_TEXT_SCENE.replace(
            r##""kind": "text","##,
            r##""kind": "text", "fontId": "absent","##,
        );
        let scene = write_at(&dir, "scenes/logo.json", &scene_text);

        let report = run(Command::Compile {
            scene,
            out: None,
            check: true,
        });
        assert_eq!(report.code, EXIT_DEPENDENCY);
        assert!(report.stderr.contains("absent"), "{}", report.stderr);
    }

    #[test]
    fn a_project_palette_resolves_a_fill_token() {
        let dir = TempDir::new("project-palette");
        write_at(&dir, "vectr.project.json", "{}");
        write_at(&dir, "palettes/brand.json", PALETTE);
        let scene = write_at(&dir, "scenes/brand.json", PALETTE_SCENE);
        let out = dir.path().join("brand.json");

        let report = run(Command::Compile {
            scene,
            out: Some(out.clone()),
            check: false,
        });
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        let text = fs::read_to_string(&out).expect("reads the model");
        let model = vectr_core::render::parse(&text).expect("a render model");
        assert_eq!(
            model.nodes[0].paint.fill,
            Some(vectr_core::render::Paint::Color {
                value: "#ff0000".to_string()
            })
        );
    }

    #[test]
    fn validate_reports_an_undefined_palette_token() {
        let dir = TempDir::new("project-undefined-token");
        write_at(&dir, "vectr.project.json", "{}");
        write_at(
            &dir,
            "palettes/brand.json",
            r##"{"id":"brand","projectId":"project","name":"Brand","tokens":[{"name":"other","value":"#ff0000"}]}"##,
        );
        let scene = write_at(&dir, "scenes/brand.json", PALETTE_SCENE);

        let report = run(Command::Validate { scene, json: false });
        assert_eq!(report.code, EXIT_INVALID_SCENE);
        assert!(report.stderr.contains("accent"), "{}", report.stderr);
    }

    #[test]
    fn a_missing_palette_is_missing_input() {
        let dir = TempDir::new("project-missing-palette");
        write_at(&dir, "vectr.project.json", "{}");
        let scene = write_at(&dir, "scenes/brand.json", PALETTE_SCENE);

        let report = run(Command::Validate { scene, json: false });
        assert_eq!(report.code, EXIT_USAGE);
        assert!(report.stderr.contains("brand"), "{}", report.stderr);
    }
}
