//! Render an elaborated [`Design`] to SVG.
//!
//! [`render_views`] is the entry point used by the CLI: it walks every
//! view in the design, resolves each to its subject instance, and
//! dispatches to the renderer named by the view's `kind`.

use crate::dsl::ir::{Design, Instance, View};
use crate::error::{Error, Result};

pub mod geometry;
pub mod schematic;

/// One rendered view: the SVG document and the file name it should be
/// written to (a slug of the view title, without a directory).
#[derive(Debug)]
#[must_use]
pub struct RenderedView {
    pub filename: String,
    pub svg: String,
}

/// Render every view in `design` to SVG, in declaration order.
///
/// Each view documents a component *type*; it is rendered against the
/// first instance of that type in the design (the root for a top-level
/// view). A view naming a `kind` this build can't render, or a subject
/// type with no instance, is an error.
pub fn render_views(design: &Design) -> Result<Vec<RenderedView>> {
    design
        .views
        .iter()
        .map(|view| {
            let subject = subject_instance(design, view)?;
            let svg = match view.kind.as_str() {
                "schematic" => schematic::SchematicRenderer.render(design, subject, view)?,
                other => return Err(Error::UnknownViewKind(other.to_string())),
            };
            Ok(RenderedView {
                filename: format!("{}.svg", slug(&view.title)),
                svg,
            })
        })
        .collect()
}

/// The instance a view renders against: the first one whose type matches
/// the view's subject.
fn subject_instance<'d>(design: &'d Design, view: &View) -> Result<&'d Instance> {
    design
        .instances
        .values()
        .find(|inst| inst.type_name == view.subject)
        .ok_or_else(|| Error::UnknownSubject {
            subject: view.subject.to_string(),
        })
}

/// Turn a human title into a safe file-name stem: lowercase, runs of
/// non-alphanumeric characters collapsed to a single underscore, ends
/// trimmed. An empty result falls back to `view`.
fn slug(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut pending_sep = false;
    for ch in title.chars() {
        if ch.is_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('_');
            }
            pending_sep = false;
            out.extend(ch.to_lowercase());
        } else {
            pending_sep = true;
        }
    }
    if out.is_empty() {
        "view".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_collapses_and_lowercases() {
        assert_eq!(slug("HV System Overview"), "hv_system_overview");
        assert_eq!(slug("  Pack / Detail  "), "pack_detail");
        assert_eq!(slug("!!!"), "view");
    }
}
