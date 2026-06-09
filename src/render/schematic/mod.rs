//! Rectangle-based SVG schematic renderer.
//!
//! Each included instance becomes a box with its ports on the sides the
//! view authors for them, and every wire segment becomes an orthogonal
//! polyline between two ports.
//!
//! - `layout` turns a design's view into positioned boxes and ports.
//! - `draw` emits the SVG for those boxes, ports, and wires.
//! - `route` finds object-avoiding orthogonal paths for the wires.

mod draw;
pub(super) mod layout;
mod route;

use svg::Document;
use svg::node::element::{Group, Style, Text};

use crate::dsl::ir::{Design, Instance, View};
use crate::error::{Error, Result};
use crate::render::stamp::{STAMP_HEIGHT, STAMP_INSET, stamp_element};

use layout::{Grid, Placement};
use route::Router;

/// Grid step (world units) used when a view doesn't specify one. Ports
/// sit two steps apart, so the default port pitch is twice this.
pub(super) const DEFAULT_GRID: f64 = 15.0;

pub(super) const MIN_WIDTH: f64 = 160.0;
pub(super) const MIN_HEIGHT: f64 = 100.0;
/// Tightest spacing between adjacent ports at which their labels and pin
/// numbers still clear each other (labels are 11px, pins 10px). Ports sit
/// two grid steps apart, so the grid step must be at least half this; a
/// finer grid errors rather than overlapping labels.
pub(super) const MIN_PORT_PITCH: f64 = 15.0;
pub(super) const LABEL_INSET: f64 = 10.0;
pub(super) const PIN_INSET: f64 = 6.0;
pub(super) const PORT_RADIUS: f64 = 4.0;
pub(super) const CHAR_WIDTH: f64 = 7.5;
pub(super) const COMPONENT_TITLE_GAP: f64 = 8.0;
pub(super) const SVG_MARGIN: f64 = 48.0;
pub(super) const TITLE_GAP: f64 = 12.0;
pub(super) const TEXT_BOX_MIN_WIDTH: f64 = 80.0;
pub(super) const TEXT_BOX_HEIGHT: f64 = 34.0;
pub(super) const TEXT_BOX_PAD_X: f64 = 12.0;

const STYLE: &str = "\
.component rect { fill: white; stroke: black; stroke-width: 1.5; }
.component-label { font: bold 13px sans-serif; text-anchor: middle; }
.port circle { fill: black; }
.port-label { font: 11px sans-serif; paint-order: stroke; stroke: white; stroke-width: 3px; stroke-linejoin: round; }
.port-pin { font: italic 10px sans-serif; fill: #555; }
.wire { fill: none; stroke: black; stroke-width: 1.25; }
.enclosure rect { fill: none; stroke: #888; stroke-width: 1.5; stroke-dasharray: 6 4; }
.enclosure-label { font: bold 13px sans-serif; text-anchor: middle; fill: #555; }
.text-box rect { fill: #fff9d6; stroke: #8a7a2f; stroke-width: 1.25; }
.text-box text { font: 12px sans-serif; fill: #2f2a13; text-anchor: middle; dominant-baseline: central; }
.title { font: bold 14px sans-serif; }
.stamp { font: 10px sans-serif; fill: #666; text-anchor: end; }\
";

/// SVG renderer for `kind: schematic` views.
#[derive(Default)]
pub struct SchematicRenderer;

impl SchematicRenderer {
    /// Render `view` (documenting `subject`) against `design` to an SVG
    /// string. Wire segments are routed against the placed boxes.
    ///
    /// `embed` switches to embed-mode output for inclusion in another
    /// document: the built-in `<style>` is dropped (the host owns the
    /// look), the bottom-right project-identity stamp is suppressed,
    /// and the root `<svg>` carries `class="wirebug wirebug-schematic"`
    /// so a host stylesheet can scope rules under `.wirebug`.
    pub(super) fn render(
        &self,
        design: &Design,
        subject: &Instance,
        view: &View,
        embed: bool,
    ) -> Result<String> {
        let step = view.grid.unwrap_or(DEFAULT_GRID);
        if step <= 0.0 {
            return Err(Error::NonPositiveGrid { grid: step });
        }
        // Ports sit two steps apart, so that pitch must clear a port
        // label or adjacent labels would overlap.
        let min_step = MIN_PORT_PITCH / 2.0;
        if step < min_step {
            return Err(Error::GridTooSmall {
                grid: step,
                minimum: min_step,
            });
        }
        let grid = Grid::new(step);
        let placement = Placement::compute(design, subject, view, grid)?;

        // Route before sizing the canvas: wires can detour outside the
        // component bounds, so the viewBox has to enclose them too.
        let router = Router::build(&placement, step);
        let pairs = placement.connection_pairs();
        // Spread parallel wires in a shared channel by the port pitch (two
        // steps), so a nudged bundle matches the spacing of the ports it
        // fans out from rather than packing twice as tight.
        let wires = router.route_all(&pairs, grid.pitch())?;

        let mut doc = Document::new().set("xmlns", "http://www.w3.org/2000/svg");
        if embed {
            doc = doc.set("class", "wirebug wirebug-schematic");
        } else {
            doc = doc.add(Style::new(STYLE));
        }

        let has_title = !view.title.is_empty();
        let mut viewbox = placement.viewbox(has_title, &wires);
        let manifest = (!embed).then_some(design.manifest.as_ref()).flatten();
        if manifest.is_some() {
            viewbox.height += STAMP_HEIGHT;
        }
        doc = doc.set(
            "viewBox",
            format!(
                "{} {} {} {}",
                viewbox.x, viewbox.y, viewbox.width, viewbox.height
            ),
        );

        if has_title {
            doc = doc.add(
                Text::new(view.title.clone())
                    .set("class", "title")
                    .set("x", viewbox.x + SVG_MARGIN)
                    .set("y", viewbox.y + SVG_MARGIN - TITLE_GAP),
            );
        }

        if let Some(manifest) = manifest {
            doc = doc.add(stamp_element(
                manifest,
                viewbox.x + viewbox.width - STAMP_INSET,
                viewbox.y + viewbox.height - STAMP_INSET,
            ));
        }

        // The enclosure is the subject's boundary; draw it behind the
        // components so its dashed wrapper reads as a backdrop.
        if let Some(enclosure) = placement.enclosure() {
            doc = doc.add(draw::render_enclosure(enclosure));
        }

        let mut components_group = Group::new().set("class", "components");
        for (name, pc) in &placement.components {
            components_group = components_group.add(draw::render_component(name, pc));
        }
        doc = doc.add(components_group);

        let mut wires_group = Group::new().set("class", "wires");
        for polyline in &wires {
            wires_group = wires_group.add(draw::render_wire(polyline));
        }
        doc = doc.add(wires_group);

        let mut texts_group = Group::new().set("class", "text-boxes");
        for text in &placement.texts {
            texts_group = texts_group.add(draw::render_text_box(text));
        }
        doc = doc.add(texts_group);

        Ok(doc.to_string())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dsl::ir::{Include, InstanceName, PortName, Side, TextBox, TypeName, ViewKind};

    /// Elaborate a single-file `.wb` source into a [`Design`]. The source
    /// must have a single top-level component (the views are built
    /// separately with [`view_of`]).
    pub(crate) fn design_from(src: &str) -> Design {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("main.wb"), src).expect("write");
        let (project, _) = crate::dsl::project::load(&dir.path().join("main.wb"));
        let project = project.expect("loads");
        let resolved = crate::dsl::resolve::resolve(&project);
        let (design, problems) = crate::dsl::elaborate::elaborate(&resolved);
        assert!(problems.is_empty(), "elaboration problems: {problems:?}");
        design.expect("a design")
    }

    /// A schematic view over `subject`, including the named instances at
    /// the given grid coordinates, each with its authored port placements
    /// `(port, side)` in order.
    #[allow(clippy::type_complexity)]
    pub(crate) fn view_of(subject: &str, includes: &[(&str, f64, f64, &[(&str, Side)])]) -> View {
        View {
            kind: ViewKind::Schematic,
            title: "T".to_string(),
            grid: None,
            subject: TypeName::from(subject),
            has_enclosure: false,
            enclosure: Vec::new(),
            includes: includes
                .iter()
                .map(|(name, x, y, ports)| Include {
                    instance: InstanceName::from(*name),
                    connector: None,
                    x: *x,
                    y: *y,
                    ports: ports
                        .iter()
                        .map(|(p, side)| (PortName::from(*p), *side))
                        .collect(),
                })
                .collect(),
            texts: Vec::new(),
        }
    }

    fn render(design: &Design, view: &View) -> Result<String> {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        SchematicRenderer.render(design, subject, view, false)
    }

    fn two_box_design() -> Design {
        design_from(
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
        )
    }

    #[test]
    fn render_contains_expected_fragments() {
        let design = two_box_design();
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let svg = render(&design, &view).expect("renders");

        assert!(svg.contains("<svg"));
        assert!(svg.contains("viewBox="));
        assert!(svg.contains("alpha"));
        assert!(svg.contains("beta"));
        assert!(svg.contains("class=\"wire\""));
        assert!(svg.contains("class=\"component\""));
        assert!(svg.contains("class=\"port\""));
    }

    #[test]
    fn grid_finer_than_min_port_pitch_errors() {
        let design = two_box_design();
        // Pitch is 2 steps, so a step of 5 gives pitch 10 < MIN_PORT_PITCH.
        let mut view = view_of("sys", &[("a", 0.0, 0.0, &[("p", Side::East)])]);
        view.grid = Some(5.0);
        assert!(matches!(
            render(&design, &view),
            Err(Error::GridTooSmall { .. })
        ));
    }

    #[test]
    fn wire_to_excluded_box_is_dropped() {
        let design = two_box_design();
        // Only `a` is included; the wire's `b` end isn't placed.
        let view = view_of("sys", &[("a", 0.0, 0.0, &[("p", Side::East)])]);
        let svg = render(&design, &view).unwrap();
        assert!(!svg.contains("class=\"wire\""));
    }

    fn render_embed(design: &Design, view: &View) -> Result<String> {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        SchematicRenderer.render(design, subject, view, true)
    }

    #[test]
    fn embed_mode_omits_embedded_style_block() {
        let design = two_box_design();
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 16.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let svg = render_embed(&design, &view).expect("renders");

        // The built-in <style> tag and its STYLE-block selectors are absent
        // in embed mode; the host stylesheet owns the look.
        assert!(!svg.contains("<style>"));
        assert!(!svg.contains(".component rect"));
    }

    #[test]
    fn embed_mode_class_tags_the_root_svg() {
        let design = two_box_design();
        let view = view_of("sys", &[("a", 0.0, 0.0, &[("p", Side::East)])]);
        let svg = render_embed(&design, &view).expect("renders");
        assert!(svg.contains("class=\"wirebug wirebug-schematic\""));
    }

    #[test]
    fn renders_text_boxes() {
        let design = design_from("component sys { }");
        let mut view = view_of("sys", &[]);
        view.texts.push(TextBox {
            name: "note".to_string(),
            x: 2.0,
            y: 3.0,
            label: "This is my textbox!".to_string(),
        });

        let svg = render(&design, &view).unwrap();

        assert!(svg.contains("class=\"text-box\""));
        assert!(svg.contains("data-text=\"note\""));
        assert!(svg.contains("This is my textbox!"));
    }
}
