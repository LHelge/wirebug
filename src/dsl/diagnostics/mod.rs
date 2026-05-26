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

    // --- Resolution ---
    /// An instance names a component type that isn't in scope.
    #[error("unknown component type `{name}`")]
    #[diagnostic(
        code(wirebug::undefined_type),
        help("define `{name}`, or `use` it from another file")
    )]
    UndefinedType {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not a component in scope")]
        at: SourceSpan,
    },

    /// A `use` resolved to a file that has no matching top-level component.
    #[error("`{name}` is not a top-level component in `{file}`")]
    #[diagnostic(code(wirebug::unresolved_import))]
    UnresolvedImport {
        name: String,
        file: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("no such component to import")]
        at: SourceSpan,
    },

    /// Two component types share a name in one file's scope.
    #[error("duplicate component type `{name}`")]
    #[diagnostic(code(wirebug::duplicate_type))]
    DuplicateType {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("redefined here")]
        at: SourceSpan,
        #[label("first defined here")]
        first: SourceSpan,
    },

    /// Two instances in one component share a name.
    #[error("duplicate instance name `{name}`")]
    #[diagnostic(code(wirebug::duplicate_instance))]
    DuplicateInstance {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("reused here")]
        at: SourceSpan,
        #[label("first used here")]
        first: SourceSpan,
    },

    /// Two ports in one component share a name (connectors are not
    /// namespaces — port names are unique across the whole component).
    #[error("duplicate port name `{name}`")]
    #[diagnostic(
        code(wirebug::duplicate_port),
        help("port names must be unique across a component, including across connectors")
    )]
    DuplicatePort {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("redeclared here")]
        at: SourceSpan,
        #[label("first declared here")]
        first: SourceSpan,
    },

    /// A wire endpoint names an instance that doesn't exist in the
    /// enclosing component.
    #[error("unknown instance `{name}`")]
    #[diagnostic(code(wirebug::unknown_instance))]
    UnknownInstance {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not an instance in this component")]
        at: SourceSpan,
    },

    /// A wire endpoint names a port that doesn't exist on its component.
    #[error("unknown port `{port}`{on}")]
    #[diagnostic(code(wirebug::unknown_port))]
    UnknownPort {
        port: String,
        /// e.g. " on `cell_module`", or empty for a self port.
        on: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("no such port")]
        at: SourceSpan,
    },

    /// A wire endpoint references a non-`pub` port from outside its owner.
    #[error("port `{port}` of `{ty}` is not `pub`")]
    #[diagnostic(
        code(wirebug::private_port),
        help("mark it `pub` in `{ty}`, or wire it through a pub port")
    )]
    PrivatePort {
        port: String,
        ty: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("referenced from outside `{ty}`")]
        at: SourceSpan,
    },

    /// A view `include` names something that isn't an instance of the
    /// component the view documents.
    #[error("unknown instance `{name}` in view")]
    #[diagnostic(code(wirebug::unknown_include))]
    UnknownInclude {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("not an instance of the documented component")]
        at: SourceSpan,
    },

    /// A view can't be bound to a single documented component (the file
    /// has zero or several top-level components).
    #[error("cannot tell which component this view documents")]
    #[diagnostic(
        code(wirebug::view_subject),
        help("a view documents the file's single top-level component")
    )]
    ViewSubject {
        #[source_code]
        src: NamedSource<String>,
        #[label("this view")]
        at: SourceSpan,
    },
}

impl Problem {
    /// True for problems that fail the run (severity error). Warnings
    /// return false unless the caller is running `--strict`.
    pub fn is_error(&self) -> bool {
        !matches!(self.severity(), Some(miette::Severity::Warning))
    }
}
