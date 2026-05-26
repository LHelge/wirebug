//! `wirebug` — text-defined electrical schematics rendered to SVG.
//!
//! The primary input is the `.wb` DSL: [`dsl::check_project`] turns a
//! multi-file project into an elaborated [`dsl::ir::Design`], and
//! [`render::render_views`] renders that design's views to SVG.
//!
//! See the workspace `README.md` for the user-facing tour and `CLAUDE.md`
//! for the architecture and conventions.

pub mod dsl;
pub mod error;
pub mod render;

pub use error::{Error, Result};
pub use render::{RenderedView, render_views};
