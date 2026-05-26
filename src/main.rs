//! `wirebug` CLI binary.
//!
//! Two subcommands over the `.wb` DSL pipeline: `check` reports problems,
//! `render` writes one SVG per view in the design. Both discover the
//! project by walking up to `main.wb` when given no target.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use miette::{Diagnostic, GraphicalReportHandler, JSONReportHandler, Severity};

use wirebug::dsl::{self, CheckReport, Format};
use wirebug::error::Error;

/// Diagnostic output format for `check`, mirrored into [`Format`].
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum OutputFormat {
    #[default]
    Human,
    Json,
}

impl From<OutputFormat> for Format {
    fn from(f: OutputFormat) -> Self {
        match f {
            OutputFormat::Human => Self::Human,
            OutputFormat::Json => Self::Json,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "wirebug",
    version,
    about = "Text-defined electrical schematics"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Parse and validate a wirebug project, reporting any problems.
    Check {
        /// A `.wb` file or project directory. Defaults to the project
        /// containing the current directory (found by walking up to
        /// `main.wb`).
        target: Option<PathBuf>,
        /// Treat warnings as errors.
        #[arg(long)]
        strict: bool,
        /// Diagnostic output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Human)]
        format: OutputFormat,
    },
    /// Render every view in a project to SVG.
    Render {
        /// A `.wb` file or project directory. Defaults to the project
        /// containing the current directory (found by walking up to
        /// `main.wb`).
        target: Option<PathBuf>,
        /// Directory to write the per-view SVGs into (created if absent).
        #[arg(long)]
        out: PathBuf,
        /// Treat warnings as errors.
        #[arg(long)]
        strict: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Check {
            target,
            strict,
            format,
        } => Ok(check_command(target.as_deref(), strict, format.into())),
        Command::Render {
            target,
            out,
            strict,
        } => render_command(target.as_deref(), &out, strict),
    }
}

/// Count a report's errors and warnings (warnings are the only
/// non-error severity the pipeline emits).
fn tally(report: &CheckReport) -> (usize, usize) {
    let errors = report
        .problems
        .iter()
        .filter(|p| !matches!(p.severity(), Some(Severity::Warning)))
        .count();
    (errors, report.problems.len() - errors)
}

fn check_command(target: Option<&Path>, strict: bool, format: Format) -> ExitCode {
    let report = dsl::check_project(target);
    let (errors, warnings) = tally(&report);

    match format {
        Format::Human => {
            eprint!("{}", render_problems_human(&report));
            match &report.design {
                Some(design) if report.problems.is_empty() => eprintln!(
                    "ok — {} instances, {} views",
                    design.instances.len(),
                    design.views.len()
                ),
                _ if report.problems.is_empty() => eprintln!("ok"),
                _ => eprintln!("{errors} error(s), {warnings} warning(s)"),
            }
        }
        Format::Json => {
            let handler = JSONReportHandler::new();
            let items: Vec<String> = report
                .problems
                .iter()
                .map(|problem| {
                    let mut s = String::new();
                    let _ = handler.render_report(&mut s, problem);
                    s
                })
                .collect();
            println!("[{}]", items.join(","));
        }
    }

    exit_code(errors, warnings, strict)
}

fn render_command(target: Option<&Path>, out_dir: &Path, strict: bool) -> Result<ExitCode> {
    let report = dsl::check_project(target);
    let (errors, warnings) = tally(&report);

    // Surface any check problems first; an erroring project (or, under
    // --strict, a warning) is not rendered.
    eprint!("{}", render_problems_human(&report));
    if errors > 0 || (strict && warnings > 0) {
        eprintln!("{errors} error(s), {warnings} warning(s) — not rendering");
        return Ok(ExitCode::FAILURE);
    }

    let Some(design) = &report.design else {
        eprintln!("no design to render");
        return Ok(ExitCode::FAILURE);
    };

    let views = wirebug::render_views(design).context("rendering views")?;

    fs::create_dir_all(out_dir).map_err(|source| Error::Write {
        path: out_dir.to_path_buf(),
        source,
    })?;
    for view in &views {
        let path = out_dir.join(&view.filename);
        fs::write(&path, &view.svg).map_err(|source| Error::Write {
            path: path.clone(),
            source,
        })?;
    }

    eprintln!("rendered {} view(s) to {}", views.len(), out_dir.display());
    Ok(ExitCode::SUCCESS)
}

/// Render a report's problems with miette's graphical handler.
fn render_problems_human(report: &CheckReport) -> String {
    let handler = GraphicalReportHandler::new();
    let mut out = String::new();
    for problem in &report.problems {
        let _ = handler.render_report(&mut out, problem);
    }
    out
}

fn exit_code(errors: usize, warnings: usize, strict: bool) -> ExitCode {
    if errors > 0 || (strict && warnings > 0) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
