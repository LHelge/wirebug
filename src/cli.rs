//! Command-line interface definition: the clap argument tree for the
//! `wirebug` binary. This module is parsing only — it owns the `Cli`,
//! `Command`, `OutputFormat`, `ExportFormat` and `PaperArg` types and the
//! mapping from the CLI's flags into the library's [`Format`],
//! [`export::Format`] and [`Paper`] (and from a [`Paper`] back to the
//! `--paper` value naming it). The command implementations live in
//! `main.rs`.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use wirebug::dsl::Format;
use wirebug::export::{self, Table};
use wirebug::render::pdf::Paper;

/// Paper size for `render --pdf`, mirrored into [`Paper`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum PaperArg {
    /// ISO A4, 210 × 297 mm.
    #[default]
    A4,
    /// ISO A3, 297 × 420 mm.
    A3,
    /// US Letter, 8.5 × 11 in.
    Letter,
    /// US Tabloid (ledger), 11 × 17 in.
    Tabloid,
}

impl From<PaperArg> for Paper {
    fn from(paper: PaperArg) -> Self {
        match paper {
            PaperArg::A4 => Self::A4,
            PaperArg::A3 => Self::A3,
            PaperArg::Letter => Self::Letter,
            PaperArg::Tabloid => Self::Tabloid,
        }
    }
}

/// The `--paper` value that asks for a paper, so a warning can say how.
impl From<Paper> for PaperArg {
    fn from(paper: Paper) -> Self {
        match paper {
            Paper::A4 => Self::A4,
            Paper::A3 => Self::A3,
            Paper::Letter => Self::Letter,
            Paper::Tabloid => Self::Tabloid,
        }
    }
}

/// The value as `--paper` takes it (`a4`, `tabloid`).
impl fmt::Display for PaperArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = self.to_possible_value().ok_or(fmt::Error)?;
        f.write_str(value.get_name())
    }
}

/// Diagnostic output format for `check`, mirrored into [`Format`].
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum OutputFormat {
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

/// Table format for `export`, mirrored into [`export::Format`].
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum ExportFormat {
    #[default]
    Csv,
    Json,
}

impl From<ExportFormat> for export::Format {
    fn from(f: ExportFormat) -> Self {
        match f {
            ExportFormat::Csv => Self::Csv,
            ExportFormat::Json => Self::Json,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "wirebug",
    version,
    about = "Text-defined electrical schematics"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Parse and validate a wirebug project, reporting any problems.
    Check {
        /// A project manifest, project directory, or `.wb` file. Defaults
        /// to the project containing the current directory (found by
        /// walking up to `wirebug.toml`).
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
        /// A project manifest, project directory, or `.wb` file. Defaults
        /// to the project containing the current directory (found by
        /// walking up to `wirebug.toml`).
        target: Option<PathBuf>,
        /// Directory to write the rendered views into (created if absent).
        /// Defaults to `out` in the project root, the directory holding
        /// `wirebug.toml`, wherever the command runs from; a relative path
        /// given here is taken from the current directory.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Treat warnings as errors.
        #[arg(long)]
        strict: bool,
        /// Rasterise each view to PNG instead of writing SVG. PNGs are at
        /// 2× the SVG's intrinsic size; the HTML index references the
        /// `.png` files.
        #[arg(long)]
        png: bool,
        /// Export every view into a single PDF (one page per view, on the
        /// `--paper` size, portrait or landscape chosen per view by aspect
        /// ratio) named after the project (`<name>.pdf`), instead of
        /// per-view SVGs and the HTML index.
        #[arg(long, conflicts_with_all = ["png", "embed"])]
        pdf: bool,
        /// The paper size of the `--pdf` pages. Each view is scaled to fit
        /// its page; a view whose smallest text would print below 5 pt is
        /// named in a warning with a paper size that would print it
        /// legibly. The warning neither stops the PDF nor fails the run,
        /// even under `--strict`, which concerns the project's own
        /// warnings.
        #[arg(long, value_enum, default_value_t = PaperArg::A4, requires = "pdf")]
        paper: PaperArg,
        /// Emit SVGs suitable for embedding into another document or
        /// site: the built-in `<style>` is dropped (the host owns the
        /// look), the project-identity stamp is suppressed, and the root
        /// `<svg>` is class-tagged `wirebug wirebug-{kind}` so a host
        /// stylesheet can scope rules under `.wirebug`. Writes a
        /// `manifest.json` sidecar in place of the HTML index.
        #[arg(long)]
        embed: bool,
    },
    /// Write a build table of a project — the wire list, the connector pin
    /// tables or the bill of materials — as CSV or JSON.
    Export {
        #[command(subcommand)]
        table: ExportCommand,
    },
    /// Inspect the project manifest (wirebug.toml).
    Manifest {
        #[command(subcommand)]
        what: ManifestCommand,
    },
    /// Run the language server over stdio (for editor integration).
    Lsp,
    /// Serve a project with live reload, re-rendering on every change.
    Serve {
        /// A project manifest, project directory, or `.wb` file. Defaults
        /// to the project containing the current directory (found by
        /// walking up to `wirebug.toml`).
        target: Option<PathBuf>,
        /// Port to listen on.
        #[arg(short, long, default_value_t = 3000)]
        port: u16,
        /// Address to bind. Defaults to localhost; pass `0.0.0.0` to expose
        /// the server on all interfaces so other devices on the network can
        /// reach it.
        #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
        host: IpAddr,
    },
}

/// Subcommands of `wirebug export`: one per build table.
#[derive(Debug, Subcommand)]
pub enum ExportCommand {
    /// The wire list (cut list): one row per conductor — each segment of
    /// a multi-endpoint wire — and one per cable screen.
    Wires(ExportArgs),
    /// The connector pin tables (crimp sheet): one row per conductor or
    /// drain in each cavity, and one per empty cavity or port without a
    /// pin.
    Connectors(ExportArgs),
    /// The bill of materials: one row per thing to buy — housings by part,
    /// cables by type, loose wire by colour and gauge, components and
    /// splices by part — with its quantity per placement.
    Bom(ExportArgs),
}

impl ExportCommand {
    /// The table this subcommand writes, and its arguments.
    pub fn into_parts(self) -> (Table, ExportArgs) {
        match self {
            Self::Wires(args) => (Table::Wires, args),
            Self::Connectors(args) => (Table::Connectors, args),
            Self::Bom(args) => (Table::Bom, args),
        }
    }
}

/// What every `export` subcommand takes.
#[derive(Debug, Args)]
pub struct ExportArgs {
    /// A project manifest, project directory, or `.wb` file. Defaults
    /// to the project containing the current directory (found by
    /// walking up to `wirebug.toml`).
    pub target: Option<PathBuf>,
    /// Table format.
    #[arg(long, value_enum, default_value_t = ExportFormat::Csv)]
    pub format: ExportFormat,
    /// File to write the table to (its directory created if absent),
    /// instead of stdout.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// Treat warnings as errors.
    #[arg(long)]
    pub strict: bool,
}

/// Subcommands of `wirebug manifest`.
#[derive(Debug, Subcommand)]
pub enum ManifestCommand {
    /// Print the project version, `v`-prefixed (e.g. `v0.1.0`), from
    /// wirebug.toml — for CI to check a git tag against.
    Version {
        /// A project manifest, project directory, or `.wb` file. Defaults
        /// to the project containing the current directory (found by
        /// walking up to `wirebug.toml`).
        target: Option<PathBuf>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_paper_maps_to_the_paper_value_that_asks_for_it() {
        for &arg in PaperArg::value_variants() {
            assert_eq!(PaperArg::from(Paper::from(arg)), arg);
            assert_eq!(PaperArg::from_str(&arg.to_string(), false), Ok(arg));
        }
        assert_eq!(PaperArg::Tabloid.to_string(), "tabloid");
        assert_eq!(Paper::from(PaperArg::default()), Paper::default());
    }
}
