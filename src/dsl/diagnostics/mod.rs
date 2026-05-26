//! miette diagnostics for the check pipeline.
//!
//! One growing enum, [`Problem`], with a variant per failure class. Each
//! carries the offending file's source (so miette can render the snippet)
//! and a label span. Severity defaults to error; warnings set it
//! explicitly. The enum grows as later phases (resolve, elaborate,
//! validate) land.

use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum Problem {
    /// A lexical error (bad character, unterminated string, …).
    #[error("{message}")]
    #[diagnostic(code(wirebug::lex))]
    Lex {
        message: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        at: SourceSpan,
    },

    /// A syntax error from the parser.
    #[error("{message}")]
    #[diagnostic(code(wirebug::parse))]
    Parse {
        message: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("here")]
        at: SourceSpan,
    },

    /// A `use` referenced a file that couldn't be found.
    #[error("cannot find imported file `{target}`")]
    #[diagnostic(
        code(wirebug::use_not_found),
        help("paths in `use` are relative to the importing file")
    )]
    UseNotFound {
        target: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("imported here")]
        at: SourceSpan,
    },

    /// No `main.wb` was found while discovering the project.
    #[error("no wirebug project found: no `main.wb` in `{start}` or any parent directory")]
    #[diagnostic(
        code(wirebug::no_project),
        help("a wirebug project is a directory containing a `main.wb`")
    )]
    NoProject { start: String },

    /// Failed to read a source file from disk.
    #[error("failed to read `{path}`: {source}")]
    #[diagnostic(code(wirebug::io))]
    Io {
        path: String,
        source: std::io::Error,
    },
}

impl Problem {
    /// True for problems that fail the run (severity error). Warnings
    /// return false unless the caller is running `--strict`.
    pub fn is_error(&self) -> bool {
        !matches!(self.severity(), Some(miette::Severity::Warning))
    }
}
