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

    // --- Elaboration ---
    /// `main.wb` doesn't define exactly one top-level component to elaborate.
    #[error("`main.wb` must define exactly one top-level component")]
    #[diagnostic(
        code(wirebug::no_root),
        help("the design root is the single top-level component in `main.wb`")
    )]
    NoRoot,

    /// A component instantiates itself, directly or transitively.
    #[error("component `{name}` contains itself: {cycle}")]
    #[diagnostic(
        code(wirebug::containment_cycle),
        help("a component cannot instantiate itself, directly or through its children")
    )]
    ContainmentCycle {
        name: String,
        cycle: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("this component")]
        at: SourceSpan,
    },

    // --- Validation ---
    /// A wire has fewer than two endpoints.
    #[error("a wire needs at least two endpoints, found {count}")]
    #[diagnostic(code(wirebug::wire_arity))]
    WireArity {
        count: usize,
        #[source_code]
        src: NamedSource<String>,
        #[label("this wire")]
        at: SourceSpan,
    },

    /// An imported component is never instantiated in the importing file.
    #[error("unused import `{name}`")]
    #[diagnostic(
        severity(Warning),
        code(wirebug::unused_import),
        help("remove the `use`, or instantiate `{name}`")
    )]
    UnusedImport {
        name: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("never instantiated")]
        at: SourceSpan,
    },

    /// A pin assignment on a port that isn't inside a `connector` block.
    #[error("pin assignment on `{port}`, which is not inside a connector")]
    #[diagnostic(
        severity(Warning),
        code(wirebug::bare_port_pin),
        help("pins are connector metadata; put the port inside a `connector` block")
    )]
    BarePortPin {
        port: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("pin here has no connector")]
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
