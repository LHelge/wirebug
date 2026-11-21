//! `wirebug init`: scaffold a new project — a `wirebug.toml` manifest, a
//! minimal `main.wb` and, when the directory has none, a `.gitignore` for
//! the `out/` that `render` writes by default.
//!
//! A scaffold never overwrites: it refuses, before writing anything, a
//! path that is not a directory or a directory that already holds a
//! manifest or a `main.wb`, and leaves an existing `.gitignore` alone.

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::dsl::{manifest, project};
use crate::error::{Error, Result};

/// The scaffolded `main.wb`: two components, one wire, one schematic view.
const MAIN_WB: &str = include_str!("../templates/init/main.wb");

/// What the scaffolded `.gitignore` keeps out of version control: the
/// directory `render` writes into when given no `--out`.
const GITIGNORE: &str = "# wirebug render output\nout/\n";

/// The project name used when the directory's name has nothing to keep.
const FALLBACK_NAME: &str = "wirebug-project";

/// A new project to write into a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scaffold {
    dir: PathBuf,
}

/// The files a scaffold wrote, and how to go on from there.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct Scaffolded {
    /// The project directory, as the scaffold was given it.
    pub dir: PathBuf,
    /// The manifest's project name.
    pub name: String,
    /// Every file written, in the order written.
    pub files: Vec<PathBuf>,
}

impl Scaffold {
    /// A scaffold for `dir`, which need not exist yet.
    #[must_use]
    pub fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    /// Write the project: create the directory (and its parents) if
    /// missing, then the manifest, `main.wb` and, if absent, `.gitignore`.
    /// Refuses, having written nothing, with [`Error::InitNotADirectory`]
    /// when the path is something other than a directory, and with
    /// [`Error::InitExists`] when the manifest or `main.wb` is already there.
    pub fn write(&self) -> Result<Scaffolded> {
        if self.dir.exists() && !self.dir.is_dir() {
            return Err(Error::InitNotADirectory {
                path: self.dir.clone(),
            });
        }
        let manifest_path = self.dir.join(manifest::FILE_NAME);
        let entry_path = self.dir.join(project::ENTRY_FILE);
        for path in [&manifest_path, &entry_path] {
            if path.exists() {
                return Err(Error::InitExists { path: path.clone() });
            }
        }

        fs::create_dir_all(&self.dir).map_err(|source| Error::Write {
            path: self.dir.clone(),
            source,
        })?;
        let name = self.project_name();

        let mut files = Vec::new();
        Self::create_new(&manifest_path, &name.manifest())?;
        files.push(manifest_path);
        Self::create_new(&entry_path, MAIN_WB)?;
        files.push(entry_path);
        let gitignore = self.dir.join(".gitignore");
        match Self::create_new(&gitignore, GITIGNORE) {
            Ok(()) => files.push(gitignore),
            Err(Error::InitExists { .. }) => {}
            Err(error) => return Err(error),
        }

        Ok(Scaffolded {
            dir: self.dir.clone(),
            name: name.0,
            files,
        })
    }

    /// The manifest name: the directory's own name, resolved through `.`
    /// and `..`.
    fn project_name(&self) -> ProjectName {
        let dir = fs::canonicalize(&self.dir).unwrap_or_else(|_| self.dir.clone());
        dir.file_name()
            .and_then(|name| name.to_str())
            .map_or_else(ProjectName::fallback, ProjectName::from)
    }

    /// Write `text` to a file that must not exist yet, so a file created
    /// since the scaffold looked is still never overwritten.
    fn create_new(path: &Path, text: &str) -> Result<()> {
        let failed = |source| Error::Write {
            path: path.to_path_buf(),
            source,
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|source| match source.kind() {
                ErrorKind::AlreadyExists => Error::InitExists {
                    path: path.to_path_buf(),
                },
                _ => failed(source),
            })?;
        file.write_all(text.as_bytes()).map_err(failed)
    }
}

/// The `name` a scaffolded manifest carries.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectName(String);

impl ProjectName {
    fn fallback() -> Self {
        Self(FALLBACK_NAME.to_string())
    }

    /// The scaffolded `wirebug.toml`.
    fn manifest(&self) -> String {
        format!("[project]\nname = \"{}\"\nversion = \"0.1.0\"\n", self.0)
    }
}

/// A project name from a directory name: letters, digits, `-` and `_` are
/// kept, every other run of characters becomes one `-`, and none leads or
/// trails. The result also needs no escaping in a TOML string.
impl From<&str> for ProjectName {
    fn from(dir_name: &str) -> Self {
        let mut name = String::new();
        for c in dir_name.chars() {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                name.push(c);
            } else if !name.is_empty() && !name.ends_with('-') {
                name.push('-');
            }
        }
        match name.trim_matches('-') {
            "" => Self::fallback(),
            name => Self(name.to_string()),
        }
    }
}

impl Scaffolded {
    /// The directory as an argument to append to a command, quoted for a
    /// POSIX shell unless every character is plain; empty for `.`, where
    /// commands find the project themselves.
    fn command_argument(&self) -> String {
        let dir = self.dir.display().to_string();
        let plain = |c: char| c.is_ascii_alphanumeric() || "._/-".contains(c);
        if dir == "." {
            String::new()
        } else if dir.chars().all(plain) {
            format!(" {dir}")
        } else {
            format!(" '{}'", dir.replace('\'', "'\\''"))
        }
    }
}

/// One `created <path>` line per file, then the commands to run next.
impl fmt::Display for Scaffolded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for file in &self.files {
            writeln!(f, "created {}", file.display())?;
        }
        let target = self.command_argument();
        write!(
            f,
            "next: `wirebug check{target}`, then `wirebug render{target}`"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(dir_name: &str) -> String {
        ProjectName::from(dir_name).0
    }

    #[test]
    fn a_directory_name_is_kept_when_it_is_already_a_project_name() {
        assert_eq!(name("my-harness_2"), "my-harness_2");
        assert_eq!(name("kabelstam-åäö"), "kabelstam-åäö");
    }

    #[test]
    fn other_characters_in_a_directory_name_become_single_dashes() {
        assert_eq!(name("My Car (v2)"), "My-Car-v2");
        assert_eq!(name("a\"b\\c"), "a-b-c");
        assert_eq!(name(".hidden"), "hidden");
    }

    #[test]
    fn a_directory_name_with_nothing_to_keep_falls_back() {
        assert_eq!(name("..."), FALLBACK_NAME);
        assert_eq!(name(""), FALLBACK_NAME);
    }

    #[test]
    fn scaffold_creates_the_directory_and_names_the_project_after_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("nested").join("my lamp");
        let written = Scaffold::new(&dir).write().expect("scaffolded");
        assert_eq!(written.name, "my-lamp");
        assert_eq!(
            written.files,
            [
                dir.join("wirebug.toml"),
                dir.join("main.wb"),
                dir.join(".gitignore")
            ]
        );
        let (manifest, problems) = manifest::load(&dir);
        assert!(problems.is_empty(), "{problems:?}");
        let manifest = manifest.expect("manifest parses");
        assert_eq!(manifest.name, "my-lamp");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(
            fs::read_to_string(dir.join("main.wb")).expect("main.wb"),
            MAIN_WB
        );
        assert_eq!(
            fs::read_to_string(dir.join(".gitignore")).expect(".gitignore"),
            GITIGNORE
        );
    }

    #[test]
    fn scaffold_refuses_a_directory_with_a_manifest_and_writes_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::write(tmp.path().join("wirebug.toml"), "mine").expect("write");
        let error = Scaffold::new(tmp.path()).write().expect_err("refused");
        assert!(
            matches!(&error, Error::InitExists { path } if *path == tmp.path().join("wirebug.toml"))
        );
        assert_eq!(
            fs::read_to_string(tmp.path().join("wirebug.toml")).expect("read"),
            "mine"
        );
        assert!(!tmp.path().join("main.wb").exists());
        assert!(!tmp.path().join(".gitignore").exists());
    }

    #[test]
    fn scaffold_refuses_a_directory_with_a_main_wb_and_writes_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::write(tmp.path().join("main.wb"), "// mine").expect("write");
        let error = Scaffold::new(tmp.path()).write().expect_err("refused");
        assert!(
            matches!(&error, Error::InitExists { path } if *path == tmp.path().join("main.wb"))
        );
        assert!(!tmp.path().join("wirebug.toml").exists());
        assert!(!tmp.path().join(".gitignore").exists());
    }

    #[test]
    fn scaffold_refuses_a_path_that_is_a_file_and_writes_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("afile");
        fs::write(&file, "mine").expect("write");
        let error = Scaffold::new(&file).write().expect_err("refused");
        assert!(matches!(&error, Error::InitNotADirectory { path } if *path == file));
        assert_eq!(fs::read_to_string(&file).expect("read"), "mine");
    }

    #[test]
    fn scaffold_leaves_an_existing_gitignore_alone() {
        let tmp = tempfile::tempdir().expect("tempdir");
        fs::write(tmp.path().join(".gitignore"), "target/\n").expect("write");
        let written = Scaffold::new(tmp.path()).write().expect("scaffolded");
        assert!(!written.files.contains(&tmp.path().join(".gitignore")));
        assert_eq!(
            fs::read_to_string(tmp.path().join(".gitignore")).expect("read"),
            "target/\n"
        );
    }

    #[test]
    fn the_summary_names_each_file_and_the_next_commands() {
        let written = Scaffolded {
            dir: PathBuf::from("lamp"),
            name: "lamp".to_string(),
            files: vec![PathBuf::from("lamp/wirebug.toml")],
        };
        assert_eq!(
            written.to_string(),
            "created lamp/wirebug.toml\n\
             next: `wirebug check lamp`, then `wirebug render lamp`"
        );
        let here = Scaffolded {
            dir: PathBuf::from("."),
            ..written
        };
        assert!(
            here.to_string()
                .ends_with("next: `wirebug check`, then `wirebug render`")
        );
        let spaced = Scaffolded {
            dir: PathBuf::from("my lamp"),
            ..here
        };
        assert!(spaced.to_string().ends_with("`wirebug render 'my lamp'`"));
        let quoted = Scaffolded {
            dir: PathBuf::from("it's mine"),
            ..spaced.clone()
        };
        assert!(
            quoted
                .to_string()
                .ends_with(r"`wirebug render 'it'\''s mine'`")
        );
        let ampersand = Scaffolded {
            dir: PathBuf::from("a&b"),
            ..spaced
        };
        assert!(ampersand.to_string().ends_with("`wirebug render 'a&b'`"));
    }
}
