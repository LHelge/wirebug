//! Best-effort git revision detection for the project manifest.
//!
//! Shells out to the `git` CLI for the short HEAD SHA and a dirty-tree
//! check. Never errors: any failure (no git binary, not a repo, no
//! commits) collapses to `None` so the renderer simply omits the
//! revision stamp.

use std::path::Path;
use std::process::{Command, Stdio};

/// Returns `Some("abc1234")` or `Some("abc1234-dirty")` when `dir` is a
/// git working tree; `None` when it isn't, when `git` isn't on `PATH`,
/// or when any git invocation fails.
pub(super) fn git_revision(dir: &Path) -> Option<String> {
    let sha = git_output(dir, &["rev-parse", "--short", "HEAD"])?;
    let status = git_output(dir, &["status", "--porcelain"])?;
    let mut rev = sha;
    if !status.is_empty() {
        rev.push_str("-dirty");
    }
    Some(rev)
}

/// Run `git <args>` in `dir`, returning its trimmed stdout on success.
/// Non-zero exit or any IO failure yields `None`.
fn git_output(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_on_path() -> bool {
        Command::new("git").arg("--version").output().is_ok()
    }

    fn git_run(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .status()
            .expect("git ran");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn returns_none_when_dir_is_not_a_repo() {
        if !git_on_path() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(git_revision(dir.path()).is_none());
    }

    #[test]
    fn returns_short_sha_for_a_clean_repo() {
        if !git_on_path() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        git_run(dir.path(), &["init", "-q"]);
        git_run(dir.path(), &["config", "user.email", "test@example.com"]);
        git_run(dir.path(), &["config", "user.name", "Test"]);
        git_run(dir.path(), &["commit", "--allow-empty", "-q", "-m", "init"]);

        let rev = git_revision(dir.path()).expect("a revision");
        assert!(!rev.ends_with("-dirty"), "clean repo, got {rev}");
        assert!(rev.chars().all(|c| c.is_ascii_hexdigit()), "{rev}");
    }

    #[test]
    fn marks_a_dirty_tree() {
        if !git_on_path() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        git_run(dir.path(), &["init", "-q"]);
        git_run(dir.path(), &["config", "user.email", "test@example.com"]);
        git_run(dir.path(), &["config", "user.name", "Test"]);
        git_run(dir.path(), &["commit", "--allow-empty", "-q", "-m", "init"]);
        std::fs::write(dir.path().join("scratch"), "x").expect("write scratch");

        let rev = git_revision(dir.path()).expect("a revision");
        assert!(rev.ends_with("-dirty"), "{rev}");
    }
}
