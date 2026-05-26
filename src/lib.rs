//! `wirebug` — text-defined electrical schematics rendered to SVG.
//!
//! See the workspace `README.md` for the user-facing tour and
//! `CLAUDE.md` for the architecture and conventions.

use std::path::Path;

pub mod dsl;
pub mod error;
pub mod model;
pub mod render;
pub mod view;

pub use error::{Error, Result};
pub use model::{Model, ValidationReport, Warning};
pub use render::render;
pub use view::{View, ViewKind};

/// The product of a render: the SVG document and any non-fatal
/// warnings that surfaced during validation.
///
/// Returned by [`render_paths`]. The caller decides what to do with
/// the warnings (the CLI writes them to stderr; a test may assert on
/// them).
#[derive(Debug)]
#[must_use]
pub struct RenderResult {
    pub svg: String,
    pub warnings: Vec<Warning>,
}

/// Load the model and view from disk, validate both, and render the
/// view to SVG.
///
/// All orchestration the CLI does — parse, validate, dispatch the
/// renderer — happens here. The CLI binary is a thin wrapper over this
/// function; end-to-end tests can call it directly without going
/// through `clap` or shelling out.
pub fn render_paths(
    model_path: impl AsRef<Path>,
    view_path: impl AsRef<Path>,
) -> Result<RenderResult> {
    let model = Model::load(model_path)?;
    let mut report = model.validate()?;

    let view = View::load(view_path)?;
    report.extend(view.validate(&model)?);

    let svg = render::render(&model, &view)?;

    Ok(RenderResult {
        svg,
        warnings: report.warnings,
    })
}
