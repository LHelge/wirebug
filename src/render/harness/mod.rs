//! WireViz-style harness renderer.
//!
//! Each included `instance.connector` becomes a pin table, placed at its
//! authored `(x, y)`. The renderer derives a vertical **spine** midway
//! between the connectors; each pin faces the connector its conductor
//! reaches, declared cables stack along it as labelled boxes, and wires flex
//! between them as cubic beziers (pin → cable → pin). A loose wire flexes
//! pin → pin, or, where that curve would meet a cable box, through a lane of
//! its own on the spine. This is the dual of the
//! schematic renderer (`super::schematic`): same subject/first-instance lookup
//! and the same chain-decomposition of wires, but keyed on connectors rather
//! than authored port sides.
//!
//! - `layout` places connector nodes, splices, the spine, and the cable
//!   boxes.
//! - `bezier` is the pure cubic-bezier math for the wire flex.
//! - `draw` emits the SVG for the pin tables, splices, cable boxes, and
//!   wires.

mod bezier;
mod draw;
pub(super) mod layout;

use svg::Document;
use svg::node::element::{Group, Style, Text};

use crate::dsl::ir::{Design, Instance, View};
use crate::error::ViewResult;
use crate::render::lint::{Conductor, IncludeBox, ViewLayout};
use crate::render::stamp::{STAMP_HEIGHT, STAMP_INSET, stamp_element};
use crate::render::{Canvas, GridFault, SvgMode};

use layout::{HarnessConductor, HarnessLayout, ViewBox};

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
/// Side length of the square M/F housing-half chip on an inline node's
/// header.
pub(super) const BADGE_SIZE: f64 = 16.0;
/// Gap between the badge chip and the header's corner.
pub(super) const BADGE_INSET: f64 = 4.0;
/// Radius of a splice's dot, the joint its conductors converge on.
pub(super) const SPLICE_DOT_RADIUS: f64 = 7.0;
/// Line height of the name, label and part under a splice's dot.
pub(super) const SPLICE_LINE_HEIGHT: f64 = 13.0;
/// Minimum vertical gap between two cable boxes stacked on the spine.
pub(super) const CABLE_GAP: f64 = 24.0;
/// The shortest a loose wire's lead into or out of its lane on the spine
/// is kept, where the gap between its ends allows: a lane narrower than its
/// annotation needs cuts the annotation short instead.
pub(super) const LANE_MIN_LEAD: f64 = 4.0 * NODE_PAD;
/// Nominal width of one half-twist in a braided (twisted-pair) box run.
pub(super) const TWIST_PITCH: f64 = 28.0;
/// Half-twists in the symbolic braid section — the drafting idiom is a few
/// visible twists, not a full-length weave. Even, so each strand exits the
/// section on the row it entered.
pub(super) const BRAID_HALF_TWISTS: usize = 4;
/// Width of the symbolic braid section between the two label zones.
pub(super) const BRAID_SECTION: f64 = BRAID_HALF_TWISTS as f64 * TWIST_PITCH;
/// How far a cable box's dashed screen outline sits inside the box; a
/// screened pair's outline sits twice as far in, so the two never meet.
pub(super) const SHIELD_INSET: f64 = 3.0;
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
.connector .inline-badge { fill: #333; stroke: none; }
.inline-badge-text { font: bold 10px sans-serif; fill: white; text-anchor: middle; }
.cable-wire-casing { fill: none; stroke: black; stroke-width: 4; stroke-linecap: butt; }
.cable-wire { fill: none; stroke-width: 2; }
.cable-wire-tracer { fill: none; stroke-width: 2; stroke-dasharray: 4 4; }
.cable-label { font: 9px sans-serif; text-anchor: middle; fill: #333; paint-order: stroke; stroke: white; stroke-width: 3px; stroke-linejoin: round; }
.cable-label-start { text-anchor: start; }
.cable-label-end { text-anchor: end; }
.connector .cable-shield { fill: none; stroke: #666; stroke-width: 1; stroke-dasharray: 6 3; }
.connector .pair-shield { fill: none; stroke: #666; stroke-width: 1; stroke-dasharray: 3 2; }
.drain-wire { fill: none; stroke: #777; stroke-width: 1.2; stroke-dasharray: 5 3; }
.drain-label { font-style: italic; }
.splice-dot { fill: black; stroke: white; stroke-width: 1.5; }
.splice-name { font: bold 11px sans-serif; text-anchor: middle; }
.splice-label { font: 10px sans-serif; text-anchor: middle; fill: #333; }
.splice-part { font: italic 9px sans-serif; text-anchor: middle; fill: #555; }
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
    ) -> ViewResult<String> {
        let step = GridFault::positive_step(view, DEFAULT_GRID)?;
        let layout = HarnessLayout::compute(design, subject, view, step);

        let mut doc = Document::new().set("xmlns", "http://www.w3.org/2000/svg");
        if mode.is_embed() {
            doc = doc.set("class", "wirebug wirebug-harness");
        } else {
            doc = doc.add(Style::new(STYLE));
        }

        let has_title = mode.shows_title(view);
        let vb = viewbox(design, view, &layout, mode);
        let manifest = mode.stamp(design);
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
        // stay legible on top. Loose wires go over the boxes: one that still
        // meets a box (it stays on one side of the spine, so it has no lane)
        // then crosses it visibly, its annotation never hidden by the box.
        let mut cables_group = Group::new().set("class", "cables");
        for cable_box in &layout.cable_boxes {
            cables_group = cables_group.add(draw::render_cable_box(cable_box));
        }
        for wire in &layout.loose {
            cables_group = cables_group.add(draw::render_loose(wire));
        }
        doc = doc.add(cables_group);

        let mut nodes_group = Group::new().set("class", "connectors");
        for node in &layout.nodes {
            nodes_group = nodes_group.add(draw::render_node(node));
        }
        doc = doc.add(nodes_group);

        // Splice dots over their conductors' converging ends.
        if !layout.splices.is_empty() {
            let mut splices_group = Group::new().set("class", "splices");
            for splice in &layout.splices {
                splices_group = splices_group.add(draw::render_splice(splice));
            }
            doc = doc.add(splices_group);
        }

        Ok(doc.to_string())
    }

    /// Each include's box (a connector's pin table, a splice's joint), the
    /// conductors drawn and the canvas of
    /// the standalone SVG, from the same layout [`Self::render`] draws. A
    /// non-positive grid lays out no boxes and no canvas, only the fault;
    /// which conductors a view draws does not depend on its grid, so they
    /// are still counted.
    pub(super) fn layout(&self, design: &Design, subject: &Instance, view: &View) -> ViewLayout {
        let (step, fault) = match GridFault::positive_step(view, DEFAULT_GRID) {
            Ok(step) => (step, None),
            Err(fault) => (DEFAULT_GRID, Some(fault)),
        };
        let layout = HarnessLayout::compute(design, subject, view, step);
        let (boxes, canvas) = match fault {
            Some(_) => (Vec::new(), None),
            None => {
                let mut boxes: Vec<IncludeBox> = layout
                    .nodes
                    .iter()
                    .map(|node| IncludeBox {
                        include: node.include,
                        bounds: node.bounds(),
                    })
                    .chain(layout.splices.iter().map(|splice| IncludeBox {
                        include: splice.include,
                        bounds: splice.bounds,
                    }))
                    .collect();
                boxes.sort_by_key(|b| b.include);
                let vb = viewbox(design, view, &layout, SvgMode::Standalone);
                (boxes, Some(Canvas::new(vb.width, vb.height)))
            }
        };
        ViewLayout {
            boxes,
            drawn: layout.drawn,
            grid: fault,
            canvas,
            ..ViewLayout::default()
        }
    }

    /// Every conductor a harness view of `subject` can draw, in wire order:
    /// the set the views' [`ViewLayout::drawn`] segments are checked against.
    pub(super) fn conductors<'d>(
        &self,
        design: &'d Design,
        subject: &'d Instance,
    ) -> Vec<Conductor<'d>> {
        HarnessConductor::all(design, subject)
            .map(|c| Conductor {
                segment: c.segment,
                wire: c.wire,
                ends: c.ends.each_ref().map(|end| end.end()),
                targets: c.ends.each_ref().map(|end| end.target()),
            })
            .collect()
    }
}

/// The viewBox `view` is drawn in under `mode`: the laid-out drawing, with
/// room for the title and the identity stamp when the mode draws them.
fn viewbox(design: &Design, view: &View, layout: &HarnessLayout, mode: SvgMode) -> ViewBox {
    let mut vb = layout.viewbox(mode.shows_title(view));
    if mode.stamp(design).is_some() {
        vb.height += STAMP_HEIGHT;
    }
    vb
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{ConnectorName, Include, InstanceName, TypeName, ViewKind};
    use crate::dsl::span::{FileId, Span};
    use crate::render::schematic::tests::design_from;

    /// A harness view over `subject`, including `(instance, connector, x, y)`.
    fn harness_view(subject: &str, includes: &[(&str, &str, f64, f64)]) -> View {
        View {
            kind: ViewKind::Harness,
            title: "Harness".to_string(),
            grid: None,
            span: Span::new(FileId(0), 0, 0),
            subject: TypeName::from(subject),
            has_enclosure: false,
            enclosure: Vec::new(),
            includes: includes
                .iter()
                .map(|(inst, conn, x, y)| Include {
                    instance: InstanceName::from(*inst),
                    connector: Some(ConnectorName::from(*conn)),
                    half: None,
                    x: *x,
                    y: *y,
                    ports: Vec::new(),
                    span: Span::new(FileId(0), 0, 0),
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

    /// One loom view per housing half: each drawing titles the inline node
    /// with its own half's part identity and M/F chip, and auto-scoping
    /// keeps the other loom's conductors out.
    #[test]
    fn inline_include_shows_the_selected_half() {
        let design = design_from(
            r#"
connector_type dt04_4p "Deutsch DT04-4P" { part: "DT04-4P"; }
connector_type dt06_4s "Deutsch DT06-4S" { part: "DT06-4S"; }
component sys {
    ecu: dev "ECU";
    pedal: dev "Pedal";
    inline ic "Pedal branch" {
        male: dt04_4p;
        female: dt06_4s;
        port sig "SIG" pin 1;
    }
    cable engine_side { wire red 1 "SIG-A" [ecu.x, ic.sig]; }
    cable pedal_side { wire blue 1 "SIG-B" [ic.sig, pedal.x]; }
    component dev {
        connector j "J 1p" {
            pub port x "X" pin 1;
        }
    }
}
view harness "Engine bay" {
    include ecu.j at (0, 0);
    include ic.female at (12, 0);
}
view harness "Pedal stub" {
    include ic.male at (0, 0);
    include pedal.j at (12, 0);
}
"#,
        );

        let engine = design
            .views
            .iter()
            .find(|v| v.title == "Engine bay")
            .unwrap();
        let svg = render(&design, engine);
        assert!(svg.contains("ic · Deutsch DT06-4S · DT06-4S"), "{svg}");
        assert!(svg.contains("class=\"inline-badge\""));
        assert!(svg.contains(">\nF\n</text>"));
        assert!(!svg.contains("DT04-4P"), "the male half stays out");
        assert!(svg.contains("SIG-A"), "engine-side conductor draws");
        assert!(!svg.contains("SIG-B"), "pedal-side conductor is scoped out");

        let pedal = design
            .views
            .iter()
            .find(|v| v.title == "Pedal stub")
            .unwrap();
        let svg = render(&design, pedal);
        assert!(svg.contains("ic · Deutsch DT04-4P · DT04-4P"), "{svg}");
        assert!(svg.contains(">\nM\n</text>"));
        assert!(!svg.contains("DT06-4S"), "the female half stays out");
        assert!(svg.contains("SIG-B"));
        assert!(!svg.contains("SIG-A"));
        // The inline's label titles the node in both drawings.
        assert!(svg.contains("Pedal branch"));
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

    /// What `check` counts as drawn is exactly what the drawing carries:
    /// one segment per loose wire and cable strand, none for a wire whose
    /// far end is not included.
    #[test]
    fn drawn_segments_are_the_conductors_the_view_draws() {
        use crate::render::lint::Segment;
        let design = design_from(
            r#"
component sys {
    a: src;
    b: src;
    c: src;
    cable feed "Feed" {
        wire orange 50 [a.pos, b.pos];
    }
    wire black 50 [a.neg, b.neg, c.neg];
    component src {
        connector hv "HV 2p" {
            pub port pos "V+" pin 1;
            pub port neg "V-" pin 2;
        }
    }
}
"#,
        );
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 16.0, 0.0)]);
        let layout = compute(&design, &view);
        assert_eq!(
            layout.drawn,
            [Segment { wire: 0, index: 0 }, Segment { wire: 1, index: 0 }]
        );
        let strands: usize = layout.cable_boxes.iter().map(|b| b.strands.len()).sum();
        assert_eq!(strands + layout.loose.len(), layout.drawn.len());
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

    /// The cable box centres in the *gap* between the two connector columns,
    /// so its lead-in and lead-out read equal even when one side is far wider
    /// than the other. Regression: centring the spine on node *centres* skewed
    /// the box toward the wider connector.
    #[test]
    fn cable_box_centres_in_the_gap_when_sides_differ_in_width() {
        let design = design_from(
            r#"
component sys {
    a: wide "Source with a deliberately very long instance label";
    b: snk "B";
    cable feed "Feed" {
        wire orange 1 "V+" [a.pos, b.pos];
    }
    component wide {
        connector hv "HV 1p" { pub port pos "V+" pin 1; }
    }
    component snk {
        connector hv "HV 1p" { pub port pos "V+" pin 1; }
    }
}
"#,
        );
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 30.0, 0.0)]);
        let layout = compute(&design, &view);

        let (a, b) = (&layout.nodes[0], &layout.nodes[1]);
        assert!(a.width > b.width, "the source node is the wider column");
        let cable = &layout.cable_boxes[0];
        // `a` faces east (attaches on its right edge), `b` west (left edge).
        let lead_in = cable.origin.x - (a.origin.x + a.width);
        let lead_out = b.origin.x - (cable.origin.x + cable.width);
        assert!(
            lead_in > 0.0 && lead_out > 0.0,
            "box sits between the columns (lead-in {lead_in}, lead-out {lead_out})"
        );
        assert!(
            (lead_in - lead_out).abs() < 1.0,
            "box centred in the gap: lead-in {lead_in}, lead-out {lead_out}"
        );
    }

    /// A CAN-style pair between `a` and `b`, with a ground stud `g` off to
    /// the side; `body` is the cable's body.
    fn screened_design(body: &str) -> Design {
        design_from(&format!(
            r#"
component sys {{
    a: ecu "ECU";
    b: dev "Device";
    g: stud "Stud";
    cable can "CAN" {{
        {body}
    }}
    component ecu {{
        connector c "ECU 4p" {{
            pub port h "CAN H" pin 1;
            pub port l "CAN L" pin 2;
            pub port gnd "GND" pin 3;
            pub port shield "SHIELD" pin 4;
        }}
    }}
    component dev {{
        connector c "Dev 3p" {{
            pub port h "CAN H" pin 1;
            pub port l "CAN L" pin 2;
            pub port shield "SHIELD" pin 3;
        }}
    }}
    component stud {{
        connector j "Stud 1p" {{ pub port gnd "GND" pin 1; }}
    }}
}}
"#
        ))
    }

    /// The `sys` harness view of `a.c` and `b.c` (and `g.j` with `stud`).
    fn screened_view(stud: bool) -> View {
        let mut includes = vec![("a", "c", 0.0, 0.0), ("b", "c", 30.0, 0.0)];
        if stud {
            includes.push(("g", "j", 30.0, 10.0));
        }
        harness_view("sys", &includes)
    }

    const PAIR: &str = r#"wire white/blue 0.5 "CAN H" [a.h, b.h];
        wire white/red 0.5 "CAN L" [a.l, b.l];"#;

    #[test]
    fn a_shielded_cable_box_says_so_and_wears_a_dashed_screen() {
        let screened = render(
            &screened_design(&format!("type: \"CAN\"; {PAIR} shield;")),
            &screened_view(false),
        );
        assert!(screened.contains("CAN · ×2 · shielded"), "{screened}");
        assert_eq!(screened.matches("class=\"cable-shield\"").count(), 1);
        // On no pin: the mark alone, no drain.
        assert!(!screened.contains("class=\"drain\""));

        let plain = render(
            &screened_design(&format!("type: \"CAN\"; {PAIR}")),
            &screened_view(false),
        );
        assert!(plain.contains("CAN · ×2"));
        assert!(!plain.contains("shielded"));
        assert!(!plain.contains("class=\"cable-shield\""));
    }

    #[test]
    fn a_drain_runs_from_the_box_to_each_terminating_pin_in_the_view() {
        let design = screened_design(&format!(r#"{PAIR} shield "SHLD" [a.shield, b.shield];"#));
        let svg = render(&design, &screened_view(false));
        // One drain: its run through the box and a lead to each end.
        assert_eq!(svg.matches("class=\"drain\"").count(), 1);
        assert_eq!(svg.matches("class=\"drain-wire\"").count(), 3);
        assert!(svg.contains("class=\"cable-label drain-label\""));
        assert!(svg.contains(">\nSHLD\n</text>"), "{svg}");

        // Both drain pins get a row in their pin tables, facing the box.
        let layout = compute(&design, &screened_view(false));
        for node in &layout.nodes {
            let row = node
                .pins
                .iter()
                .find(|r| r.port.as_str() == "shield")
                .expect("the drain pin has a row");
            assert_eq!(row.side, node.facing);
        }
        let [cable] = layout.cable_boxes.as_slice() else {
            panic!("expected one cable box");
        };
        let [drain] = cable.drains.as_slice() else {
            panic!("expected one drain");
        };
        assert_eq!(drain.attaches.len(), 2);
        // The drain is the box's last row, below both strands.
        assert!(cable.strands.iter().all(|s| s.row_y < drain.row_y));
        assert!(drain.row_y < cable.origin.y + cable.height);
    }

    #[test]
    fn a_drain_end_on_a_connector_the_view_leaves_out_is_not_drawn() {
        let design = screened_design(&format!("{PAIR} shield [a.shield, g.gnd];"));
        let without = render(&design, &screened_view(false));
        assert_eq!(without.matches("class=\"drain-wire\"").count(), 2);
        assert!(!without.contains("Stud"));
        // An unlabelled cable screen is called what it is.
        assert!(without.contains(">\nShield\n</text>"));

        let with = render(&design, &screened_view(true));
        assert_eq!(with.matches("class=\"drain-wire\"").count(), 3);
    }

    #[test]
    fn a_shield_alone_never_draws_a_cable_box() {
        // The conductors' far ends are left out, so the cable is not drawn,
        // and neither is its shield, though both its ends are included.
        let design = screened_design(&format!("{PAIR} shield [a.shield, g.gnd];"));
        let view = harness_view("sys", &[("a", "c", 0.0, 0.0), ("g", "j", 30.0, 0.0)]);
        let svg = render(&design, &view);
        assert!(!svg.contains("class=\"connector cable-box\""));
        assert!(!svg.contains("class=\"drain\""));
        assert!(
            !svg.contains("SHIELD"),
            "the drain pin stays out of the table"
        );
    }

    #[test]
    fn a_drain_may_share_a_pin_with_a_wire() {
        let design = screened_design(&format!(
            "{PAIR} wire black 0.5 \"GND\" [a.gnd, b.shield]; shield [a.gnd];"
        ));
        let layout = compute(&design, &screened_view(false));
        let ecu = &layout.nodes[0];
        assert_eq!(
            ecu.pins.iter().filter(|r| r.port.as_str() == "gnd").count(),
            1,
            "one row for the pin the wire and the drain share"
        );
        assert_eq!(layout.cable_boxes[0].drains.len(), 1);
    }

    #[test]
    fn a_shielded_pair_is_outlined_with_its_drain_below_it() {
        let design = screened_design(&format!(
            r#"wire red 0.5 "12V" [a.gnd, b.shield];
            twisted {{ {PAIR} shield [a.shield]; }}"#
        ));
        let svg = render(&design, &screened_view(false));
        assert_eq!(svg.matches("class=\"pair-shield\"").count(), 1);
        assert!(!svg.contains("class=\"cable-shield\""), "no overall screen");
        assert!(svg.contains(">\nPair shield\n</text>"));

        let layout = compute(&design, &screened_view(false));
        let cable = &layout.cable_boxes[0];
        let pair_rows: Vec<usize> = cable
            .strands
            .iter()
            .filter(|s| s.group == Some(0))
            .map(|s| s.row)
            .collect();
        let drain_row = cable.drains[0].row;
        assert_eq!(drain_row, pair_rows.iter().max().unwrap() + 1);

        let unshielded = render(
            &screened_design(&format!("twisted {{ {PAIR} }}")),
            &screened_view(false),
        );
        assert!(!unshielded.contains("class=\"pair-shield\""));
    }

    #[test]
    fn shields_are_not_counted_as_drawn_conductors() {
        use crate::render::lint::Segment;
        let design = screened_design(&format!("{PAIR} shield [a.shield, b.shield];"));
        let layout = compute(&design, &screened_view(false));
        assert_eq!(
            layout.drawn,
            [Segment { wire: 0, index: 0 }, Segment { wire: 1, index: 0 }]
        );
    }

    /// Three devices whose grounds join at splice `sp`, and the splice's
    /// run on to `bat` inside cable `run`.
    fn spliced_design() -> Design {
        design_from(
            r#"
component sys {
    a: dev "A";
    b: dev "B";
    bat: dev "Battery";
    splice sp "Ground splice" { part: "D-436"; }
    wire black 1 "GND" [a.gnd, sp];
    wire brown 0.5 "GND" [b.gnd, sp];
    cable run "Run" { wire black 2.5 "GND" [sp, bat.gnd]; }
    component dev {
        connector j "J 1p" { pub port gnd "GND" pin 1; }
    }
}
"#,
        )
    }

    /// A harness view of `sys` including `a.j`, `b.j` and `bat.j` and, when
    /// `with_splice`, the splice `sp` between them.
    fn spliced_view(with_splice: bool) -> View {
        let mut view = harness_view(
            "sys",
            &[
                ("a", "j", 0.0, 0.0),
                ("b", "j", 0.0, 10.0),
                ("bat", "j", 40.0, 5.0),
            ],
        );
        if with_splice {
            view.includes.push(Include {
                instance: InstanceName::from("sp"),
                connector: None,
                half: None,
                x: 20.0,
                y: 5.0,
                ports: Vec::new(),
                span: Span::new(FileId(0), 0, 0),
            });
        }
        view
    }

    #[test]
    fn a_splice_draws_as_a_labelled_joint_its_conductors_meet_at() {
        let design = spliced_design();
        let svg = render(&design, &spliced_view(true));
        assert_eq!(svg.matches("class=\"splice\"").count(), 1);
        assert_eq!(svg.matches("class=\"splice-dot\"").count(), 1);
        assert!(svg.contains("class=\"splice-name\""));
        assert!(svg.contains(">\nsp\n</text>"), "{svg}");
        assert!(svg.contains(">\nGround splice\n</text>"));
        assert!(svg.contains(">\nD-436\n</text>"));
        // A loose conductor stays a loose wire, a cable's runs through its box.
        assert!(svg.contains("GND · 0.5mm² · BN"));
        assert!(svg.contains("class=\"connector cable-box\""));

        let layout = compute(&design, &spliced_view(true));
        let [splice] = layout.splices.as_slice() else {
            panic!("one splice node");
        };
        assert_eq!(
            splice.centre,
            crate::render::geometry::Point::new(400.0, 100.0)
        );
        // Every conductor ends at the dot: both loose wires, and the cable
        // strand's left end. The loose wires stay on the splice's side of
        // the spine, so each is one curve, with no lane.
        assert_eq!(layout.loose.len(), 2);
        assert!(layout.loose.iter().all(|w| w.to == splice.centre));
        assert!(layout.loose.iter().all(|w| w.lane.is_none()));
        assert_eq!(layout.cable_boxes[0].strands[0].left_attach, splice.centre);
        // The pins face the splice they run to.
        use crate::render::geometry::Side;
        assert!(
            layout.nodes[..2]
                .iter()
                .all(|n| n.pins[0].side == Side::East)
        );
        assert_eq!(layout.nodes[2].pins[0].side, Side::West);
    }

    #[test]
    fn conductors_to_a_splice_the_view_leaves_out_are_not_drawn() {
        use crate::render::lint::Segment;
        let design = spliced_design();
        let without = compute(&design, &spliced_view(false));
        assert!(without.splices.is_empty());
        assert!(without.drawn.is_empty());
        assert!(without.cable_boxes.is_empty() && without.loose.is_empty());
        // And with it, all three are drawn, in wire order.
        let with = compute(&design, &spliced_view(true));
        assert_eq!(
            with.drawn,
            (0..3)
                .map(|wire| Segment { wire, index: 0 })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_splice_is_an_include_box_of_its_own() {
        let design = spliced_design();
        let subject = design.get(&design.root).expect("root");
        let layout = HarnessRenderer.layout(&design, subject, &spliced_view(true));
        let includes: Vec<usize> = layout.boxes.iter().map(|b| b.include).collect();
        assert_eq!(includes, [0, 1, 2, 3], "every include, in include order");
        let splice = &layout.boxes[3].bounds;
        assert!(splice.width > 0.0 && splice.height > 0.0);
    }

    /// The `loose_wires_project` fixture's design and each of its harness
    /// views' layouts, by title.
    fn loose_wire_layouts() -> Vec<(String, HarnessLayout)> {
        let main = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/loose_wires_project/main.wb");
        let report = crate::check_project(Some(&main));
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        let design = report.design.expect("a design");
        design
            .views
            .iter()
            .filter(|view| view.kind == ViewKind::Harness)
            .map(|view| (view.title.clone(), compute(&design, view)))
            .collect()
    }

    /// The layout of the fixture view titled `title`.
    fn loose_wire_layout(title: &str) -> HarnessLayout {
        loose_wire_layouts()
            .into_iter()
            .find(|(t, _)| t == title)
            .map(|(_, layout)| layout)
            .unwrap_or_else(|| panic!("view {title:?}"))
    }

    /// The fixture view titled `title`, rendered standalone.
    fn render_fixture_view(title: &str) -> String {
        let main = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/loose_wires_project/main.wb");
        let design = crate::check_project(Some(&main)).design.expect("a design");
        let view = design
            .views
            .iter()
            .find(|v| v.title == title)
            .expect("view");
        render(&design, view)
    }

    #[test]
    fn a_view_with_no_cable_box_draws_every_loose_wire_as_one_curve() {
        let design = two_connector_design();
        let view = harness_view("sys", &[("a", "hv", 0.0, 0.0), ("b", "hv", 12.0, 0.0)]);
        let layout = compute(&design, &view);
        assert!(layout.cable_boxes.is_empty());
        assert_eq!(layout.loose.len(), 2);
        for wire in &layout.loose {
            assert!(wire.lane.is_none());
            assert_eq!(wire.curves().len(), 1);
        }
    }

    /// A loose wire that crosses the spine well clear of every box keeps
    /// its one curve, and is drawn over the boxes, so none could hide it.
    #[test]
    fn a_loose_wire_clear_of_every_box_keeps_its_one_curve() {
        let design = design_from(
            r#"
component sys {
    a: dev "A";
    b: dev "B";
    c: dev "C";
    d: dev "D";
    cable feed "Feed" { wire red 1 "FEED" [a.p, b.p]; }
    wire blue 0.5 "SIG" [c.p, d.p];
    component dev {
        connector j "J 1p" { pub port p "P" pin 1; }
    }
}
"#,
        );
        let view = harness_view(
            "sys",
            &[
                ("a", "j", 0.0, 0.0),
                ("b", "j", 30.0, 0.0),
                ("c", "j", 0.0, 20.0),
                ("d", "j", 30.0, 20.0),
            ],
        );
        let layout = compute(&design, &view);
        let [wire] = layout.loose.as_slice() else {
            panic!("one loose wire");
        };
        assert!(wire.lane.is_none(), "nothing to clear: no lane");

        let svg = render(&design, &view);
        let cable_box = svg.find("class=\"connector cable-box\"").expect("box");
        let loose = svg.find("SIG · 0.5mm² · BU").expect("annotation");
        assert!(cable_box < loose, "loose wires draw over the boxes");
    }

    /// A lane stacked onto the spine pushes cable B down onto HORN's
    /// direct line, which was clear of every box until then: the second
    /// pass gives HORN a lane too.
    #[test]
    fn a_wire_a_pushed_box_lands_on_takes_a_lane_too() {
        let horn_loom = loose_wire_layout("Horn loom");
        let horn = horn_loom
            .loose
            .iter()
            .find(|w| w.label.as_deref() == Some("HORN"))
            .expect("HORN");
        assert!(horn.lane.is_some(), "HORN runs in a lane");
        let b = horn_loom
            .cable_boxes
            .iter()
            .find(|c| c.title == "B")
            .expect("cable B");
        // Where B was pushed to, HORN's direct line runs through it.
        assert_eq!(horn.from.y, horn.to.y);
        assert!(b.origin.y < horn.from.y && horn.from.y < b.origin.y + b.height);
    }

    /// A loose wire whose annotation is wider than the gap between its
    /// connectors keeps its lane inside the gap and cuts the annotation
    /// short; the other lane between the same connectors keeps its own,
    /// shorter span.
    #[test]
    fn a_long_annotation_is_cut_short_rather_than_the_lane_overrunning_the_gap() {
        let door = loose_wire_layout("Door loom");
        let widest = door.cable_boxes.iter().map(|b| b.width).fold(0.0, f64::max);
        let lane_of = |label: &str| {
            let wire = door
                .loose
                .iter()
                .find(|w| w.label.as_deref().is_some_and(|l| l.starts_with(label)))
                .expect("wire");
            (wire, wire.lane.expect("lane"))
        };
        let (wake, lane) = lane_of("WAKE");
        let (left, right) = (wake.from.x.min(wake.to.x), wake.from.x.max(wake.to.x));
        assert!(lane.left >= left + LANE_MIN_LEAD - 1e-6, "{lane:?}");
        assert!(lane.right <= right - LANE_MIN_LEAD + 1e-6, "{lane:?}");
        let shown = wake.annotation();
        assert!(shown.starts_with("WAKE FROM THE BODY"), "{shown}");
        assert!(shown.ends_with('…'), "{shown}");
        assert!(
            shown.chars().count() as f64 * LABEL_CHAR_WIDTH <= lane.right - lane.left,
            "the shortened annotation fits its run"
        );

        let (fault, lane) = lane_of("FAULT");
        assert_eq!(fault.annotation(), fault.full_annotation());
        let own =
            fault.full_annotation().chars().count() as f64 * LABEL_CHAR_WIDTH + 2.0 * NODE_PAD;
        assert_eq!(lane.right - lane.left, widest.max(own), "its own span");

        let svg = render_fixture_view("Door loom");
        assert!(
            svg.contains(&format!("<title>{}</title>", wake.full_annotation())),
            "the full annotation as a tooltip"
        );
    }

    /// Whether `p` lies strictly inside the cable box (its outline itself
    /// is not inside).
    fn inside(cable: &layout::CableBox, p: crate::render::geometry::Point) -> bool {
        const EPS: f64 = 1e-6;
        p.x > cable.origin.x + EPS
            && p.x < cable.origin.x + cable.width - EPS
            && p.y > cable.origin.y + EPS
            && p.y < cable.origin.y + cable.height - EPS
    }

    /// Regression: a loose wire was one pin-to-pin curve that ran straight
    /// behind any cable box on the spine (and its annotation with it), so
    /// it read as a conductor of that cable. Every loose wire's path and
    /// annotation now stay outside every box.
    #[test]
    fn loose_wires_and_their_annotations_clear_every_cable_box() {
        let layouts = loose_wire_layouts();
        assert_eq!(layouts.len(), 4);
        for (title, layout) in &layouts {
            assert!(!layout.cable_boxes.is_empty(), "{title} draws cables");
            assert!(!layout.loose.is_empty(), "{title} draws loose wires");
            for (w, wire) in layout.loose.iter().enumerate() {
                let samples: Vec<_> = wire
                    .curves()
                    .into_iter()
                    .flat_map(|curve| (0..=64).map(move |i| curve.point_at(f64::from(i) / 64.0)))
                    .collect();
                for cable in &layout.cable_boxes {
                    for &p in &samples {
                        assert!(
                            !inside(cable, p),
                            "{title}: loose wire {w} passes behind {:?} at {p:?}",
                            cable.title
                        );
                    }
                    assert!(
                        !wire.annotation_bounds().overlaps(&cable.bounds()),
                        "{title}: loose wire {w}'s annotation is under {:?}",
                        cable.title
                    );
                }
            }
            // What `check` counts as drawn is unchanged: one segment per
            // strand and loose wire.
            let strands: usize = layout.cable_boxes.iter().map(|b| b.strands.len()).sum();
            assert_eq!(strands + layout.loose.len(), layout.drawn.len(), "{title}");
        }
    }

    #[test]
    fn parallel_loose_wires_each_run_in_a_lane_of_their_own() {
        let layouts = loose_wire_layouts();
        let (_, pump) = layouts
            .iter()
            .find(|(title, _)| title == "Pump loom")
            .expect("pump loom");
        let lanes: Vec<(Option<&str>, layout::Lane)> = pump
            .loose
            .iter()
            .map(|w| (w.label.as_deref(), w.lane.expect("crosses the spine")))
            .collect();
        let [(Some("WAKE"), wake), (Some("FAULT"), fault)] = lanes.as_slice() else {
            panic!("WAKE and FAULT, in source order: {lanes:?}");
        };
        // One slot, a row each, in pin order; each at least as wide as the
        // widest cable box, so its leads clear every box.
        assert_eq!(fault.y - wake.y, ROW_HEIGHT);
        let widest = pump.cable_boxes.iter().map(|b| b.width).fold(0.0, f64::max);
        assert!(wake.right - wake.left >= widest);
        assert!(fault.right - fault.left >= widest);
        // Each annotation sits over its own lane's run.
        for wire in &pump.loose {
            let lane = wire.lane.expect("lane");
            let at = wire.annotation_at();
            assert_eq!(at.y, lane.y - 4.0);
            assert!(lane.left < at.x && at.x < lane.right);
        }
    }

    #[test]
    fn a_loose_wire_to_a_splice_across_the_spine_takes_a_lane() {
        let layouts = loose_wire_layouts();
        let (_, lamp) = layouts
            .iter()
            .find(|(title, _)| title == "Lamp loom")
            .expect("lamp loom");
        let [splice] = lamp.splices.as_slice() else {
            panic!("one splice");
        };
        let [to_splice, from_splice] = lamp.loose.as_slice() else {
            panic!("two loose wires");
        };
        // From the ECU across the spine to the splice: through a lane, and
        // still converging on the splice's dot.
        assert!(to_splice.lane.is_some());
        assert_eq!(to_splice.to, splice.centre);
        let curves = to_splice.curves();
        assert_eq!(curves.last().map(|c| c.p3), Some(splice.centre));
        // From the splice on to the lamp, both right of the spine: a
        // single curve.
        assert!(from_splice.lane.is_none());
        assert_eq!(from_splice.curves().len(), 1);
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
            let lane = w
                .lane
                .map(|l| format!(" lane y={:.0} {:.0}..{:.0}", l.y, l.left, l.right))
                .unwrap_or_default();
            writeln!(s, "loose {} {}->{}{lane}", w.color, pt(w.from), pt(w.to)).unwrap();
        }
        s
    }
}
