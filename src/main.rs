//! `wirebug` CLI binary.
//!
//! Thin shim over [`wirebug::render_paths`]: parse CLI args, call the
//! library, print warnings to stderr, write the SVG to disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use wirebug::error::Error;

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
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Render { model, view, out } => render_command(&model, &view, &out),
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
