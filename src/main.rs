//! `wirebug` CLI binary.
//!
//! Thin shim over [`wirebug::render_paths`]: parse CLI args, call the
//! library, print warnings to stderr, write the SVG to disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use wirebug::error::Error;

/// Diagnostic output format for `check`, mirrored into [`wirebug::dsl::Format`].
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum OutputFormat {
    #[default]
    Human,
    Json,
}

impl From<OutputFormat> for wirebug::dsl::Format {
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
    /// Render a view to SVG.
    Render {
        /// Path to the model YAML.
        #[arg(long)]
        model: PathBuf,
        /// Path to the view YAML.
        #[arg(long)]
        view: PathBuf,
        /// Where to write the SVG.
        #[arg(long)]
        out: PathBuf,
    },
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
        Command::Render { model, view, out } => {
            render_command(&model, &view, &out)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Check {
            target,
            strict,
            format,
        } => Ok(check_command(target.as_deref(), strict, format.into())),
    }
}

fn check_command(target: Option<&Path>, strict: bool, format: wirebug::dsl::Format) -> ExitCode {
    use miette::{Diagnostic, GraphicalReportHandler, JSONReportHandler, Severity};
    use wirebug::dsl::Format;

    let report = wirebug::dsl::check_project(target);

    let errors = report
        .problems
        .iter()
        .filter(|p| !matches!(p.severity(), Some(Severity::Warning)))
        .count();
    let warnings = report.problems.len() - errors;

    match format {
        Format::Human => {
            let handler = GraphicalReportHandler::new();
            let mut out = String::new();
            for problem in &report.problems {
                let _ = handler.render_report(&mut out, problem);
            }
            eprint!("{out}");
            // One summary line on stderr.
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
            // A JSON array of diagnostics on stdout.
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

    if errors > 0 || (strict && warnings > 0) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn render_command(model_path: &Path, view_path: &Path, out_path: &Path) -> Result<()> {
    let result = wirebug::render_paths(model_path, view_path)
        .with_context(|| format!("rendering {}", view_path.display()))?;

    for warning in &result.warnings {
        eprintln!("warning: {warning}");
    }

    fs::write(out_path, &result.svg).map_err(|source| Error::Write {
        path: out_path.to_path_buf(),
        source,
    })?;

    Ok(())
}
