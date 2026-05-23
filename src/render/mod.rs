//! Render a validated [`Model`] + [`View`] to SVG.
//!
//! [`Renderer`] is the extension point; [`render`] is the
//! `ViewKind`-dispatching entry point used by the CLI.

use crate::error::Result;
use crate::model::Model;
use crate::view::{View, ViewKind};

pub mod schematic;

/// Anything that knows how to turn a model + view into an SVG string.
pub trait Renderer {
    fn render(&self, model: &Model, view: &View) -> Result<String>;
}

/// Dispatch to the appropriate renderer for `view.kind`.
pub fn render(model: &Model, view: &View) -> Result<String> {
    match view.kind {
        ViewKind::Schematic => schematic::SchematicRenderer.render(model, view),
    }
}
