//! `wirebug` CLI binary.
//!
//! Three subcommands over the `.wb` DSL pipeline: `check` reports problems,
//! `render` writes one SVG (or PNG) per view in the design — or, with
//! `--pdf`, a single multi-page A4 PDF — and `serve` runs a live-reloading
//! dev server. Each discovers the project by walking up to `wirebug.toml`
//! when given no target. A view that fails to render is reported by title
//! and left out; the others are still written, and `render` exits non-zero.

mod cli;

use std::fs;
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
use wirebug::error::{Error, ViewError};
use wirebug::render::{RenderedView, ViewOutcome};

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

    /// Write every rendered view to `out_dir` in this format, returning
    /// the views as the index should list them (file names in this
    /// format). Failed views pass through, and a view that fails to
    /// rasterise joins them; only a disk error stops the render.
    fn write_views(self, views: Vec<ViewOutcome>, out_dir: &Path) -> Result<Vec<ViewOutcome>> {
        match self {
            Self::Svg => {
                for view in views.iter().flatten() {
                    write_file(out_dir, &view.filename, view.svg.as_bytes())?;
                }
                Ok(views)
            }
            Self::Png { scale } => {
                let rasterizer = wirebug::render::png::PngRasterizer::new();
                views
                    .into_iter()
                    .map(|view| {
                        let view = match view {
                            Ok(view) => view,
                            Err(failure) => return Ok(Err(failure)),
                        };
                        let bytes = match rasterizer.to_png(&view.svg, scale) {
                            Ok(bytes) => bytes,
                            Err(cause) => return Ok(Err(view.failed(cause))),
                        };
                        let filename = Path::new(&view.filename)
                            .with_extension("png")
                            .to_string_lossy()
                            .into_owned();
                        write_file(out_dir, &filename, &bytes)?;
                        Ok(Ok(RenderedView {
                            title: view.title,
                            filename,
                            kind: view.kind,
                            svg: String::new(),
                        }))
                    })
                    .collect()
            }
        }
    }
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

    // PDF page furniture carries the view title and project identity at
    // a fixed size, so its SVGs render plain for a homogeneous look.
    let mode = if pdf {
        wirebug::render::SvgMode::Plain
    } else if embed {
        wirebug::render::SvgMode::Embed
    } else {
        wirebug::render::SvgMode::Standalone
    };
    let views = wirebug::render_views(design, mode);

    fs::create_dir_all(out_dir).map_err(|source| Error::Write {
        path: out_dir.to_path_buf(),
        source,
    })?;

    // PDF is one artifact for the whole design, not per-view files plus a
    // sidecar, so it exits before the per-view format machinery. Its pages
    // are the views that rendered; with views but none rendered, no PDF is
    // written at all.
    if pdf {
        let total = views.len();
        let (rendered, failed): (Vec<_>, Vec<_>) = views.into_iter().partition(Result::is_ok);
        let rendered: Vec<RenderedView> = rendered.into_iter().flatten().collect();
        let mut failures: Vec<ViewError> = failed.into_iter().filter_map(Result::err).collect();
        let export =
            wirebug::render::pdf::PdfExporter::new().to_pdf(&rendered, design.manifest.as_ref());
        failures.extend(export.failures);
        report_failures(&failures);
        let filename = wirebug::render::pdf_filename(design.manifest.as_ref());
        if export.pages == 0 && !failures.is_empty() {
            eprintln!("no view rendered — {filename} not written");
        } else {
            write_file(out_dir, &filename, &export.bytes)?;
            eprintln!(
                "rendered {} as pdf to {} ({})",
                rendered_count(export.pages, total),
                out_dir.display(),
                out_dir.join(&filename).display(),
            );
        }
        return Ok(exit_code_for(&failures));
    }

    let total = views.len();
    let render_format = RenderFormat::from_png_flag(png);
    let index_views = render_format.write_views(views, out_dir)?;

    // Embed-mode writes a structured manifest sidecar for a downstream
    // host (e.g. a static site generator). The HTML index is for
    // wirebug's own browsing UI and would only get in the host's way. The
    // manifest lists only the views that were written; the HTML index
    // shows a failed view's error in its place.
    let sidecar_path = if embed {
        let manifest =
            wirebug::render::embed_manifest(index_views.iter().flatten(), design.manifest.as_ref());
        let json = serde_json::to_string_pretty(&manifest).context("serializing embed manifest")?;
        let path = out_dir.join(wirebug::render::EMBED_MANIFEST_FILENAME);
        write_file(
            out_dir,
            wirebug::render::EMBED_MANIFEST_FILENAME,
            json.as_bytes(),
        )?;
        write_file(
            out_dir,
            wirebug::render::EMBED_STYLESHEET_FILENAME,
            wirebug::render::embed_stylesheet().as_bytes(),
        )?;
        path
    } else {
        let index = wirebug::index_html(&index_views, design.manifest.as_ref(), false)
            .context("rendering HTML index")?;
        let path = out_dir.join(wirebug::render::INDEX_FILENAME);
        write_file(out_dir, wirebug::render::INDEX_FILENAME, index.as_bytes())?;
        path
    };

    let failures: Vec<ViewError> = index_views.into_iter().filter_map(Result::err).collect();
    report_failures(&failures);
    eprintln!(
        "rendered {} as {} to {} ({})",
        rendered_count(total - failures.len(), total),
        render_format.label(),
        out_dir.display(),
        sidecar_path.display(),
    );
    Ok(exit_code_for(&failures))
}

/// Print each view that failed to render, naming it.
fn report_failures(failures: &[ViewError]) {
    for failure in failures {
        eprintln!("error: {failure}");
    }
}

/// `3 view(s)` when every view rendered, `2 of 3 view(s)` when some failed.
fn rendered_count(rendered: usize, total: usize) -> String {
    if rendered == total {
        format!("{rendered} view(s)")
    } else {
        format!("{rendered} of {total} view(s)")
    }
}

/// A render that left out a failed view fails, so scripts and CI notice.
fn exit_code_for(failures: &[ViewError]) -> ExitCode {
    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn write_file(out_dir: &Path, filename: &str, contents: &[u8]) -> Result<()> {
    let path = out_dir.join(filename);
    fs::write(&path, contents).map_err(|source| Error::Write { path, source })?;
    Ok(())
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
