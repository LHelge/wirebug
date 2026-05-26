//! The `.wb` DSL parse-and-check pipeline.
//!
//! Stages: discover the project (the directory rooted at `main.wb`),
//! load and lex/parse every reachable file, resolve names, elaborate the
//! type/instance hierarchy into the [`ir`], then validate it. The
//! terminal artifact is `ir::Design` — nothing here renders.
//!
//! Built up across several changes. Today the pipeline discovers, loads,
//! and parses the whole project, reporting lex/parse/import problems;
//! resolution, elaboration, and validation land in later changes.

pub mod ast;
pub mod diagnostics;
pub mod ir;
pub mod lex;
pub mod parse;
pub mod project;
pub mod resolve;
pub mod span;

use std::path::Path;

use diagnostics::Problem;

/// Diagnostic output format for `wirebug check`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    /// miette's pretty terminal renderer.
    #[default]
    Human,
    /// Machine-readable JSON (miette's `JSONReportHandler`).
    Json,
}

/// Everything a check run produced: the problems found and (later) the
/// elaborated IR. The CLI renders the problems and derives an exit code.
#[derive(Debug, Default)]
#[must_use]
pub struct CheckReport {
    pub problems: Vec<Problem>,
}

/// Run the parse-and-check pipeline against the project containing
/// `target` (or the project discovered by walking up from the current
/// directory when `target` is `None`).
pub fn check_project(target: Option<&Path>) -> CheckReport {
    let entry = match project::discover(target) {
        Ok(entry) => entry,
        Err(problem) => {
            return CheckReport {
                problems: vec![problem],
            };
        }
    };

    let (project, mut problems) = project::load(&entry);
    if let Some(project) = project {
        let resolved = resolve::resolve(&project);
        problems.extend(resolved.problems);
        // Elaboration and validation consume `resolved` in later changes.
    }
    CheckReport { problems }
}
