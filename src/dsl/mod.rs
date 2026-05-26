//! The `.wb` DSL parse-and-check pipeline.
//!
//! Stages: discover the project (the directory rooted at `main.wb`),
//! load and lex/parse every reachable file, resolve names, elaborate the
//! type/instance hierarchy into the [`ir`], then validate it. The
//! terminal artifact is [`ir::Design`] — nothing here renders.
//!
//! The pipeline is built up across several changes; today it is a stub
//! that reports it is not yet implemented.

pub mod lex;
pub mod span;

use std::path::Path;

/// Diagnostic output format for `wirebug check`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    /// miette's pretty terminal renderer.
    #[default]
    Human,
    /// Machine-readable JSON (miette's `JSONReportHandler`).
    Json,
}

/// Tallied outcome of a check run. `errors == 0` means the project is
/// well-formed (warnings alone don't fail unless `--strict` promoted
/// them upstream).
#[derive(Debug, Default)]
#[must_use]
pub struct Summary {
    pub errors: usize,
    pub warnings: usize,
}

impl Summary {
    /// Whether the run found no errors.
    pub fn is_ok(&self) -> bool {
        self.errors == 0
    }
}

/// Run the parse-and-check pipeline against the project containing
/// `target` (or the project discovered by walking up from the current
/// directory when `target` is `None`).
///
/// Stub: returns an empty summary until the pipeline lands.
pub fn check_project(target: Option<&Path>, strict: bool, format: Format) -> Summary {
    let _ = (target, strict, format);
    Summary::default()
}
