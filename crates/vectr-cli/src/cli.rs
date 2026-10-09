//! `vectr` command parsing and dispatch (C-004).
//!
//! The grammar and exit codes are frozen by the contract: `init`, `validate`,
//! `compile`, `export`, `render`, `inspect` and `schema`, with 0 success, 1
//! invalid scene, 2 usage or unreadable input, 3 compilation failure, 4 missing
//! export dependency, and 5 output I/O failure. Parsing is hand-rolled rather
//! than pulled from a CLI crate so the binary depends only on the engine and can
//! control the exit code of every path, including "no arguments prints usage and
//! exits zero".
//!
//! Every command returns a [`Report`] holding the text to print and the status
//! to exit with; only [`main`](crate::main) touches the process. Diagnostics are
//! the engine's structured findings, so a failure names its code, message and
//! location (NFR-011).
//!
//! `validate`, `compile`, `export` and `inspect` load the assets the scene's
//! project provides — its palette, stroke profiles, and fonts — and compile
//! against them, so a scene's style and font references resolve to concrete
//! values before anything is written (FEAT-005, FEAT-024).
//!
//! A scene is named by its identifier, resolved among the project's scene
//! documents under `scenes/`, with the project discovered from the working
//! directory. Omitting the scene uses the project's `defaultSceneId`; a project
//! that names no default reports that no scene was selected (FEAT-016, D-032).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use vectr_core::compiler::FONT;
use vectr_core::export::pdf as pdf_export;
use vectr_core::export::png as png_export;
use vectr_core::export::svg as svg_export;
use vectr_core::scene::INVALID_COLOR;
use vectr_core::{
    compile_definition, compile_subtree, compile_with_style, export_pdf_reporting,
    export_png_reporting, export_svg_reporting, parse as parse_scene_source, schema, schema_for,
    validate as validate_scene_model, validate_gradient_usage, validate_palette_usage, Diagnostic,
    DiagnosticCode, Diagnostics, PdfOptions, RasterOptions, RenderModel, Scene, SchemaForm,
    SvgOptions,
};

use crate::init;
use crate::output::write_atomic;
use vectr_project::{
    project_root_from, resolve_part, resolve_scene, ProjectAssets, ProjectScene, ResolvedPart,
};

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

/// The environment variable that enables PDF export, a rollout flag that is off
/// by default (FEAT-014).
pub const ENABLE_PDF_ENV: &str = "VECTR_ENABLE_PDF_EXPORT";

/// The usage block shared by the help text and every usage error.
const USAGE: &str = "\
Usage:
  vectr init [dir]
  vectr validate [<scene>] [--json]
  vectr compile [<scene>] [--out <file>] [--check]
  vectr export [<scene>] --format svg|png|pdf [--out <file>] [--width <n>] [--height <n>] [--density <n>] [--background <color|transparent>] [--profile <srgb|cmyk>]
  vectr inspect [<scene>] [--out <file>] [--width <n>] [--height <n>] [--density <n>] [--background <color|transparent>]
  vectr render <part> [--out <file>] [--format svg|png] [--width <n>] [--height <n>] [--density <n>] [--background <color|transparent>]
  vectr schema [--type <name>] [--compact]

<scene> is a scene identifier resolved among the project's scenes; the project
is found from the working directory. Omitting it uses the project's default
scene. <part> is a reusable definition's identifier, or an element subtree's
identifier, resolved within the project; it is rendered on its own, framed to
its own bounds.";

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
        /// The scene identifier to read; the project's default when absent.
        scene: Option<String>,
        /// Emit the findings as JSON rather than as text.
        json: bool,
    },
    /// Compile a scene into its render model.
    Compile {
        /// The scene identifier to read; the project's default when absent.
        scene: Option<String>,
        /// Where to write the model; the default `dist/` path when absent.
        out: Option<PathBuf>,
        /// Compile without writing anything.
        check: bool,
    },
    /// Export a scene to a raster or vector target.
    Export {
        /// The scene identifier to read; the project's default when absent.
        scene: Option<String>,
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
        /// The print colour profile a PDF export targets (FEAT-014).
        profile: Option<Profile>,
    },
    /// Render a whole-scene preview for inspection (FEAT-022).
    Inspect {
        /// The scene identifier to read; the project's default when absent.
        scene: Option<String>,
        /// Where to write the preview; the default `dist/` path when absent.
        out: Option<PathBuf>,
        /// Preview width override.
        width: Option<f64>,
        /// Preview height override.
        height: Option<f64>,
        /// Pixel density multiplier.
        density: Option<f64>,
        /// Background override, or `transparent`.
        background: Option<String>,
    },
    /// Render one part on its own: a reusable definition or an element subtree.
    Render {
        /// The part's identifier, resolved within the project.
        part: String,
        /// Where to write the preview; the default `dist/` path when absent.
        out: Option<PathBuf>,
        /// The output format.
        format: Format,
        /// Output width override.
        width: Option<f64>,
        /// Output height override.
        height: Option<f64>,
        /// Pixel density multiplier; PNG only.
        density: Option<f64>,
        /// Background override, or `transparent`.
        background: Option<String>,
    },
    /// Print the language contract, or one of its types.
    Schema {
        /// The type to print; the whole contract when absent.
        type_name: Option<String>,
        /// Emit minified JSON for machine consumption.
        compact: bool,
    },
}

/// The export targets the contract names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A portable SVG document.
    Svg,
    /// A rasterized PNG image.
    Png,
    /// A vector PDF document (FEAT-014); gated by `enable_pdf_export`.
    Pdf,
}

impl Format {
    /// The file extension the format writes.
    fn extension(self) -> &'static str {
        match self {
            Format::Svg => "svg",
            Format::Png => "png",
            Format::Pdf => "pdf",
        }
    }
}

/// The print colour profile a PDF export targets (FEAT-014).
///
/// The profile is a PDF-only concern: `srgb` carries per-paint transparency,
/// while `cmyk` is a print space that cannot, so the exporter flattens it. The
/// CLI names the two profiles the contract freezes and leaves the emission to
/// the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Device RGB with transparency.
    Srgb,
    /// Device CMYK with transparency flattened.
    Cmyk,
}

impl Profile {
    /// The profile name the exporter understands.
    fn name(self) -> &'static str {
        match self {
            Profile::Srgb => "srgb",
            Profile::Cmyk => "cmyk",
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
         \x20 export    Export a scene as SVG, PNG or PDF.\n\
         \x20 inspect   Render a whole-scene preview for inspection.\n\
         \x20 render    Render one part on its own, framed to its bounds.\n\
         \x20 schema    Print the language contract.\n\n\
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
        "inspect" => parse_inspect(rest),
        "render" => parse_render(rest),
        "schema" => parse_schema(rest),
        _ if name.starts_with('-') => Err(format!("unknown option `{name}`")),
        _ => Err(format!("unknown command `{name}`")),
    }
}

/// Runs a parsed command in the process's working directory.
pub fn run(command: Command) -> Report {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    run_in(command, &cwd)
}

/// Runs a parsed command against `cwd`.
///
/// The project is discovered from `cwd`, scene identifiers resolve within it,
/// and relative output paths are written under it. Splitting this from [`run`]
/// keeps the working directory injectable, so the command logic is testable
/// without changing the process's directory.
pub fn run_in(command: Command, cwd: &Path) -> Report {
    match command {
        Command::Help => Report::success(help_text()),
        Command::Version => Report::success(format!("vectr {}\n", env!("CARGO_PKG_VERSION"))),
        Command::Init { dir } => {
            // The default target is the working directory itself, not `cwd/.`.
            let dir = if dir.as_os_str() == "." {
                cwd.to_path_buf()
            } else {
                absolute(cwd, &dir)
            };
            init::scaffold(&dir)
        }
        Command::Validate { scene, json } => match resolve(cwd, scene.as_deref()) {
            Ok(scene) => validate_scene(&scene, json),
            Err(report) => report,
        },
        Command::Compile { scene, out, check } => match resolve(cwd, scene.as_deref()) {
            Ok(scene) => compile_scene(cwd, &scene, out.as_deref(), check),
            Err(report) => report,
        },
        Command::Export {
            scene,
            format,
            out,
            width,
            height,
            density,
            background,
            profile,
        } => {
            // PDF is a gated capability, off by default (FEAT-014); the gate is
            // checked before the scene is read, so a disabled export writes
            // nothing (NFR-011).
            if format == Format::Pdf {
                if let Some(report) = pdf_disabled() {
                    return report;
                }
            }
            match resolve(cwd, scene.as_deref()) {
                Ok(scene) => {
                    let target = out
                        .as_deref()
                        .map(|path| absolute(cwd, path))
                        .unwrap_or_else(|| {
                            cwd.join(default_output(scene.id(), format.extension()))
                        });
                    export_scene(
                        &scene,
                        format,
                        &target,
                        width,
                        height,
                        density,
                        background.as_deref(),
                        profile,
                    )
                }
                Err(report) => report,
            }
        }
        Command::Inspect {
            scene,
            out,
            width,
            height,
            density,
            background,
        } => match resolve(cwd, scene.as_deref()) {
            Ok(scene) => {
                let target = out
                    .as_deref()
                    .map(|path| absolute(cwd, path))
                    .unwrap_or_else(|| cwd.join(default_output(scene.id(), "png")));
                inspect_scene(
                    &scene,
                    &target,
                    width,
                    height,
                    density,
                    background.as_deref(),
                )
            }
            Err(report) => report,
        },
        Command::Render {
            part,
            out,
            format,
            width,
            height,
            density,
            background,
        } => {
            let target = out
                .as_deref()
                .map(|path| absolute(cwd, path))
                .unwrap_or_else(|| cwd.join(default_output(&part, format.extension())));
            render_part(
                cwd,
                &part,
                format,
                &target,
                width,
                height,
                density,
                background.as_deref(),
            )
        }
        Command::Schema { type_name, compact } => schema_command(type_name.as_deref(), compact),
    }
}

/// Resolves the scene a command named, or the project's default (FEAT-016).
///
/// A scene that cannot be resolved is missing input: exit 2 with a diagnostic
/// naming the scene, never a silent choice among the project's scenes.
fn resolve(cwd: &Path, requested: Option<&str>) -> Result<ProjectScene, Report> {
    let root = project_root_from(cwd);
    resolve_scene(&root, requested)
        .map_err(|diagnostics| Report::failure(EXIT_USAGE, diagnostics_text(&diagnostics)))
}

/// Resolves a path against the working directory when it is relative.
fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
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
        set_dir(&mut dir, arg, "`init` accepts at most one directory")?;
    }
    Ok(Command::Init {
        dir: dir.unwrap_or_else(|| PathBuf::from(".")),
    })
}

fn parse_validate(args: Vec<OsString>) -> Result<Command, String> {
    let mut scene: Option<String> = None;
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
        set_scene_id(&mut scene, arg)?;
    }
    Ok(Command::Validate { scene, json })
}

fn parse_compile(args: Vec<OsString>) -> Result<Command, String> {
    let mut scene: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut check = false;
    let mut i = 0;
    while i < args.len() {
        let raw = args[i].clone();
        let text = raw.to_str().map(str::to_owned);
        match text.as_deref() {
            None => set_scene_id(&mut scene, raw)?,
            Some("-h") | Some("--help") => return Ok(Command::Help),
            Some("--check") => check = true,
            Some("--out") => out = Some(PathBuf::from(take_value(&args, &mut i, "--out")?)),
            Some(text) => {
                if let Some(value) = text.strip_prefix("--out=") {
                    out = Some(PathBuf::from(value));
                } else if text.starts_with('-') && text != "-" {
                    return Err(format!("unknown option `{text}` for `compile`"));
                } else {
                    set_scene_id(&mut scene, raw)?;
                }
            }
        }
        i += 1;
    }
    Ok(Command::Compile { scene, out, check })
}

fn parse_export(args: Vec<OsString>) -> Result<Command, String> {
    let mut scene: Option<String> = None;
    let mut format: Option<Format> = None;
    let mut out: Option<PathBuf> = None;
    let mut width: Option<f64> = None;
    let mut height: Option<f64> = None;
    let mut density: Option<f64> = None;
    let mut background: Option<String> = None;
    let mut profile: Option<Profile> = None;

    let mut i = 0;
    while i < args.len() {
        let raw = args[i].clone();
        let text = raw.to_str().map(str::to_owned);
        match text.as_deref() {
            None => set_scene_id(&mut scene, raw)?,
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
                } else if let Some(value) = text.strip_prefix("--profile=") {
                    profile = Some(parse_profile(value)?);
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
                        "--profile" => {
                            profile = Some(parse_profile(
                                &take_value(&args, &mut i, "--profile")?.to_string_lossy(),
                            )?)
                        }
                        t if t.starts_with('-') && t != "-" => {
                            return Err(format!("unknown option `{t}` for `export`"))
                        }
                        _ => set_scene_id(&mut scene, raw)?,
                    }
                }
            }
        }
        i += 1;
    }

    let format = format.ok_or_else(|| "`export` needs `--format svg|png|pdf`".to_string())?;
    Ok(Command::Export {
        scene,
        format,
        out,
        width,
        height,
        density,
        background,
        profile,
    })
}

/// Parses the `inspect` command: a scene plus the preview-size options.
///
/// A whole-scene preview is always a PNG, so `inspect` takes no `--format`; the
/// size is configurable so a preview too small to judge can be raised
/// (FEAT-022).
fn parse_inspect(args: Vec<OsString>) -> Result<Command, String> {
    let mut scene: Option<String> = None;
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
            None => set_scene_id(&mut scene, raw)?,
            Some("-h") | Some("--help") => return Ok(Command::Help),
            Some(text) => {
                if let Some(value) = text.strip_prefix("--out=") {
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
                            return Err(format!("unknown option `{t}` for `inspect`"))
                        }
                        _ => set_scene_id(&mut scene, raw)?,
                    }
                }
            }
        }
        i += 1;
    }

    Ok(Command::Inspect {
        scene,
        out,
        width,
        height,
        density,
        background,
    })
}

/// Parses the `render` command: one part plus the export-style options.
///
/// The format is optional here, defaulting to SVG: a vector preview is always
/// available, while PNG depends on the rasterizer (FEAT-031).
fn parse_render(args: Vec<OsString>) -> Result<Command, String> {
    let mut part: Option<String> = None;
    let mut format = Format::Svg;
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
            None => set_part(&mut part, raw)?,
            Some("-h") | Some("--help") => return Ok(Command::Help),
            Some(text) => {
                if let Some(value) = text.strip_prefix("--format=") {
                    format = parse_render_format(value)?;
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
                            format = parse_render_format(
                                &take_value(&args, &mut i, "--format")?.to_string_lossy(),
                            )?
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
                            return Err(format!("unknown option `{t}` for `render`"))
                        }
                        _ => set_part(&mut part, raw)?,
                    }
                }
            }
        }
        i += 1;
    }

    let part = part.ok_or_else(|| "`render` needs a part identifier".to_string())?;
    Ok(Command::Render {
        part,
        out,
        format,
        width,
        height,
        density,
        background,
    })
}

/// Parses the `schema` command's `--type` and `--compact` flags.
fn parse_schema(args: Vec<OsString>) -> Result<Command, String> {
    let mut type_name: Option<String> = None;
    let mut compact = false;
    let mut i = 0;
    while i < args.len() {
        let raw = args[i].clone();
        let text = raw.to_str().map(str::to_owned);
        match text.as_deref() {
            None => return Err("`schema` accepts no positional arguments".to_string()),
            Some("-h") | Some("--help") => return Ok(Command::Help),
            Some("--compact") => compact = true,
            Some(text) => {
                if let Some(value) = text.strip_prefix("--type=") {
                    type_name = Some(require_type(value)?);
                } else {
                    match text {
                        "--type" => {
                            let value = take_value(&args, &mut i, "--type")?;
                            type_name = Some(require_type(&value.to_string_lossy())?);
                        }
                        t if t.starts_with('-') && t != "-" => {
                            return Err(format!("unknown option `{t}` for `schema`"))
                        }
                        _ => return Err(format!("unexpected argument `{text}` for `schema`")),
                    }
                }
            }
        }
        i += 1;
    }
    Ok(Command::Schema { type_name, compact })
}

/// A `--type` value must name something.
fn require_type(value: &str) -> Result<String, String> {
    if value.is_empty() {
        Err("`--type` needs a type name".to_string())
    } else {
        Ok(value.to_string())
    }
}

/// Consumes the next argument as a flag's value.
fn take_value(args: &[OsString], index: &mut usize, flag: &str) -> Result<OsString, String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("`{flag}` needs a value"))
}

/// Records the single directory `init` accepts.
fn set_dir(slot: &mut Option<PathBuf>, value: OsString, message: &str) -> Result<(), String> {
    if slot.is_some() {
        return Err(message.to_string());
    }
    *slot = Some(PathBuf::from(value));
    Ok(())
}

/// Records the single scene identifier a subcommand accepts.
fn set_scene_id(slot: &mut Option<String>, value: OsString) -> Result<(), String> {
    if slot.is_some() {
        return Err("a command accepts at most one scene identifier".to_string());
    }
    let id = value
        .into_string()
        .map_err(|_| "the scene identifier is not valid UTF-8".to_string())?;
    *slot = Some(id);
    Ok(())
}

/// Records the single part identifier `render` accepts.
fn set_part(slot: &mut Option<String>, value: OsString) -> Result<(), String> {
    if slot.is_some() {
        return Err("`render` accepts at most one part identifier".to_string());
    }
    let id = value
        .into_string()
        .map_err(|_| "the part identifier is not valid UTF-8".to_string())?;
    *slot = Some(id);
    Ok(())
}

fn parse_format(value: &str) -> Result<Format, String> {
    match value {
        "svg" => Ok(Format::Svg),
        "png" => Ok(Format::Png),
        "pdf" => Ok(Format::Pdf),
        _ => Err(format!(
            "`--format` must be `svg`, `png` or `pdf`, got `{value}`"
        )),
    }
}

/// Parses the print colour profile the contract names, case-insensitively.
///
/// The profile is refused at the command line rather than passed through, so an
/// unusable value fails as a usage error before any scene is read (FEAT-014).
fn parse_profile(value: &str) -> Result<Profile, String> {
    match value.to_ascii_lowercase().as_str() {
        "srgb" => Ok(Profile::Srgb),
        "cmyk" => Ok(Profile::Cmyk),
        _ => Err(format!(
            "`--profile` must be `srgb` or `cmyk`, got `{value}`"
        )),
    }
}

/// Parses the `render` command's format: a part preview is a vector or raster
/// image, so PDF is not one of its targets (C-004).
fn parse_render_format(value: &str) -> Result<Format, String> {
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

fn validate_scene(scene: &ProjectScene, json: bool) -> Report {
    let parsed = match parse_project_scene(scene) {
        Ok(parsed) => parsed,
        Err((code, diagnostics)) => return report_findings(code, diagnostics, json),
    };

    let assets = match ProjectAssets::load(scene.root(), &parsed) {
        Ok(assets) => assets,
        Err(diagnostics) => {
            return report_findings(asset_exit_code(&diagnostics), diagnostics, json)
        }
    };

    let findings = structural_findings(&parsed, &assets);

    let code = if findings.has_errors() {
        dependency_exit_code(&findings, EXIT_INVALID_SCENE)
    } else {
        EXIT_SUCCESS
    };
    report_findings(code, findings, json)
}

/// The structural checks a scene and its project assets must pass (FEAT-018).
///
/// A palette token the palette no longer defines, a stroke profile, or a font
/// that does not resolve is an error naming it, so a restyle that broke a scene
/// is caught before anything is compiled (FEAT-005, FEAT-024). The checks run
/// against the scene with its definition instances expanded, so a reference
/// written inside a reusable part is checked like one written in the scene
/// (FEAT-030).
fn structural_findings(scene: &Scene, assets: &ProjectAssets) -> Diagnostics {
    let mut findings = validate_scene_model(scene);
    let expanded = assets.expanded_scene(scene);
    let usage_scene = expanded.as_ref().unwrap_or(scene);
    if let Some(palette) = assets.palette() {
        findings.extend(validate_palette_usage(
            usage_scene,
            palette,
            assets.gradients(),
        ));
    }
    findings.extend(validate_gradient_usage(usage_scene, assets.gradients()));
    findings.extend(assets.check_references(scene));
    findings
}

fn compile_scene(cwd: &Path, scene: &ProjectScene, out: Option<&Path>, check: bool) -> Report {
    let parsed = match parse_project_scene(scene) {
        Ok(parsed) => parsed,
        Err((code, diagnostics)) => return Report::failure(code, diagnostics_text(&diagnostics)),
    };
    let assets = match ProjectAssets::load(scene.root(), &parsed) {
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
        .map(|path| absolute(cwd, path))
        .unwrap_or_else(|| cwd.join(default_output(scene.id(), "json")));
    let text = match model.to_json_pretty() {
        Ok(text) => text,
        Err(diagnostics) => return Report::failure(EXIT_COMPILE, diagnostics_text(&diagnostics)),
    };
    write_report(&target, text.as_bytes(), &warnings)
}

#[allow(clippy::too_many_arguments)]
fn export_scene(
    scene: &ProjectScene,
    format: Format,
    target: &Path,
    width: Option<f64>,
    height: Option<f64>,
    density: Option<f64>,
    background: Option<&str>,
    profile: Option<Profile>,
) -> Report {
    if format != Format::Png && density.is_some() {
        return Report::failure(
            EXIT_USAGE,
            "error: `--density` applies only to PNG output\n".to_string(),
        );
    }
    // The profile names a print colour space, so it is meaningful only for the
    // PDF target; naming it elsewhere is refused rather than ignored (FEAT-014).
    if format != Format::Pdf && profile.is_some() {
        return Report::failure(
            EXIT_USAGE,
            "error: `--profile` applies only to PDF output\n".to_string(),
        );
    }

    // An export background override is a colour value like any other; a value
    // the target cannot render is refused before the scene is even read, so no
    // partial output can be produced (FEAT-005, FEAT-018).
    if let Some(background) = background {
        if !vectr_core::scene::is_color(background) {
            let diagnostics = Diagnostics::from(Diagnostic::error(
                INVALID_COLOR,
                format!("`--background` is not a colour SVG supports: `{background}`"),
            ));
            return Report::failure(EXIT_USAGE, diagnostics_text(&diagnostics));
        }
    }

    let parsed = match parse_project_scene(scene) {
        Ok(parsed) => parsed,
        Err((code, diagnostics)) => return Report::failure(code, diagnostics_text(&diagnostics)),
    };
    let assets = match ProjectAssets::load(scene.root(), &parsed) {
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

    let bytes = match export_bytes(
        &model,
        format,
        width,
        height,
        density,
        background,
        profile,
        &mut warnings,
    ) {
        Ok(bytes) => bytes,
        Err(report) => return report,
    };

    write_report(target, &bytes, &warnings)
}

/// The finding recorded when no in-process capability can compare a preview
/// with the original request, so the caller performs the inspection (FEAT-022).
const INSPECTION_UNAVAILABLE: DiagnosticCode = DiagnosticCode::new("W_INSPECTION_UNAVAILABLE");

/// Renders a whole-scene preview for inspection (FEAT-022).
///
/// The scene passes the same structural gate `validate` applies and is compiled
/// exactly as `export` does, so a broken scene is reported before a preview is
/// attempted and a render failure is reported before any inspection (NFR-011).
/// The preview is written as PNG at the requested size, and the command reports
/// its path and the size it rendered at. The command line has no inspection
/// capability of its own: it renders the preview for a person or a model to
/// compare against the request, and records that limitation rather than hiding
/// it (FEAT-022).
fn inspect_scene(
    scene: &ProjectScene,
    target: &Path,
    width: Option<f64>,
    height: Option<f64>,
    density: Option<f64>,
    background: Option<&str>,
) -> Report {
    // A background override is a colour value like any other, refused before
    // the scene is read so no partial preview can be produced (FEAT-005).
    if let Some(background) = background {
        if !vectr_core::scene::is_color(background) {
            let diagnostics = Diagnostics::from(Diagnostic::error(
                INVALID_COLOR,
                format!("`--background` is not a colour SVG supports: `{background}`"),
            ));
            return Report::failure(EXIT_USAGE, diagnostics_text(&diagnostics));
        }
    }

    let parsed = match parse_project_scene(scene) {
        Ok(parsed) => parsed,
        Err((code, diagnostics)) => return Report::failure(code, diagnostics_text(&diagnostics)),
    };
    let assets = match ProjectAssets::load(scene.root(), &parsed) {
        Ok(assets) => assets,
        Err(diagnostics) => {
            return Report::failure(
                asset_exit_code(&diagnostics),
                diagnostics_text(&diagnostics),
            )
        }
    };

    // Structural checks run first and gate the preview: a scene that fails them
    // is reported and nothing is rendered, so the structural findings still
    // stand even when no inspection can follow (FEAT-018, FEAT-022).
    let structural = structural_findings(&parsed, &assets);
    if structural.has_errors() {
        return Report::failure(
            dependency_exit_code(&structural, EXIT_INVALID_SCENE),
            diagnostics_text(&structural),
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

    let options = RasterOptions {
        width,
        height,
        density,
        background: background.map(str::to_string),
    };
    let export = match export_png_reporting(&model, &options) {
        Ok(export) => export,
        Err(diagnostics) => {
            return Report::failure(
                export_exit_code(&diagnostics),
                diagnostics_text(&diagnostics),
            )
        }
    };

    // The preview is complete: report the structural and export findings, then
    // note that the visual comparison with the request is the caller's.
    let mut warnings = diagnostics_text(&structural);
    warnings.push_str(&diagnostics_text(&model.diagnostics));
    warnings.push_str(&diagnostics_text(&export.diagnostics));
    warnings.push_str(&diagnostics_text(&Diagnostics::from(Diagnostic::warning(
        INSPECTION_UNAVAILABLE,
        "no inspection capability is available; the preview is rendered for a person or a model to compare against the request",
    ))));

    let size = png_size(&export.png);
    write_preview(target, &export.png, &warnings, size)
}

/// The pixel size a PNG encodes in its IHDR header, as `(width, height)`.
///
/// The exporter wrote a well-formed PNG, so this reads the size it actually
/// produced rather than re-deriving it from the requested options. A byte slice
/// that is not a PNG header has no size.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let read = |offset: usize| {
        u32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    Some((read(16), read(20)))
}

/// Writes an inspection preview, reporting the path and the size it rendered at.
fn write_preview(target: &Path, bytes: &[u8], warnings: &str, size: Option<(u32, u32)>) -> Report {
    match write_atomic(target, bytes) {
        Ok(()) => {
            let size = match size {
                Some((width, height)) => format!("{width}x{height}"),
                None => "unknown".to_string(),
            };
            Report {
                code: EXIT_SUCCESS,
                stdout: format!("wrote {}\npreview {size}\n", target.display()),
                stderr: warnings.to_string(),
            }
        }
        Err(error) => Report::failure(
            EXIT_OUTPUT,
            format!(
                "{warnings}error: cannot write output `{}`: {error}\n",
                target.display()
            ),
        ),
    }
}

/// Renders one part on its own, framed to its own bounds (FEAT-031).
///
/// The part is a reusable definition or an element subtree, resolved in the
/// project's shared definition-and-element namespace. A definition is previewed
/// against the project's default palette and default recipe, so its tokens and
/// style resolve with no placing scene; an element subtree is previewed against
/// the style of the scene that owns it (FEAT-005, D-039). A part identifier
/// that resolves to nothing, or to more than one definition or element, is
/// missing input (exit 2).
#[allow(clippy::too_many_arguments)]
fn render_part(
    cwd: &Path,
    part: &str,
    format: Format,
    target: &Path,
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

    // A background override is a colour value like any other, refused before
    // anything is rendered so no partial output can be produced (FEAT-005).
    if let Some(background) = background {
        if !vectr_core::scene::is_color(background) {
            let diagnostics = Diagnostics::from(Diagnostic::error(
                INVALID_COLOR,
                format!("`--background` is not a colour SVG supports: `{background}`"),
            ));
            return Report::failure(EXIT_USAGE, diagnostics_text(&diagnostics));
        }
    }

    let root = project_root_from(cwd);
    let resolved = match resolve_part(&root, part) {
        Ok(resolved) => resolved,
        Err(diagnostics) => {
            return Report::failure(
                asset_exit_code(&diagnostics),
                diagnostics_text(&diagnostics),
            )
        }
    };

    let compiled = match &resolved {
        ResolvedPart::Definition { definition, assets } => {
            compile_definition(definition, &assets.style_context())
        }
        ResolvedPart::Element { scene, assets } => {
            compile_subtree(scene, part, &assets.style_context())
        }
    };
    let model = match compiled {
        Ok(model) => model,
        Err(diagnostics) => {
            return Report::failure(
                dependency_exit_code(&diagnostics, EXIT_COMPILE),
                diagnostics_text(&diagnostics),
            )
        }
    };

    let frame = output_frame(&model, format, width, height, density);
    let mut warnings = diagnostics_text(&model.diagnostics);
    let bytes = match export_bytes(
        &model,
        format,
        width,
        height,
        density,
        background,
        None,
        &mut warnings,
    ) {
        Ok(bytes) => bytes,
        Err(report) => return report,
    };
    write_part(target, &bytes, &warnings, frame)
}

/// The output frame a preview actually uses, after size and density compose.
///
/// A single requested dimension scales the other to keep the part's aspect
/// ratio, and PNG density multiplies the resolved size, mirroring the
/// exporters. Reporting the resolved frame tells a caller how large the preview
/// came out when the part did not fit the requested size (FEAT-031).
fn output_frame(
    model: &RenderModel,
    format: Format,
    width: Option<f64>,
    height: Option<f64>,
    density: Option<f64>,
) -> (f64, f64) {
    let canvas = &model.canvas;
    let mut resolved_width = width.unwrap_or(canvas.width);
    let mut resolved_height = height.unwrap_or(canvas.height);
    let scalable = canvas.width > 0.0 && canvas.height > 0.0;
    match (width, height) {
        (Some(width), None) if scalable => {
            resolved_height = width * canvas.height / canvas.width;
        }
        (None, Some(height)) if scalable => {
            resolved_width = height * canvas.width / canvas.height;
        }
        _ => {}
    }
    if format == Format::Png {
        let density = density.unwrap_or(1.0);
        resolved_width *= density;
        resolved_height *= density;
    }
    (resolved_width, resolved_height)
}

/// Exports a compiled model to bytes, mapping an export failure to a report.
///
/// Shared by `export` and `render`, so both surface the same exporter warnings
/// and classify a missing dependency the same way (C-004).
#[allow(clippy::too_many_arguments)]
fn export_bytes(
    model: &RenderModel,
    format: Format,
    width: Option<f64>,
    height: Option<f64>,
    density: Option<f64>,
    background: Option<&str>,
    profile: Option<Profile>,
    warnings: &mut String,
) -> Result<Vec<u8>, Report> {
    match format {
        Format::Svg => {
            let options = SvgOptions {
                width,
                height,
                background: background.map(str::to_string),
            };
            match export_svg_reporting(model, &options) {
                Ok(export) => {
                    warnings.push_str(&diagnostics_text(&export.diagnostics));
                    Ok(export.svg.into_bytes())
                }
                Err(diagnostics) => Err(Report::failure(
                    export_exit_code(&diagnostics),
                    diagnostics_text(&diagnostics),
                )),
            }
        }
        Format::Png => {
            let options = RasterOptions {
                width,
                height,
                density,
                background: background.map(str::to_string),
            };
            match export_png_reporting(model, &options) {
                Ok(export) => {
                    warnings.push_str(&diagnostics_text(&export.diagnostics));
                    Ok(export.png)
                }
                Err(diagnostics) => Err(Report::failure(
                    export_exit_code(&diagnostics),
                    diagnostics_text(&diagnostics),
                )),
            }
        }
        Format::Pdf => {
            let options = PdfOptions {
                page_width: width,
                page_height: height,
                profile: profile.map(Profile::name).map(str::to_string),
                background: background.map(str::to_string),
            };
            match export_pdf_reporting(model, &options) {
                Ok(export) => {
                    warnings.push_str(&diagnostics_text(&export.diagnostics));
                    Ok(export.pdf)
                }
                Err(diagnostics) => Err(Report::failure(
                    export_exit_code(&diagnostics),
                    diagnostics_text(&diagnostics),
                )),
            }
        }
    }
}

/// Writes a part preview, reporting the frame it was rendered in (FEAT-031).
fn write_part(target: &Path, bytes: &[u8], warnings: &str, frame: (f64, f64)) -> Report {
    match write_atomic(target, bytes) {
        Ok(()) => Report {
            code: EXIT_SUCCESS,
            stdout: format!(
                "wrote {}\nframe {}x{}\n",
                target.display(),
                frame.0,
                frame.1
            ),
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

/// Prints the language contract, or one type, mapping a bad request to exit 2.
///
/// An unknown type or a published schema that does not match the tool is a
/// usage-class failure (FEAT-017, C-004).
fn schema_command(type_name: Option<&str>, compact: bool) -> Report {
    let rendered = match type_name {
        Some(name) => {
            schema_for(name).and_then(|text| if compact { minify(text) } else { Ok(text) })
        }
        None => schema(if compact {
            SchemaForm::Compact
        } else {
            SchemaForm::Full
        }),
    };
    match rendered {
        Ok(text) => Report::success(format!("{text}\n")),
        Err(diagnostics) => Report::failure(EXIT_USAGE, diagnostics_text(&diagnostics)),
    }
}

/// Compacts an already-valid schema document for machine consumption.
fn minify(schema: String) -> Result<String, Diagnostics> {
    let value: serde_json::Value = serde_json::from_str(&schema).map_err(|error| {
        Diagnostics::from(Diagnostic::error(
            DiagnosticCode::SCHEMA,
            format!("the schema is not valid JSON: {error}"),
        ))
    })?;
    serde_json::to_string(&value).map_err(|error| {
        Diagnostics::from(Diagnostic::error(
            DiagnosticCode::SCHEMA,
            format!("could not compact the schema: {error}"),
        ))
    })
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

/// Reads and parses a resolved scene document.
///
/// A document that cannot be read is missing input (exit 2); one that does not
/// parse is an invalid scene (exit 1) (C-004).
fn parse_project_scene(scene: &ProjectScene) -> Result<Scene, (i32, Diagnostics)> {
    let source = scene
        .source()
        .map_err(|diagnostics| (EXIT_USAGE, diagnostics))?;
    parse_scene_source(&source).map_err(|diagnostics| (EXIT_INVALID_SCENE, diagnostics))
}

/// The default output path for a scene: `dist/<id>.<extension>`.
fn default_output(scene_id: &str, extension: &str) -> PathBuf {
    Path::new("dist").join(format!("{scene_id}.{extension}"))
}

/// Classifies an export failure: a missing dependency outranks a usage error,
/// which outranks a defined size limit.
fn export_exit_code(diagnostics: &Diagnostics) -> i32 {
    let mut code = EXIT_COMPILE;
    for error in diagnostics.errors() {
        if error.code == png_export::RASTERIZER {
            return EXIT_DEPENDENCY;
        }
        if error.code == svg_export::OPTIONS
            || error.code == png_export::OPTIONS
            || error.code == pdf_export::OPTIONS
            || error.code == pdf_export::PROFILE
        {
            code = EXIT_USAGE;
        }
    }
    code
}

/// The report for a disabled PDF export, when the rollout flag is off.
///
/// PDF export is off by default (FEAT-014); the flag is read from the
/// environment so the capability can be enabled without a rebuild.
fn pdf_disabled() -> Option<Report> {
    if enabled_value(std::env::var(ENABLE_PDF_ENV).ok().as_deref()) {
        None
    } else {
        Some(Report::failure(
            EXIT_USAGE,
            format!("error: PDF export is disabled; set {ENABLE_PDF_ENV}=1 to enable it\n"),
        ))
    }
}

/// Whether a flag value enables a gated capability.
pub fn enabled_value(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("on")
    )
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
    use std::fs;

    /// Serializes the tests that toggle the PDF rollout flag, which lives in the
    /// process environment and is shared by every test in this binary.
    static PDF_FLAG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Holds the PDF-flag lock for the duration of a test.
    fn pdf_flag() -> std::sync::MutexGuard<'static, ()> {
        PDF_FLAG_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Writes a project whose `defaultSceneId` is `id`, holding one scene at
    /// `scenes/<id>.json`.
    fn project(dir: &TempDir, id: &str, text: &str) {
        write_at(
            dir,
            "vectr.project.json",
            &format!(r#"{{"defaultSceneId":"{id}"}}"#),
        );
        write_at(dir, &format!("scenes/{id}.json"), text);
    }

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
    fn inspect_parses_the_scene_and_defaults_to_the_project_scene() {
        assert_eq!(
            parse_args(&["inspect"]).unwrap(),
            Command::Inspect {
                scene: None,
                out: None,
                width: None,
                height: None,
                density: None,
                background: None,
            }
        );
        assert_eq!(
            parse_args(&[
                "inspect",
                "logo",
                "--out",
                "dist/logo.png",
                "--width",
                "640",
                "--height",
                "480",
                "--density",
                "2",
                "--background",
                "transparent",
            ])
            .unwrap(),
            Command::Inspect {
                scene: Some("logo".to_string()),
                out: Some(PathBuf::from("dist/logo.png")),
                width: Some(640.0),
                height: Some(480.0),
                density: Some(2.0),
                background: Some("transparent".to_string()),
            }
        );
    }

    #[test]
    fn inspect_refuses_unknown_options_and_extra_positionals() {
        assert!(parse_args(&["inspect", "a", "b"]).is_err());
        assert!(parse_args(&["inspect", "logo", "--nope"]).is_err());
        assert!(parse_args(&["inspect", "logo", "--format", "png"]).is_err());
    }

    #[test]
    fn schema_parses_its_type_and_compact_flags() {
        assert_eq!(
            parse_args(&["schema"]).unwrap(),
            Command::Schema {
                type_name: None,
                compact: false
            }
        );
        assert_eq!(
            parse_args(&["schema", "--compact"]).unwrap(),
            Command::Schema {
                type_name: None,
                compact: true
            }
        );
        assert_eq!(
            parse_args(&["schema", "--type", "Element"]).unwrap(),
            parse_args(&["schema", "--type=Element"]).unwrap()
        );
        assert_eq!(
            parse_args(&["schema", "--type", "Element", "--compact"]).unwrap(),
            Command::Schema {
                type_name: Some("Element".to_string()),
                compact: true
            }
        );
    }

    #[test]
    fn schema_refuses_a_missing_type_and_unknown_options() {
        assert!(parse_args(&["schema", "--type"]).is_err());
        assert!(parse_args(&["schema", "--type="]).is_err());
        assert!(parse_args(&["schema", "--nope"]).is_err());
        assert!(parse_args(&["schema", "Element"]).is_err());
    }

    #[test]
    fn schema_prints_the_contract_and_reports_an_unknown_type() {
        let report = run(Command::Schema {
            type_name: None,
            compact: false,
        });
        assert_eq!(report.code, EXIT_SUCCESS);
        serde_json::from_str::<serde_json::Value>(&report.stdout).expect("valid JSON");

        let compact = run(Command::Schema {
            type_name: None,
            compact: true,
        });
        assert_eq!(compact.code, EXIT_SUCCESS);
        assert!(!compact.stdout.trim().contains('\n'));

        let element = run(Command::Schema {
            type_name: Some("Element".to_string()),
            compact: false,
        });
        assert_eq!(element.code, EXIT_SUCCESS);
        assert!(
            element.stdout.contains("\"ElementKind\""),
            "{}",
            element.stdout
        );

        let unknown = run(Command::Schema {
            type_name: Some("Palete".to_string()),
            compact: false,
        });
        assert_eq!(unknown.code, EXIT_USAGE);
        assert!(unknown.stderr.contains("Palette"), "{}", unknown.stderr);
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
            parse_args(&["validate", "logo"]).unwrap(),
            Command::Validate {
                scene: Some("logo".to_string()),
                json: false
            }
        );
        assert_eq!(
            parse_args(&["validate", "--json", "logo"]).unwrap(),
            Command::Validate {
                scene: Some("logo".to_string()),
                json: true
            }
        );
        // Omitting the scene is valid: the project's default is used.
        assert_eq!(
            parse_args(&["validate", "--json"]).unwrap(),
            Command::Validate {
                scene: None,
                json: true
            }
        );
    }

    #[test]
    fn compile_parses_out_and_check_in_both_flag_forms() {
        assert_eq!(
            parse_args(&["compile", "logo", "--check"]).unwrap(),
            Command::Compile {
                scene: Some("logo".to_string()),
                out: None,
                check: true
            }
        );
        assert_eq!(
            parse_args(&["compile", "logo", "--out", "dist/a.json"]).unwrap(),
            parse_args(&["compile", "logo", "--out=dist/a.json"]).unwrap()
        );
    }

    #[test]
    fn export_parses_every_option() {
        let command = parse_args(&[
            "export",
            "logo",
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
                scene: Some("logo".to_string()),
                format: Format::Png,
                out: Some(PathBuf::from("dist/a.png")),
                width: Some(64.0),
                height: Some(64.0),
                density: Some(2.0),
                background: Some("transparent".to_string()),
                profile: None,
            }
        );
    }

    #[test]
    fn export_requires_a_known_format() {
        assert!(parse_args(&["export", "scene.json"]).is_err());
        assert!(parse_args(&["export", "scene.json", "--format", "tiff"]).is_err());
        assert_eq!(
            parse_args(&["export", "scene.json", "--format", "pdf"]).unwrap(),
            Command::Export {
                scene: Some("scene.json".to_string()),
                format: Format::Pdf,
                out: None,
                width: None,
                height: None,
                density: None,
                background: None,
                profile: None,
            }
        );
    }

    #[test]
    fn export_parses_the_print_profile_in_both_flag_forms() {
        let expected = |profile: Option<Profile>| Command::Export {
            scene: Some("logo".to_string()),
            format: Format::Pdf,
            out: None,
            width: None,
            height: None,
            density: None,
            background: None,
            profile,
        };
        assert_eq!(
            parse_args(&["export", "logo", "--format", "pdf", "--profile", "cmyk"]).unwrap(),
            expected(Some(Profile::Cmyk))
        );
        assert_eq!(
            parse_args(&["export", "logo", "--format=pdf", "--profile=srgb"]).unwrap(),
            expected(Some(Profile::Srgb))
        );
    }

    #[test]
    fn export_refuses_an_unknown_profile_or_a_missing_value() {
        assert!(parse_args(&[
            "export",
            "logo",
            "--format",
            "pdf",
            "--profile",
            "adobe-rgb"
        ])
        .is_err());
        assert!(parse_args(&["export", "logo", "--format", "pdf", "--profile"]).is_err());
        assert!(parse_args(&["export", "logo", "--format", "pdf", "--profile="]).is_err());
    }

    #[test]
    fn render_does_not_accept_pdf() {
        assert!(parse_args(&["render", "badge", "--format", "pdf"]).is_err());
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
            default_output("logo", "svg"),
            PathBuf::from("dist").join("logo.svg")
        );
        assert_eq!(
            default_output("habit-logo", "json"),
            PathBuf::from("dist").join("habit-logo.json")
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
        project(&dir, "scene-1", VALID_SCENE);
        let report = run_in(
            Command::Validate {
                scene: None,
                json: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS);
        assert!(report.stderr.is_empty(), "{}", report.stderr);
    }

    #[test]
    fn validate_reports_an_invalid_scene_with_exit_one() {
        let dir = TempDir::new("validate-invalid");
        project(&dir, "scene-1", INVALID_SCENE);
        let report = run_in(
            Command::Validate {
                scene: None,
                json: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_INVALID_SCENE);
        assert!(report.stderr.contains("E_SCHEMA"), "{}", report.stderr);
    }

    #[test]
    fn validate_json_emits_a_machine_readable_array() {
        let valid = TempDir::new("validate-json-valid");
        project(&valid, "scene-1", VALID_SCENE);
        let report = run_in(
            Command::Validate {
                scene: None,
                json: true,
            },
            valid.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS);
        assert_eq!(report.stdout.trim(), "[]");

        let invalid = TempDir::new("validate-json-invalid");
        project(&invalid, "scene-1", INVALID_SCENE);
        let report = run_in(
            Command::Validate {
                scene: None,
                json: true,
            },
            invalid.path(),
        );
        assert_eq!(report.code, EXIT_INVALID_SCENE);
        let parsed: serde_json::Value =
            serde_json::from_str(report.stdout.trim()).expect("valid JSON");
        assert!(parsed.as_array().is_some_and(|items| !items.is_empty()));
    }

    #[test]
    fn a_scene_identifier_no_document_provides_is_a_usage_error() {
        let dir = TempDir::new("missing-named");
        write_at(&dir, "vectr.project.json", "{}");
        let report = run_in(
            Command::Compile {
                scene: Some("absent".to_string()),
                out: None,
                check: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(report.stderr.contains("absent"), "{}", report.stderr);
    }

    #[test]
    fn a_project_that_names_no_default_reports_no_scene_selected() {
        let dir = TempDir::new("missing-default");
        write_at(&dir, "vectr.project.json", "{}");
        write_at(&dir, "scenes/one.json", VALID_SCENE);
        write_at(&dir, "scenes/two.json", VALID_SCENE);

        let report = run_in(
            Command::Validate {
                scene: None,
                json: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(
            report.stderr.contains("no default scene"),
            "{}",
            report.stderr
        );
    }

    #[test]
    fn a_default_scene_that_resolves_to_no_document_names_it() {
        let dir = TempDir::new("default-missing");
        write_at(&dir, "vectr.project.json", r#"{"defaultSceneId":"absent"}"#);

        let report = run_in(
            Command::Validate {
                scene: None,
                json: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(report.stderr.contains("absent"), "{}", report.stderr);
    }

    #[test]
    fn a_named_scene_overrides_the_project_default() {
        let dir = TempDir::new("named-override");
        // The default is invalid; naming the valid scene must win.
        write_at(&dir, "vectr.project.json", r#"{"defaultSceneId":"bad"}"#);
        write_at(&dir, "scenes/bad.json", INVALID_SCENE);
        write_at(&dir, "scenes/good.json", VALID_SCENE);

        let report = run_in(
            Command::Validate {
                scene: Some("good".to_string()),
                json: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
    }

    #[test]
    fn a_valid_scene_compiles_to_a_render_model() {
        let dir = TempDir::new("compile-valid");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("model.json");
        let report = run_in(
            Command::Compile {
                scene: None,
                out: Some(out.clone()),
                check: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS);
        let text = fs::read_to_string(&out).expect("reads the model");
        vectr_core::render::parse(&text).expect("the output is a render model");
    }

    #[test]
    fn a_compilation_failure_exits_three() {
        let dir = TempDir::new("compile-cycle");
        project(&dir, "scene-1", CYCLIC_SCENE);
        let report = run_in(
            Command::Compile {
                scene: None,
                out: None,
                check: true,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_COMPILE);
        assert!(report.stderr.contains("E_CYCLE"), "{}", report.stderr);
    }

    #[test]
    fn check_writes_nothing() {
        let dir = TempDir::new("compile-check");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("model.json");
        let report = run_in(
            Command::Compile {
                scene: None,
                out: Some(out.clone()),
                check: true,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS);
        assert!(!out.exists(), "no output under --check");
    }

    #[test]
    fn a_failed_compile_leaves_an_existing_output_untouched() {
        let dir = TempDir::new("compile-untouched");
        project(&dir, "scene-1", INVALID_SCENE);
        let out = dir.path().join("model.json");
        fs::write(&out, "previous").expect("seeds the output");
        let report = run_in(
            Command::Compile {
                scene: None,
                out: Some(out.clone()),
                check: false,
            },
            dir.path(),
        );
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
        project(&dir, "scene-1", VALID_SCENE);
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, "not a directory").expect("seeds the blocker");
        let report = run_in(
            Command::Compile {
                scene: None,
                out: Some(blocker.join("model.json")),
                check: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_OUTPUT);
    }

    #[test]
    fn a_valid_scene_exports_svg() {
        let dir = TempDir::new("export-svg");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("out.svg");
        let report = run_in(
            Command::Export {
                scene: None,
                format: Format::Svg,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: None,
                profile: None,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS);
        let svg = fs::read_to_string(&out).expect("reads the svg");
        assert!(svg.contains("<svg"), "{svg}");
        assert!(svg.contains("</svg>"), "{svg}");
    }

    #[test]
    fn a_valid_scene_exports_png() {
        let dir = TempDir::new("export-png");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("out.png");
        let report = run_in(
            Command::Export {
                scene: None,
                format: Format::Png,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: None,
                profile: None,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS);
        let bytes = fs::read(&out).expect("reads the png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    }

    #[test]
    fn pdf_export_is_gated_and_writes_a_vector_document() {
        let _guard = pdf_flag();
        let dir = TempDir::new("export-pdf");
        write_at(&dir, "vectr.project.json", "{}");
        write_at(&dir, "palettes/brand.json", PALETTE);
        write_at(&dir, "scenes/brand.json", PALETTE_SCENE);
        let out = dir.path().join("out.pdf");

        // Off by default: a usage error and nothing written (FEAT-014).
        std::env::remove_var(ENABLE_PDF_ENV);
        let disabled = run_in(
            Command::Export {
                scene: Some("brand".to_string()),
                format: Format::Pdf,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: None,
                profile: None,
            },
            dir.path(),
        );
        assert_eq!(disabled.code, EXIT_USAGE, "{}", disabled.stderr);
        assert!(!out.exists(), "nothing is written while the flag is off");

        // Enabled: a vector PDF is written.
        std::env::set_var(ENABLE_PDF_ENV, "1");
        let report = run_in(
            Command::Export {
                scene: Some("brand".to_string()),
                format: Format::Pdf,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: None,
                profile: None,
            },
            dir.path(),
        );
        std::env::remove_var(ENABLE_PDF_ENV);
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        let bytes = fs::read(&out).expect("reads the pdf");
        assert!(bytes.starts_with(b"%PDF-"), "a PDF signature");
        assert!(bytes.ends_with(b"%%EOF\n"), "a complete document");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("/Subtype /Image"), "vector only: {text}");
        assert!(text.contains(" re\n"), "vector path operators: {text}");
        assert!(
            text.contains("1 0 0 rg"),
            "the resolved fill colour: {text}"
        );
    }

    #[test]
    fn a_profile_on_a_non_pdf_export_is_a_usage_error() {
        let dir = TempDir::new("export-profile-svg");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("out.svg");
        let report = run_in(
            Command::Export {
                scene: None,
                format: Format::Svg,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: None,
                profile: Some(Profile::Cmyk),
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(
            report.stderr.contains("applies only to PDF output"),
            "{}",
            report.stderr
        );
        assert!(
            !out.exists(),
            "no output is written for an inapplicable profile"
        );
    }

    #[test]
    fn a_pdf_export_honours_the_requested_profile() {
        let _guard = pdf_flag();
        let dir = TempDir::new("export-pdf-profile");
        write_at(&dir, "vectr.project.json", "{}");
        write_at(&dir, "palettes/brand.json", PALETTE);
        write_at(&dir, "scenes/brand.json", PALETTE_SCENE);

        std::env::set_var(ENABLE_PDF_ENV, "1");
        let cmyk_out = dir.path().join("cmyk.pdf");
        let cmyk = run_in(
            Command::Export {
                scene: Some("brand".to_string()),
                format: Format::Pdf,
                out: Some(cmyk_out.clone()),
                width: None,
                height: None,
                density: None,
                background: None,
                profile: Some(Profile::Cmyk),
            },
            dir.path(),
        );
        let srgb_out = dir.path().join("srgb.pdf");
        let srgb = run_in(
            Command::Export {
                scene: Some("brand".to_string()),
                format: Format::Pdf,
                out: Some(srgb_out.clone()),
                width: None,
                height: None,
                density: None,
                background: None,
                profile: Some(Profile::Srgb),
            },
            dir.path(),
        );
        std::env::remove_var(ENABLE_PDF_ENV);

        assert_eq!(cmyk.code, EXIT_SUCCESS, "{}", cmyk.stderr);
        let cmyk_bytes = fs::read(&cmyk_out).expect("reads the pdf");
        let cmyk_text = String::from_utf8_lossy(&cmyk_bytes);
        assert!(
            cmyk_text.contains(" k\n"),
            "the CMYK colour operator: {cmyk_text}"
        );
        assert!(
            !cmyk_text.contains(" rg\n"),
            "no device RGB under CMYK: {cmyk_text}"
        );

        assert_eq!(srgb.code, EXIT_SUCCESS, "{}", srgb.stderr);
        let srgb_bytes = fs::read(&srgb_out).expect("reads the pdf");
        let srgb_text = String::from_utf8_lossy(&srgb_bytes);
        assert!(
            srgb_text.contains(" rg\n"),
            "device RGB under sRGB: {srgb_text}"
        );
    }

    #[test]
    fn a_cmyk_profile_flattens_transparency_with_a_notice() {
        let _guard = pdf_flag();
        let dir = TempDir::new("export-pdf-flatten");
        write_at(&dir, "vectr.project.json", "{}");
        // An alpha token, so the paint carries transparency the CMYK space
        // cannot represent (FEAT-005, FEAT-014).
        write_at(
            &dir,
            "palettes/brand.json",
            r##"{"id":"brand","projectId":"project","name":"Brand","tokens":[{"name":"accent","value":"#ff000080"}]}"##,
        );
        write_at(&dir, "scenes/brand.json", PALETTE_SCENE);
        let out = dir.path().join("cmyk.pdf");

        std::env::set_var(ENABLE_PDF_ENV, "1");
        let report = run_in(
            Command::Export {
                scene: Some("brand".to_string()),
                format: Format::Pdf,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: None,
                profile: Some(Profile::Cmyk),
            },
            dir.path(),
        );
        std::env::remove_var(ENABLE_PDF_ENV);

        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        assert!(
            report.stderr.contains("W_PDF_TRANSPARENCY_FLATTENED"),
            "the flattening is reported: {}",
            report.stderr
        );
    }

    #[test]
    fn the_pdf_flag_recognizes_the_enabling_values() {
        assert!(enabled_value(Some("1")));
        assert!(enabled_value(Some("true")));
        assert!(enabled_value(Some("on")));
        assert!(!enabled_value(Some("0")));
        assert!(!enabled_value(None));
    }

    #[test]
    fn density_on_svg_is_a_usage_error() {
        let dir = TempDir::new("export-density");
        project(&dir, "scene-1", VALID_SCENE);
        let report = run_in(
            Command::Export {
                scene: None,
                format: Format::Svg,
                out: None,
                width: None,
                height: None,
                density: Some(2.0),
                background: None,
                profile: None,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
    }

    #[test]
    fn an_export_background_that_is_not_a_colour_is_refused_before_any_output() {
        let dir = TempDir::new("export-background");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("out.svg");
        let report = run_in(
            Command::Export {
                scene: None,
                format: Format::Svg,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: Some("not-a-colour".to_string()),
                profile: None,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(
            report.stderr.contains("E_INVALID_COLOR"),
            "{}",
            report.stderr
        );
        assert!(report.stderr.contains("not-a-colour"), "{}", report.stderr);
        assert!(!out.exists(), "no output is written for an invalid colour");
    }

    #[test]
    fn a_named_export_background_colour_is_honoured() {
        let dir = TempDir::new("export-background-named");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("out.svg");
        let report = run_in(
            Command::Export {
                scene: None,
                format: Format::Svg,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: Some("red".to_string()),
                profile: None,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        let svg = fs::read_to_string(&out).expect("reads the svg");
        assert!(svg.contains("fill=\"red\""), "{svg}");
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
        write_at(&dir, "scenes/logo.json", PROJECT_TEXT_SCENE);
        let out = dir.path().join("logo.svg");

        let report = run_in(
            Command::Export {
                scene: Some("logo".to_string()),
                format: Format::Svg,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: None,
                profile: None,
            },
            dir.path(),
        );
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
        write_at(&dir, "scenes/logo.json", &scene_text);

        let report = run_in(
            Command::Compile {
                scene: Some("logo".to_string()),
                out: None,
                check: true,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_DEPENDENCY);
        assert!(report.stderr.contains("absent"), "{}", report.stderr);
    }

    #[test]
    fn a_project_palette_resolves_a_fill_token() {
        let dir = TempDir::new("project-palette");
        write_at(&dir, "vectr.project.json", "{}");
        write_at(&dir, "palettes/brand.json", PALETTE);
        write_at(&dir, "scenes/brand.json", PALETTE_SCENE);
        let out = dir.path().join("brand.json");

        let report = run_in(
            Command::Compile {
                scene: Some("brand".to_string()),
                out: Some(out.clone()),
                check: false,
            },
            dir.path(),
        );
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
        write_at(&dir, "scenes/brand.json", PALETTE_SCENE);

        let report = run_in(
            Command::Validate {
                scene: Some("brand".to_string()),
                json: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_INVALID_SCENE);
        assert!(report.stderr.contains("accent"), "{}", report.stderr);
    }

    #[test]
    fn validate_reports_a_palette_token_that_is_not_a_colour() {
        let dir = TempDir::new("project-invalid-colour");
        write_at(&dir, "vectr.project.json", "{}");
        write_at(
            &dir,
            "palettes/brand.json",
            r##"{"id":"brand","projectId":"project","name":"Brand","tokens":[{"name":"accent","value":"not-a-colour"}]}"##,
        );
        write_at(&dir, "scenes/brand.json", PALETTE_SCENE);

        let report = run_in(
            Command::Validate {
                scene: Some("brand".to_string()),
                json: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(
            report.stderr.contains("E_INVALID_COLOR"),
            "{}",
            report.stderr
        );
        assert!(report.stderr.contains("accent"), "{}", report.stderr);
        assert!(report.stderr.contains("not-a-colour"), "{}", report.stderr);
    }

    #[test]
    fn a_missing_palette_is_missing_input() {
        let dir = TempDir::new("project-missing-palette");
        write_at(&dir, "vectr.project.json", "{}");
        write_at(&dir, "scenes/brand.json", PALETTE_SCENE);

        let report = run_in(
            Command::Validate {
                scene: Some("brand".to_string()),
                json: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(report.stderr.contains("brand"), "{}", report.stderr);
    }

    #[test]
    fn a_project_recipe_reaches_the_compiled_model() {
        let dir = TempDir::new("project-recipe");
        write_at(&dir, "vectr.project.json", "{}");
        write_at(
            &dir,
            "recipes/line.json",
            r#"{"id":"line","projectId":"project","name":"line-art","parameters":{"strokeWeight":2}}"#,
        );
        let scene_text = VALID_SCENE.replace(
            r##""formatVersion": "0.2","##,
            r##""formatVersion": "0.2", "recipeId": "line","##,
        );
        write_at(&dir, "scenes/logo.json", &scene_text);
        let out = dir.path().join("model.json");

        let report = run_in(
            Command::Compile {
                scene: Some("logo".to_string()),
                out: Some(out.clone()),
                check: false,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        let text = fs::read_to_string(&out).expect("reads the model");
        let model = vectr_core::render::parse(&text).expect("a render model");
        assert_eq!(model.meta.recipe.as_deref(), Some("line-art"));
    }

    #[test]
    fn a_missing_recipe_is_missing_input() {
        let dir = TempDir::new("project-missing-recipe");
        write_at(&dir, "vectr.project.json", "{}");
        let scene_text = VALID_SCENE.replace(
            r##""formatVersion": "0.2","##,
            r##""formatVersion": "0.2", "recipeId": "absent","##,
        );
        write_at(&dir, "scenes/logo.json", &scene_text);

        let report = run_in(
            Command::Compile {
                scene: Some("logo".to_string()),
                out: None,
                check: true,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(report.stderr.contains("absent"), "{}", report.stderr);
    }

    // -----------------------------------------------------------------------
    // Part-scoped rendering (FEAT-031)
    // -----------------------------------------------------------------------

    /// A definition whose single rect sits off the origin, so a preview must
    /// frame it to its own bounds.
    const BADGE_DEFINITION: &str = r##"{
      "id": "badge",
      "projectId": "project",
      "name": "Badge",
      "parameters": [],
      "origin": { "x": 0, "y": 0 },
      "elements": [
        {
          "id": "r1", "definitionId": "badge", "order": 0, "kind": "rect",
          "geometry": { "x": 10, "y": 20, "width": 30, "height": 40 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "fill": { "kind": "token", "ref": "accent" },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    /// A definition with no drawable geometry.
    const EMPTY_DEFINITION: &str = r##"{
      "id": "empty",
      "projectId": "project",
      "name": "Empty",
      "parameters": [],
      "origin": { "x": 0, "y": 0 },
      "elements": []
    }"##;

    /// A scene holding a named group subtree beside an unrelated shape.
    const SUBTREE_SCENE: &str = r##"{
      "id": "main",
      "projectId": "project",
      "name": "Main",
      "formatVersion": "0.2",
      "paletteId": "brand",
      "canvas": { "width": 200, "height": 200, "background": "#ffffff" },
      "elements": [
        {
          "id": "mark", "sceneId": "main", "order": 0, "kind": "group", "name": "Mark",
          "geometry": {},
          "transform": { "translateX": 50, "translateY": 50, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        },
        {
          "id": "mark-rect", "sceneId": "main", "parentId": "mark", "order": 0, "kind": "rect",
          "geometry": { "x": 0, "y": 0, "width": 20, "height": 10 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "fill": { "kind": "token", "ref": "accent" },
          "opacity": 1, "visible": true
        },
        {
          "id": "other", "sceneId": "main", "order": 1, "kind": "rect",
          "geometry": { "x": 100, "y": 100, "width": 50, "height": 50 },
          "transform": { "translateX": 0, "translateY": 0, "rotate": 0, "scaleX": 1, "scaleY": 1 },
          "opacity": 1, "visible": true
        }
      ]
    }"##;

    /// Writes a project with a default scene, a default palette, and two
    /// definitions.
    ///
    /// The project names `brand` as its default palette, so an isolated
    /// definition resolves its `accent` token without a placing scene
    /// (FEAT-005, D-039).
    fn part_project(dir: &TempDir) {
        write_at(
            dir,
            "vectr.project.json",
            r#"{"defaultSceneId":"main","defaultPaletteId":"brand"}"#,
        );
        write_at(dir, "palettes/brand.json", PALETTE);
        write_at(dir, "definitions/badge.json", BADGE_DEFINITION);
        write_at(dir, "definitions/empty.json", EMPTY_DEFINITION);
        write_at(dir, "scenes/main.json", SUBTREE_SCENE);
    }

    fn render(part: &str, out: Option<PathBuf>, format: Format, width: Option<f64>) -> Command {
        Command::Render {
            part: part.to_string(),
            out,
            format,
            width,
            height: None,
            density: None,
            background: None,
        }
    }

    #[test]
    fn render_parses_the_part_and_defaults_to_svg() {
        assert_eq!(
            parse_args(&["render", "badge"]).unwrap(),
            Command::Render {
                part: "badge".to_string(),
                out: None,
                format: Format::Svg,
                width: None,
                height: None,
                density: None,
                background: None,
            }
        );
    }

    #[test]
    fn render_parses_every_option() {
        let command = parse_args(&[
            "render",
            "badge",
            "--format",
            "png",
            "--out",
            "dist/badge.png",
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
            Command::Render {
                part: "badge".to_string(),
                out: Some(PathBuf::from("dist/badge.png")),
                format: Format::Png,
                width: Some(64.0),
                height: Some(64.0),
                density: Some(2.0),
                background: Some("transparent".to_string()),
            }
        );
    }

    #[test]
    fn render_requires_a_part_and_refuses_unknown_options() {
        assert!(parse_args(&["render"]).is_err());
        assert!(parse_args(&["render", "a", "b"]).is_err());
        assert!(parse_args(&["render", "badge", "--nope"]).is_err());
    }

    #[test]
    fn render_writes_a_definition_preview_framed_to_its_bounds() {
        let dir = TempDir::new("render-definition");
        part_project(&dir);
        let out = dir.path().join("badge.svg");
        let report = run_in(
            render("badge", Some(out.clone()), Format::Svg, None),
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        assert!(report.stdout.contains("frame 30x40"), "{}", report.stdout);

        let svg = fs::read_to_string(&out).expect("reads the preview");
        assert!(svg.contains("viewBox=\"0 0 30 40\""), "{svg}");
        assert!(svg.contains("fill=\"#ff0000\""), "{svg}");
    }

    #[test]
    fn render_writes_an_element_subtree_preview() {
        let dir = TempDir::new("render-subtree");
        part_project(&dir);
        let out = dir.path().join("mark.svg");
        let report = run_in(
            render("mark", Some(out.clone()), Format::Svg, None),
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        assert!(report.stdout.contains("frame 20x10"), "{}", report.stdout);

        let svg = fs::read_to_string(&out).expect("reads the preview");
        assert!(svg.contains("viewBox=\"0 0 20 10\""), "{svg}");
        assert!(svg.contains("mark-rect"), "{svg}");
        assert!(
            !svg.contains("other"),
            "the rest of the scene is absent: {svg}"
        );
    }

    #[test]
    fn render_reports_the_resolved_output_frame() {
        let dir = TempDir::new("render-frame");
        part_project(&dir);
        let report = run_in(render("mark", None, Format::Svg, Some(100.0)), dir.path());
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        assert!(
            report.stdout.contains("frame 100x50"),
            "a single dimension scales the other: {}",
            report.stdout
        );
    }

    #[test]
    fn render_reports_an_unknown_part_by_name() {
        let dir = TempDir::new("render-unknown");
        part_project(&dir);
        let out = dir.path().join("absent.svg");
        let report = run_in(
            render("absent", Some(out.clone()), Format::Svg, None),
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(report.stderr.contains("E_PART"), "{}", report.stderr);
        assert!(report.stderr.contains("absent"), "{}", report.stderr);
        assert!(!out.exists(), "no preview is written for an unknown part");
    }

    #[test]
    fn render_reports_an_empty_part_with_the_fallback_frame() {
        let dir = TempDir::new("render-empty");
        part_project(&dir);
        let out = dir.path().join("empty.svg");
        let report = run_in(
            render("empty", Some(out.clone()), Format::Svg, None),
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        assert!(report.stdout.contains("frame 100x100"), "{}", report.stdout);
        assert!(
            report.stderr.contains("W_EMPTY_PART_FRAME"),
            "{}",
            report.stderr
        );
        assert!(out.exists(), "an empty part still yields a preview");
    }

    #[test]
    fn render_refuses_a_density_on_svg() {
        let dir = TempDir::new("render-density");
        part_project(&dir);
        let report = run_in(
            Command::Render {
                part: "badge".to_string(),
                out: None,
                format: Format::Svg,
                width: None,
                height: None,
                density: Some(2.0),
                background: None,
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
    }

    #[test]
    fn render_writes_a_png_preview() {
        let dir = TempDir::new("render-png");
        part_project(&dir);
        let out = dir.path().join("badge.png");
        let report = run_in(
            render("badge", Some(out.clone()), Format::Png, None),
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        let bytes = fs::read(&out).expect("reads the png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
    }

    #[test]
    fn render_resolves_a_definition_from_the_project_default_palette() {
        // The default scene names a different palette, so a preview that still
        // resolves `accent` to the project default proves an isolated definition
        // takes the project's default palette, not its default scene's
        // (FEAT-005, FEAT-031).
        let dir = TempDir::new("render-project-palette");
        part_project(&dir);
        write_at(
            &dir,
            "palettes/scene.json",
            r##"{"id":"scene","projectId":"project","name":"Scene","tokens":[{"name":"accent","value":"#0000ff"}]}"##,
        );
        write_at(
            &dir,
            "scenes/main.json",
            &SUBTREE_SCENE.replace(r##""paletteId": "brand""##, r##""paletteId": "scene""##),
        );

        let out = dir.path().join("badge.svg");
        let report = run_in(
            render("badge", Some(out.clone()), Format::Svg, None),
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        let svg = fs::read_to_string(&out).expect("reads the preview");
        assert!(
            svg.contains("fill=\"#ff0000\""),
            "the project default palette resolved: {svg}"
        );
    }

    #[test]
    fn render_refuses_a_definition_when_the_project_names_no_default_palette() {
        let dir = TempDir::new("render-no-default-palette");
        write_at(&dir, "vectr.project.json", r#"{"defaultSceneId":"main"}"#);
        write_at(&dir, "palettes/brand.json", PALETTE);
        write_at(&dir, "definitions/badge.json", BADGE_DEFINITION);
        write_at(&dir, "scenes/main.json", SUBTREE_SCENE);

        let out = dir.path().join("badge.svg");
        let report = run_in(
            render("badge", Some(out.clone()), Format::Svg, None),
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE, "{}", report.stderr);
        assert!(
            report.stderr.contains("default palette"),
            "the missing palette is named: {}",
            report.stderr
        );
        assert!(!out.exists(), "no preview is written");
    }

    #[test]
    fn render_refuses_a_part_identifier_that_is_not_unique() {
        // The `mark` element in the default scene and a `mark` definition share
        // one namespace, so the identifier is a duplicate rather than resolved
        // by an arbitrary ordering (FEAT-016, FEAT-031).
        let dir = TempDir::new("render-duplicate");
        part_project(&dir);
        write_at(
            &dir,
            "definitions/mark.json",
            r##"{"id":"mark","projectId":"project","name":"Mark","parameters":[],"origin":{"x":0,"y":0},"elements":[]}"##,
        );

        let out = dir.path().join("mark.svg");
        let report = run_in(
            render("mark", Some(out.clone()), Format::Svg, None),
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE, "{}", report.stderr);
        assert!(
            report.stderr.contains("E_DUPLICATE_ID"),
            "the duplicate is reported: {}",
            report.stderr
        );
        assert!(
            !out.exists(),
            "no preview is written before the duplicate fails"
        );
    }

    // -----------------------------------------------------------------------
    // Whole-scene verification (FEAT-022)
    // -----------------------------------------------------------------------

    fn inspect(out: Option<PathBuf>, width: Option<f64>) -> Command {
        Command::Inspect {
            scene: None,
            out,
            width,
            height: None,
            density: None,
            background: None,
        }
    }

    #[test]
    fn inspect_writes_a_preview_and_reports_its_size() {
        let dir = TempDir::new("inspect-preview");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("preview.png");
        let report = run_in(inspect(Some(out.clone()), None), dir.path());

        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        let bytes = fs::read(&out).expect("reads the preview");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "PNG signature");
        assert!(
            report.stdout.contains("preview 100x100"),
            "the size is reported: {}",
            report.stdout
        );
        assert!(
            report.stderr.contains("W_INSPECTION_UNAVAILABLE"),
            "the missing inspection capability is noted: {}",
            report.stderr
        );
    }

    #[test]
    fn inspect_size_is_configurable() {
        let dir = TempDir::new("inspect-size");
        project(&dir, "scene-1", VALID_SCENE);
        let report = run_in(
            inspect(Some(dir.path().join("preview.png")), Some(200.0)),
            dir.path(),
        );
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        assert!(
            report.stdout.contains("preview 200x200"),
            "a requested size is honoured: {}",
            report.stdout
        );
    }

    #[test]
    fn inspect_defaults_its_output_to_dist_scene_png() {
        let dir = TempDir::new("inspect-default-out");
        project(&dir, "scene-1", VALID_SCENE);
        let report = run_in(inspect(None, None), dir.path());
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        assert!(
            dir.path().join("dist/scene-1.png").exists(),
            "the default output is dist/<scene>.png"
        );
    }

    #[test]
    fn inspect_runs_structural_checks_before_rendering() {
        let dir = TempDir::new("inspect-structural");
        project(&dir, "scene-1", INVALID_SCENE);
        let out = dir.path().join("preview.png");
        let report = run_in(inspect(Some(out.clone()), None), dir.path());

        assert_eq!(report.code, EXIT_INVALID_SCENE);
        assert!(report.stderr.contains("E_SCHEMA"), "{}", report.stderr);
        assert!(!out.exists(), "no preview is written for an invalid scene");
    }

    #[test]
    fn inspect_reports_structural_warnings_alongside_the_preview() {
        // An unused palette token is a structural warning, not an error, so it
        // is reported without blocking the preview (FEAT-018, FEAT-022).
        let dir = TempDir::new("inspect-warning");
        write_at(&dir, "vectr.project.json", r#"{"defaultSceneId":"brand"}"#);
        write_at(
            &dir,
            "palettes/brand.json",
            r##"{"id":"brand","projectId":"project","name":"Brand","tokens":[{"name":"accent","value":"#ff0000"},{"name":"unused","value":"#00ff00"}]}"##,
        );
        write_at(&dir, "scenes/brand.json", PALETTE_SCENE);

        let out = dir.path().join("preview.png");
        let report = run_in(inspect(Some(out.clone()), None), dir.path());
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        assert!(
            report.stderr.contains("W_UNUSED_TOKEN"),
            "the structural warning is reported: {}",
            report.stderr
        );
        assert!(out.exists(), "the preview is still written");
    }

    #[test]
    fn inspect_refuses_an_invalid_background_before_any_output() {
        let dir = TempDir::new("inspect-background");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("preview.png");
        let report = run_in(
            Command::Inspect {
                scene: None,
                out: Some(out.clone()),
                width: None,
                height: None,
                density: None,
                background: Some("not-a-colour".to_string()),
            },
            dir.path(),
        );
        assert_eq!(report.code, EXIT_USAGE);
        assert!(
            report.stderr.contains("E_INVALID_COLOR"),
            "{}",
            report.stderr
        );
        assert!(!out.exists(), "no preview is written for an invalid colour");
    }

    #[test]
    fn inspect_reports_a_render_failure_before_inspection() {
        // A size beyond the rasterizer's budget fails the render, so the
        // failure is reported and no inspection follows (FEAT-022).
        let dir = TempDir::new("inspect-render-failure");
        project(&dir, "scene-1", VALID_SCENE);
        let out = dir.path().join("preview.png");
        let report = run_in(inspect(Some(out.clone()), Some(100_000.0)), dir.path());

        assert_eq!(report.code, EXIT_COMPILE);
        assert!(
            report.stderr.contains("E_RASTER_LIMIT"),
            "the render failure is reported: {}",
            report.stderr
        );
        assert!(
            !report.stderr.contains("W_INSPECTION_UNAVAILABLE"),
            "inspection is not attempted after a render failure: {}",
            report.stderr
        );
        assert!(!out.exists(), "no partial preview is written");
    }

    #[test]
    fn inspect_re_renders_a_correction() {
        let dir = TempDir::new("inspect-correction");
        write_at(&dir, "vectr.project.json", r#"{"defaultSceneId":"s"}"#);
        write_at(&dir, "palettes/brand.json", PALETTE);
        write_at(&dir, "scenes/s.json", PALETTE_SCENE);
        let out = dir.path().join("preview.png");
        run_in(inspect(Some(out.clone()), None), dir.path());
        let before = fs::read(&out).expect("reads the first preview");

        let corrected = PALETTE_SCENE.replace(
            r##""x": 0, "y": 0, "width": 10, "height": 10"##,
            r##""x": 0, "y": 0, "width": 60, "height": 60"##,
        );
        write_at(&dir, "scenes/s.json", &corrected);
        let report = run_in(inspect(Some(out.clone()), None), dir.path());
        assert_eq!(report.code, EXIT_SUCCESS, "{}", report.stderr);
        let after = fs::read(&out).expect("reads the second preview");
        assert_ne!(before, after, "the preview reflects the correction");
    }
}
