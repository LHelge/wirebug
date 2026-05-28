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
use miette::{GraphicalReportHandler, JSONReportHandler};
use tracing_subscriber::EnvFilter;

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

#[derive(Debug, Clone, Copy)]
enum RenderFormat {
    Svg,
    Png { scale: f32 },
}

impl RenderFormat {
    fn from_png_flag(png: bool) -> Self {
        if png {
            Self::Png { scale: 2.0 }
        } else {
            Self::Svg
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png { .. } => "png",
        }
    }

    fn write_views(
        self,
        views: Vec<wirebug::render::RenderedView>,
        out_dir: &Path,
    ) -> Result<Vec<wirebug::render::RenderedView>> {
        match self {
            Self::Svg => {
                for view in &views {
                    write_file(out_dir, &view.filename, view.svg.as_bytes())?;
                }
                Ok(views)
            }
            Self::Png { scale } => {
                let mut index_views = Vec::with_capacity(views.len());
                for view in views {
                    let filename = Path::new(&view.filename)
                        .with_extension("png")
                        .to_string_lossy()
                        .into_owned();
                    let bytes = wirebug::render::png::svg_to_png(&view.svg, scale)
                        .context("rasterising view to PNG")?;
                    write_file(out_dir, &filename, &bytes)?;
                    index_views.push(wirebug::render::RenderedView {
                        title: view.title,
                        filename,
                        kind: view.kind,
                        svg: String::new(),
                    });
                }
                Ok(index_views)
            }
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
        /// Rasterise each view to PNG instead of writing SVG. PNGs are at
        /// 2× the SVG's intrinsic size; the HTML index references the
        /// `.png` files.
        #[arg(long)]
        png: bool,
        /// Emit SVGs suitable for embedding into another document or
        /// site: the built-in `<style>` is dropped (the host owns the
        /// look), the project-identity stamp is suppressed, and the root
        /// `<svg>` is class-tagged `wirebug wirebug-{kind}` so a host
        /// stylesheet can scope rules under `.wirebug`. Writes a
        /// `manifest.json` sidecar in place of the HTML index.
        #[arg(long)]
        embed: bool,
    },
    /// Serve a project with live reload, re-rendering on every change.
    Serve {
        /// A `.wb` file or project directory. Defaults to the project
        /// containing the current directory (found by walking up to
        /// `main.wb`).
        target: Option<PathBuf>,
        /// Port to listen on.
        #[arg(short, long, default_value_t = 3000)]
        port: u16,
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
            png,
            embed,
        } => render_command(target.as_deref(), &out, strict, png, embed),
        Command::Serve { target, port } => serve_command(target.as_deref(), port),
    }
}

/// Run the live-reloading dev server. `serve` is the only async command, so
/// it spins up a Tokio runtime locally rather than making all of `main`
/// async; `check` and `render` stay synchronous.
fn serve_command(target: Option<&Path>, port: u16) -> Result<ExitCode> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let runtime = tokio::runtime::Runtime::new().context("starting the async runtime")?;
    runtime.block_on(wirebug::serve::serve(target, port))?;
    Ok(ExitCode::SUCCESS)
}

fn check_command(target: Option<&Path>, strict: bool, format: Format) -> ExitCode {
    let report = dsl::check_project(target);
    let counts = report.counts();

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
    embed: bool,
) -> Result<ExitCode> {
    let report = dsl::check_project(target);
    let counts = report.counts();

    // Surface any check problems first; an erroring project (or, under
    // --strict, a warning) is not rendered.
    eprint!("{}", render_problems_human(&report));
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

    let views = wirebug::render_views(design, embed).context("rendering views")?;

    fs::create_dir_all(out_dir).map_err(|source| Error::Write {
        path: out_dir.to_path_buf(),
        source,
    })?;

    let render_format = RenderFormat::from_png_flag(png);
    let index_views = render_format.write_views(views, out_dir)?;

    // Embed-mode writes a structured manifest sidecar for a downstream
    // host (e.g. a static site generator). The HTML index is for
    // wirebug's own browsing UI and would only get in the host's way.
    let sidecar_path = if embed {
        let manifest = wirebug::render::embed_manifest(&index_views, design.manifest.as_ref());
        let json = serde_json::to_string_pretty(&manifest).context("serializing embed manifest")?;
        let path = out_dir.join(wirebug::render::EMBED_MANIFEST_FILENAME);
        write_file(
            out_dir,
            wirebug::render::EMBED_MANIFEST_FILENAME,
            json.as_bytes(),
        )?;
        path
    } else {
        let index = wirebug::index_html(&index_views, design.manifest.as_ref(), false)
            .context("rendering HTML index")?;
        let path = out_dir.join(wirebug::render::INDEX_FILENAME);
        write_file(out_dir, wirebug::render::INDEX_FILENAME, index.as_bytes())?;
        path
    };

    eprintln!(
        "rendered {} view(s) as {} to {} ({})",
        index_views.len(),
        render_format.label(),
        out_dir.display(),
        sidecar_path.display(),
    );
    Ok(ExitCode::SUCCESS)
}

fn write_file(out_dir: &Path, filename: &str, contents: &[u8]) -> Result<()> {
    let path = out_dir.join(filename);
    fs::write(&path, contents).map_err(|source| Error::Write { path, source })?;
    Ok(())
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

fn exit_code(report: &CheckReport, strict: bool) -> ExitCode {
    if report.has_blocking_problems(strict) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
