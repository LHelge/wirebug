//! `wirebug` CLI binary.
//!
//! `init` scaffolds a new project and `skill` prints or installs the agent
//! skill for the language. The other subcommands run over the
//! `.wb` DSL pipeline: `check` reports problems,
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
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use miette::{GraphicalReportHandler, JSONReportHandler};
use tracing_subscriber::EnvFilter;

use cli::{Cli, Command, ExportArgs, Harness, ManifestCommand, PaperArg};
use wirebug::dsl::diagnostics::Problem;
use wirebug::dsl::format::ProjectFormat;
use wirebug::dsl::ir::Design;
use wirebug::dsl::project;
use wirebug::dsl::{self, CheckReport, Format};
use wirebug::export::{Export, Table};
use wirebug::init::Scaffold;
use wirebug::render::output::{Output, ViewFormat};
use wirebug::render::pdf::{BestPaper, Paper};
use wirebug::render::png::PNG_SCALE;
use wirebug::skill::Skill;

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
        Command::Init { dir } => init_command(dir.as_deref().unwrap_or(Path::new("."))),
        Command::Check {
            target,
            strict,
            format,
        } => check_command(target.as_deref(), strict, format.into()),
        Command::Fmt { target, check } => fmt_command(target.as_deref(), check),
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
            out.as_deref(),
            strict,
            png,
            pdf.then_some(paper.into()),
            embed,
        ),
        Command::Export { table } => {
            let (table, args) = table.into_parts();
            export_command(table, &args)
        }
        Command::Manifest { what } => Ok(manifest_command(what)),
        Command::Lsp => lsp_command(),
        Command::Skill { install } => skill_command(install),
        Command::Serve { target, port, host } => serve_command(target.as_deref(), host, port),
    }
}

/// Scaffold a project in `dir`, then say what was written and what to run
/// next.
fn init_command(dir: &Path) -> Result<ExitCode> {
    eprintln!("{}", Scaffold::new(dir).write()?);
    Ok(ExitCode::SUCCESS)
}

/// Print the skill to stdout or, given a harness, install it under the
/// current directory and name the file written on stderr.
fn skill_command(install: Option<Harness>) -> Result<ExitCode> {
    let skill = Skill::current();
    match install {
        None => print_stdout(skill.as_str()).context("writing the skill to stdout")?,
        Some(harness) => {
            let path = harness.install_path();
            skill.install(path)?;
            eprintln!("wrote {}", path.display());
        }
    }
    Ok(ExitCode::SUCCESS)
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

/// Format the project's files in place (or, with `check`, only say which
/// would change): each such file on stdout, a summary on stderr. Fails
/// when a file could not be formatted, or with `check` when one would
/// change.
fn fmt_command(target: Option<&Path>, check: bool) -> Result<ExitCode> {
    let run = ProjectFormat::load(target);
    eprint!("{}", render_problems(&run.problems));
    let cwd = std::env::current_dir()
        .and_then(|dir| dir.canonicalize())
        .ok();
    let mut listed = String::new();
    let mut changed = 0;
    for file in run.changed() {
        if !check {
            file.write()
                .with_context(|| format!("writing {}", file.path.display()))?;
        }
        let shown = cwd
            .as_deref()
            .and_then(|cwd| file.path.strip_prefix(cwd).ok())
            .unwrap_or(&file.path);
        listed.push_str(&format!("{}\n", shown.display()));
        changed += 1;
    }
    print_stdout(&listed).context("writing the file list to stdout")?;
    let total = run.files.len();
    let errors = run.problems.len();
    if total == 0 && errors > 0 {
        // No project, or none of its files parse: there is no file count
        // to give.
        eprintln!("{errors} error(s) — nothing formatted");
    } else {
        if check {
            eprintln!("{changed} of {total} file(s) would be reformatted");
        } else {
            eprintln!("{changed} of {total} file(s) reformatted");
        }
        if errors > 0 {
            eprintln!("{errors} error(s) — the files they are in were left as they are");
        }
    }
    Ok(if run.problems.is_empty() && !(check && changed > 0) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn render_command(
    target: Option<&Path>,
    out_dir: Option<&Path>,
    strict: bool,
    png: bool,
    pdf: Option<Paper>,
    embed: bool,
) -> Result<ExitCode> {
    let report = wirebug::check_project(target);
    let Some(design) = checked_design(&report, strict, "render") else {
        return Ok(ExitCode::FAILURE);
    };
    let out_dir = match out_dir {
        Some(dir) => dir.to_path_buf(),
        None => default_out_dir(target)?,
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
    let written = output.write(design, &out_dir)?;
    for failure in &written.failures {
        eprintln!("error: {failure}");
    }
    // Text too small to read on paper is the paper's fault, not the
    // project's: warned of, never fatal, so `--strict` leaves it be.
    for small in &written.small_text {
        match small.best {
            BestPaper::Legible(paper) => {
                eprintln!("warning: {small} (--paper {})", PaperArg::from(paper));
            }
            BestPaper::Closest { .. } => eprintln!("warning: {small}"),
        }
    }
    eprintln!("{written}");
    // A render that left out a failed view fails, so scripts and CI notice.
    Ok(if written.failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// `out` in the project root, the directory of the entry file that
/// discovery finds (beside `wirebug.toml`), so `render` run from anywhere in
/// a project writes to the same place.
fn default_out_dir(target: Option<&Path>) -> Result<PathBuf> {
    let entry = project::discover(target)
        .map_err(|problem| anyhow::anyhow!("{problem}"))
        .context("locating the project")?;
    let root = entry
        .parent()
        .context("project root has no parent directory")?;
    Ok(root.join("out"))
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
