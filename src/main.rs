//! `wirebug` CLI binary.
//!
//! Subcommands over the `.wb` DSL pipeline: `check` reports problems,
//! `render` writes one SVG (or PNG) per view in the design — or, with
//! `--pdf`, a single multi-page PDF on the `--paper` size — `export`
//! writes a build table as CSV or JSON, and `serve` runs a live-reloading
//! dev server. Each discovers the project by walking up to `wirebug.toml`
//! when given no target. A view that fails to render is reported by title
//! and left out; the others are still written, and `render` exits
//! non-zero. A PDF page whose text prints too small to read is written and
//! warned of, without changing the exit status.

mod cli;

use std::io::{ErrorKind, Write};
use std::net::IpAddr;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use miette::{GraphicalReportHandler, JSONReportHandler};
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command, ExportArgs, ManifestCommand};
use wirebug::dsl::diagnostics::Problem;
use wirebug::dsl::ir::Design;
use wirebug::dsl::{self, CheckReport, Format};
use wirebug::export::{Export, Table};
use wirebug::render::output::{Output, ViewFormat};
use wirebug::render::pdf::Paper;
use wirebug::render::png::PNG_SCALE;

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
        } => check_command(target.as_deref(), strict, format.into()),
        Command::Render {
            target,
            out,
            strict,
            png,
            pdf,
            paper,
            embed,
        } => render_command(
            target.as_deref(),
            &out,
            strict,
            png,
            pdf.then_some(paper),
            embed,
        ),
        Command::Export { table } => {
            let (table, args) = table.into_parts();
            export_command(table, &args)
        }
        Command::Manifest { what } => Ok(manifest_command(what)),
        Command::Lsp => lsp_command(),
        Command::Serve { target, port, host } => serve_command(target.as_deref(), host, port),
    }
}

/// Run the language server. Stdout is the protocol channel, so tracing
/// must write to stderr — the only difference from `serve`'s setup.
fn lsp_command() -> Result<ExitCode> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    wirebug::lsp::run()?;
    Ok(ExitCode::SUCCESS)
}

/// Run the live-reloading dev server. `serve` is the only async command, so
/// it spins up a Tokio runtime locally rather than making all of `main`
/// async; `check` and `render` stay synchronous.
fn serve_command(target: Option<&Path>, host: IpAddr, port: u16) -> Result<ExitCode> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let runtime = tokio::runtime::Runtime::new().context("starting the async runtime")?;
    runtime.block_on(wirebug::serve::serve(target, host, port))?;
    Ok(ExitCode::SUCCESS)
}

fn check_command(target: Option<&Path>, strict: bool, format: Format) -> Result<ExitCode> {
    let report = wirebug::check_project(target);
    let counts = report.counts();

    match format {
        Format::Human => {
            eprint!("{}", render_problems(&report.problems));
            match &report.design {
                Some(design) if report.problems.is_empty() => eprintln!(
                    "ok — {} instances, {} views",
                    design.instances.len(),
                    design.views.len()
                ),
                _ if report.problems.is_empty() => eprintln!("ok"),
                _ => eprintln!("{} error(s), {} warning(s)", counts.errors, counts.warnings),
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
            print_stdout(&format!("[{}]\n", items.join(",")))
                .context("writing the diagnostics to stdout")?;
        }
    }

    Ok(exit_code(&report, strict))
}

fn render_command(
    target: Option<&Path>,
    out_dir: &Path,
    strict: bool,
    png: bool,
    pdf: Option<Paper>,
    embed: bool,
) -> Result<ExitCode> {
    let report = wirebug::check_project(target);
    let Some(design) = checked_design(&report, strict, "render") else {
        return Ok(ExitCode::FAILURE);
    };

    let output = if let Some(paper) = pdf {
        Output::Pdf { paper }
    } else {
        let format = if png {
            ViewFormat::Png { scale: PNG_SCALE }
        } else {
            ViewFormat::Svg
        };
        Output::Views { format, embed }
    };
    let written = output.write(design, out_dir)?;
    for failure in &written.failures {
        eprintln!("error: {failure}");
    }
    // Text too small to read on paper is the paper's fault, not the
    // project's: warned of, never fatal, so `--strict` leaves it be.
    for small in &written.small_text {
        eprintln!("warning: {small}");
    }
    eprintln!("{written}");
    // A render that left out a failed view fails, so scripts and CI notice.
    Ok(if written.failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Write one build table of the project to stdout, or to `--out` with a
/// summary line on stderr. Diagnostics go to stderr, so stdout holds the
/// table alone.
fn export_command(table: Table, args: &ExportArgs) -> Result<ExitCode> {
    let report = wirebug::check_project(args.target.as_deref());
    let Some(design) = checked_design(&report, args.strict, "export") else {
        return Ok(ExitCode::FAILURE);
    };
    let export = Export::new(design, table, args.format.into())?;
    match &args.out {
        Some(path) => eprintln!("{}", export.write(path)?),
        None => print_stdout(export.text()).context("writing the table to stdout")?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Write `text` to stdout. A reader that stops early (`| head`) closes the
/// pipe while it is still being written; that is the reader's choice, not
/// a failure, so a broken pipe counts as written.
fn print_stdout(text: &str) -> std::io::Result<()> {
    match std::io::stdout().lock().write_all(text.as_bytes()) {
        Err(error) if error.kind() == ErrorKind::BrokenPipe => Ok(()),
        written => written,
    }
}

/// Print the check's problems to stderr, then the design to `action`
/// (`render`, `export`) — or `None`, saying why, when the project has
/// errors (or, under `strict`, warnings) or no design.
fn checked_design<'r>(report: &'r CheckReport, strict: bool, action: &str) -> Option<&'r Design> {
    let counts = report.counts();
    eprint!("{}", render_problems(&report.problems));
    if report.has_blocking_problems(strict) {
        eprintln!(
            "{} error(s), {} warning(s) — not {action}ing",
            counts.errors, counts.warnings
        );
        return None;
    }
    if report.design.is_none() {
        eprintln!("no design to {action}");
    }
    report.design.as_ref()
}

/// Print the project's manifest version (`v<version>`) to stdout, or the
/// discovery/parse problems to stderr. Skips the check pipeline, so it
/// reports a version even when the `.wb` sources don't compile.
fn manifest_command(what: ManifestCommand) -> ExitCode {
    match what {
        ManifestCommand::Version { target } => {
            let (manifest, problems) = dsl::load_manifest(target.as_deref());
            match manifest {
                Some(manifest) => {
                    println!("v{}", manifest.version);
                    ExitCode::SUCCESS
                }
                None => {
                    eprint!("{}", render_problems(&problems));
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// Render a list of problems with miette's graphical handler.
fn render_problems(problems: &[Problem]) -> String {
    let handler = GraphicalReportHandler::new();
    let mut out = String::new();
    for problem in problems {
        let _ = handler.render_report(&mut out, problem);
    }
    out
}

fn exit_code(report: &CheckReport, strict: bool) -> ExitCode {
    if report.has_blocking_problems(strict) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
