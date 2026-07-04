//! The `.wb` DSL parse-and-check pipeline.
//!
//! Stages: discover the project (the directory rooted at `wirebug.toml`),
//! load and lex/parse every reachable file, resolve names, elaborate the
//! type/instance hierarchy into the [`ir`], then validate it. The
//! terminal artifact is `ir::Design` — nothing here renders.

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

/// Everything a check run produced: the problems found and the elaborated
/// IR. The CLI renders the problems and derives an exit code.
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

/// Load just the project manifest for `target` — project discovery plus a
/// `wirebug.toml` parse, skipping the full check pipeline so it works even
/// when the `.wb` sources have errors. Backs `wirebug manifest version`.
///
/// Returns `None` together with at least one [`Problem`] when the project
/// can't be discovered or the manifest can't be read/parsed.
pub fn load_manifest(target: Option<&Path>) -> (Option<manifest::Manifest>, Vec<Problem>) {
    match project::discover(target) {
        Ok(entry) => manifest::load(entry.parent().unwrap_or_else(|| Path::new("."))),
        Err(problem) => (None, vec![problem]),
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
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures")).join(name)
    }

    #[test]
    fn load_manifest_reads_the_project_version() {
        let (manifest, problems) = load_manifest(Some(&fixture("basic_project")));
        assert!(problems.is_empty(), "unexpected problems: {problems:?}");
        assert_eq!(manifest.expect("manifest loaded").version, "0.1.0");
    }

    #[test]
    fn load_manifest_ignores_broken_wb_sources() {
        // The manifest is independent of the check pipeline, so a project
        // whose `.wb` won't parse still yields its version.
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("wirebug.toml"),
            "[project]\nname = \"broken\"\nversion = \"9.9.9\"\n",
        )
        .expect("write wirebug.toml");
        std::fs::write(dir.path().join("main.wb"), "component c { @ }\n").expect("write main.wb");

        let (manifest, problems) = load_manifest(Some(dir.path()));
        assert!(problems.is_empty(), "unexpected problems: {problems:?}");
        assert_eq!(manifest.expect("manifest loaded").version, "9.9.9");
    }

    #[test]
    fn load_manifest_reports_a_problem_when_no_project() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manifest, problems) = load_manifest(Some(dir.path()));
        assert!(manifest.is_none());
        assert!(!problems.is_empty(), "expected a discovery problem");
    }

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
