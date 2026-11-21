//! `wirebug` — text-defined electrical schematics rendered to SVG.
//!
//! The primary input is the `.wb` DSL: [`check_project`] turns a
//! multi-file project into an elaborated [`dsl::ir::Design`] and reports
//! its problems, and
//! [`render::render_views`] renders that design's views to SVG;
//! [`export`] derives the build tables (the wire list and the pin tables)
//! from it and writes them as CSV or JSON; [`init`] scaffolds a new
//! project.
//!
//! See the workspace `README.md` for the user-facing tour, `ARCHITECTURE.md`
//! for how it works and `AGENTS.md` for the conventions.

pub mod dsl;
pub mod error;
pub mod export;
pub mod init;
pub mod lsp;
mod pipeline;
pub mod render;
pub mod serve;

pub use error::{Error, Result};
pub use pipeline::check_project;
pub use render::{RenderedView, index_html, render_views};

/// The wirebug that produced an output, as the embed manifest and the
/// exported tables name it: `wirebug v<version>`.
pub(crate) const GENERATOR: &str = concat!("wirebug v", env!("CARGO_PKG_VERSION"));
