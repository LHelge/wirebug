//! Render an elaborated [`Design`] to SVG.
//!
//! [`render_views`] is the entry point used by the CLI: it walks every
//! view in the design, resolves each to its subject instance, and
//! dispatches to the renderer named by the view's `kind`.

use askama::Template;

use crate::dsl::ir::{Design, Instance, View};
use crate::error::{Error, Result};

pub mod geometry;
pub mod schematic;

/// One rendered view: the SVG document, the file name it should be
/// written to (a slug of the view title, without a directory), and the
/// human title it was rendered from (for the HTML index).
#[derive(Debug)]
#[must_use]
pub struct RenderedView {
    pub title: String,
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
                title: view.title.clone(),
                filename: format!("{}.svg", slug(&view.title)),
                svg,
            })
        })
        .collect()
}

/// File name for the HTML index page written alongside the per-view SVGs.
pub const INDEX_FILENAME: &str = "index.html";

/// The HTML index that embeds every rendered view, in render order, each
/// under its title. The SVGs are referenced by their sibling file names, so
/// the page expects to live in the same directory as them (or be served with
/// each SVG reachable at its file name).
///
/// `live_reload` injects the `serve` websocket client script; `render` leaves
/// it off for a self-contained static page. The template auto-escapes the
/// author-supplied titles and the slugged file names.
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    views: &'a [RenderedView],
    live_reload: bool,
}

/// Render the HTML index for `views`. Shared by `render` (static, no
/// live-reload) and `serve` (in-memory, live-reload on).
pub fn index_html(views: &[RenderedView], live_reload: bool) -> Result<String> {
    IndexTemplate { views, live_reload }
        .render()
        .map_err(Error::Template)
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

    #[test]
    fn index_references_each_view_by_title_and_file() {
        let views = vec![
            RenderedView {
                title: "HV Overview".to_string(),
                filename: "hv_overview.svg".to_string(),
                svg: String::new(),
            },
            RenderedView {
                title: "Pack Detail".to_string(),
                filename: "pack_detail.svg".to_string(),
                svg: String::new(),
            },
        ];
        let html = index_html(&views, false).unwrap();
        assert!(html.contains("<h2>HV Overview</h2>"));
        assert!(html.contains("<img src=\"hv_overview.svg\" alt=\"HV Overview\">"));
        assert!(html.contains("<img src=\"pack_detail.svg\" alt=\"Pack Detail\">"));
    }

    #[test]
    fn index_escapes_author_supplied_titles() {
        let views = vec![RenderedView {
            title: "A & B <hv>".to_string(),
            filename: "a_b_hv.svg".to_string(),
            svg: String::new(),
        }];
        let html = index_html(&views, false).unwrap();
        // Askama auto-escapes (with numeric entities); the author-supplied
        // angle brackets must not survive as a literal tag.
        assert!(!html.contains("<hv>"));
        assert!(!html.contains("A & B"));
    }

    #[test]
    fn index_notes_a_design_with_no_views() {
        assert!(index_html(&[], false).unwrap().contains("no views"));
    }

    #[test]
    fn live_reload_flag_toggles_the_websocket_script() {
        assert!(index_html(&[], true).unwrap().contains("new WebSocket"));
        assert!(!index_html(&[], false).unwrap().contains("new WebSocket"));
    }
}
