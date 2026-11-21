//! Render an elaborated [`Design`] to SVG.
//!
//! [`render_views`] is the entry point used by the CLI: it walks every
//! view in the design, resolves each to its subject instance, and
//! dispatches to the renderer named by the view's `kind`. Each view
//! renders or fails on its own: a failure is a [`ViewError`] naming the
//! view, and the other views still render.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

use askama::Template;
use serde::Serialize;
use svg::Document;

use crate::dsl::ir::{Design, Instance, View, ViewKind};
use crate::dsl::manifest::Manifest;
use crate::error::{Error, Result, ViewError, ViewErrorKind, ViewResult};
use crate::export::{Format as ExportFormat, PinTables, Table};
use crate::render::geometry::Point;

pub(crate) mod fonts;
pub mod geometry;
pub mod harness;
pub(crate) mod lint;
pub mod output;
pub mod pdf;
pub mod pinout;
pub mod png;
pub mod schematic;
pub(crate) mod stamp;
pub(crate) mod wire_stroke;

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

    /// The manifest whose identity stamp `design`'s views carry in this
    /// mode, if they carry one.
    pub(crate) fn stamp(self, design: &Design) -> Option<&Manifest> {
        match self {
            Self::Standalone => design.manifest.as_ref(),
            Self::Plain | Self::Embed => None,
        }
    }

    /// Whether `view`, drawn in this mode, shows its title.
    pub(crate) fn shows_title(self, view: &View) -> bool {
        !view.title.is_empty() && !matches!(self, Self::Plain)
    }

    /// `root`, a view's `<svg>`, given the viewBox that starts at `origin`
    /// and spans `canvas`.
    ///
    /// Outside embed mode the root also carries the canvas as its
    /// intrinsic `width` and `height`, one world unit to the px, so a
    /// browser shows every view at the same text size rather than scaling
    /// each to its container. Equal to the viewBox's size, they leave the
    /// size the rasteriser and the PDF export read (`usvg::Tree::size`)
    /// as it was. An embed root keeps the viewBox alone: its host's CSS
    /// sizes it, and [`EmbedManifestView`] offers the canvas as data.
    pub(crate) fn frame(self, root: Document, origin: Point, canvas: Canvas) -> Document {
        let Canvas { width, height } = canvas;
        let root = root.set(
            "viewBox",
            format!("{} {} {width} {height}", origin.x, origin.y),
        );
        if self.is_embed() {
            root
        } else {
            root.set("width", width.to_string())
                .set("height", height.to_string())
        }
    }
}

/// A view drawn to SVG: the document, and the canvas its viewBox spans.
#[derive(Debug)]
pub(crate) struct Drawing {
    svg: String,
    canvas: Canvas,
}

impl Drawing {
    /// `root`, the finished `<svg>` of a view, on `canvas`.
    pub(crate) fn new(root: Document, canvas: Canvas) -> Self {
        Self {
            svg: root.to_string(),
            canvas,
        }
    }
}

/// One rendered view: the SVG document, the file name it should be
/// written to (a slug of the view title, without a directory), the human
/// title it was rendered from, the view `kind` (so the HTML index can
/// group views into tabs), and the canvas its viewBox spans (so the embed
/// manifest can give each view's natural size).
#[derive(Debug)]
#[must_use]
pub struct RenderedView {
    pub title: String,
    pub filename: String,
    pub kind: ViewKind,
    pub svg: String,
    pub canvas: Canvas,
}

impl RenderedView {
    /// A failure converting this view (to PNG or PDF), named after it.
    #[must_use]
    pub fn failed(&self, cause: ViewErrorKind) -> ViewError {
        cause.in_view(&self.title, &self.kind)
    }

    /// The smallest text a view of this kind can carry, in SVG user units
    /// (px), or `None` for a kind no renderer draws.
    ///
    /// Read from the renderer's own stylesheet — the smallest `font` size
    /// in any of its rules — plus, for a pinout, the floor its cavity
    /// labels shrink to, the one text sized outside the stylesheet. So a
    /// rule added with a smaller font is counted without being listed
    /// here. It is the kind's smallest, not this drawing's: a schematic
    /// without wire codes is judged by their size all the same.
    #[must_use]
    pub fn smallest_text(&self) -> Option<f64> {
        let (style, shrinks_to) = match &self.kind {
            ViewKind::Schematic => (schematic::STYLE, None),
            ViewKind::Harness => (harness::STYLE, None),
            ViewKind::Pinout => (pinout::STYLE, Some(pinout::MIN_LABEL_SIZE)),
            ViewKind::Other(_) => return None,
        };
        font_sizes(style).chain(shrinks_to).reduce(f64::min)
    }
}

/// Every font size, in px, that the rules of `style` set.
fn font_sizes(style: &str) -> impl Iterator<Item = f64> + '_ {
    font_declarations(style).flat_map(px_sizes)
}

/// The value of every `font` shorthand and `font-size` declaration in the
/// rules of `style`.
fn font_declarations(style: &str) -> impl Iterator<Item = &str> {
    style
        .split(['{', '}'])
        .skip(1)
        .step_by(2)
        .flat_map(|body| body.split(';'))
        .filter_map(|declaration| declaration.split_once(':'))
        .filter(|(property, _)| matches!(property.trim(), "font" | "font-size"))
        .map(|(_, value)| value)
}

/// The px sizes in a font declaration's `value`: `11px`, or `11px/1.2`
/// with a line height. Sizes in other units are not read — a test holds
/// every built-in stylesheet to one px size per declaration.
fn px_sizes(value: &str) -> impl Iterator<Item = f64> + '_ {
    value.split_whitespace().filter_map(|token| {
        let size = token.split('/').next()?;
        size.strip_suffix("px")?.parse().ok()
    })
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
    let pin_tables = OnceCell::new();
    design
        .views
        .iter()
        .map(|view| {
            let filename = filenames.svg_filename(&view.title);
            render_view(design, view, mode, &pin_tables)
                .map(|Drawing { svg, canvas }| RenderedView {
                    title: view.title.clone(),
                    filename,
                    kind: view.kind.clone(),
                    svg,
                    canvas,
                })
                .map_err(|cause| cause.in_view(&view.title, &view.kind))
        })
        .collect()
}

/// Render one view to SVG with the renderer its kind names. A pinout
/// reads the design's pin tables, built in `pin_tables` by the first
/// pinout view of a run and shared by the rest.
fn render_view<'d>(
    design: &'d Design,
    view: &View,
    mode: SvgMode,
    pin_tables: &OnceCell<PinTables<'d>>,
) -> ViewResult<Drawing> {
    let subject = subject_instance(design, view)?;
    match &view.kind {
        ViewKind::Schematic => schematic::SchematicRenderer.render(design, subject, view, mode),
        ViewKind::Harness => harness::HarnessRenderer.render(design, subject, view, mode),
        ViewKind::Pinout => {
            let tables = pin_tables.get_or_init(|| PinTables::from(design));
            pinout::PinoutRenderer.render(design, tables, subject, view, mode)
        }
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
/// a file that was not written — and the build tables written beside them.
/// Borrows from the data so a host needs only to call
/// `serde_json::to_string_pretty(&embed_manifest(...))`.
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
    /// The build tables, as `wirebug export <table> --format json` writes
    /// them, by file name beside the manifest. Last, so the fields before
    /// it keep their place for hosts that read the manifest.
    pub exports: EmbedExports,
}

/// [`EmbedManifest::exports`]: the file name of each build table's JSON,
/// a sibling of the manifest.
#[derive(Debug, Serialize)]
pub struct EmbedExports {
    /// The wire list's JSON (`wires.json`).
    pub wires: String,
    /// The connector pin tables' JSON (`connectors.json`).
    pub connectors: String,
    /// The bill of materials' JSON (`bom.json`).
    pub bom: String,
}

impl Default for EmbedExports {
    fn default() -> Self {
        Self {
            wires: Table::Wires.filename(ExportFormat::Json),
            connectors: Table::Connectors.filename(ExportFormat::Json),
            bom: Table::Bom.filename(ExportFormat::Json),
        }
    }
}

/// One row of [`EmbedManifest::views`].
#[derive(Debug, Serialize)]
pub struct EmbedManifestView<'a> {
    pub title: &'a str,
    pub filename: &'a str,
    pub kind: &'a str,
    /// The view's natural width: its viewBox's, in world units. An embed
    /// SVG carries no intrinsic size, so a host that wants one world unit
    /// to the px (the text size of standalone output) sets it from here.
    /// After the fields hosts already read, so they keep their place.
    pub width: f64,
    /// The view's natural height, as [`Self::width`].
    pub height: f64,
}

/// Build the embed-mode manifest from the rendered views and the project's
/// own manifest (when available — synthetic designs in tests pass `None`).
pub fn embed_manifest<'a>(
    views: impl IntoIterator<Item = &'a RenderedView>,
    project: Option<&'a Manifest>,
) -> EmbedManifest<'a> {
    EmbedManifest {
        project,
        generator: crate::GENERATOR,
        stylesheet: EMBED_STYLESHEET_FILENAME,
        views: views
            .into_iter()
            .map(|v| EmbedManifestView {
                title: &v.title,
                filename: &v.filename,
                kind: v.kind.as_str(),
                width: v.canvas.width(),
                height: v.canvas.height(),
            })
            .collect(),
        exports: EmbedExports::default(),
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
    /// The tab's kind; its name (`ViewKind::as_str`) keys the tab's element
    /// ids and CSS.
    kind: ViewKind,
    label: &'static str,
    views: Vec<&'a ViewOutcome>,
}

impl IndexPanel<'_> {
    /// The kinds that get a tab, with their labels. A view of any other
    /// kind never renders, and `check` rejects it before render runs.
    const TABS: [(ViewKind, &'static str); 3] = [
        (ViewKind::Schematic, "Schematics"),
        (ViewKind::Harness, "Harnesses"),
        (ViewKind::Pinout, "Pinouts"),
    ];

    /// The tab's name in element ids and CSS classes.
    fn id(&self) -> &str {
        self.kind.as_str()
    }
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
    let of_kind = |kind: &ViewKind, view: &ViewOutcome| match view {
        Ok(view) => &view.kind == kind,
        Err(failure) => &failure.view_kind == kind,
    };
    let panels = IndexPanel::TABS
        .into_iter()
        .map(|(kind, label)| IndexPanel {
            views: views.iter().filter(|view| of_kind(&kind, view)).collect(),
            kind,
            label,
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

/// The instance a view renders against ([`Design::subject_of`]).
fn subject_instance<'d>(design: &'d Design, view: &View) -> ViewResult<&'d Instance> {
    design
        .subject_of(view)
        .ok_or_else(|| ViewErrorKind::UnknownSubject {
            subject: view.subject.to_string(),
        })
}

/// Why a renderer cannot draw a view on the grid its `grid:` asks for. Each
/// renderer's own grid validation returns it, so `render` (through
/// [`ViewErrorKind`]) and `check` (through `lint`) reject the same grids.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum GridFault {
    /// Zero or negative.
    NonPositive { grid: f64 },
    /// Finer than the renderer's `minimum` step.
    TooFine { grid: f64, minimum: f64 },
}

impl GridFault {
    /// The step `view` is drawn on — its `grid:`, or the renderer's
    /// `default` when it names none — unless it is not positive.
    pub(crate) fn positive_step(view: &View, default: f64) -> std::result::Result<f64, Self> {
        let step = view.grid.map_or(default, |grid| grid.step);
        if step <= 0.0 {
            Err(Self::NonPositive { grid: step })
        } else {
            Ok(step)
        }
    }
}

/// The longest side, in world units, a view's canvas (its viewBox) may
/// have.
///
/// `--png` allocates its whole pixmap up front, 4 bytes a pixel, so a view
/// whose includes sit far apart would ask for terabytes and abort the
/// process. At [`png::PNG_SCALE`] this is 16384 px a side, a pixmap of at
/// most 1 GiB, which still holds an A0 sheet at 300 dpi (14043 px on its
/// long edge) and is almost four times the widest view of the example
/// project (about 2200 units). SVG and PDF output have no such cost, but
/// `check` holds every view to the limit, so a project that passes it
/// renders in every mode.
pub(crate) const MAX_CANVAS: f64 = 8192.0;

/// A view's canvas: its viewBox's width and height, in world units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canvas {
    width: f64,
    height: f64,
}

impl Canvas {
    /// A `width` × `height` canvas.
    #[must_use]
    pub fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    /// Whether neither side is longer than [`MAX_CANVAS`]. Compared as
    /// `f32`, the precision the rasteriser reads the viewBox at, so `check`
    /// and `--png` judge the very same numbers.
    pub(crate) fn fits(&self) -> bool {
        let limit = MAX_CANVAS as f32;
        self.width as f32 <= limit && self.height as f32 <= limit
    }

    /// Whether neither side is longer than `other`'s.
    pub(crate) fn within(&self, other: &Canvas) -> bool {
        self.width <= other.width && self.height <= other.height
    }

    /// Why the canvas cannot be rasterised, when it does not fit.
    pub(crate) fn admit(self) -> ViewResult<()> {
        if self.fits() {
            Ok(())
        } else {
            Err(ViewErrorKind::CanvasTooLarge {
                width: self.width,
                height: self.height,
                limit: MAX_CANVAS,
            })
        }
    }

    /// The canvas's width, in world units.
    #[must_use]
    pub fn width(&self) -> f64 {
        self.width
    }

    /// The canvas's height, in world units.
    #[must_use]
    pub fn height(&self) -> f64 {
        self.height
    }
}

impl From<GridFault> for ViewErrorKind {
    fn from(fault: GridFault) -> Self {
        match fault {
            GridFault::NonPositive { grid } => Self::NonPositiveGrid { grid },
            GridFault::TooFine { grid, minimum } => Self::GridTooSmall { grid, minimum },
        }
    }
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
pub(crate) mod tests {
    use super::*;
    use crate::render::geometry::Side;

    /// The value of `name` on `svg`'s root `<svg>` element, as written.
    pub(crate) fn root_attribute<'s>(svg: &'s str, name: &str) -> Option<&'s str> {
        let start = svg.find("<svg ")?;
        let tag = &svg[start..start + svg[start..].find('>')?];
        let key = format!(" {name}=\"");
        let value = &tag[tag.find(&key)? + key.len()..];
        Some(&value[..value.find('"')?])
    }

    /// Assert that `svg`'s root carries its viewBox's width and height as
    /// its own `width` and `height`, written as the viewBox writes them.
    pub(crate) fn assert_sized_by_its_viewbox(svg: &str) {
        let viewbox: Vec<&str> = root_attribute(svg, "viewBox")
            .expect("a viewBox")
            .split(' ')
            .collect();
        assert_eq!(root_attribute(svg, "width"), Some(viewbox[2]), "{svg}");
        assert_eq!(root_attribute(svg, "height"), Some(viewbox[3]), "{svg}");
    }

    /// Assert that `svg`'s root has a viewBox but no intrinsic size.
    pub(crate) fn assert_unsized(svg: &str) {
        assert!(root_attribute(svg, "viewBox").is_some(), "{svg}");
        assert_eq!(root_attribute(svg, "width"), None, "{svg}");
        assert_eq!(root_attribute(svg, "height"), None, "{svg}");
    }

    /// `svg` with its root's `width` and `height` taken off again.
    fn without_size(svg: &str) -> String {
        let mut svg = svg.to_string();
        for name in ["width", "height"] {
            let value = root_attribute(&svg, name)
                .expect("a sized root")
                .to_string();
            // The root is the document's first element, so the first match
            // is its own attribute.
            svg = svg.replacen(&format!(" {name}=\"{value}\""), "", 1);
        }
        assert_unsized(&svg);
        svg
    }

    #[test]
    fn a_frame_sizes_the_root_by_its_viewbox_except_in_embed_mode() {
        // A fractional side, and one whose shortest form is long: the size
        // is written exactly as the viewBox writes it.
        let canvas = Canvas::new(1542.5, 0.1 + 0.2);
        let root = |mode: SvgMode| {
            mode.frame(Document::new(), Point::new(-88.0, -48.0), canvas)
                .to_string()
        };
        for mode in [SvgMode::Standalone, SvgMode::Plain] {
            let svg = root(mode);
            assert_eq!(
                root_attribute(&svg, "viewBox"),
                Some("-88 -48 1542.5 0.30000000000000004")
            );
            assert_sized_by_its_viewbox(&svg);
        }
        let embed = root(SvgMode::Embed);
        assert_eq!(
            root_attribute(&embed, "viewBox"),
            Some("-88 -48 1542.5 0.30000000000000004")
        );
        assert_unsized(&embed);
    }

    #[test]
    fn a_rendered_view_carries_the_canvas_its_viewbox_spans() {
        let design = design_with_views(&["Overview"]);
        for mode in [SvgMode::Standalone, SvgMode::Plain, SvgMode::Embed] {
            let views = render_views(&design, mode);
            let [Ok(view)] = views.as_slice() else {
                panic!("expected one rendered view: {views:?}");
            };
            let viewbox = root_attribute(&view.svg, "viewBox").expect("a viewBox");
            let size = format!("{} {}", view.canvas.width(), view.canvas.height());
            assert!(viewbox.ends_with(&size), "{viewbox} spans {size}");
        }
    }

    /// The root's intrinsic size equals its viewBox's, so `usvg` reads the
    /// same size (`Tree::size`) and the same one world unit per size unit
    /// as before it was written: the PNG's pixels, the PDF's bytes and its
    /// legibility check all stay as they were.
    #[test]
    fn the_intrinsic_size_leaves_png_and_pdf_output_as_they_were() {
        let mut design = design_with_views(&["Wide"]);
        // Far enough apart that the text prints too small on A4, so the
        // legibility check has something to say.
        design.views[0].includes[1].x = 400.0;

        let views = render_views(&design, SvgMode::Standalone);
        let [Ok(view)] = views.as_slice() else {
            panic!("expected one rendered view: {views:?}");
        };
        let rasterizer = png::PngRasterizer::new();
        let raster = |svg: &str| rasterizer.to_png(svg, png::PNG_SCALE).expect("rasterises");
        let sized = raster(&view.svg);
        assert_eq!(sized, raster(&without_size(&view.svg)));
        // The pixel size, from the PNG's header, is the canvas × the scale.
        let side = |at: usize| u32::from_be_bytes(sized[at..at + 4].try_into().expect("4 bytes"));
        let pixels = |units: f64| (units * f64::from(png::PNG_SCALE)).round() as u32;
        assert_eq!(
            (side(16), side(20)),
            (pixels(view.canvas.width()), pixels(view.canvas.height()))
        );

        let exporter = pdf::PdfExporter::new();
        let export = |strip: bool| {
            let views = render_views(&design, SvgMode::Plain)
                .into_iter()
                .map(|view| {
                    view.map(|view| RenderedView {
                        svg: if strip {
                            without_size(&view.svg)
                        } else {
                            view.svg
                        },
                        ..view
                    })
                })
                .collect();
            exporter.to_pdf(views, None, pdf::Paper::A4)
        };
        let (sized, before) = (export(false), export(true));
        assert_eq!(sized.document(), before.document());
        assert!(!sized.small_text.is_empty(), "a wide view prints small");
        assert_eq!(sized.small_text, before.small_text);
    }

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
            canvas: Canvas::new(400.0, 300.0),
        }
    }

    #[test]
    fn font_sizes_reads_font_and_font_size_declarations_only() {
        let style = "\
.a { font: bold 13px sans-serif; stroke-width: 3px; }
.b text { font-size: 7.5px; fill: #555; }
.c { stroke: black; }";
        assert_eq!(font_sizes(style).collect::<Vec<_>>(), [13.0, 7.5]);
    }

    #[test]
    fn px_sizes_reads_a_size_with_a_line_height_but_no_other_unit() {
        let sizes = |value| px_sizes(value).collect::<Vec<_>>();
        assert_eq!(sizes(" italic 8px/1.2 sans-serif"), [8.0]);
        assert_eq!(sizes(" 7pt"), [] as [f64; 0]);
        assert_eq!(sizes(" .6em"), [] as [f64; 0]);
    }

    /// The smallest-text check reads only px sizes, so a rule written in
    /// any other form would slip past it: every font declaration of every
    /// built-in stylesheet must carry exactly one size the check reads.
    #[test]
    fn every_built_in_font_declaration_has_one_size_the_check_reads() {
        for (kind, style) in [
            ("schematic", schematic::STYLE),
            ("harness", harness::STYLE),
            ("pinout", pinout::STYLE),
        ] {
            let declarations: Vec<&str> = font_declarations(style).collect();
            assert!(!declarations.is_empty(), "{kind} sets no font");
            for value in declarations {
                assert_eq!(
                    px_sizes(value).count(),
                    1,
                    "{kind} font `{}` must give one size in px",
                    value.trim()
                );
            }
        }
    }

    #[test]
    fn smallest_text_is_the_smallest_font_a_view_kind_draws() {
        // Schematic wire codes and harness cable labels are the smallest
        // stylesheet fonts; a pinout's cavity label can shrink below its
        // stylesheet's smallest, to the fitting floor.
        assert_eq!(view("S", "s.svg", "schematic").smallest_text(), Some(9.0));
        assert_eq!(view("H", "h.svg", "harness").smallest_text(), Some(9.0));
        assert_eq!(view("P", "p.svg", "pinout").smallest_text(), Some(6.0));
        assert_eq!(view("O", "o.svg", "bom").smallest_text(), None);
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

    /// The index shows each drawing as an `<img>`, which a standalone
    /// SVG's intrinsic size gives a natural size: the `img` rule lets it
    /// show at that size, shrinks a wider one to the container keeping its
    /// aspect, and no rule outside the zoomed lightbox stretches it.
    #[test]
    fn index_shows_each_drawing_at_its_natural_size_capped_at_the_container() {
        let views = vec![Ok(view("Overview", "overview.svg", "schematic"))];
        let html = index_html(&views, None, false).unwrap();
        assert!(html.contains("<section>"));
        assert!(html.contains("<a href=\"#lb-overview.svg\"><img src=\"overview.svg\""));
        assert!(html.contains("\n    img { max-width: 100%; height: auto; }\n"));
        let stretching = html
            .lines()
            .filter(|rule| rule.contains("img {"))
            .filter(|rule| rule.contains(" width:") || rule.contains("{ width:"));
        for rule in stretching {
            assert!(rule.trim_start().starts_with(".lightbox img"), "{rule}");
        }
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
    pub(crate) fn design_with_views(titles: &[&str]) -> Design {
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
            exporter.to_pdf(render_views(&design, SvgMode::Plain), None, pdf::Paper::A4)
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
    fn manifest_lists_each_view_with_title_filename_kind_and_size() {
        let sized = |view: RenderedView, width, height| RenderedView {
            canvas: Canvas::new(width, height),
            ..view
        };
        let views = vec![
            sized(
                view("HV Overview", "hv_overview.svg", "schematic"),
                2076.0,
                656.0,
            ),
            sized(
                view("Main Harness", "main_harness.svg", "harness"),
                1192.0,
                429.625,
            ),
            sized(view("X1 Pinout", "x1_pinout.svg", "pinout"), 645.5, 484.0),
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
        // Each view's natural size, its viewBox's, as numbers.
        assert_eq!(entries[0]["width"], 2076.0);
        assert_eq!(entries[0]["height"], 656.0);
        assert_eq!(entries[1]["width"], 1192.0);
        assert_eq!(entries[1]["height"], 429.625);
        assert_eq!(entries[2]["width"], 645.5);
        assert_eq!(entries[2]["height"], 484.0);
        // The build tables written beside the manifest, by file name; the
        // last field, after the ones hosts already read.
        assert_eq!(parsed["exports"]["wires"], "wires.json");
        assert_eq!(parsed["exports"]["connectors"], "connectors.json");
        assert_eq!(parsed["exports"]["bom"], "bom.json");
        let at = |field: &str| json.find(&format!("\"{field}\":")).expect(field);
        let order = ["project", "generator", "stylesheet", "views", "exports"].map(at);
        assert!(order.is_sorted(), "{json}");
        // Within a view entry, the size comes after the fields hosts
        // already read.
        let entry = &json[at("views")..];
        let at = |field: &str| entry.find(&format!("\"{field}\":")).expect(field);
        let order = ["title", "filename", "kind", "width", "height"].map(at);
        assert!(order.is_sorted(), "{json}");
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
