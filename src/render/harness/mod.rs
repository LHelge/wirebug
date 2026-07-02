//! WireViz-style harness renderer.
//!
//! Each included `instance.connector` becomes a pin table, placed at its
//! authored `(x, y)`. The renderer derives a vertical **spine** midway
//! between the connectors; each pin faces the connector its conductor
//! reaches, declared cables stack along it as labelled boxes, and wires flex
//! between them as cubic beziers
//! (pin → cable → pin, or pin → pin for loose wires). This is the dual of the
//! schematic renderer (`super::schematic`): same subject/first-instance lookup
//! and the same chain-decomposition of wires, but keyed on connectors rather
//! than authored port sides.
//!
//! - `layout` places connector nodes, the spine, and the cable boxes.
//! - `bezier` is the pure cubic-bezier math for the wire flex.
//! - `draw` emits the SVG for the pin tables, cable boxes, and wires.

mod bezier;
mod draw;
pub(super) mod layout;

use svg::Document;
use svg::node::element::{Group, Style, Text};

use crate::dsl::ir::{Design, Instance, View};
use crate::error::{Error, Result};
use crate::render::stamp::{STAMP_HEIGHT, STAMP_INSET, stamp_element};

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
/// Minimum vertical gap between two cable boxes stacked on the spine.
pub(super) const CABLE_GAP: f64 = 24.0;
pub(super) const SVG_MARGIN: f64 = 48.0;
pub(super) const TITLE_GAP: f64 = 12.0;

const STYLE: &str = "\
.connector rect { fill: white; stroke: black; stroke-width: 1.5; }
.connector .header { fill: #f0f0f0; }
.connector-title { font: bold 13px sans-serif; text-anchor: middle; }
.connector-part { font: 10px sans-serif; text-anchor: middle; fill: #555; }
.row-sep { stroke: #ddd; stroke-width: 1; }
.pin-num { font: italic 10px sans-serif; fill: #555; text-anchor: middle; }
.pin-label { font: 11px sans-serif; }
.pin-dot { fill: black; }
.cable-wire-casing { fill: none; stroke: black; stroke-width: 4; stroke-linecap: butt; }
.cable-wire { fill: none; stroke-width: 2; }
.cable-label { font: 9px sans-serif; text-anchor: middle; fill: #333; paint-order: stroke; stroke: white; stroke-width: 3px; stroke-linejoin: round; }
.title { font: bold 14px sans-serif; }
.stamp { font: 10px sans-serif; fill: #666; text-anchor: end; }\
";

/// SVG renderer for `kind: harness` views.
#[derive(Default)]
pub struct HarnessRenderer;

impl HarnessRenderer {
    /// Render `view` (documenting `subject`) against `design` to an SVG
    /// string.
    ///
    /// `embed` switches to embed-mode output for inclusion in another
    /// document: the built-in `<style>` is dropped (the host owns the
    /// look), the bottom-right project-identity stamp is suppressed,
    /// and the root `<svg>` carries `class="wirebug wirebug-harness"`
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

        let layout = HarnessLayout::compute(design, subject, view, step);

        let mut doc = Document::new().set("xmlns", "http://www.w3.org/2000/svg");
        if embed {
            doc = doc.set("class", "wirebug wirebug-harness");
        } else {
            doc = doc.add(Style::new(STYLE));
        }

        let has_title = !view.title.is_empty();
        let mut vb = layout.viewbox(has_title);
        let manifest = (!embed).then_some(design.manifest.as_ref()).flatten();
        if manifest.is_some() {
            vb.height += STAMP_HEIGHT;
        }
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

        if let Some(manifest) = manifest {
            doc = doc.add(stamp_element(
                manifest,
                vb.x + vb.width - STAMP_INSET,
                vb.y + vb.height - STAMP_INSET,
            ));
        }

        // Wires and cable boxes under the nodes so attach dots and pin labels
        // stay legible on top.
        let mut cables_group = Group::new().set("class", "cables");
        for wire in &layout.loose {
            cables_group = cables_group.add(draw::render_loose(wire));
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
    use crate::dsl::ir::{ConnectorName, Include, InstanceName, TypeName, ViewKind};
    use crate::render::schematic::tests::design_from;

    /// A harness view over `subject`, including `(instance, connector, x, y)`.
    fn harness_view(subject: &str, includes: &[(&str, &str, f64, f64)]) -> View {
        View {
            kind: ViewKind::Harness,
            title: "Harness".to_string(),
            grid: None,
            subject: TypeName::from(subject),
            has_enclosure: false,
            enclosure: Vec::new(),
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
            texts: Vec::new(),
        }
    }

    fn render(design: &Design, view: &View) -> String {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        HarnessRenderer
            .render(design, subject, view, false)
            .expect("renders")
    }

    /// Two boxes, each exposing a 2-pin connector, wired pos↔pos and neg↔neg.
    fn two_connector_design() -> Design {
        design_from(
            r#"
component sys {
    a: src "Source";
    b: snk "Sink";
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
        // The labelled wire's annotation shows (with its color code);
        // colors pass through as stroke, over a black casing.
        assert!(svg.contains("V+ · 50mm² · OG"));
        assert!(svg.contains("stroke=\"orange\""));
        assert!(svg.contains("class=\"cable-wire-casing\""));
        assert!(svg.contains("Source"));
    }

    #[test]
    fn wires_carry_data_color_for_host_styling() {
        let design = two_connector_design();
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 12.0, 0.0)]);
        let svg = render(&design, &view);

        assert!(svg.contains("data-color=\"orange\""));
        assert!(svg.contains("data-color=\"black\""));
    }

    fn render_embed(design: &Design, view: &View) -> String {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        HarnessRenderer
            .render(design, subject, view, true)
            .expect("renders")
    }

    #[test]
    fn embed_mode_omits_embedded_style_block() {
        let design = two_connector_design();
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 12.0, 0.0)]);
        let svg = render_embed(&design, &view);

        assert!(!svg.contains("<style>"));
        assert!(!svg.contains(".connector rect"));
    }

    #[test]
    fn embed_mode_class_tags_the_root_svg() {
        let design = two_connector_design();
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 12.0, 0.0)]);
        let svg = render_embed(&design, &view);
        assert!(svg.contains("class=\"wirebug wirebug-harness\""));
    }

    /// Like `two_connector_design`, but the two conductors are grouped into a
    /// declared `cable` with metadata.
    fn cabled_design() -> Design {
        design_from(
            r#"
component sys {
    a: src "Source";
    b: snk "Sink";
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
        let layout = compute(&design, &view);
        use crate::render::geometry::Side;
        assert_eq!(layout.nodes[0].facing, Side::East, "left node faces right");
        assert_eq!(layout.nodes[1].facing, Side::West, "right node faces left");
    }

    /// A node whose two pins wire in opposite directions: one toward a
    /// connector on its left, the other toward one on its right.
    fn bridging_design() -> Design {
        design_from(
            r#"
component sys {
    l: end "Left";
    m: mid "Mid";
    r: end "Right";
    wire orange 1 [l.p, m.w];
    wire orange 1 [m.e, r.p];
    component end {
        connector c "C 1p" { pub port p "P" pin 1; }
    }
    component mid {
        connector c "C 2p" {
            pub port w "W" pin 1;
            pub port e "E" pin 2;
        }
    }
}
"#,
        )
    }

    #[test]
    fn each_pin_faces_the_connector_its_conductor_reaches() {
        let design = bridging_design();
        let view = harness_view(
            "sys",
            &[
                ("l", "c", 0.0, 0.0),
                ("m", "c", 12.0, 0.0),
                ("r", "c", 24.0, 0.0),
            ],
        );
        let layout = compute(&design, &view);
        use crate::render::geometry::Side;

        let mid = layout.nodes.iter().find(|n| n.title == "Mid").expect("mid");
        let side = |port: &str| {
            mid.pins
                .iter()
                .find(|r| r.port.to_string() == port)
                .expect("pin")
                .side
        };
        // The pin wired left attaches on the west edge, the one wired right on
        // the east edge — each goes the short way, not the whole table forced
        // to a single facing.
        assert_eq!(
            side("w"),
            Side::West,
            "pin toward the left connector faces west"
        );
        assert_eq!(
            side("e"),
            Side::East,
            "pin toward the right connector faces east"
        );
    }

    /// A cable whose conductors are declared bottom-pin-first, to prove the
    /// box rows reorder by endpoint y rather than declaration order.
    fn reordered_cable_design() -> Design {
        design_from(
            r#"
component sys {
    a: src "A";
    b: snk "B";
    cable feed "Feed" {
        wire red   1 "hi" [a.p2, b.p2];
        wire green 1 "lo" [a.p1, b.p1];
    }
    component src {
        connector c "C 2p" { pub port p1 "P1" pin 1; pub port p2 "P2" pin 2; }
    }
    component snk {
        connector c "C 2p" { pub port p1 "P1" pin 1; pub port p2 "P2" pin 2; }
    }
}
"#,
        )
    }

    #[test]
    fn cable_box_rows_sort_by_endpoint_y() {
        let design = reordered_cable_design();
        let view = harness_view("sys", &[("a", "c", 0.0, 0.0), ("b", "c", 16.0, 0.0)]);
        let layout = compute(&design, &view);
        let cb = &layout.cable_boxes[0];
        // `green` lands on pin 1 (top, smaller y) so it takes the first row,
        // even though `red` (pin 2) was declared first.
        assert_eq!(cb.strands[0].color.as_str(), "green");
        assert_eq!(cb.strands[1].color.as_str(), "red");
        assert!(cb.strands[0].row_y < cb.strands[1].row_y);
    }

    #[test]
    fn cabled_layout_structure_snapshot() {
        let design = cabled_design();
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 16.0, 0.0)]);
        insta::assert_snapshot!(structural(&compute(&design, &view)));
    }

    /// Build the layout for `view` against `design`'s subject instance.
    fn compute(design: &Design, view: &View) -> HarnessLayout {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        HarnessLayout::compute(design, subject, view, 20.0)
    }

    /// A float-stable, layout-only dump for snapshotting (coordinates rounded
    /// so renderer pixel tweaks don't churn the snapshot).
    fn structural(layout: &HarnessLayout) -> String {
        use std::fmt::Write;
        let pt = |p: crate::render::geometry::Point| format!("({:.0},{:.0})", p.x, p.y);
        let mut s = String::new();
        for n in &layout.nodes {
            writeln!(
                s,
                "node {:?} @{} {:?} pins={}",
                n.title,
                pt(n.origin),
                n.facing,
                n.pins.len()
            )
            .unwrap();
        }
        for cb in &layout.cable_boxes {
            writeln!(
                s,
                "cable {:?} [{}] @{}",
                cb.title,
                cb.subtitle,
                pt(cb.origin)
            )
            .unwrap();
            for st in &cb.strands {
                writeln!(
                    s,
                    "  strand {} row={:.0} {}->{}",
                    st.color,
                    st.row_y,
                    pt(st.left_attach),
                    pt(st.right_attach)
                )
                .unwrap();
            }
        }
        for w in &layout.loose {
            writeln!(s, "loose {} {}->{}", w.color, pt(w.from), pt(w.to)).unwrap();
        }
        s
    }
}
