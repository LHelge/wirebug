//! Render an elaborated [`Design`] to SVG.
//!
//! [`render_views`] is the entry point used by the CLI: it walks every
//! view in the design, resolves each to its subject instance, and
//! dispatches to the renderer named by the view's `kind`. Each view
//! renders or fails on its own: a failure is a [`ViewError`] naming the
//! view, and the other views still render.

use std::collections::{HashMap, HashSet};

use askama::Template;
use serde::Serialize;

use crate::dsl::ir::{Design, Instance, View, ViewKind};
use crate::dsl::manifest::Manifest;
use crate::error::{Error, Result, ViewError, ViewErrorKind, ViewResult};

pub mod geometry;
pub mod harness;
pub mod lint;
pub mod pdf;
pub mod pinout;
pub mod png;
pub mod schematic;
pub(crate) mod stamp;

/// How a rendered SVG carries its presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SvgMode {
    /// Self-contained: embedded `<style>` plus the bottom-right
    /// project-identity stamp.
    Standalone,
    /// Self-contained styles, but no in-drawing furniture (view title,
    /// identity stamp) — for `--pdf`, where the page header carries the
    /// title and the footer the identity, both at a fixed size instead
    /// of scaling with the view.
    Plain,
    /// Host-styled "naked" SVG for `--embed`: no `<style>`, no stamp, and
    /// a class-tagged root so a downstream stylesheet owns the look.
    Embed,
}

impl SvgMode {
    pub(crate) fn is_embed(self) -> bool {
        matches!(self, Self::Embed)
    }

    pub(crate) fn stamped(self) -> bool {
        matches!(self, Self::Standalone)
    }

    pub(crate) fn titled(self) -> bool {
        !matches!(self, Self::Plain)
    }
}

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
    /// A failure converting this view (to PNG or PDF), named after it.
    #[must_use]
    pub fn failed(&self, cause: ViewErrorKind) -> ViewError {
        cause.in_view(&self.title, &self.kind)
    }
}

/// One view's entry in what [`render_views`] returns: the rendered view, or
/// why it failed.
pub type ViewOutcome = std::result::Result<RenderedView, ViewError>;

/// Render every view in `design` to SVG, in declaration order: one entry
/// per view, the rendered view or why it failed.
///
/// Each view documents a component *type*; it is rendered against the
/// first instance of that type in the design (the root for a top-level
/// view). A view naming a `kind` this build can't render, a subject type
/// with no instance, an invalid grid, or a wire the router cannot route
/// fails that view alone, as a [`ViewError`] naming it. File names are
/// handed out to failed views too, so one failure never renames another
/// view's file.
///
/// `mode` picks the presentation each SVG carries: [`SvgMode::Standalone`]
/// for self-contained SVGs intended to render on their own,
/// [`SvgMode::Plain`] for the PDF export (page furniture replaces the
/// title and stamp), or [`SvgMode::Embed`] for host-styled naked SVGs.
#[must_use]
pub fn render_views(design: &Design, mode: SvgMode) -> Vec<ViewOutcome> {
    let mut filenames = FilenameAllocator::default();
    design
        .views
        .iter()
        .map(|view| {
            let filename = filenames.svg_filename(&view.title);
            render_view(design, view, mode)
                .map(|svg| RenderedView {
                    title: view.title.clone(),
                    filename,
                    kind: view.kind.clone(),
                    svg,
                })
                .map_err(|cause| cause.in_view(&view.title, &view.kind))
        })
        .collect()
}

/// Render one view to SVG with the renderer its kind names.
fn render_view(design: &Design, view: &View, mode: SvgMode) -> ViewResult<String> {
    let subject = subject_instance(design, view)?;
    match &view.kind {
        ViewKind::Schematic => schematic::SchematicRenderer.render(design, subject, view, mode),
        ViewKind::Harness => harness::HarnessRenderer.render(design, subject, view, mode),
        ViewKind::Pinout => pinout::PinoutRenderer.render(design, subject, view, mode),
        ViewKind::Other(other) => Err(ViewErrorKind::UnknownViewKind(other.clone())),
    }
}

/// File name for the HTML index page written alongside the per-view SVGs.
pub const INDEX_FILENAME: &str = "index.html";

/// File name for the JSON sidecar written instead of the HTML index when
/// rendering in embed mode.
pub const EMBED_MANIFEST_FILENAME: &str = "manifest.json";

/// File name for the companion stylesheet written alongside the embed
/// manifest.
pub const EMBED_STYLESHEET_FILENAME: &str = "wirebug.css";

/// The embed-mode companion stylesheet: every renderer's built-in rules,
/// re-scoped under the class its embed root carries (`.wirebug-schematic`
/// etc.), so linking it reproduces the standalone look. It is a starting
/// point for theming — a host copies it into its own assets and edits the
/// copy; the embed output directory is generated and disposable.
#[must_use]
pub fn embed_stylesheet() -> String {
    let sections = [
        ("schematic", schematic::STYLE),
        ("harness", harness::STYLE),
        ("pinout", pinout::STYLE),
    ];
    let mut css = String::new();
    for (kind, style) in sections {
        // Each built-in STYLE is one `selector { body }` rule per line.
        for rule in style.lines() {
            css.push_str(&format!(".wirebug-{kind} {rule}\n"));
        }
    }
    css
}

/// Serialized shape of [`EMBED_MANIFEST_FILENAME`]: the project's identity
/// (or `null` for a synthetic design) plus the list of rendered views in
/// declaration order — only views that rendered, so a host never looks for
/// a file that was not written. Borrows from the data so a host needs only
/// to call `serde_json::to_string_pretty(&embed_manifest(...))`.
#[derive(Debug, Serialize)]
pub struct EmbedManifest<'a> {
    pub project: Option<&'a Manifest>,
    /// The wirebug that produced these SVGs (`wirebug v<version>`). Embed
    /// SVGs suppress the in-drawing stamp, so the generator identity lives
    /// here instead.
    pub generator: &'static str,
    /// File name of the companion stylesheet (a sibling of the manifest),
    /// so a host discovers it the same way it discovers views.
    pub stylesheet: &'static str,
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
    views: impl IntoIterator<Item = &'a RenderedView>,
    project: Option<&'a Manifest>,
) -> EmbedManifest<'a> {
    EmbedManifest {
        project,
        generator: concat!("wirebug v", env!("CARGO_PKG_VERSION")),
        stylesheet: EMBED_STYLESHEET_FILENAME,
        views: views
            .into_iter()
            .map(|v| EmbedManifestView {
                title: &v.title,
                filename: &v.filename,
                kind: v.kind.as_str(),
            })
            .collect(),
    }
}

/// The HTML index that embeds every rendered view, in render order, each
/// under its title, and shows a failed view's error where its drawing
/// would be. The SVGs are referenced by their sibling file names, so the
/// page expects to live in the same directory as them (or be served with
/// each SVG reachable at its file name).
///
/// `live_reload` injects the `serve` websocket client script; `render` leaves
/// it off for a self-contained static page. The template auto-escapes the
/// author-supplied titles, the error messages, and the slugged file names.
#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate<'a> {
    project_name: Option<&'a str>,
    project_version: Option<&'a str>,
    project_description: Option<&'a str>,
    /// Whether the design has no views at all.
    no_views: bool,
    /// One tab per view kind that has a view, in [`IndexPanel::TABS`] order.
    panels: Vec<IndexPanel<'a>>,
    live_reload: bool,
}

/// One tab of the HTML index: the views of one kind, rendered or failed.
struct IndexPanel<'a> {
    /// The kind's name, as `ViewKind::as_str` spells it; keys the tab's
    /// element ids and CSS.
    id: &'static str,
    label: &'static str,
    views: Vec<&'a ViewOutcome>,
}

impl IndexPanel<'_> {
    /// The kinds that get a tab, with their labels. A view of any other
    /// kind never renders, and `check` rejects it before render runs.
    const TABS: [(&'static str, &'static str); 3] = [
        ("schematic", "Schematics"),
        ("harness", "Harnesses"),
        ("pinout", "Pinouts"),
    ];
}

/// Render the HTML index for `views` (as [`render_views`] returns them).
/// Shared by `render` (static, no live-reload) and `serve` (in-memory,
/// live-reload on). Views are grouped into Schematics/Harnesses/Pinouts
/// tabs by their `kind`; a failed view keeps its place and shows its error.
///
/// `manifest` supplies the project header (name, version, description); pass
/// `None` for callers that don't have one (e.g. unit tests that build a
/// `Design` by hand).
pub fn index_html(
    views: &[ViewOutcome],
    manifest: Option<&crate::dsl::manifest::Manifest>,
    live_reload: bool,
) -> Result<String> {
    let of_kind = |id: &str, view: &ViewOutcome| {
        let kind = match view {
            Ok(view) => &view.kind,
            Err(failure) => &failure.view_kind,
        };
        kind.as_str() == id
    };
    let panels = IndexPanel::TABS
        .into_iter()
        .map(|(id, label)| IndexPanel {
            id,
            label,
            views: views.iter().filter(|view| of_kind(id, view)).collect(),
        })
        .filter(|panel| !panel.views.is_empty())
        .collect();
    IndexTemplate {
        project_name: manifest.map(|m| m.name.as_str()),
        project_version: manifest.map(|m| m.version.as_str()),
        project_description: manifest.and_then(|m| m.description.as_deref()),
        no_views: views.is_empty(),
        panels,
        live_reload,
    }
    .render()
    .map_err(Error::Template)
}

/// File name for the single-file PDF export: the project name slugged,
/// `views.pdf` for a manifest-less (synthetic) design.
#[must_use]
pub fn pdf_filename(manifest: Option<&Manifest>) -> String {
    let stem = manifest
        .map(|m| slug(&m.name))
        .unwrap_or_else(|| "views".to_string());
    format!("{stem}.pdf")
}

/// The instance a view renders against: the first one whose type matches
/// the view's subject. Synthetic inline-connector instances are skipped —
/// their `type_name` is the inline's designator, which could shadow a real
/// component type, and an inline is never a view subject.
fn subject_instance<'d>(design: &'d Design, view: &View) -> ViewResult<&'d Instance> {
    design
        .instances
        .values()
        .find(|inst| inst.inline.is_none() && inst.type_name == view.subject)
        .ok_or_else(|| ViewErrorKind::UnknownSubject {
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
    use crate::render::geometry::Side;

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
    fn pdf_filename_slugs_the_project_name() {
        let manifest = Manifest {
            name: "Aphid EVPack".to_string(),
            version: "0.1.0".to_string(),
            description: None,
            authors: Vec::new(),
            license: None,
            revision: None,
            date: None,
        };
        assert_eq!(pdf_filename(Some(&manifest)), "aphid_evpack.pdf");
        assert_eq!(pdf_filename(None), "views.pdf");
    }

    #[test]
    fn index_references_each_view_by_title_and_file() {
        let views = vec![
            Ok(view("HV Overview", "hv_overview.svg", "schematic")),
            Ok(view("Pack Detail", "pack_detail.svg", "schematic")),
        ];
        let html = index_html(&views, None, false).unwrap();
        assert!(html.contains("<h2>HV Overview</h2>"));
        assert!(html.contains("<img src=\"hv_overview.svg\" alt=\"HV Overview\">"));
        assert!(html.contains("<img src=\"pack_detail.svg\" alt=\"Pack Detail\">"));
    }

    #[test]
    fn index_groups_views_into_kind_tabs() {
        let views = vec![
            Ok(view("Overview", "overview.svg", "schematic")),
            Ok(view("Main Harness", "main_harness.svg", "harness")),
            Ok(view("X1 Pinout", "x1_pinout.svg", "pinout")),
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
        let views = vec![Ok(view("Overview", "overview.svg", "schematic"))];
        let html = index_html(&views, None, false).unwrap();
        assert!(html.contains("id=\"tab-schematic\""));
        assert!(!html.contains("id=\"tab-harness\""));
        assert!(!html.contains("id=\"tab-pinout\""));
    }

    #[test]
    fn index_escapes_author_supplied_titles() {
        let views = vec![Ok(view("A & B <hv>", "a_b_hv.svg", "schematic"))];
        let html = index_html(&views, None, false).unwrap();
        // Askama auto-escapes (with numeric entities); the author-supplied
        // angle brackets must not survive as a literal tag.
        assert!(!html.contains("<hv>"));
        assert!(!html.contains("A & B"));
    }

    #[test]
    fn index_shows_a_failed_view_in_place_of_its_drawing() {
        let failure = ViewErrorKind::UnknownSubject {
            subject: "Gone".to_string(),
        }
        .in_view("Broken <view>", &ViewKind::Harness);
        let views = vec![
            Ok(view("Overview", "overview.svg", "schematic")),
            Err(failure),
            Ok(view("Main Harness", "main_harness.svg", "harness")),
        ];
        let html = index_html(&views, None, false).unwrap();
        // The other views still show their drawings.
        assert!(html.contains("<img src=\"overview.svg\""));
        assert!(html.contains("<img src=\"main_harness.svg\""));
        // The failure sits in its own kind's tab, before the view after it,
        // titled and explained, escaped like any author text.
        let harness_tab = &html[html.find("panel-harness").expect("harness tab")..];
        let failed = harness_tab.find("class=\"failed\"").expect("failed view");
        assert!(failed < harness_tab.find("main_harness.svg").expect("next view"));
        assert!(harness_tab.contains("<h2>Broken &#60;view&#62;</h2>"));
        assert!(harness_tab.contains("has no instance in the design"));
    }

    /// A design with two boxes wired together, and one view per title in
    /// `views`; a view titled `Lost …` documents a type with no instance.
    fn design_with_views(titles: &[&str]) -> Design {
        let mut design = schematic::tests::design_from(
            r#"
component sys {
    a: alpha;
    b: beta;
    wire red 1 [a.p, b.p];
    component alpha {
        pub port p "Out" pin 1;
    }
    component beta {
        pub port p "In" pin 1;
    }
}
"#,
        );
        design.views = titles
            .iter()
            .map(|title| {
                let subject = if title.starts_with("Lost") {
                    "gone"
                } else {
                    "sys"
                };
                let mut view = schematic::tests::view_of(
                    subject,
                    &[
                        ("a", 0.0, 0.0, &[("p", Side::East)]),
                        ("b", 16.0, 0.0, &[("p", Side::West)]),
                    ],
                );
                view.title = title.to_string();
                view
            })
            .collect();
        design
    }

    #[test]
    fn a_failing_view_leaves_the_others_rendered() {
        let design = design_with_views(&["Overview", "Lost", "Detail"]);
        let views = render_views(&design, SvgMode::Standalone);

        let [Ok(first), Err(failure), Ok(last)] = views.as_slice() else {
            panic!("expected rendered, failed, rendered: {views:?}");
        };
        assert_eq!(first.title, "Overview");
        assert!(first.svg.contains("<svg"));
        assert_eq!(last.title, "Detail");
        assert!(last.svg.contains("<svg"));
        assert_eq!(failure.title, "Lost");
        assert_eq!(failure.view_kind, ViewKind::Schematic);
        assert_eq!(
            failure.to_string(),
            "view \"Lost\": view subject \"gone\" has no instance in the design"
        );
    }

    #[test]
    fn a_failed_view_keeps_its_file_name_so_no_other_view_is_renamed() {
        let mut design = design_with_views(&["Overview", "Lost", "Overview"]);
        // One title throughout: the failing second view still takes
        // `overview_2.svg`, so the third keeps the name it has when all
        // three render.
        design.views[1].title = "Overview".to_string();
        let names: Vec<_> = render_views(&design, SvgMode::Standalone)
            .into_iter()
            .flatten()
            .map(|view| view.filename)
            .collect();
        assert_eq!(names, ["overview.svg", "overview_3.svg"]);
    }

    #[test]
    fn a_pdf_is_written_unless_every_view_failed() {
        let exporter = pdf::PdfExporter::new();
        let export = |titles: &[&str]| {
            let design = design_with_views(titles);
            exporter.to_pdf(render_views(&design, SvgMode::Plain), None)
        };

        let none_rendered = export(&["Lost", "Lost Too"]);
        assert_eq!(none_rendered.pages, 0);
        let failed: Vec<&str> = none_rendered
            .failures
            .iter()
            .map(|f| f.title.as_str())
            .collect();
        assert_eq!(failed, ["Lost", "Lost Too"]);
        assert!(none_rendered.document().is_none(), "nothing to write");

        let some_rendered = export(&["Lost", "Overview"]);
        assert_eq!(some_rendered.pages, 1);
        assert_eq!(some_rendered.failures.len(), 1);
        let document = some_rendered.document().expect("the rendered page");
        assert!(document.starts_with(b"%PDF-"));

        // A design without views has nothing that failed: its empty
        // document is still written, as before.
        assert!(export(&[]).document().is_some());
    }

    #[test]
    fn index_notes_a_design_with_no_views() {
        assert!(index_html(&[], None, false).unwrap().contains("no views"));
    }

    #[test]
    fn embed_stylesheet_scopes_every_rule_under_its_kind() {
        let css = embed_stylesheet();
        // One section per renderer, each rule prefixed with its embed
        // root's class.
        assert!(css.contains(".wirebug-schematic .component rect {"));
        assert!(css.contains(".wirebug-harness .cable-wire {"));
        assert!(css.contains(".wirebug-pinout .cavity {"));
        // No rule escapes its scope.
        for line in css.lines().filter(|l| !l.is_empty()) {
            assert!(
                line.starts_with(".wirebug-"),
                "unscoped stylesheet rule: {line}"
            );
        }
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
        assert_eq!(
            parsed["generator"],
            concat!("wirebug v", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(parsed["stylesheet"], EMBED_STYLESHEET_FILENAME);
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
