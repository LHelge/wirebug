//! WireViz-style harness renderer.
//!
//! Each included `instance.connector` becomes a pin table; the subject's
//! wires running between two included connectors become cable bundles. This
//! is the dual of the schematic renderer (`super::schematic`): same
//! subject/first-instance lookup and the same chain-decomposition of wires,
//! but keyed on connectors rather than authored port sides.
//!
//! - `layout` places connector nodes and groups wires into cables.
//! - `draw` emits the SVG for the pin tables and cable bundles.
//!
//! Cable routing is intentionally simple for now: each wire is an
//! orthogonal three-segment path through a per-wire channel offset, so a
//! bundle reads as parallel strands. Reusing the schematic's
//! object-avoiding router is a later refinement.

mod draw;
pub(super) mod layout;

use svg::Document;
use svg::node::element::{Group, Style, Text};

use crate::dsl::ir::{Design, Instance, View};
use crate::error::{Error, Result};

use layout::HarnessLayout;

/// Grid step (world units) used when a harness view omits one.
pub(super) const DEFAULT_GRID: f64 = 20.0;

/// Pin-table row height (world units).
pub(super) const ROW_HEIGHT: f64 = 22.0;
/// Header band height: instance title over the connector subtitle.
pub(super) const HEADER_HEIGHT: f64 = 40.0;
/// Width of the pin-number column along the facing edge.
pub(super) const PIN_COL_WIDTH: f64 = 28.0;
/// Inner padding around the label column.
pub(super) const NODE_PAD: f64 = 10.0;
/// Smallest a node gets, so short connectors still read as tables.
pub(super) const MIN_NODE_WIDTH: f64 = 120.0;
/// Nominal glyph advance for sizing label/title columns.
pub(super) const CHAR_WIDTH: f64 = 7.0;
/// Radius of the dot marking a pin's cable attach point.
pub(super) const PIN_DOT_RADIUS: f64 = 3.0;
/// Horizontal spacing between adjacent wires' vertical channels in a bundle.
pub(super) const BUNDLE_SPACING: f64 = 7.0;
pub(super) const SVG_MARGIN: f64 = 48.0;
pub(super) const TITLE_GAP: f64 = 12.0;

const STYLE: &str = "\
.connector rect { fill: white; stroke: black; stroke-width: 1.5; }
.connector .header { fill: #f0f0f0; }
.connector-title { font: bold 12px sans-serif; text-anchor: middle; }
.connector-part { font: 10px sans-serif; text-anchor: middle; fill: #555; }
.row-sep { stroke: #ddd; stroke-width: 1; }
.pin-num { font: italic 10px sans-serif; fill: #555; text-anchor: middle; }
.pin-label { font: 11px sans-serif; }
.pin-dot { fill: black; }
.cable-wire { fill: none; stroke-width: 2.5; }
.cable-label { font: 9px sans-serif; text-anchor: middle; fill: #333; }
.title { font: bold 14px sans-serif; }\
";

/// SVG renderer for `kind: harness` views.
#[derive(Default)]
pub struct HarnessRenderer;

impl HarnessRenderer {
    /// Render `view` (documenting `subject`) against `design` to an SVG
    /// string.
    pub(super) fn render(
        &self,
        design: &Design,
        subject: &Instance,
        view: &View,
    ) -> Result<String> {
        let step = view.grid.unwrap_or(DEFAULT_GRID);
        if step <= 0.0 {
            return Err(Error::NonPositiveGrid { grid: step });
        }

        let layout = HarnessLayout::compute(design, subject, view, step);

        let mut doc = Document::new()
            .set("xmlns", "http://www.w3.org/2000/svg")
            .add(Style::new(STYLE));

        let has_title = !view.title.is_empty();
        let vb = layout.viewbox(has_title);
        doc = doc.set(
            "viewBox",
            format!("{} {} {} {}", vb.x, vb.y, vb.width, vb.height),
        );

        if has_title {
            doc = doc.add(
                Text::new(view.title.clone())
                    .set("class", "title")
                    .set("x", vb.x + SVG_MARGIN)
                    .set("y", vb.y + SVG_MARGIN - TITLE_GAP),
            );
        }

        // Cables under the nodes so attach dots and labels stay legible.
        let mut cables_group = Group::new().set("class", "cables");
        for cable in &layout.cables {
            cables_group = cables_group.add(draw::render_cable(cable));
        }
        for cable_box in &layout.cable_boxes {
            cables_group = cables_group.add(draw::render_cable_box(cable_box));
        }
        doc = doc.add(cables_group);

        let mut nodes_group = Group::new().set("class", "connectors");
        for node in &layout.nodes {
            nodes_group = nodes_group.add(draw::render_node(node));
        }
        doc = doc.add(nodes_group);

        Ok(doc.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{ConnectorName, Include, InstanceName, TypeName};
    use crate::render::schematic::tests::design_from;

    /// A harness view over `subject`, including `(instance, connector, x, y)`.
    fn harness_view(subject: &str, includes: &[(&str, &str, f64, f64)]) -> View {
        View {
            kind: "harness".to_string(),
            title: "Harness".to_string(),
            grid: None,
            subject: TypeName::from(subject),
            includes: includes
                .iter()
                .map(|(inst, conn, x, y)| Include {
                    instance: InstanceName::from(*inst),
                    connector: Some(ConnectorName::from(*conn)),
                    x: *x,
                    y: *y,
                    ports: Vec::new(),
                })
                .collect(),
        }
    }

    fn render(design: &Design, view: &View) -> String {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        HarnessRenderer
            .render(design, subject, view)
            .expect("renders")
    }

    /// Two boxes, each exposing a 2-pin connector, wired pos↔pos and neg↔neg.
    fn two_connector_design() -> Design {
        design_from(
            r#"
component sys {
    src a "Source";
    snk b "Sink";
    wire orange 50 "V+" [a.pos, b.pos];
    wire black 50 [a.neg, b.neg];
    component src {
        connector hv "HV 2p" {
            pub port pos "V+" pin 1;
            pub port neg "V-" pin 2;
        }
    }
    component snk {
        connector hv "HV 2p" {
            pub port pos "V+" pin 1;
            pub port neg "V-" pin 2;
        }
    }
}
"#,
        )
    }

    #[test]
    fn renders_connectors_and_cable_fragments() {
        let design = two_connector_design();
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 12.0, 0.0)]);
        let svg = render(&design, &view);

        assert!(svg.contains("<svg"));
        assert!(svg.contains("class=\"connector\""));
        assert!(svg.contains("class=\"cable-wire\""));
        // Both wires of the cable survive (pos–pos and neg–neg).
        assert_eq!(svg.matches("class=\"cable-wire\"").count(), 2);
        // The labelled wire's annotation shows; colors pass through as stroke.
        assert!(svg.contains("V+ · 50mm²"));
        assert!(svg.contains("stroke=\"orange\""));
        assert!(svg.contains("Source"));
    }

    /// Like `two_connector_design`, but the two conductors are grouped into a
    /// declared `cable` with metadata.
    fn cabled_design() -> Design {
        design_from(
            r#"
component sys {
    src a "Source";
    snk b "Sink";
    cable feed "Power feed" {
        type: "2-core";
        length: 0.8;
        wire orange 50 "V+" [a.pos, b.pos];
        wire black 50 "V-" [a.neg, b.neg];
    }
    component src {
        connector hv "HV 2p" {
            pub port pos "V+" pin 1;
            pub port neg "V-" pin 2;
        }
    }
    component snk {
        connector hv "HV 2p" {
            pub port pos "V+" pin 1;
            pub port neg "V-" pin 2;
        }
    }
}
"#,
        )
    }

    #[test]
    fn declared_cable_renders_as_a_titled_box() {
        let design = cabled_design();
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 16.0, 0.0)]);
        let svg = render(&design, &view);

        // The cable becomes a box, not the bare node-pair bundle.
        assert!(svg.contains("class=\"connector cable-box\""));
        assert!(svg.contains("Power feed"));
        assert!(svg.contains("2-core · 0.8 m"));
        // Both conductors show, each per-strand annotation present.
        assert!(svg.contains("V+ · 50mm²"));
        assert!(svg.contains("V- · 50mm²"));
        // Each strand draws three coloured segments (lead-in, through, lead-out).
        assert_eq!(svg.matches("stroke=\"orange\"").count(), 3);
    }

    #[test]
    fn cable_to_excluded_connector_is_dropped() {
        let design = two_connector_design();
        // Only `a` is included; the wires' `b` ends have no node.
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0)]);
        let svg = render(&design, &view);
        assert!(!svg.contains("class=\"cable-wire\""));
    }

    #[test]
    fn left_node_faces_east_right_node_faces_west() {
        let design = two_connector_design();
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 12.0, 0.0)]);
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == TypeName::from("sys"))
            .unwrap();
        let layout = HarnessLayout::compute(&design, subject, &view, 20.0);
        use crate::render::geometry::Side;
        assert_eq!(layout.nodes[0].facing, Side::East, "left node faces right");
        assert_eq!(layout.nodes[1].facing, Side::West, "right node faces left");
    }
}
