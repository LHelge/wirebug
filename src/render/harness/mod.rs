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
use crate::render::SvgMode;
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
/// Nominal glyph advance of the 9px `cable-label` font, for sizing a
/// braided box's label zones (narrower than the table fonts' CHAR_WIDTH).
pub(super) const LABEL_CHAR_WIDTH: f64 = 5.5;
/// Space between a cable box's header band and its first strand row, so
/// the row's annotation (drawn above the wire, halo included) clears the
/// header instead of overlapping its bottom edge.
pub(super) const CABLE_LABEL_PAD: f64 = 12.0;
/// Radius of the dot marking a pin's cable attach point.
pub(super) const PIN_DOT_RADIUS: f64 = 3.0;
/// Minimum vertical gap between two cable boxes stacked on the spine.
pub(super) const CABLE_GAP: f64 = 24.0;
/// Nominal width of one half-twist in a braided (twisted-pair) box run.
pub(super) const TWIST_PITCH: f64 = 28.0;
/// Half-twists in the symbolic braid section — the drafting idiom is a few
/// visible twists, not a full-length weave. Even, so each strand exits the
/// section on the row it entered.
pub(super) const BRAID_HALF_TWISTS: usize = 4;
/// Width of the symbolic braid section between the two label zones.
pub(super) const BRAID_SECTION: f64 = BRAID_HALF_TWISTS as f64 * TWIST_PITCH;
pub(super) const SVG_MARGIN: f64 = 48.0;
pub(super) const TITLE_GAP: f64 = 12.0;

pub(super) const STYLE: &str = "\
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
.cable-wire-tracer { fill: none; stroke-width: 2; stroke-dasharray: 4 4; }
.cable-label { font: 9px sans-serif; text-anchor: middle; fill: #333; paint-order: stroke; stroke: white; stroke-width: 3px; stroke-linejoin: round; }
.cable-label-start { text-anchor: start; }
.cable-label-end { text-anchor: end; }
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
    /// `mode` picks the presentation: [`SvgMode::Embed`] drops the
    /// built-in `<style>` (the host owns the look), suppresses the
    /// bottom-right project-identity stamp, and class-tags the root
    /// `<svg>` `wirebug wirebug-harness`; [`SvgMode::Plain`] keeps the
    /// styles but omits the view title and the stamp (the PDF page
    /// header/footer carry them instead).
    pub(super) fn render(
        &self,
        design: &Design,
        subject: &Instance,
        view: &View,
        mode: SvgMode,
    ) -> Result<String> {
        let step = view.grid.unwrap_or(DEFAULT_GRID);
        if step <= 0.0 {
            return Err(Error::NonPositiveGrid { grid: step });
        }

        let layout = HarnessLayout::compute(design, subject, view, step);

        let mut doc = Document::new().set("xmlns", "http://www.w3.org/2000/svg");
        if mode.is_embed() {
            doc = doc.set("class", "wirebug wirebug-harness");
        } else {
            doc = doc.add(Style::new(STYLE));
        }

        let has_title = !view.title.is_empty() && mode.titled();
        let mut vb = layout.viewbox(has_title);
        let manifest = mode.stamped().then_some(design.manifest.as_ref()).flatten();
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
            .render(design, subject, view, SvgMode::Standalone)
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

    /// A pin table auto-scopes to the pins wired in this view: the unwired
    /// aux pin gets no row, so its label stays out of the SVG entirely.
    #[test]
    fn pin_tables_scope_to_wired_pins() {
        let design = design_from(
            r#"
component sys {
    a: src;
    b: snk;
    wire orange 50 [a.pos, b.pos];
    component src {
        connector hv "HV 3p" {
            pub port pos "V+" pin 1;
            pub port aux "AUX" pin 2;
        }
    }
    component snk {
        connector hv "HV 2p" {
            pub port pos "V+" pin 1;
        }
    }
}
"#,
        );
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 12.0, 0.0)]);
        let svg = render(&design, &view);
        assert!(svg.contains("V+"));
        assert!(
            !svg.contains("AUX"),
            "unwired pin is scoped out of the table"
        );
    }

    #[test]
    fn two_tone_wire_draws_a_tracer_overlay() {
        let design = design_from(
            r#"
component sys {
    a: src;
    b: snk;
    wire green/yellow 2.5 "PE" [a.pe, b.pe];
    component src {
        connector c "PE 1p" {
            pub port pe "PE" pin 1;
        }
    }
    component snk {
        connector c "PE 1p" {
            pub port pe "PE" pin 1;
        }
    }
}
"#,
        );
        let view = harness_view("sys", &[("a", "c", 0.0, 0.0), ("b", "c", 12.0, 0.0)]);
        let svg = render(&design, &view);

        // Base color strokes the core; the tracer overlays it dashed.
        assert!(svg.contains("stroke=\"green\""));
        assert!(svg.contains("class=\"cable-wire-tracer\""));
        assert!(svg.contains("stroke=\"yellow\""));
        // data-color keeps the authored two-tone form for host CSS.
        assert!(svg.contains("data-color=\"green/yellow\""));
        // The annotation writes the combined IEC code.
        assert!(svg.contains("PE · 2.5mm² · GN/YE"));
    }

    #[test]
    fn single_color_wire_has_no_tracer_overlay() {
        let design = two_connector_design();
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 12.0, 0.0)]);
        let svg = render(&design, &view);
        // The selector exists in the style block; no element carries it.
        assert!(!svg.contains("class=\"cable-wire-tracer\""));
    }

    /// Two 2-pin connectors joined by a 2-conductor cable; `body` is the
    /// cable's conductor block.
    fn cabled_pair_design(body: &str) -> Design {
        design_from(&format!(
            r#"
component sys {{
    a: src;
    b: snk;
    cable pair "Pair" {{
        {body}
    }}
    component src {{
        connector c "CAN 2p" {{
            pub port h "H" pin 1;
            pub port l "L" pin 2;
        }}
    }}
    component snk {{
        connector c "CAN 2p" {{
            pub port h "H" pin 1;
            pub port l "L" pin 2;
        }}
    }}
}}
"#
        ))
    }

    /// Total cubic segments across all `cable-wire` core paths. Straight
    /// runs contribute none (they are `L` lines); lead-in/lead-out flexes
    /// contribute one each; a braid contributes its half-twist chain.
    fn cable_wire_curve_segments(svg: &str) -> usize {
        svg.match_indices("class=\"cable-wire\"")
            .map(|(at, _)| {
                let end = svg[at..].find("/>").map_or(svg.len(), |e| at + e);
                svg[at..end].matches(" C").count()
            })
            .sum()
    }

    #[test]
    fn twisted_pair_braids_the_box_run() {
        let twisted = render(
            &cabled_pair_design(
                r#"twisted {
                    wire white/blue 0.5 "H" [a.h, b.h];
                    wire white/red 0.5 "L" [a.l, b.l];
                }"#,
            ),
            &harness_view("sys", &[("a", "c", 0.0, 0.0), ("b", "c", 16.0, 0.0)]),
        );
        let straight = render(
            &cabled_pair_design(
                r#"wire white/blue 0.5 "H" [a.h, b.h];
                wire white/red 0.5 "L" [a.l, b.l];"#,
            ),
            &harness_view("sys", &[("a", "c", 0.0, 0.0), ("b", "c", 16.0, 0.0)]),
        );

        // A straight box run is a single `L` line per strand; a braided run
        // is a chain of cubics, so the twisted render carries strictly more
        // curve segments and no straight run.
        assert!(
            cable_wire_curve_segments(&twisted) > cable_wire_curve_segments(&straight),
            "twisted: {} vs straight: {}",
            cable_wire_curve_segments(&twisted),
            cable_wire_curve_segments(&straight)
        );
        assert!(straight.contains(" L"), "straight run kept its line");

        // Braided labels leave the noisy centre: they anchor over the
        // straight ends, first strand left, second right. Straight boxes
        // keep the centred label (no anchor modifier).
        assert!(twisted.contains("class=\"cable-label cable-label-start\""));
        assert!(twisted.contains("class=\"cable-label cable-label-end\""));
        // (Class-attribute form: the selector always exists in the style block.)
        assert!(!straight.contains("class=\"cable-label cable-label-start\""));
    }

    #[test]
    fn mixed_cable_braids_only_the_twisted_pair() {
        let design = design_from(
            r#"
component sys {
    a: src;
    b: snk;
    cable loom "Sensor loom" {
        wire red 1.5 "12V" [a.pwr, b.pwr];
        twisted {
            wire white/blue 0.5 "H" [a.h, b.h];
            wire white/red 0.5 "L" [a.l, b.l];
        }
        wire black 1.5 "GND" [a.gnd, b.gnd];
    }
    component src {
        connector c "Sensor 4p" {
            pub port pwr "12V" pin 1;
            pub port h "H" pin 2;
            pub port l "L" pin 3;
            pub port gnd "GND" pin 4;
        }
    }
    component snk {
        connector c "Sensor 4p" {
            pub port pwr "12V" pin 1;
            pub port h "H" pin 2;
            pub port l "L" pin 3;
            pub port gnd "GND" pin 4;
        }
    }
}
"#,
        );
        let view = harness_view("sys", &[("a", "c", 0.0, 0.0), ("b", "c", 24.0, 0.0)]);

        // The pair's strands land in adjacent rows even though the sort is
        // by endpoint y — groups sort as one unit.
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == TypeName::from("sys"))
            .expect("subject");
        let layout = HarnessLayout::compute(&design, subject, &view, DEFAULT_GRID);
        let groups: Vec<Option<u32>> = layout.cable_boxes[0]
            .strands
            .iter()
            .map(|s| s.group)
            .collect();
        assert_eq!(groups, [None, Some(0), Some(0), None]);

        let svg = render(&design, &view);
        // Straight conductors keep their line runs and centred labels; the
        // pair braids and pushes its labels to the edges.
        assert!(svg.contains(" L"));
        assert!(svg.contains("class=\"cable-label\""));
        assert!(svg.contains("class=\"cable-label cable-label-start\""));
        assert!(svg.contains("class=\"cable-label cable-label-end\""));
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
            .render(design, subject, view, SvgMode::Embed)
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
        assert_eq!(cb.strands[0].color.css(), "green");
        assert_eq!(cb.strands[1].color.css(), "red");
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
