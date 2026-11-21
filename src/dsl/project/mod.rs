//! Project discovery and multi-file loading.
//!
//! A wirebug project is rooted at a `wirebug.toml` manifest. We discover
//! it by walking up from a target (or the CWD), then load `main.wb` from
//! that root plus every file
//! reachable through `use` and `fragment` statements, resolving paths
//! relative to the importing file and de-duplicating by canonical path (so
//! an import cycle is harmless — we load each file once). Logical hierarchy
//! depends only on `use` and `fragment` paths and DSL nesting, never on
//! directory layout.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use miette::NamedSource;

use crate::dsl::ast;
use crate::dsl::diagnostics::Problem;
use crate::dsl::lex::{lex, significant};
use crate::dsl::manifest::{self, Manifest};
use crate::dsl::parse::{Completeness, ParseError, parse_file};
use crate::dsl::span::FileId;

/// One loaded source file. `files[id.0]` is the file with that [`FileId`].
pub struct LoadedFile {
    pub path: PathBuf,
    pub src: String,
    pub ast: ast::File,
    /// Whether the parser skipped statements of this file it could not
    /// read (each reported as a syntax error), so `ast` lacks what they
    /// declared.
    pub holes: bool,
}

impl LoadedFile {
    /// The file's source wrapped for miette diagnostics.
    pub fn named_source(&self) -> NamedSource<String> {
        NamedSource::new(self.path.display().to_string(), self.src.clone())
    }
}

/// A loaded project: every reachable file, with the entry file's id and
/// the project manifest (`wirebug.toml`) if one was loaded.
pub struct Project {
    pub root: FileId,
    pub files: Vec<LoadedFile>,
    pub manifest: Option<Manifest>,
    /// Whether every reachable file lexed and parsed whole. When not, a
    /// statement the parser skipped, or a file it could not load, may have
    /// declared what a later phase finds missing.
    pub parsed_whole: bool,
}

impl Project {
    /// The file with the given id.
    pub fn file(&self, id: FileId) -> &LoadedFile {
        &self.files[id.0]
    }

    /// The source of `id`, wrapped for miette diagnostics.
    pub fn source(&self, id: FileId) -> NamedSource<String> {
        self.file(id).named_source()
    }

    /// The loaded file that `path`, written in a `use` or `fragment` of
    /// file `from`, names: resolved relative to `from`'s directory, as the
    /// loader followed it. `None` when it names no file, or one the load
    /// left out.
    pub fn import_target(&self, from: FileId, path: &str) -> Option<FileId> {
        let target = self
            .file(from)
            .path
            .parent()?
            .join(path)
            .canonicalize()
            .ok()?;
        self.files
            .iter()
            .position(|file| file.path == target)
            .map(FileId)
    }

    /// The path a `use` or `fragment` written in file `from` gives to name
    /// file `to`.
    pub fn import_path(&self, from: FileId, to: FileId) -> String {
        let from = &self.file(from).path;
        relative_path(from.parent().unwrap_or(from), &self.file(to).path)
    }

    /// The path of `id` as diagnostics name it: relative to the entry
    /// file's directory, the project's.
    pub fn display_path(&self, id: FileId) -> String {
        self.import_path(self.root, id)
    }

    /// Drop from `problems` those a source that did not parse whole can
    /// cause on its own ([`Problem::reports_an_absence`]): a name that
    /// resolves to nothing may have been declared in a skipped statement.
    /// The syntax errors stand for them until they are fixed. A project
    /// that parsed whole keeps every problem.
    pub(crate) fn drop_parse_cascades(&self, problems: &mut Vec<Problem>) {
        if !self.parsed_whole {
            problems.retain(|problem| !problem.reports_an_absence());
        }
    }
}

/// `to` relative to `from_dir`, with forward slashes — the form a `use`
/// or `fragment` path is written in. Both paths must be absolute (they
/// come from the loader's canonicalization).
pub(crate) fn relative_path(from_dir: &Path, to: &Path) -> String {
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    parts.extend(
        to[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// Find the entry `.wb` file for `target`:
/// - a `wirebug.toml` file resolves to the `main.wb` beside it;
/// - a `.wb` file path is used directly as the entry;
/// - a directory is searched for `wirebug.toml` (then its parents);
/// - `None` walks up from the current directory.
// A `Problem` is large (it carries source text), but discovery is a
// once-per-run path, so the size of the error variant doesn't matter.
#[allow(clippy::result_large_err)]
pub fn discover(target: Option<&Path>) -> Result<PathBuf, Problem> {
    match target {
        Some(p) if p.is_file() && is_manifest(p) => Ok(p.with_file_name("main.wb")),
        Some(p) if p.is_file() => Ok(p.to_path_buf()),
        Some(p) if p.is_dir() => walk_up_manifest(p),
        Some(p) => Err(Problem::Io {
            path: p.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file or directory"),
        }),
        None => {
            let cwd = std::env::current_dir().map_err(|source| Problem::Io {
                path: ".".to_string(),
                source,
            })?;
            walk_up_manifest(&cwd)
        }
    }
}

/// Walk up from `start`, returning the `main.wb` beside the first
/// `wirebug.toml` found.
#[allow(clippy::result_large_err)]
fn walk_up_manifest(start: &Path) -> Result<PathBuf, Problem> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let candidate = d.join(manifest::FILE_NAME);
        if candidate.is_file() {
            return Ok(d.join("main.wb"));
        }
        dir = d.parent();
    }
    Err(Problem::NoProject {
        start: start.display().to_string(),
    })
}

fn is_manifest(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name == manifest::FILE_NAME)
}

/// In-memory source overrides, keyed by canonical path and checked before
/// the filesystem. The LSP loads through this so unsaved editor buffers
/// participate in a check; an empty overlay reads straight from disk.
///
/// A file that exists *only* in the overlay can be the entry, but a `use`
/// can't reach it: import targets are resolved by canonicalizing a real
/// path on disk.
#[derive(Default)]
pub struct Overlay(HashMap<PathBuf, String>);

impl Overlay {
    /// Set the text for `path`, replacing the on-disk content during loads.
    /// The key is canonicalized with the loader's own fallback, so a
    /// relative or symlinked path still matches the loaded file.
    pub fn insert(&mut self, path: &Path, text: String) {
        self.0.insert(canonical(path), text);
    }

    /// Drop the override for `path`; loads see the file on disk again.
    pub fn remove(&mut self, path: &Path) {
        self.0.remove(&canonical(path));
    }

    /// The override for `path`, if one is set.
    pub fn get(&self, path: &Path) -> Option<&str> {
        self.0.get(&canonical(path)).map(String::as_str)
    }

    fn read_to_string(&self, path: &Path) -> std::io::Result<String> {
        match self.0.get(path) {
            Some(text) => Ok(text.clone()),
            None => std::fs::read_to_string(path),
        }
    }
}

/// Load `entry` and every file reachable through `use` and `fragment`,
/// collecting all lex/parse/import problems. Returns `None` for the
/// project only if the entry file itself couldn't be loaded.
pub fn load(entry: &Path) -> (Option<Project>, Vec<Problem>) {
    load_with(entry, &Overlay::default())
}

/// [`load`], with open-buffer text from `overlay` shadowing the disk.
pub fn load_with(entry: &Path, overlay: &Overlay) -> (Option<Project>, Vec<Problem>) {
    let mut loader = Loader {
        overlay,
        files: Vec::new(),
        by_path: HashMap::new(),
        attempted: HashSet::new(),
        problems: Vec::new(),
        parsed_whole: true,
    };
    let entry = canonical(entry);
    let root = loader.load_file(&entry);
    let Loader {
        files,
        problems,
        parsed_whole,
        ..
    } = loader;

    let project_dir = entry.parent().unwrap_or(Path::new("."));
    let (manifest, manifest_problems) = manifest::load(project_dir);
    let mut problems = problems;
    problems.extend(manifest_problems);

    match root {
        Some(root) => (
            Some(Project {
                root,
                files,
                manifest,
                parsed_whole,
            }),
            problems,
        ),
        None => (None, problems),
    }
}

/// Canonicalize when possible; otherwise keep the path as given so the
/// not-found diagnostic still names something sensible.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

struct Loader<'a> {
    overlay: &'a Overlay,
    files: Vec<LoadedFile>,
    /// Successfully loaded files, by canonical path.
    by_path: HashMap<PathBuf, FileId>,
    /// Every path we've *started* loading, success or not — so a file
    /// reached via two imports is processed (and its errors reported)
    /// exactly once.
    attempted: HashSet<PathBuf>,
    problems: Vec<Problem>,
    /// No file so far failed to lex or parse whole.
    parsed_whole: bool,
}

impl Loader<'_> {
    /// Load one canonical path (and its imports), returning its id. Reuses
    /// an already-loaded file. Returns `None` if the file can't be read or
    /// lexed, or its braces don't balance (its AST would put statements in
    /// the wrong blocks, so it is left out, as if it did not parse at all).
    /// A file with statements the parser skipped loads without them.
    fn load_file(&mut self, path: &Path) -> Option<FileId> {
        // Process each path once, whether or not it loads successfully —
        // otherwise a file imported via two `use`s re-reports its errors.
        if !self.attempted.insert(path.to_path_buf()) {
            return self.by_path.get(path).copied();
        }

        let id = FileId(self.files.len());
        let src = match self.overlay.read_to_string(path) {
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
                self.parsed_whole = false;
                return None;
            }
        };

        let parsed = parse_file(significant(&lexemes), id, &src);
        for err in parsed.errors {
            self.problems.push(match err {
                ParseError::Syntax { span, message } => Problem::Parse {
                    message,
                    src: named(),
                    at: span.into(),
                },
                ParseError::ReservedWord { span, word } => Problem::ReservedWord {
                    word,
                    src: named(),
                    at: span.into(),
                },
            });
        }
        let holes = match parsed.completeness {
            Completeness::Whole => false,
            Completeness::Holes => true,
            Completeness::Misnested => {
                self.parsed_whole = false;
                return None;
            }
        };
        let Some(ast) = parsed.file else {
            self.parsed_whole = false;
            return None;
        };
        self.parsed_whole &= !holes;

        // Register before following imports so an import cycle terminates.
        // `files` index stays equal to the FileId because parsing doesn't
        // recurse — we push the fully-parsed file here, then follow imports,
        // `use` and `fragment` alike, in source order.
        let imports: Vec<(String, crate::dsl::span::Span)> = ast
            .imports()
            .iter()
            .map(|import| (import.path().node.clone(), import.path().span))
            .collect();
        self.files.push(LoadedFile {
            path: path.to_path_buf(),
            src,
            ast,
            holes,
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

    fn write_manifest(dir: &Path) {
        std::fs::write(
            dir.join(manifest::FILE_NAME),
            "[project]\nname = \"test\"\nversion = \"0.0.0\"\n",
        )
        .expect("write wirebug.toml");
    }

    fn fixture_main() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/basic_project/main.wb"
        ))
    }

    #[test]
    fn relative_paths_walk_up_and_down() {
        assert_eq!(
            relative_path(
                Path::new("/p/systems"),
                Path::new("/p/components/hv/inv.wb")
            ),
            "../components/hv/inv.wb"
        );
        assert_eq!(
            relative_path(Path::new("/p"), Path::new("/p/hv.wb")),
            "hv.wb"
        );
    }

    #[test]
    fn loads_a_multi_file_project() {
        let (project, problems) = load(&fixture_main());
        assert!(problems.is_empty(), "unexpected problems: {problems:?}");
        let project = project.expect("project loaded");
        // main.wb, two imported component files, and the connector-type
        // library inverter.wb pulls in — all reachable via `use`.
        assert_eq!(project.files.len(), 4, "reachable file count");
    }

    #[test]
    fn discover_finds_manifest_by_walking_up() {
        let from = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/basic_project/components"
        ));
        let found = discover(Some(&from)).expect("walk up to wirebug.toml");
        assert!(found.ends_with("main.wb"));
    }

    #[test]
    fn discover_accepts_a_manifest_file() {
        let manifest = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/basic_project/wirebug.toml"
        ));
        let found = discover(Some(&manifest)).expect("manifest resolves to main.wb");
        assert!(found.ends_with("main.wb"));
    }

    #[test]
    fn dangling_use_reports_use_not_found() {
        let dir = tempfile::tempdir().expect("tempdir");
        let main = dir.path().join("main.wb");
        std::fs::write(
            &main,
            "use missing from \"does_not_exist.wb\";\ncomponent c { pub port a \"A\"; }\n",
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
    fn file_imported_twice_reports_its_error_once() {
        // leaf.wb is broken and imported by both a.wb and b.wb, which
        // main.wb imports — a diamond. The lex error must appear once.
        let dir = tempfile::tempdir().expect("tempdir");
        let write = |name: &str, body: &str| {
            std::fs::write(dir.path().join(name), body).expect("write");
        };
        write("leaf.wb", "component leaf { pub port a \"A\" @; }\n");
        write(
            "a.wb",
            "use leaf from \"leaf.wb\";\ncomponent a { l: leaf; }\n",
        );
        write(
            "b.wb",
            "use leaf from \"leaf.wb\";\ncomponent b { l: leaf; }\n",
        );
        write(
            "main.wb",
            "use a from \"a.wb\";\nuse b from \"b.wb\";\ncomponent m { x: a; y: b; }\n",
        );

        let (_project, problems) = load(&dir.path().join("main.wb"));
        let lex_errors = problems
            .iter()
            .filter(|p| matches!(p, Problem::Lex { .. }))
            .count();
        assert_eq!(
            lex_errors, 1,
            "diamond import should report once: {problems:?}"
        );
    }

    #[test]
    fn file_with_a_broken_statement_loads_without_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("lamp.wb"),
            "component Lamp { pub port a \"A\" pin; pub port b \"B\"; }\n",
        )
        .expect("write lamp.wb");
        let main = dir.path().join("main.wb");
        std::fs::write(
            &main,
            "use Lamp from \"lamp.wb\";\ncomponent m { l: Lamp; }\n",
        )
        .expect("write main.wb");

        let (project, problems) = load(&main);
        let parse_errors = problems
            .iter()
            .filter(|p| matches!(p, Problem::Parse { .. }))
            .count();
        assert_eq!(parse_errors, 1, "{problems:?}");
        let project = project.expect("project loaded");
        assert!(!project.parsed_whole);
        let lamp = &project.files[1];
        assert!(lamp.holes);
        let ast::Item::Definition(def) = &lamp.ast.items[0] else {
            panic!("expected Lamp");
        };
        assert_eq!(def.members.len(), 1, "port b stays, port a is skipped");
        assert!(!project.files[0].holes);
    }

    #[test]
    fn file_whose_braces_do_not_balance_is_left_out() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("lamp.wb"),
            "component Lamp { pub port a \"A\";\n",
        )
        .expect("write lamp.wb");
        let main = dir.path().join("main.wb");
        std::fs::write(
            &main,
            "use Lamp from \"lamp.wb\";\ncomponent m { l: Lamp; }\n",
        )
        .expect("write main.wb");

        let (project, problems) = load(&main);
        assert!(
            problems.iter().any(|p| matches!(p, Problem::Parse { .. })),
            "{problems:?}"
        );
        let project = project.expect("main.wb loads");
        assert_eq!(project.files.len(), 1, "lamp.wb is left out");
        assert!(!project.parsed_whole);
    }

    #[test]
    fn a_whole_project_is_marked_parsed_whole() {
        let (project, _) = load(&fixture_main());
        let project = project.expect("project loaded");
        assert!(project.parsed_whole);
        assert!(project.files.iter().all(|f| !f.holes));
    }

    #[test]
    fn overlay_text_replaces_disk_content() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_manifest(dir.path());
        let main = dir.path().join("main.wb");
        std::fs::write(&main, "component Broken { @ }\n").expect("write main.wb");

        // The broken file on disk is shadowed by a fixed buffer…
        let mut overlay = Overlay::default();
        overlay.insert(&main, "component Fixed { pub port a \"A\"; }\n".to_string());
        let (project, problems) = load_with(&main, &overlay);
        assert!(problems.is_empty(), "overlay should win: {problems:?}");
        assert!(
            project
                .expect("loads")
                .file(FileId(0))
                .src
                .contains("Fixed")
        );

        // …and a broken buffer shadows a good file once the override flips.
        std::fs::write(&main, "component Fine { pub port a \"A\"; }\n").expect("rewrite main.wb");
        overlay.insert(&main, "component Broken { @ }\n".to_string());
        let (_, problems) = load_with(&main, &overlay);
        assert!(
            problems.iter().any(|p| matches!(p, Problem::Lex { .. })),
            "expected the overlay's lex error, got {problems:?}"
        );

        // Removing the override restores disk truth.
        overlay.remove(&main);
        let (_, problems) = load_with(&main, &overlay);
        assert!(problems.is_empty(), "disk is fine again: {problems:?}");
    }

    #[test]
    fn overlay_keys_are_canonical_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_manifest(dir.path());
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        let main = dir.path().join("main.wb");
        std::fs::write(&main, "component Broken { @ }\n").expect("write main.wb");

        // Inserted via an unnormalized path, found via the canonical one.
        let mut overlay = Overlay::default();
        let roundabout = dir.path().join("sub").join("..").join("main.wb");
        overlay.insert(
            &roundabout,
            "component Fixed { pub port a \"A\"; }\n".to_string(),
        );
        let (_, problems) = load_with(&main, &overlay);
        assert!(problems.is_empty(), "canonical keys match: {problems:?}");
    }

    #[test]
    fn no_project_when_manifest_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = discover(Some(dir.path())).expect_err("no wirebug.toml");
        assert!(matches!(err, Problem::NoProject { .. }));
    }
}
