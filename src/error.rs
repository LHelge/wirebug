//! Library-level error types for the render path.
//!
//! Parsing and validation of the `.wb` DSL report through miette
//! `Diagnostic`s (`dsl::diagnostics::Problem`); these types cover only the
//! rendering stage. A failure while rendering one view is a [`ViewError`]
//! — the view's title and kind plus a [`ViewErrorKind`] saying why — so
//! the other views still render; [`Error`] is what stops a whole render:
//! disk IO, the HTML index and the embed manifest, what stops an export of
//! a build table, and what stops `init` scaffolding a project.

use std::path::PathBuf;

use thiserror::Error;

use crate::dsl::ir::{ViewKind, WireEnd};
use crate::export::Table;

/// Errors that stop a whole render, not just one view, an export, or a
/// scaffold.
#[derive(Debug, Error)]
pub enum Error {
    /// Failure writing an output file to disk.
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Rendering the HTML index template failed.
    #[error("failed to render HTML index: {0}")]
    Template(#[from] askama::Error),

    /// Serializing the `--embed` manifest sidecar failed.
    #[error("failed to serialize the embed manifest: {0}")]
    EmbedManifest(#[from] serde_json::Error),

    /// Writing a build table as CSV failed.
    #[error("failed to write the {table} table as CSV: {source}")]
    ExportCsv {
        table: Table,
        #[source]
        source: csv::Error,
    },

    /// Serializing a build table as JSON failed.
    #[error("failed to serialize the {table} table as JSON: {source}")]
    ExportJson {
        table: Table,
        #[source]
        source: serde_json::Error,
    },

    /// `init` found a project file already in place, so it wrote nothing
    /// rather than overwrite it.
    #[error("{path} already exists; not overwriting it")]
    InitExists { path: PathBuf },

    /// `init` was pointed at something other than a directory, so there is
    /// nowhere to put a project.
    #[error("{path} is not a directory; cannot create a project in it")]
    InitNotADirectory { path: PathBuf },
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// One view that could not be rendered (or converted to PNG/PDF): which
/// view, and why. The view is named by its title, as the author wrote it.
#[derive(Debug, Error)]
#[error("view {title:?}: {cause}")]
pub struct ViewError {
    /// The view's title.
    pub title: String,
    /// The view's kind, so a listing of views can place the failure where
    /// the drawing would have gone.
    pub view_kind: ViewKind,
    /// What went wrong. Boxed so a `Result` carrying a `ViewError` stays
    /// small on its happy path.
    pub cause: Box<ViewErrorKind>,
}

/// Why a view could not be rendered: what a renderer or converter reports
/// before the view's identity is attached (`ViewErrorKind::in_view`).
#[derive(Debug, Error)]
pub enum ViewErrorKind {
    /// A view named a renderer this build doesn't know how to dispatch.
    #[error("unknown view kind {0:?}")]
    UnknownViewKind(String),

    /// A view documents a component type with no instance in the design,
    /// so there is nothing to render it against.
    #[error("view subject {subject:?} has no instance in the design")]
    UnknownSubject { subject: String },

    /// A render view referenced a connector that does not exist on its
    /// subject instance.
    #[error("unknown connector {connector:?} on view subject {subject:?}")]
    UnknownConnector { subject: String, connector: String },

    /// A view's `grid:` step was zero or negative.
    #[error("grid step must be positive, got {grid}")]
    NonPositiveGrid { grid: f64 },

    /// A view's `grid:` step is smaller than one port (with its label)
    /// needs. One grid step is the spacing between adjacent ports, so too
    /// fine a grid would overlap labels.
    #[error("grid step {grid} is too small; ports need at least {minimum} per step")]
    GridTooSmall { grid: f64, minimum: f64 },

    /// The schematic router could not find an object-avoiding route for a
    /// connection. Rendering fails instead of drawing a misleading straight
    /// segment through obstacles. Each end is `instance.port`, or a bare
    /// `port` for one of the subject's own ports on the enclosure.
    #[error("failed to route schematic wire between `{from}` and `{to}`")]
    RouteFailed { from: WireEnd, to: WireEnd },

    /// A view's canvas is longer on a side than `render::MAX_CANVAS`, so
    /// rasterising it would allocate more than any drawing needs. Refused
    /// before anything is allocated for it.
    #[error(
        "canvas is {width} × {height} units, more than the {limit} units a view may span on a side"
    )]
    CanvasTooLarge {
        /// The canvas's sides, rounded up to whole units
        /// (`render::Canvas::reported`).
        width: f64,
        height: f64,
        limit: f64,
    },

    /// Failed to parse our own rendered SVG before converting it (to PNG
    /// or PDF).
    #[error("failed to parse rendered SVG: {0}")]
    SvgParse(String),

    /// Failed to allocate or encode a PNG pixmap.
    #[error("failed to encode PNG: {0}")]
    PngEncode(String),

    /// Failed to convert a rendered SVG into PDF objects.
    #[error("failed to convert SVG to PDF: {0}")]
    PdfConvert(String),
}

impl ViewErrorKind {
    /// Attach the identity of the view this failure happened in.
    #[must_use]
    pub(crate) fn in_view(self, title: &str, view_kind: &ViewKind) -> ViewError {
        ViewError {
            title: title.to_string(),
            view_kind: view_kind.clone(),
            cause: Box::new(self),
        }
    }
}

/// Result of rendering or converting one view, before the view's identity
/// is attached.
pub type ViewResult<T> = std::result::Result<T, ViewErrorKind>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{InstanceName, PortName};

    #[test]
    fn a_routing_failure_names_the_view_and_both_ends() {
        let error = ViewErrorKind::RouteFailed {
            from: WireEnd::Child {
                instance: InstanceName::from("ecu"),
                port: PortName::from("sig"),
            },
            to: WireEnd::Own(PortName::from("out")),
        }
        .in_view("Overview", &ViewKind::Schematic);
        assert_eq!(
            error.to_string(),
            "view \"Overview\": failed to route schematic wire between `ecu.sig` and `out`"
        );
    }
}
