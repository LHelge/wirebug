//! Library-level error type for the render path.
//!
//! Parsing and validation of the `.wb` DSL report through miette
//! `Diagnostic`s (`dsl::diagnostics::Problem`); this enum covers only the
//! rendering stage — locating a view's subject, the grid constraints the
//! schematic renderer enforces, and disk IO.

use std::path::PathBuf;

use thiserror::Error;

/// All errors produced by the render path.
#[derive(Debug, Error)]
pub enum Error {
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
    /// segment through obstacles.
    #[error("failed to route schematic wire between {from:?} and {to:?}")]
    RouteFailed { from: String, to: String },

    /// Failure writing the output SVG to disk.
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Rendering the HTML index template failed.
    #[error("failed to render HTML index: {0}")]
    Template(#[from] askama::Error),

    /// Failed to parse our own rendered SVG before rasterising it to PNG.
    #[error("failed to parse rendered SVG for PNG conversion: {0}")]
    SvgParse(String),

    /// Failed to allocate or encode a PNG pixmap.
    #[error("failed to encode PNG: {0}")]
    PngEncode(String),
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;
