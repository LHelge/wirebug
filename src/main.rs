//! `wirebug` CLI binary.
//!
//! Three subcommands over the `.wb` DSL pipeline: `check` reports problems,
//! `render` writes one SVG (or PNG) per view in the design — or, with
//! `--pdf`, a single multi-page A4 PDF — and `serve` runs a live-reloading
//! dev server. Each discovers the project by walking up to `wirebug.toml`
//! when given no target. A view that fails to render is reported by title
//! and left out; the others are still written, and `render` exits non-zero.

mod cli;

use std::net::IpAddr;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use miette::{GraphicalReportHandler, JSONReportHandler};
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command, ManifestCommand};
use wirebug::dsl::diagnostics::Problem;
use wirebug::dsl::{self, CheckReport, Format};
use wirebug::render::output::{Output, ViewFormat};

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
            png,
            pdf,
            embed,
        } => render_command(target.as_deref(), &out, strict, png, pdf, embed),
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

fn check_command(target: Option<&Path>, strict: bool, format: Format) -> ExitCode {
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
            println!("[{}]", items.join(","));
        }
    }

    exit_code(&report, strict)
}

fn render_command(
    target: Option<&Path>,
    out_dir: &Path,
    strict: bool,
    png: bool,
    pdf: bool,
    embed: bool,
) -> Result<ExitCode> {
    let report = wirebug::check_project(target);
    let counts = report.counts();

    // Surface any check problems first; an erroring project (or, under
    // --strict, a warning) is not rendered.
    eprint!("{}", render_problems(&report.problems));
    if report.has_blocking_problems(strict) {
        eprintln!(
            "{} error(s), {} warning(s) — not rendering",
            counts.errors, counts.warnings
        );
        return Ok(ExitCode::FAILURE);
    }

    let Some(design) = &report.design else {
        eprintln!("no design to render");
        return Ok(ExitCode::FAILURE);
    };

    let output = if pdf {
        Output::Pdf
    } else {
        let format = if png {
            ViewFormat::Png { scale: 2.0 }
        } else {
            ViewFormat::Svg
        };
        Output::Views { format, embed }
    };
    let written = output.write(design, out_dir)?;
    for failure in &written.failures {
        eprintln!("error: {failure}");
    }
    eprintln!("{written}");
    // A render that left out a failed view fails, so scripts and CI notice.
    Ok(if written.failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
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
