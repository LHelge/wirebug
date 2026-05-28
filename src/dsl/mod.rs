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
pub mod elaborate;
pub mod ir;
pub mod lex;
pub mod manifest;
pub mod parse;
pub mod project;
pub mod resolve;
pub mod span;
pub mod validate;

use std::path::Path;

use diagnostics::Problem;
use ir::Design;

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
    /// The elaborated design, when the project got far enough to build one.
    pub design: Option<Design>,
}

/// Number of failing errors and non-failing warnings in a [`CheckReport`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProblemCounts {
    pub errors: usize,
    pub warnings: usize,
}

impl CheckReport {
    /// Count the report's errors and warnings.
    pub fn counts(&self) -> ProblemCounts {
        let errors = self.problems.iter().filter(|p| p.is_error()).count();
        ProblemCounts {
            errors,
            warnings: self.problems.len() - errors,
        }
    }

    /// Whether this report should block check/render success. Warnings block
    /// only in strict mode.
    pub fn has_blocking_problems(&self, strict: bool) -> bool {
        let counts = self.counts();
        counts.errors > 0 || (strict && counts.warnings > 0)
    }
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
                design: None,
            };
        }
    };

    let (project, mut problems) = project::load(&entry);
    let mut design = None;
    if let Some(project) = project {
        let mut resolved = resolve::resolve(&project);
        problems.append(&mut resolved.problems);
        let (elaborated, elab_problems) = elaborate::elaborate(&resolved);
        problems.extend(elab_problems);
        problems.extend(validate::validate(&resolved));
        design = elaborated;
    }
    CheckReport { problems, design }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miette::NamedSource;

    #[test]
    fn check_report_counts_errors_and_warnings() {
        let report = CheckReport {
            problems: vec![
                Problem::NoRoot,
                Problem::UnusedImport {
                    name: "leaf".to_string(),
                    src: NamedSource::new("main.wb", String::new()),
                    at: (0, 0).into(),
                },
            ],
            design: None,
        };

        assert_eq!(
            report.counts(),
            ProblemCounts {
                errors: 1,
                warnings: 1,
            }
        );
        assert!(report.has_blocking_problems(false));
        assert!(report.has_blocking_problems(true));
    }

    #[test]
    fn check_report_warnings_block_only_under_strict() {
        let report = CheckReport {
            problems: vec![Problem::UnusedImport {
                name: "leaf".to_string(),
                src: NamedSource::new("main.wb", String::new()),
                at: (0, 0).into(),
            }],
            design: None,
        };

        assert_eq!(
            report.counts(),
            ProblemCounts {
                errors: 0,
                warnings: 1,
            }
        );
        assert!(!report.has_blocking_problems(false));
        assert!(report.has_blocking_problems(true));
    }
}
