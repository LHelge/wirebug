//! Library-level error type.
//!
//! One enum covers parsing, validation, and rendering. Each variant
//! carries enough context to act on without round-tripping through a
//! string.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// All errors produced by the library.
#[derive(Debug, Error)]
pub enum Error {
    /// Failed to read a file from disk.
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// YAML parser rejected the input. Carries the source path when
    /// known (file loads attach it; `FromStr` parses leave it absent)
    /// and the line/column when the parser reports it.
    #[error(
        "failed to parse{p}{a}: {message}",
        p = format_yaml_path(.path),
        a = .at.as_deref().unwrap_or(""),
    )]
    Yaml {
        path: Option<PathBuf>,
        /// Pre-formatted " at line L column C" suffix, or empty if the
        /// parser didn't report a location.
        at: Option<String>,
        message: String,
    },

    /// An identifier contained a `.`, which is reserved as the port-ref
    /// separator.
    #[error("identifier {0:?} contains '.', which is reserved")]
    DottedIdentifier(String),

    /// A port reference string had the wrong number of dot-separated
    /// segments.
    #[error("port reference {raw:?} is not in the form 'component.connector.port'")]
    MalformedPortRef { raw: String },

    /// A connector-scoped port reference (used inside a view's per-side
    /// list) had the wrong number of dot-separated segments.
    #[error("port reference {raw:?} inside view ports must be in the form 'connector.port'")]
    MalformedConnectorPortRef { raw: String },

    /// A `connections:` entry pointed at a component/connector/port that
    /// doesn't exist in the model.
    #[error("connection references unknown port {port}")]
    UnknownConnectionPort { port: String },

    /// A view's `layout:` listed a component that isn't in the model.
    #[error("view layout references unknown component {component:?}")]
    UnknownLayoutComponent { component: String },

    /// A view's `ports:` block referenced a component that isn't in
    /// the model.
    #[error("view ports references unknown component {component:?}")]
    UnknownViewComponent { component: String },

    /// A view's `ports:` block referenced a component that isn't in
    /// its own `layout:` block.
    #[error("view ports lists {component:?} but it has no entry in layout")]
    PortsWithoutLayout { component: String },

    /// A view's per-side port list referenced a connector/port that
    /// doesn't exist on the named component.
    #[error("view ports for component {component:?} references unknown port {connector}.{port}")]
    UnknownViewPort {
        component: String,
        connector: String,
        port: String,
    },

    /// A view named a renderer this build doesn't know how to dispatch.
    #[error("unknown view kind {0:?}")]
    UnknownViewKind(String),

    /// Failure writing the output SVG to disk.
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<serde_yml::Error> for Error {
    /// Wrap a `serde_yml` parse error. The resulting [`Self::Yaml`]
    /// carries no source path — call [`Self::with_path`] to attach one
    /// when loading from a file.
    fn from(err: serde_yml::Error) -> Self {
        Self::Yaml {
            path: None,
            at: err
                .location()
                .map(|loc| format!(" at line {} column {}", loc.line(), loc.column())),
            message: err.to_string(),
        }
    }
}

impl Error {
    /// Attach a source-file path to a [`Self::Yaml`] error. No-op for
    /// any other variant; lets `Foo::load` reuse `Foo::from_str` and
    /// then enrich the error.
    #[must_use]
    pub fn with_path(mut self, path: &Path) -> Self {
        if let Self::Yaml { path: slot, .. } = &mut self {
            *slot = Some(path.to_path_buf());
        }
        self
    }
}

/// Format helper used by `Error::Yaml`'s `Display` impl. Free function
/// because it formats a field for a derived trait — there's no natural
/// type to hang it on.
fn format_yaml_path(path: &Option<PathBuf>) -> String {
    match path {
        Some(p) => format!(" {}", p.display()),
        None => String::new(),
    }
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;
