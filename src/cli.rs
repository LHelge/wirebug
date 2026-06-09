//! Command-line interface definition: the clap argument tree for the
//! `wirebug` binary. This module is parsing only — it owns the `Cli`,
//! `Command`, and `OutputFormat` types and the mapping from the CLI's
//! output-format flag into the library's [`Format`]. The command
//! implementations live in `main.rs`.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use wirebug::dsl::Format;

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
    },
}
