//! Render an elaborated [`Design`] to SVG.
//!
//! [`render_views`] is the entry point used by the CLI: it walks every
//! view in the design, resolves each to its subject instance, and
//! dispatches to the renderer named by the view's `kind`.

use std::collections::HashMap;

use askama::Template;

use crate::dsl::ir::{Design, Instance, View, ViewKind};
use crate::error::{Error, Result};

pub mod geometry;
pub mod harness;
pub mod png;
pub mod schematic;

/// One rendered view: the SVG document, the file name it should be
/// written to (a slug of the view title, without a directory), the human
/// title it was rendered from, and the view `kind` (so the HTML index can
/// group views into tabs).
#[derive(Debug)]
#[must_use]
pub struct RenderedView {
    pub title: String,
    pub filename: String,
    pub kind: ViewKind,
    pub svg: String,
}

impl RenderedView {
    pub fn is_schematic(&self) -> bool {
        self.kind.is_schematic()
    }

    pub fn is_harness(&self) -> bool {
        self.kind.is_harness()
    }
}

/// Render every view in `design` to SVG, in declaration order.
///
/// Each view documents a component *type*; it is rendered against the
/// first instance of that type in the design (the root for a top-level
/// view). A view naming a `kind` this build can't render, or a subject
/// type with no instance, is an error.
pub fn render_views(design: &Design) -> Result<Vec<RenderedView>> {
    let mut filenames = FilenameAllocator::default();
    let mut rendered = Vec::with_capacity(design.views.len());
    for view in &design.views {
        let subject = subject_instance(design, view)?;
        let svg = match &view.kind {
            ViewKind::Schematic => schematic::SchematicRenderer.render(design, subject, view)?,
            ViewKind::Harness => harness::HarnessRenderer.render(design, subject, view)?,
            ViewKind::Other(other) => return Err(Error::UnknownViewKind(other.clone())),
        };
        rendered.push(RenderedView {
            title: view.title.clone(),
            filename: filenames.svg_filename(&view.title),
            kind: view.kind.clone(),
            svg,
        });
    }
    Ok(rendered)
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
    /// Whether either tab has any views, so the template can hide an empty
    /// tab and pick a sensible default selection.
    has_schematic: bool,
    has_harness: bool,
}

/// Render the HTML index for `views`. Shared by `render` (static, no
/// live-reload) and `serve` (in-memory, live-reload on). Views are grouped
/// into Schematics/Harnesses tabs by their `kind`.
pub fn index_html(views: &[RenderedView], live_reload: bool) -> Result<String> {
    IndexTemplate {
        views,
        live_reload,
        has_schematic: views.iter().any(RenderedView::is_schematic),
        has_harness: views.iter().any(RenderedView::is_harness),
    }
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

/// Allocates stable, non-overwriting SVG file names from view titles.
#[derive(Default)]
struct FilenameAllocator {
    seen: HashMap<String, usize>,
}

impl FilenameAllocator {
    fn svg_filename(&mut self, title: &str) -> String {
        let base = slug(title);
        let count = self.seen.entry(base.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            format!("{base}.svg")
        } else {
            format!("{base}_{count}.svg")
        }
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
    fn filename_allocator_disambiguates_duplicate_titles() {
        let mut filenames = FilenameAllocator::default();
        assert_eq!(filenames.svg_filename("Pack Detail"), "pack_detail.svg");
        assert_eq!(filenames.svg_filename("Pack Detail"), "pack_detail_2.svg");
        assert_eq!(filenames.svg_filename("Pack Detail!"), "pack_detail_3.svg");
        assert_eq!(filenames.svg_filename("!!!"), "view.svg");
        assert_eq!(filenames.svg_filename("???"), "view_2.svg");
    }

    fn view(title: &str, filename: &str, kind: &str) -> RenderedView {
        RenderedView {
            title: title.to_string(),
            filename: filename.to_string(),
            kind: ViewKind::from(kind),
            svg: String::new(),
        }
    }

    #[test]
    fn index_references_each_view_by_title_and_file() {
        let views = vec![
            view("HV Overview", "hv_overview.svg", "schematic"),
            view("Pack Detail", "pack_detail.svg", "schematic"),
        ];
        let html = index_html(&views, false).unwrap();
        assert!(html.contains("<h2>HV Overview</h2>"));
        assert!(html.contains("<img src=\"hv_overview.svg\" alt=\"HV Overview\">"));
        assert!(html.contains("<img src=\"pack_detail.svg\" alt=\"Pack Detail\">"));
    }

    #[test]
    fn index_groups_views_into_kind_tabs() {
        let views = vec![
            view("Overview", "overview.svg", "schematic"),
            view("Main Harness", "main_harness.svg", "harness"),
        ];
        let html = index_html(&views, false).unwrap();
        // Both tab controls present when both kinds exist.
        assert!(html.contains("Schematics"));
        assert!(html.contains("Harnesses"));
        assert!(html.contains("id=\"tab-schematic\""));
        assert!(html.contains("id=\"tab-harness\""));
    }

    #[test]
    fn index_omits_a_tab_with_no_views() {
        let views = vec![view("Overview", "overview.svg", "schematic")];
        let html = index_html(&views, false).unwrap();
        assert!(html.contains("id=\"tab-schematic\""));
        assert!(!html.contains("id=\"tab-harness\""));
    }

    #[test]
    fn index_escapes_author_supplied_titles() {
        let views = vec![view("A & B <hv>", "a_b_hv.svg", "schematic")];
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
