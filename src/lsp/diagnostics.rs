//! One check cycle: run the DSL pipeline over every project containing an
//! open document (with the overlay applied) and convert the resulting
//! [`Problem`]s into LSP diagnostics grouped by file URI.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString,
    Position, Range, Uri,
};
use miette::Diagnostic as _;

use crate::dsl::diagnostics::Problem;
use crate::dsl::project::{self, Overlay};
use crate::dsl::{elaborate, resolve, validate};

use super::line_index::LineIndex;
use super::uri;

/// Check every distinct project containing an open document and group the
/// diagnostics by file URI. Documents outside any project (no
/// `wirebug.toml` above them) are checked as single-file entries — lex and
/// parse problems still surface, at the cost of project-level noise like
/// `no_root`.
pub(crate) fn check_open_documents<'a>(
    open: impl Iterator<Item = &'a PathBuf>,
    overlay: &Overlay,
) -> HashMap<Uri, Vec<Diagnostic>> {
    let mut entries = BTreeSet::new();
    for doc in open {
        let parent = doc.parent().unwrap_or(Path::new("."));
        let entry = project::discover(Some(parent)).unwrap_or_else(|_| doc.clone());
        entries.insert(entry);
    }

    let mut by_uri: HashMap<Uri, Vec<Diagnostic>> = HashMap::new();
    for entry in entries {
        for (uri, diagnostic) in check_one(&entry, overlay) {
            by_uri.entry(uri).or_default().push(diagnostic);
        }
    }
    by_uri
}

/// Run the pipeline for one project entry — `check_project`'s stages over
/// the overlay — and convert each problem to a located diagnostic.
fn check_one(entry: &Path, overlay: &Overlay) -> Vec<(Uri, Diagnostic)> {
    let (project, mut problems) = project::load_with(entry, overlay);

    let mut files = HashMap::new();
    if let Some(project) = &project {
        for file in &project.files {
            if let Some(uri) = uri::to_uri(&file.path) {
                let key = file.path.display().to_string();
                files.insert(key, (uri, LineIndex::new(&file.src), file.src.as_str()));
            }
        }

        let mut resolved = resolve::resolve(project);
        problems.append(&mut resolved.problems);
        let (_, elab_problems) = elaborate::elaborate(&resolved);
        problems.extend(elab_problems);
        problems.extend(validate::validate(&resolved));
    }

    problems
        .iter()
        .filter_map(|problem| convert(problem, &files, entry))
        .collect()
}

type FileTable<'a> = HashMap<String, (Uri, LineIndex, &'a str)>;

/// Convert one [`Problem`] to `(uri, diagnostic)` via its miette surface:
/// the primary label gives the range, secondary labels become related
/// information, and a span-less problem anchors at the top of the entry
/// file.
fn convert(problem: &Problem, files: &FileTable, entry: &Path) -> Option<(Uri, Diagnostic)> {
    let labels: Vec<miette::LabeledSpan> =
        problem.labels().map(Iterator::collect).unwrap_or_default();

    let (uri, range) = match labels.first() {
        Some(primary) => locate(problem, primary, files)?,
        None => (uri::to_uri(entry)?, Range::default()),
    };

    let related: Vec<DiagnosticRelatedInformation> = labels
        .iter()
        .skip(1)
        .filter_map(|label| {
            let (uri, range) = locate(problem, label, files)?;
            Some(DiagnosticRelatedInformation {
                location: Location { uri, range },
                message: label.label().unwrap_or("related").to_string(),
            })
        })
        .collect();

    let severity = match problem.severity() {
        Some(miette::Severity::Warning) => DiagnosticSeverity::WARNING,
        _ => DiagnosticSeverity::ERROR,
    };

    let mut message = problem.to_string();
    if let Some(help) = problem.help() {
        message = format!("{message}\n{help}");
    }

    Some((
        uri,
        Diagnostic {
            range,
            severity: Some(severity),
            code: problem
                .code()
                .map(|code| NumberOrString::String(code.to_string())),
            source: Some("wirebug".to_string()),
            message,
            related_information: (!related.is_empty()).then_some(related),
            ..Diagnostic::default()
        },
    ))
}

/// Resolve one label to a file URI and range. The label's file comes back
/// out of the problem's own `#[source_code]` (its `NamedSource` name is the
/// path string the loader stamped); a file the project also loaded gets a
/// precise UTF-16 range via its [`LineIndex`], anything else (e.g.
/// `wirebug.toml`) falls back to miette's char-counted line/column.
fn locate(
    problem: &Problem,
    label: &miette::LabeledSpan,
    files: &FileTable,
) -> Option<(Uri, Range)> {
    let source = problem.source_code()?;
    let contents = source.read_span(label.inner(), 0, 0).ok()?;
    let name = contents.name()?.to_string();

    if let Some((uri, index, src)) = files.get(&name) {
        let start = index.position(src, label.offset());
        let end = index.position(src, label.offset() + label.len());
        return Some((uri.clone(), Range::new(start, end)));
    }

    // Not a loaded `.wb` file: trust miette's line/column for the start
    // and leave the range zero-width. Char columns equal UTF-16 columns
    // for the ASCII content this covers in practice.
    let start = Position::new(contents.line() as u32, contents.column() as u32);
    let uri = uri::to_uri(Path::new(&name))?;
    Some((uri, Range::new(start, start)))
}

/// Fold the previously-published URI set into a fresh result: any URI we
/// reported on last cycle but not this one gets an explicit empty entry,
/// clearing its squiggles client-side.
pub(crate) fn clear_stale(
    by_uri: &mut HashMap<Uri, Vec<Diagnostic>>,
    published: &std::collections::HashSet<Uri>,
) {
    for stale in published {
        by_uri.entry(stale.clone()).or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::str::FromStr;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write");
        path
    }

    fn manifest(dir: &Path) {
        write(
            dir,
            "wirebug.toml",
            "[project]\nname = \"test\"\nversion = \"0.0.0\"\n",
        );
    }

    /// A two-file project where main.wb wires a port that `lamp.wb`
    /// doesn't declare — the error must land on main.wb with a real span.
    #[test]
    fn unknown_port_lands_on_the_wiring_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        manifest(dir.path());
        write(
            dir.path(),
            "lamp.wb",
            "component Lamp { pub port a \"A\"; }\n",
        );
        let main = write(
            dir.path(),
            "main.wb",
            "use Lamp from \"lamp.wb\";\ncomponent Root {\n    l: Lamp;\n    pub port p \"P\";\n    wire red 1 [p, l.bogus];\n}\n",
        );

        let by_uri = check_open_documents([main.clone()].iter(), &Overlay::default());
        let main_uri = uri::to_uri(&main.canonicalize().unwrap()).unwrap();
        let diags = by_uri.get(&main_uri).expect("diagnostics on main.wb");

        let unknown = diags
            .iter()
            .find(|d| {
                matches!(&d.code, Some(NumberOrString::String(c)) if c == "wirebug::unknown_port")
            })
            .expect("an unknown_port diagnostic");
        assert_eq!(unknown.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(unknown.range.start.line, 4, "the wire line");
        assert_ne!(unknown.range.start, unknown.range.end, "non-empty span");
    }

    /// The overlay drives the check: an error introduced only in the
    /// buffer must surface without touching the disk.
    #[test]
    fn overlay_text_is_what_gets_checked() {
        let dir = tempfile::tempdir().expect("tempdir");
        manifest(dir.path());
        let main = write(
            dir.path(),
            "main.wb",
            "component Root { pub port p \"P\"; }\n",
        );

        let mut overlay = Overlay::default();
        overlay.insert(&main, "component Root { pub port p \"P\" @ }\n".to_string());
        let by_uri = check_open_documents([main.clone()].iter(), &overlay);
        let main_uri = uri::to_uri(&main.canonicalize().unwrap()).unwrap();
        assert!(
            by_uri.get(&main_uri).is_some_and(|d| !d.is_empty()),
            "expected the overlay's lex error: {by_uri:?}"
        );
    }

    #[test]
    fn warning_severity_maps_to_warning() {
        let dir = tempfile::tempdir().expect("tempdir");
        manifest(dir.path());
        write(
            dir.path(),
            "lamp.wb",
            "component Lamp { pub port a \"A\"; }\n",
        );
        // Imported but never instantiated — the unused-import warning.
        let main = write(
            dir.path(),
            "main.wb",
            "use Lamp from \"lamp.wb\";\ncomponent Root { pub port p \"P\"; }\n",
        );

        let by_uri = check_open_documents([main.clone()].iter(), &Overlay::default());
        let main_uri = uri::to_uri(&main.canonicalize().unwrap()).unwrap();
        let diags = by_uri.get(&main_uri).expect("diagnostics on main.wb");
        assert!(
            diags
                .iter()
                .any(|d| d.severity == Some(DiagnosticSeverity::WARNING)),
            "expected a warning: {diags:?}"
        );
    }

    #[test]
    fn fixed_file_gets_an_explicit_empty_publish() {
        let healed = Uri::from_str("file:///tmp/healed.wb").unwrap();
        let still = Uri::from_str("file:///tmp/still.wb").unwrap();
        let mut by_uri: HashMap<Uri, Vec<Diagnostic>> =
            HashMap::from([(still.clone(), vec![Diagnostic::default()])]);
        let published = HashSet::from([healed.clone(), still.clone()]);

        clear_stale(&mut by_uri, &published);
        assert_eq!(by_uri.get(&healed).map(Vec::len), Some(0), "cleared");
        assert_eq!(by_uri.get(&still).map(Vec::len), Some(1), "kept");
    }
}
