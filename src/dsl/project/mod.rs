//! Project discovery and multi-file loading.
//!
//! A wirebug project is the directory rooted at a `main.wb`. We discover
//! it by walking up from a target (or the CWD), then load every file
//! reachable through `use` declarations, resolving paths relative to the
//! importing file and de-duplicating by canonical path (so a `use` cycle
//! is harmless — we load each file once). Logical hierarchy depends only
//! on `use` paths and DSL nesting, never on directory layout.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use miette::NamedSource;

use crate::dsl::ast;
use crate::dsl::diagnostics::Problem;
use crate::dsl::lex::{lex, significant};
use crate::dsl::parse::parse_file;
use crate::dsl::span::FileId;

/// One loaded source file. `files[id.0]` is the file with that [`FileId`].
pub struct LoadedFile {
    pub path: PathBuf,
    pub src: String,
    pub ast: ast::File,
}

impl LoadedFile {
    /// The file's source wrapped for miette diagnostics.
    pub fn named_source(&self) -> NamedSource<String> {
        NamedSource::new(self.path.display().to_string(), self.src.clone())
    }
}

/// A loaded project: every reachable file, with the entry file's id.
pub struct Project {
    pub root: FileId,
    pub files: Vec<LoadedFile>,
}

/// Find the entry `.wb` file for `target`:
/// - a file path is used directly as the entry;
/// - a directory is searched for `main.wb` (then its parents);
/// - `None` walks up from the current directory.
pub fn discover(target: Option<&Path>) -> Result<PathBuf, Problem> {
    match target {
        Some(p) if p.is_file() => Ok(p.to_path_buf()),
        Some(p) if p.is_dir() => walk_up(p),
        Some(p) => Err(Problem::Io {
            path: p.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file or directory"),
        }),
        None => {
            let cwd = std::env::current_dir().map_err(|source| Problem::Io {
                path: ".".to_string(),
                source,
            })?;
            walk_up(&cwd)
        }
    }
}

/// Walk up from `start`, returning the first `main.wb` found.
fn walk_up(start: &Path) -> Result<PathBuf, Problem> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join("main.wb");
        if candidate.is_file() {
            return Ok(candidate);
        }
        dir = d.parent();
    }
    Err(Problem::NoProject {
        start: start.display().to_string(),
    })
}

/// Load `entry` and every file reachable through `use`, collecting all
/// lex/parse/import problems. Returns `None` for the project only if the
/// entry file itself couldn't be loaded.
pub fn load(entry: &Path) -> (Option<Project>, Vec<Problem>) {
    let mut loader = Loader {
        files: Vec::new(),
        by_path: HashMap::new(),
        problems: Vec::new(),
    };
    let entry = canonical(entry);
    let root = loader.load_file(&entry);
    let Loader {
        files, problems, ..
    } = loader;
    match root {
        Some(root) => (Some(Project { root, files }), problems),
        None => (None, problems),
    }
}

/// Canonicalize when possible; otherwise keep the path as given so the
/// not-found diagnostic still names something sensible.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

struct Loader {
    files: Vec<LoadedFile>,
    by_path: HashMap<PathBuf, FileId>,
    problems: Vec<Problem>,
}

impl Loader {
    /// Load one canonical path (and its imports), returning its id. Reuses
    /// an already-loaded file. Returns `None` if the file can't be read or
    /// parsed into an AST.
    fn load_file(&mut self, path: &Path) -> Option<FileId> {
        if let Some(id) = self.by_path.get(path) {
            return Some(*id);
        }

        let id = FileId(self.files.len());
        let src = match std::fs::read_to_string(path) {
            Ok(src) => src,
            Err(source) => {
                self.problems.push(Problem::Io {
                    path: path.display().to_string(),
                    source,
                });
                return None;
            }
        };

        let named = || NamedSource::new(path.display().to_string(), src.clone());

        let lexemes = match lex(&src, id) {
            Ok(lexemes) => lexemes,
            Err(err) => {
                self.problems.push(Problem::Lex {
                    message: lex_message(&err),
                    src: named(),
                    at: err.span().into(),
                });
                return None;
            }
        };

        let parsed = parse_file(significant(&lexemes), id, src.len());
        for err in parsed.errors {
            self.problems.push(Problem::Parse {
                message: err.message,
                src: named(),
                at: err.span.into(),
            });
        }
        let ast = parsed.file?;

        // Register before following imports so a `use` cycle terminates.
        // `files` index stays equal to the FileId because parsing doesn't
        // recurse — we push the fully-parsed file here, then follow uses.
        let imports: Vec<(String, crate::dsl::span::Span)> = ast
            .uses
            .iter()
            .map(|u| (u.path.node.clone(), u.path.span))
            .collect();
        self.files.push(LoadedFile {
            path: path.to_path_buf(),
            src,
            ast,
        });
        self.by_path.insert(path.to_path_buf(), id);

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        for (rel, span) in imports {
            let resolved = parent.join(&rel);
            match resolved.canonicalize() {
                Ok(canon) => {
                    self.load_file(&canon);
                }
                Err(_) => {
                    self.problems.push(Problem::UseNotFound {
                        target: rel,
                        src: self.files[id.0].named_source(),
                        at: span.into(),
                    });
                }
            }
        }

        Some(id)
    }
}

/// Render a [`LexError`](crate::dsl::lex::LexError) to a message string.
fn lex_message(err: &crate::dsl::lex::LexError) -> String {
    use crate::dsl::lex::LexError::*;
    match err {
        UnterminatedString { .. } => "unterminated string: missing closing `\"`".to_string(),
        NewlineInString { .. } => {
            "string label spans a newline; labels are single-line".to_string()
        }
        UnexpectedChar { ch, .. } => format!("unexpected character `{ch}`"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn examples_main() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/main.wb"))
    }

    #[test]
    fn loads_the_whole_seed_project() {
        let (project, problems) = load(&examples_main());
        assert!(problems.is_empty(), "unexpected problems: {problems:?}");
        let project = project.expect("project loaded");
        // main.wb plus 12 component files, all reachable via `use`.
        assert_eq!(project.files.len(), 13, "reachable file count");
    }

    #[test]
    fn discover_finds_main_by_walking_up() {
        let from = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/components"));
        let found = discover(Some(&from)).expect("walk up to main.wb");
        assert!(found.ends_with("main.wb"));
    }

    #[test]
    fn dangling_use_reports_use_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let main = dir.path().join("main.wb");
        std::fs::write(
            &main,
            "use missing from \"does_not_exist.wb\"\ncomponent c { pub port a \"A\"; }\n",
        )
        .expect("write main.wb");

        let (project, problems) = load(&main);
        assert!(project.is_some(), "entry still parses");
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, Problem::UseNotFound { .. })),
            "expected a UseNotFound, got {problems:?}"
        );
    }

    #[test]
    fn no_project_when_main_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = discover(Some(dir.path())).expect_err("no main.wb");
        assert!(matches!(err, Problem::NoProject { .. }));
    }
}
