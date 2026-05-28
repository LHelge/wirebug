//! Project manifest: a `wirebug.toml` beside `main.wb`.
//!
//! Carries the project's identity (name, version) plus a few optional
//! engineering-drawing fields (description, authors, license, revision,
//! date). Mirrors Cargo's manifest in shape: a TOML file with a single
//! `[project]` table at the project root. Unknown fields are rejected
//! (`deny_unknown_fields`) so typos surface as a parse error.
//!
//! `revision` is auto-filled from git when absent (a short SHA, suffixed
//! `-dirty` if the working tree has changes). An authored `revision`
//! always wins; if the directory isn't a git repo (or `git` isn't on
//! `PATH`), the field stays `None` and nothing is rendered.

mod git;

use std::path::Path;

use chrono::NaiveDate;
use miette::NamedSource;
use serde::Deserialize;

use crate::dsl::diagnostics::Problem;

/// File name expected at the project root.
pub const FILE_NAME: &str = "wirebug.toml";

/// The parsed project manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub authors: Vec<String>,
    pub license: Option<String>,
    /// Either authored in TOML or filled from git when absent.
    pub revision: Option<String>,
    pub date: Option<NaiveDate>,
}

/// On-disk shape: `[project]` table wrapping the fields above.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    project: ProjectTable,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectTable {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    authors: Vec<String>,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    date: Option<NaiveDate>,
}

impl From<ProjectTable> for Manifest {
    fn from(t: ProjectTable) -> Self {
        Self {
            name: t.name,
            version: t.version,
            description: t.description,
            authors: t.authors,
            license: t.license,
            revision: t.revision,
            date: t.date,
        }
    }
}

/// Read `<dir>/wirebug.toml`, parse it, and (when `revision` is absent)
/// fill it from git. Returns `None` together with at least one
/// `Problem` on any failure (missing file, IO error, TOML
/// syntax/schema error). Mirrors `project::load`'s shape.
pub fn load(dir: &Path) -> (Option<Manifest>, Vec<Problem>) {
    let path = dir.join(FILE_NAME);
    let src = match std::fs::read_to_string(&path) {
        Ok(src) => src,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return (
                None,
                vec![Problem::ManifestMissing {
                    dir: dir.display().to_string(),
                }],
            );
        }
        Err(source) => {
            return (
                None,
                vec![Problem::Io {
                    path: path.display().to_string(),
                    source,
                }],
            );
        }
    };

    let parsed: ManifestFile = match toml::from_str(&src) {
        Ok(parsed) => parsed,
        Err(err) => {
            let span: std::ops::Range<usize> = err.span().unwrap_or(0..src.len().max(1));
            let at = (span.start, span.end.saturating_sub(span.start)).into();
            return (
                None,
                vec![Problem::ManifestParse {
                    message: err.message().to_string(),
                    src: NamedSource::new(path.display().to_string(), src),
                    at,
                }],
            );
        }
    };

    let mut manifest = Manifest::from(parsed.project);
    if manifest.revision.is_none() {
        manifest.revision = git::git_revision(dir);
    }
    (Some(manifest), Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, body: &str) {
        std::fs::write(dir.join(FILE_NAME), body).expect("write wirebug.toml");
    }

    #[test]
    fn parses_a_minimal_manifest() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            r#"
[project]
name = "demo"
version = "0.1.0"
"#,
        );
        let (manifest, problems) = load(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        let manifest = manifest.expect("parsed");
        assert_eq!(manifest.name, "demo");
        assert_eq!(manifest.version, "0.1.0");
        assert!(manifest.description.is_none());
        assert!(manifest.authors.is_empty());
        assert!(manifest.license.is_none());
        assert!(manifest.date.is_none());
    }

    #[test]
    fn parses_all_optional_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            r#"
[project]
name = "demo"
version = "1.2.3"
description = "a demo"
authors = ["Alice <a@b.c>", "Bob"]
license = "MIT"
revision = "B"
date = "2026-05-28"
"#,
        );
        let (manifest, problems) = load(dir.path());
        assert!(problems.is_empty(), "{problems:?}");
        let manifest = manifest.expect("parsed");
        assert_eq!(manifest.description.as_deref(), Some("a demo"));
        assert_eq!(manifest.authors, vec!["Alice <a@b.c>", "Bob"]);
        assert_eq!(manifest.license.as_deref(), Some("MIT"));
        assert_eq!(manifest.revision.as_deref(), Some("B"));
        assert_eq!(
            manifest.date,
            Some(NaiveDate::from_ymd_opt(2026, 5, 28).unwrap())
        );
    }

    #[test]
    fn missing_file_reports_manifest_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (manifest, problems) = load(dir.path());
        assert!(manifest.is_none());
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, Problem::ManifestMissing { .. })),
            "expected ManifestMissing, got {problems:?}"
        );
    }

    #[test]
    fn missing_required_field_reports_parse_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "[project]\nname = \"demo\"\n");
        let (manifest, problems) = load(dir.path());
        assert!(manifest.is_none());
        let msg = match &problems[..] {
            [Problem::ManifestParse { message, .. }] => message.clone(),
            other => panic!("expected one ManifestParse, got {other:?}"),
        };
        assert!(
            msg.contains("version"),
            "message should mention version: {msg}"
        );
    }

    #[test]
    fn unknown_field_reports_parse_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            r#"
[project]
name = "demo"
version = "0.1.0"
oops = "what"
"#,
        );
        let (manifest, problems) = load(dir.path());
        assert!(manifest.is_none());
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, Problem::ManifestParse { .. })),
            "{problems:?}"
        );
    }

    #[test]
    fn malformed_toml_reports_parse_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(dir.path(), "[project]\nname =\n");
        let (manifest, problems) = load(dir.path());
        assert!(manifest.is_none());
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, Problem::ManifestParse { .. })),
            "{problems:?}"
        );
    }

    #[test]
    fn malformed_date_reports_parse_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            r#"
[project]
name = "demo"
version = "0.1.0"
date = "not-a-date"
"#,
        );
        let (manifest, problems) = load(dir.path());
        assert!(manifest.is_none());
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, Problem::ManifestParse { .. })),
            "{problems:?}"
        );
    }

    #[test]
    fn authored_revision_wins_over_git() {
        let dir = tempfile::tempdir().expect("tempdir");
        write(
            dir.path(),
            r#"
[project]
name = "demo"
version = "0.1.0"
revision = "B"
"#,
        );
        let (manifest, _) = load(dir.path());
        let manifest = manifest.expect("parsed");
        assert_eq!(manifest.revision.as_deref(), Some("B"));
    }

    #[test]
    fn missing_revision_is_filled_from_git_when_available() {
        // Skip if `git` isn't on PATH — auto-detection is best-effort.
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .expect("git ran");
            assert!(status.success(), "git {args:?} failed");
        };
        run(&["init", "-q"]);
        // Identity is required before the first commit on a fresh init.
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
        run(&["commit", "--allow-empty", "-q", "-m", "init"]);

        write(
            dir.path(),
            r#"
[project]
name = "demo"
version = "0.1.0"
"#,
        );
        let (manifest, _) = load(dir.path());
        let manifest = manifest.expect("parsed");
        let rev = manifest.revision.expect("git revision filled in");
        // Short SHA: 7+ hex chars, optionally a `-dirty` suffix.
        let bare = rev.trim_end_matches("-dirty");
        assert!(bare.len() >= 7, "short SHA, got {rev}");
        assert!(
            bare.chars().all(|c| c.is_ascii_hexdigit()),
            "hex-only short SHA, got {rev}"
        );

        // Touching a tracked-but-modifiable file makes the tree dirty.
        std::fs::write(dir.path().join("scratch"), "x").expect("write scratch");
        let (manifest, _) = load(dir.path());
        let rev = manifest.unwrap().revision.expect("git revision filled in");
        assert!(rev.ends_with("-dirty"), "dirty tree marker, got {rev}");
    }
}
