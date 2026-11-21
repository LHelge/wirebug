//! The check pipeline every command runs: discover and load the project,
//! the DSL stages (`dsl::check_loaded`), then the checks that need a view
//! laid out (`render::lint`). `check_project` runs it from a path; the LSP,
//! which loads through its overlay of open buffers, calls
//! [`check_loaded`] on the loaded project.
//!
//! The layout checks live in `render`, beside the layout code they ask, and
//! need the loaded sources to point at includes; `dsl` stays free of any
//! dependency on `render`. This module is where both are visible.

use std::path::Path;

use crate::dsl::diagnostics::Problem;
use crate::dsl::ir::Design;
use crate::dsl::project::{self, Project};
use crate::dsl::{self, CheckReport, Checked};
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

    let (project, problems) = project::load(&entry);
    match project {
        Some(project) => {
            let checked = check_loaded(&project, problems);
            CheckReport {
                problems: checked.problems,
                design: checked.design,
            }
        }
        None => CheckReport {
            problems,
            design: None,
        },
    }
}

/// Every stage after loading, in order: the DSL stages
/// (`dsl::check_loaded`), then the layout checks. `load_problems` are what
/// loading found; they lead the result's problems. This is the one place
/// the stages are sequenced: [`check_project`] and the LSP (which also
/// wants the `Resolved` registry, for completion) both come through here.
pub(crate) fn check_loaded(project: &Project, load_problems: Vec<Problem>) -> Checked<'_> {
    let mut checked = dsl::check_loaded(project);
    let mut problems = load_problems;
    problems.append(&mut checked.problems);
    extend_with_layout_problems(project, checked.design.as_ref(), &mut problems);
    checked.problems = problems;
    checked
}

/// Append the layout checks' problems to `problems`, the ones the earlier
/// stages found. They run only on a design without errors: an erroring
/// design is never rendered, and its views may hold includes that failed to
/// resolve or are duplicated, which would only add noise to the real error.
fn extend_with_layout_problems(
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

    fn project_of(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("wirebug.toml"),
            "[project]\nname = \"pipeline\"\nversion = \"0.1.0\"\n",
        )
        .expect("write wirebug.toml");
        for (name, body) in files {
            std::fs::write(dir.path().join(name), body).expect("write");
        }
        dir
    }

    #[test]
    fn a_broken_statement_hides_what_its_absence_causes_and_nothing_else() {
        // Port `b` and instance `o` are in statements that do not parse:
        // `l.b` is not an unknown port and `Other` is not an unused import.
        // A port's `pub` may be what a skipped statement held, so `secret`
        // is not reported private either. The zero gauge and the duplicate
        // port are wrong whatever the skipped statements said.
        let dir = project_of(&[
            (
                "lamp.wb",
                "component Lamp { pub port a \"A\"; pub port b \"B\" pin; port secret \"S\"; port secret \"S\"; }\ncomponent Other { }\n",
            ),
            (
                "main.wb",
                "use Lamp from \"lamp.wb\";\nuse Other from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n    m: Lamp;\n    o: Other \"O\" extra;\n    wire red 1 [l.a, l.b];\n    wire red 1 [m.secret, l.a];\n    wire red 0 [l.a, m.a];\n}\n",
            ),
        ]);
        let report = check_project(Some(dir.path()));
        assert_eq!(
            codes(&report),
            [
                "wirebug::parse",
                "wirebug::parse",
                "wirebug::duplicate_port",
                "wirebug::invalid_gauge",
            ]
        );
        assert!(
            report.design.is_none(),
            "a partial design is never handed out"
        );
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
