//! Render an elaborated [`Design`] to SVG.
//!
//! [`render_views`] is the entry point used by the CLI: it walks every
//! view in the design, resolves each to its subject instance, and
//! dispatches to the renderer named by the view's `kind`.

use std::collections::{HashMap, HashSet};

use askama::Template;
use serde::Serialize;

use crate::dsl::ir::{Design, Instance, View, ViewKind};
use crate::dsl::manifest::Manifest;
use crate::error::{Error, Result};

pub(crate) mod color;
pub mod geometry;
pub mod harness;
pub mod pinout;
pub mod png;
pub mod schematic;
pub(crate) mod stamp;

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

    pub fn is_pinout(&self) -> bool {
        self.kind.is_pinout()
    }
}

/// Render every view in `design` to SVG, in declaration order.
///
/// Each view documents a component *type*; it is rendered against the
/// first instance of that type in the design (the root for a top-level
/// view). A view naming a `kind` this build can't render, or a subject
/// type with no instance, is an error.
///
/// `embed` switches every view to embed-mode output: the built-in
/// `<style>` is dropped, the bottom-right project-identity stamp is
/// suppressed, and the root `<svg>` is tagged with a `wirebug` class so
/// a downstream stylesheet can take full control of the look. Pass
/// `false` for self-contained SVGs intended to render on their own.
pub fn render_views(design: &Design, embed: bool) -> Result<Vec<RenderedView>> {
    let mut filenames = FilenameAllocator::default();
    let mut rendered = Vec::with_capacity(design.views.len());
    for view in &design.views {
        let subject = subject_instance(design, view)?;
        let svg = match &view.kind {
            ViewKind::Schematic => {
                schematic::SchematicRenderer.render(design, subject, view, embed)?
            }
            ViewKind::Harness => harness::HarnessRenderer.render(design, subject, view, embed)?,
            ViewKind::Pinout => pinout::PinoutRenderer.render(design, subject, view, embed)?,
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

/// File name for the JSON sidecar written instead of the HTML index when
/// rendering in embed mode.
pub const EMBED_MANIFEST_FILENAME: &str = "manifest.json";

/// Serialized shape of [`EMBED_MANIFEST_FILENAME`]: the project's identity
/// (or `null` for a synthetic design) plus the list of rendered views in
/// declaration order. Borrows from the data so a host needs only to call
/// `serde_json::to_string_pretty(&embed_manifest(...))`.
#[derive(Debug, Serialize)]
pub struct EmbedManifest<'a> {
    pub project: Option<&'a Manifest>,
    pub views: Vec<EmbedManifestView<'a>>,
}

/// One row of [`EmbedManifest::views`].
#[derive(Debug, Serialize)]
pub struct EmbedManifestView<'a> {
    pub title: &'a str,
    pub filename: &'a str,
    pub kind: &'a str,
}

/// Build the embed-mode manifest from the rendered views and the project's
/// own manifest (when available — synthetic designs in tests pass `None`).
pub fn embed_manifest<'a>(
    views: &'a [RenderedView],
    project: Option<&'a Manifest>,
) -> EmbedManifest<'a> {
    EmbedManifest {
        project,
        views: views
            .iter()
            .map(|v| EmbedManifestView {
                title: &v.title,
                filename: &v.filename,
                kind: v.kind.as_str(),
            })
            .collect(),
    }
}

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
    project_name: Option<&'a str>,
    project_version: Option<&'a str>,
    project_description: Option<&'a str>,
    views: &'a [RenderedView],
    live_reload: bool,
    /// Whether either tab has any views, so the template can hide an empty
    /// tab and pick a sensible default selection.
    has_schematic: bool,
    has_harness: bool,
    has_pinout: bool,
}

/// Render the HTML index for `views`. Shared by `render` (static, no
/// live-reload) and `serve` (in-memory, live-reload on). Views are grouped
/// into Schematics/Harnesses tabs by their `kind`.
///
/// `manifest` supplies the project header (name, version, description); pass
/// `None` for callers that don't have one (e.g. unit tests that build a
/// `Design` by hand).
pub fn index_html(
    views: &[RenderedView],
    manifest: Option<&crate::dsl::manifest::Manifest>,
    live_reload: bool,
) -> Result<String> {
    IndexTemplate {
        project_name: manifest.map(|m| m.name.as_str()),
        project_version: manifest.map(|m| m.version.as_str()),
        project_description: manifest.and_then(|m| m.description.as_deref()),
        views,
        live_reload,
        has_schematic: views.iter().any(RenderedView::is_schematic),
        has_harness: views.iter().any(RenderedView::is_harness),
        has_pinout: views.iter().any(RenderedView::is_pinout),
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
    /// Next suffix index to try per base slug.
    counts: HashMap<String, usize>,
    /// Every file name already handed out, so a disambiguated name (e.g.
    /// `overview_2.svg`) can't collide with a later title that slugs to it
    /// directly (e.g. `"Overview 2"`).
    taken: HashSet<String>,
}

impl FilenameAllocator {
    fn svg_filename(&mut self, title: &str) -> String {
        let base = slug(title);
        let count = self.counts.entry(base.clone()).or_insert(0);
        loop {
            *count += 1;
            let name = if *count == 1 {
                format!("{base}.svg")
            } else {
                format!("{base}_{count}.svg")
            };
            if self.taken.insert(name.clone()) {
                return name;
            }
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

    #[test]
    fn filename_allocator_avoids_colliding_with_a_disambiguated_name() {
        // "X 2" slugs to `x_2`, the same name a second "X" gets via
        // disambiguation — so it must be pushed to `x_2_2`, not overwrite it.
        let mut filenames = FilenameAllocator::default();
        let names = [
            filenames.svg_filename("X"),
            filenames.svg_filename("X"),
            filenames.svg_filename("X 2"),
        ];
        assert_eq!(names, ["x.svg", "x_2.svg", "x_2_2.svg"]);
        let unique: HashSet<&String> = names.iter().collect();
        assert_eq!(unique.len(), names.len(), "file names must be distinct");
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
        let html = index_html(&views, None, false).unwrap();
        assert!(html.contains("<h2>HV Overview</h2>"));
        assert!(html.contains("<img src=\"hv_overview.svg\" alt=\"HV Overview\">"));
        assert!(html.contains("<img src=\"pack_detail.svg\" alt=\"Pack Detail\">"));
    }

    #[test]
    fn index_groups_views_into_kind_tabs() {
        let views = vec![
            view("Overview", "overview.svg", "schematic"),
            view("Main Harness", "main_harness.svg", "harness"),
            view("X1 Pinout", "x1_pinout.svg", "pinout"),
        ];
        let html = index_html(&views, None, false).unwrap();
        // All tab controls present when all kinds exist.
        assert!(html.contains("Schematics"));
        assert!(html.contains("Harnesses"));
        assert!(html.contains("Pinouts"));
        assert!(html.contains("id=\"tab-schematic\""));
        assert!(html.contains("id=\"tab-harness\""));
        assert!(html.contains("id=\"tab-pinout\""));
    }

    #[test]
    fn index_omits_a_tab_with_no_views() {
        let views = vec![view("Overview", "overview.svg", "schematic")];
        let html = index_html(&views, None, false).unwrap();
        assert!(html.contains("id=\"tab-schematic\""));
        assert!(!html.contains("id=\"tab-harness\""));
        assert!(!html.contains("id=\"tab-pinout\""));
    }

    #[test]
    fn index_escapes_author_supplied_titles() {
        let views = vec![view("A & B <hv>", "a_b_hv.svg", "schematic")];
        let html = index_html(&views, None, false).unwrap();
        // Askama auto-escapes (with numeric entities); the author-supplied
        // angle brackets must not survive as a literal tag.
        assert!(!html.contains("<hv>"));
        assert!(!html.contains("A & B"));
    }

    #[test]
    fn index_notes_a_design_with_no_views() {
        assert!(index_html(&[], None, false).unwrap().contains("no views"));
    }

    #[test]
    fn manifest_lists_each_view_with_title_filename_kind() {
        let views = vec![
            view("HV Overview", "hv_overview.svg", "schematic"),
            view("Main Harness", "main_harness.svg", "harness"),
            view("X1 Pinout", "x1_pinout.svg", "pinout"),
        ];
        let manifest = embed_manifest(&views, None);
        let json = serde_json::to_string(&manifest).expect("serializes");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("re-parses");

        assert!(parsed["project"].is_null());
        let entries = parsed["views"].as_array().expect("views array");
        assert_eq!(entries.len(), 3);
        // Order matches the input (view declaration order from the design).
        assert_eq!(entries[0]["title"], "HV Overview");
        assert_eq!(entries[0]["filename"], "hv_overview.svg");
        assert_eq!(entries[0]["kind"], "schematic");
        assert_eq!(entries[1]["title"], "Main Harness");
        assert_eq!(entries[1]["filename"], "main_harness.svg");
        assert_eq!(entries[1]["kind"], "harness");
        assert_eq!(entries[2]["title"], "X1 Pinout");
        assert_eq!(entries[2]["filename"], "x1_pinout.svg");
        assert_eq!(entries[2]["kind"], "pinout");
    }

    #[test]
    fn live_reload_flag_toggles_the_websocket_script() {
        assert!(
            index_html(&[], None, true)
                .unwrap()
                .contains("new WebSocket")
        );
        assert!(
            !index_html(&[], None, false)
                .unwrap()
                .contains("new WebSocket")
        );
    }
}
