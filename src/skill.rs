//! `wirebug skill`: the agent skill for the `.wb` language, as this build
//! of wirebug knows it.
//!
//! The binary carries the skill's source (`skill/SKILL.md`) so the copy an
//! agent reads always matches the wirebug it drives. Emitted, the skill's
//! frontmatter names the version that produced it; the source itself
//! carries none, so a release has nothing more to bump.

use std::fmt;
use std::fs;
use std::path::Path;

use crate::error::{Error, Result};

/// The skill as written, without a version stamp.
pub const SOURCE: &str = include_str!("../skill/SKILL.md");

/// The frontmatter key, under `metadata:`, that names the wirebug version
/// an emitted skill came from.
pub const VERSION_KEY: &str = "wirebug-version";

/// The skill as this wirebug emits it: [`SOURCE`] with a `metadata:` entry
/// naming the version added at the end of its frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Skill(String);

impl Skill {
    /// The skill stamped with this build's version.
    pub fn current() -> Self {
        Self::stamped(SOURCE, env!("CARGO_PKG_VERSION"))
    }

    /// `source` with `version` added to its frontmatter, kept a valid YAML
    /// mapping that agent harnesses accept (`metadata` is a string-to-string
    /// map in the Agent Skills format). A source with no frontmatter is
    /// emitted as it is.
    fn stamped(source: &str, version: &str) -> Self {
        let split = source
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"));
        match split {
            Some((front, body)) => Self(format!(
                "---\n{front}\nmetadata:\n  {VERSION_KEY}: \"{version}\"\n---\n{body}"
            )),
            None => Self(source.to_owned()),
        }
    }

    /// The emitted text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Write the skill to `path`, creating its directory and overwriting a
    /// file already there. Fails with [`Error::SkillInstall`] naming the
    /// path that could not be created or written.
    pub fn install(&self, path: &Path) -> Result<()> {
        let failed = |path: &Path| {
            let path = path.to_path_buf();
            move |source| Error::SkillInstall { path, source }
        };
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(failed(dir))?;
        }
        fs::write(path, &self.0).map_err(failed(path))
    }
}

impl fmt::Display for Skill {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_added_at_the_end_of_the_frontmatter() {
        let skill = Skill::stamped("---\nname: x\ndescription: y\n---\n\n# Body\n", "1.2.3");
        assert_eq!(
            skill.as_str(),
            "---\nname: x\ndescription: y\nmetadata:\n  wirebug-version: \"1.2.3\"\n---\n\n# Body\n"
        );
    }

    #[test]
    fn the_emitted_skill_names_this_build_and_keeps_the_source_otherwise() {
        let skill = Skill::current();
        let stamp = format!("  {VERSION_KEY}: \"{}\"\n", env!("CARGO_PKG_VERSION"));
        assert!(skill.as_str().contains(&stamp), "{}", skill.as_str());
        assert_eq!(
            skill
                .as_str()
                .replacen(&stamp, "", 1)
                .replacen("metadata:\n", "", 1),
            SOURCE
        );
    }

    #[test]
    fn the_source_carries_no_version_of_its_own() {
        assert!(!SOURCE.contains(VERSION_KEY));
        assert!(!SOURCE.contains("\nmetadata:"));
    }

    /// Whether `value` reads in YAML as the one-line string it shows: a
    /// double-quoted scalar with nothing to escape, or a plain scalar
    /// without the `: ` or ` #` that would end it early and without a
    /// leading indicator character.
    fn is_safe_yaml_string(value: &str) -> bool {
        if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            return !inner.contains(['"', '\\']);
        }
        !value.is_empty()
            && !value.contains(": ")
            && !value.contains(" #")
            && !value.starts_with([
                '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"',
                '%', '@', '`',
            ])
    }

    #[test]
    fn every_frontmatter_value_of_the_source_is_a_yaml_string() {
        // Other harnesses parse the frontmatter with a strict YAML parser;
        // an unquoted `: ` inside the description is a syntax error there.
        let front = SOURCE
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map(|(front, _)| front)
            .expect("the source opens with frontmatter");
        for line in front.lines() {
            let (key, value) = line.split_once(": ").expect("a `key: value` line");
            assert!(
                key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "unexpected frontmatter key in `{line}`"
            );
            assert!(
                is_safe_yaml_string(value),
                "quote the frontmatter value of `{key}`"
            );
        }
    }

    #[test]
    fn an_unquoted_value_with_a_colon_is_not_a_safe_yaml_string() {
        assert!(!is_safe_yaml_string("harnesses: components"));
        assert!(is_safe_yaml_string("\"harnesses: components\""));
        assert!(is_safe_yaml_string("wirebug-dsl"));
    }

    #[test]
    fn a_source_without_frontmatter_is_emitted_unchanged() {
        assert_eq!(Skill::stamped("# Body\n", "1.2.3").as_str(), "# Body\n");
    }

    #[test]
    fn install_creates_the_directories_and_overwrites_a_stale_copy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a").join("b").join("SKILL.md");
        let skill = Skill::current();
        skill.install(&path).expect("installed");
        fs::write(&path, "stale").expect("write");
        skill.install(&path).expect("reinstalled");
        assert_eq!(fs::read_to_string(&path).expect("read"), skill.as_str());
    }

    #[test]
    fn install_failure_names_the_path_it_could_not_create() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("afile");
        fs::write(&file, "mine").expect("write");
        let error = Skill::current()
            .install(&file.join("SKILL.md"))
            .expect_err("a file is in the way");
        assert!(matches!(&error, Error::SkillInstall { path, .. } if *path == file));
        assert!(error.to_string().contains(&file.display().to_string()));
    }
}
