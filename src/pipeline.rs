//! The check pipeline every command runs: discover and load the project,
//! the DSL stages (`dsl::check_loaded`), then the checks that need a view
//! laid out (`render::lint`).
//!
//! The layout checks live in `render`, beside the layout code they ask, and
//! need the loaded sources to point at includes; `dsl` stays free of any
//! dependency on `render`. This module is where both are visible.

use std::path::Path;

use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::Design;
use crate::dsl::project::{self, Project};
use crate::dsl::{self, CheckReport};
use crate::render::lint;

/// Run the full check against the project containing `target` (or the
/// project discovered by walking up from the current directory when
/// `target` is `None`): load, resolve, elaborate, validate, then the
/// layout checks.
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
        let (elaborated, dsl_problems) = dsl::check_loaded(&project);
        problems.extend(dsl_problems);
        extend_with_layout_problems(&project, elaborated.as_ref(), &mut problems);
        design = elaborated;
    }
    CheckReport { problems, design }
}

/// Append the layout checks' problems to `problems`, the ones the earlier
/// stages found. They run only on a design without errors: an erroring
/// design is never rendered, and its views may hold includes that failed to
/// resolve or are duplicated, which would only add noise to the real error.
pub(crate) fn extend_with_layout_problems(
    project: &Project,
    design: Option<&Design>,
    problems: &mut Vec<Problem>,
) {
    if problems.iter().any(Problem::is_error) {
        return;
    }
    if let Some(design) = design {
        problems.extend(lint::layout_problems(design, project));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(report: &CheckReport) -> Vec<String> {
        use miette::Diagnostic;
        report
            .problems
            .iter()
            .filter_map(|p| p.code().map(|c| c.to_string()))
            .collect()
    }

    fn project(main: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("wirebug.toml"),
            "[project]\nname = \"pipeline\"\nversion = \"0.1.0\"\n",
        )
        .expect("write wirebug.toml");
        std::fs::write(
            dir.path().join("lamp.wb"),
            "component Lamp { pub port a \"A\"; }\n",
        )
        .expect("write lamp.wb");
        std::fs::write(dir.path().join("main.wb"), main).expect("write main.wb");
        dir
    }

    const OVERLAPPING: &str = "use Lamp from \"lamp.wb\";\ncomponent Root { l: Lamp; m: Lamp; }\nview schematic \"S\" { include l at (0, 0); include m at (1, 1); }\n";

    #[test]
    fn layout_checks_run_once_the_dsl_stages_are_clean() {
        let dir = project(OVERLAPPING);
        let report = check_project(Some(dir.path()));
        assert_eq!(codes(&report), ["wirebug::overlapping_includes"]);
        assert!(report.design.is_some());
    }

    #[test]
    fn layout_checks_wait_until_the_errors_are_fixed() {
        // An undefined type is an error; the overlap is left for later.
        let main = OVERLAPPING.replace("m: Lamp;", "m: Lamp; x: Missing;");
        let dir = project(&main);
        let report = check_project(Some(dir.path()));
        assert_eq!(codes(&report), ["wirebug::undefined_type"]);
    }

    #[test]
    fn layout_checks_run_alongside_warnings() {
        // A non-IEC wire color only warns, so the overlap is still reported.
        let main = OVERLAPPING.replace("m: Lamp;", "m: Lamp; wire chartreuse 1 [l.a, m.a];");
        let dir = project(&main);
        let report = check_project(Some(dir.path()));
        assert_eq!(
            codes(&report),
            [
                "wirebug::unknown_wire_color",
                "wirebug::overlapping_includes"
            ]
        );
    }
}
