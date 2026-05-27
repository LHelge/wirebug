//! Harness layout: turn a view's harness includes into positioned
//! connector nodes (pin tables) and the cables that bundle the subject's
//! wires running between them.
//!
//! A harness include names `instance.connector`; the node is that whole
//! connector, drawn as a table of pin rows. Cables mirror the schematic's
//! rule — each subject wire is chain-decomposed into consecutive pairs
//! (`[a, b, c]` → `a–b, b–c`), and a pair is kept only when both ends land
//! on *included* connectors. Connectorless ports, `Own` ends, and ends on
//! connectors the view doesn't include drop silently.

use std::collections::HashMap;

use indexmap::IndexMap;

use super::draw::wire_annotation;
use super::{
    BUNDLE_SPACING, CHAR_WIDTH, HEADER_HEIGHT, MIN_NODE_WIDTH, NODE_PAD, PIN_COL_WIDTH, ROW_HEIGHT,
    SVG_MARGIN,
};
use crate::dsl::ir::{
    CableMeta, CableName, ConnectorName, Design, Instance, InstanceName, Pin, PortName, WireEnd,
};
use crate::render::geometry::{Point, Side};

/// A node's identity within a harness view: which instance, which connector.
type NodeKey = (InstanceName, ConnectorName);

/// One pin of a connector node, as a table row.
pub(super) struct PinRow {
    pub(super) port: PortName,
    pub(super) pin: Option<String>,
    pub(super) label: String,
    /// Row centre line (world y).
    pub(super) y: f64,
    /// Where cables attach: on the node's facing edge, at the row centre.
    /// Filled once facing is known.
    pub(super) attach: Point,
}

/// A connector drawn as a pin table.
pub(super) struct ConnectorNode {
    /// Instance label (or instance name) shown as the node title.
    pub(super) title: String,
    /// `<designator> · <part>`, the connector subtitle.
    pub(super) subtitle: String,
    pub(super) origin: Point,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) facing: Side,
    pub(super) pins: Vec<PinRow>,
}

impl ConnectorNode {
    fn centre(&self) -> Point {
        Point::new(
            self.origin.x + self.width / 2.0,
            self.origin.y + self.height / 2.0,
        )
    }
}

/// One wire within a cable, resolved to its two attach points.
pub(super) struct CableWire {
    pub(super) from: Point,
    pub(super) to: Point,
    pub(super) color: String,
    pub(super) gauge: f64,
    pub(super) label: Option<String>,
}

/// A bundle of wires between two connector nodes.
pub(super) struct Cable {
    pub(super) wires: Vec<CableWire>,
}

/// One conductor of a declared cable, drawn as a coloured strand entering the
/// cable box's left edge and leaving its right edge at the strand's row.
pub(super) struct CableStrand {
    /// Connector attach on the box's left side.
    pub(super) left_attach: Point,
    /// Connector attach on the box's right side.
    pub(super) right_attach: Point,
    /// World y of this strand's row inside the box.
    pub(super) row_y: f64,
    pub(super) color: String,
    pub(super) gauge: f64,
    pub(super) label: Option<String>,
}

/// A declared `cable`, drawn WireViz-style as a labelled box between the two
/// connectors its conductors span, one row per strand.
pub(super) struct CableBox {
    /// Cable label (or designator) shown as the title.
    pub(super) title: String,
    /// `<type> · <length> m`, omitting absent parts; empty when neither is set.
    pub(super) subtitle: String,
    pub(super) origin: Point,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) strands: Vec<CableStrand>,
}

/// The full harness layout: nodes in include order, loose cables in wire
/// order, and a box per declared cable.
pub(super) struct HarnessLayout {
    pub(super) nodes: Vec<ConnectorNode>,
    pub(super) cables: Vec<Cable>,
    pub(super) cable_boxes: Vec<CableBox>,
}

/// A pending connection between two pin rows, before facing (and thus the
/// attach points) is known.
struct RawWire {
    a: usize,
    ra: usize,
    b: usize,
    rb: usize,
    color: String,
    gauge: f64,
    label: Option<String>,
    /// The declared cable this conductor belongs to, if any.
    cable: Option<CableName>,
}

impl HarnessLayout {
    /// Build the layout for `view`'s harness includes, placing nodes on the
    /// grid (`step` world units per grid unit) and bundling the subject's
    /// wires into cables.
    pub(super) fn compute(
        design: &Design,
        subject: &Instance,
        view: &crate::dsl::ir::View,
        step: f64,
    ) -> Self {
        let mut nodes = Vec::new();
        let mut index: HashMap<NodeKey, usize> = HashMap::new();

        for inc in &view.includes {
            let Some(conn) = &inc.connector else { continue };
            let Some(child) = subject
                .children
                .get(&inc.instance)
                .and_then(|p| design.get(p))
            else {
                continue;
            };
            let Some(node) = build_node(child, conn, inc.x * step, inc.y * step) else {
                continue;
            };
            index.insert((inc.instance.clone(), conn.clone()), nodes.len());
            nodes.push(node);
        }

        // Locate a wire endpoint's (node, pin row), if it lands on an
        // included connector.
        let locate = |end: &WireEnd| -> Option<(usize, usize)> {
            let WireEnd::Child { instance, port } = end else {
                return None;
            };
            let child = subject.children.get(instance).and_then(|p| design.get(p))?;
            let conn = child.ports.get(port)?.connector.as_ref()?.name.clone()?;
            let &ni = index.get(&(instance.clone(), conn))?;
            let ri = nodes[ni].pins.iter().position(|r| &r.port == port)?;
            Some((ni, ri))
        };

        let mut raws: Vec<RawWire> = Vec::new();
        for wire in &subject.wires {
            for pair in wire.endpoints.windows(2) {
                let (Some((a, ra)), Some((b, rb))) = (locate(&pair[0]), locate(&pair[1])) else {
                    continue;
                };
                if a == b {
                    continue; // a wire within one connector isn't a cable
                }
                raws.push(RawWire {
                    a,
                    ra,
                    b,
                    rb,
                    color: wire.color.clone(),
                    gauge: wire.gauge,
                    label: wire.label.clone(),
                    cable: wire.cable.clone(),
                });
            }
        }

        Self::orient_nodes(&mut nodes, &raws);
        Self::set_attach_points(&mut nodes);

        // Declared cables draw as boxes; everything else keeps the simple
        // node-pair bundle.
        let (tagged, loose): (Vec<RawWire>, Vec<RawWire>) =
            raws.into_iter().partition(|r| r.cable.is_some());
        let cables = Self::bundle(&nodes, loose);
        let cable_boxes = Self::build_cable_boxes(&nodes, subject, tagged);
        HarnessLayout {
            nodes,
            cables,
            cable_boxes,
        }
    }

    /// Group tagged conductors by their cable designator and lay out one box
    /// per cable, centred between the connectors its strands span.
    fn build_cable_boxes(
        nodes: &[ConnectorNode],
        subject: &Instance,
        raws: Vec<RawWire>,
    ) -> Vec<CableBox> {
        let mut groups: IndexMap<CableName, Vec<RawWire>> = IndexMap::new();
        for raw in raws {
            let name = raw.cable.clone().expect("partitioned to tagged only");
            groups.entry(name).or_default().push(raw);
        }
        groups
            .into_iter()
            .map(|(name, raws)| build_cable_box(nodes, subject, &name, raws))
            .collect()
    }

    /// Auto-orient each node: pins face the net horizontal direction of the
    /// connectors it cables to (East when the bulk lies right, else West).
    /// Table rows are horizontal, so facing is constrained to East/West.
    fn orient_nodes(nodes: &mut [ConnectorNode], raws: &[RawWire]) {
        let centres: Vec<Point> = nodes.iter().map(ConnectorNode::centre).collect();
        let mut dx = vec![0.0f64; nodes.len()];
        for raw in raws {
            dx[raw.a] += centres[raw.b].x - centres[raw.a].x;
            dx[raw.b] += centres[raw.a].x - centres[raw.b].x;
        }
        for (node, &sum) in nodes.iter_mut().zip(&dx) {
            node.facing = if sum < 0.0 { Side::West } else { Side::East };
        }
    }

    /// With facing known, pin attach points sit on the facing edge at each
    /// row's centre line.
    fn set_attach_points(nodes: &mut [ConnectorNode]) {
        for node in nodes.iter_mut() {
            let edge_x = match node.facing {
                Side::West => node.origin.x,
                _ => node.origin.x + node.width,
            };
            for row in &mut node.pins {
                row.attach = Point::new(edge_x, row.y);
            }
        }
    }

    /// Group connections by node pair into cables, resolving each to its
    /// attach points.
    fn bundle(nodes: &[ConnectorNode], raws: Vec<RawWire>) -> Vec<Cable> {
        let mut groups: IndexMap<(usize, usize), Vec<CableWire>> = IndexMap::new();
        for raw in raws {
            let key = (raw.a.min(raw.b), raw.a.max(raw.b));
            let from = nodes[raw.a].pins[raw.ra].attach;
            let to = nodes[raw.b].pins[raw.rb].attach;
            groups.entry(key).or_default().push(CableWire {
                from,
                to,
                color: raw.color,
                gauge: raw.gauge,
                label: raw.label,
            });
        }
        groups.into_values().map(|wires| Cable { wires }).collect()
    }

    /// The drawing's bounding box, padded for margins and the title.
    pub(super) fn viewbox(&self, has_title: bool) -> ViewBox {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        let mut grow = |p: Point| {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        };
        for node in &self.nodes {
            grow(node.origin);
            grow(Point::new(
                node.origin.x + node.width,
                node.origin.y + node.height,
            ));
        }
        for cable in &self.cables {
            for w in &cable.wires {
                grow(w.from);
                grow(w.to);
            }
        }
        for cb in &self.cable_boxes {
            grow(cb.origin);
            grow(Point::new(cb.origin.x + cb.width, cb.origin.y + cb.height));
            for s in &cb.strands {
                grow(s.left_attach);
                grow(s.right_attach);
            }
        }

        if !min_x.is_finite() {
            return ViewBox {
                x: 0.0,
                y: 0.0,
                width: MIN_NODE_WIDTH,
                height: HEADER_HEIGHT,
            };
        }

        let pad = SVG_MARGIN;
        let title_pad = if has_title { 20.0 } else { 0.0 };
        ViewBox {
            x: min_x - pad,
            y: min_y - pad - title_pad,
            width: (max_x - min_x) + 2.0 * pad,
            height: (max_y - min_y) + 2.0 * pad + title_pad,
        }
    }
}

/// The SVG viewBox for a harness drawing.
pub(super) struct ViewBox {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

/// Build one connector node from `child`'s ports that belong to connector
/// `conn`, centred at world (`cx`, `cy`). Returns `None` if the connector
/// has no ports on this instance (already reported by resolve as unknown).
fn build_node(child: &Instance, conn: &ConnectorName, cx: f64, cy: f64) -> Option<ConnectorNode> {
    let mut part = None;
    let mut rows: Vec<(PortName, Option<String>, String, Option<u32>)> = Vec::new();
    for (name, port) in &child.ports {
        let Some(cref) = &port.connector else {
            continue;
        };
        if cref.name.as_ref() != Some(conn) {
            continue;
        }
        part.get_or_insert_with(|| cref.part.clone());
        rows.push((
            name.clone(),
            format_pins(&port.pins),
            port.label.clone(),
            port.pins.first().map(|p| p.0),
        ));
    }
    let part = part?;

    // Order by first pin number; ports without a pin keep their relative
    // (source) order, after the numbered ones.
    rows.sort_by(|a, b| match (a.3, b.3) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    let title = child
        .label
        .clone()
        .unwrap_or_else(|| child.type_name.to_string());
    let subtitle = format!("{conn} · {part}");

    let widest_label = rows.iter().map(|r| r.2.chars().count()).max().unwrap_or(0);
    let body_width = PIN_COL_WIDTH + widest_label as f64 * CHAR_WIDTH + 2.0 * NODE_PAD;
    let title_width = subtitle.chars().count() as f64 * CHAR_WIDTH + 2.0 * NODE_PAD;
    let width = body_width.max(title_width).max(MIN_NODE_WIDTH);
    let height = HEADER_HEIGHT + rows.len() as f64 * ROW_HEIGHT;

    let origin = Point::new(cx - width / 2.0, cy - height / 2.0);
    let pins = rows
        .into_iter()
        .enumerate()
        .map(|(i, (port, pin, label, _))| PinRow {
            port,
            pin,
            label,
            y: origin.y + HEADER_HEIGHT + (i as f64 + 0.5) * ROW_HEIGHT,
            attach: Point::ORIGIN, // set in set_attach_points
        })
        .collect();

    Some(ConnectorNode {
        title,
        subtitle,
        origin,
        width,
        height,
        facing: Side::East,
        pins,
    })
}

/// Lay out one cable box for the conductors tagged with `name`, centred at
/// the centroid of its strands' attach points. Strands are stacked as rows in
/// the box, the leftmost attach feeding the left edge and the rightmost the
/// right edge.
fn build_cable_box(
    nodes: &[ConnectorNode],
    subject: &Instance,
    name: &CableName,
    raws: Vec<RawWire>,
) -> CableBox {
    let meta = subject.cables.get(name);
    let title = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| name.to_string());
    let subtitle = meta.map(cable_subtitle).unwrap_or_default();

    struct Strand {
        left: Point,
        right: Point,
        color: String,
        gauge: f64,
        label: Option<String>,
    }
    let strands: Vec<Strand> = raws
        .into_iter()
        .map(|raw| {
            let p1 = nodes[raw.a].pins[raw.ra].attach;
            let p2 = nodes[raw.b].pins[raw.rb].attach;
            let (left, right) = if p1.x <= p2.x { (p1, p2) } else { (p2, p1) };
            Strand {
                left,
                right,
                color: raw.color,
                gauge: raw.gauge,
                label: raw.label,
            }
        })
        .collect();

    let n = strands.len().max(1) as f64;
    let cx = strands
        .iter()
        .map(|s| (s.left.x + s.right.x) / 2.0)
        .sum::<f64>()
        / n;
    let cy = strands
        .iter()
        .map(|s| (s.left.y + s.right.y) / 2.0)
        .sum::<f64>()
        / n;

    let widest = strands
        .iter()
        .map(|s| wire_annotation(s.label.as_deref(), s.gauge).chars().count())
        .max()
        .unwrap_or(0)
        .max(title.chars().count())
        .max(subtitle.chars().count());
    let width = (widest as f64 * CHAR_WIDTH + 2.0 * NODE_PAD).max(MIN_NODE_WIDTH);
    let height = HEADER_HEIGHT + strands.len() as f64 * ROW_HEIGHT;
    let origin = Point::new(cx - width / 2.0, cy - height / 2.0);

    let strands = strands
        .into_iter()
        .enumerate()
        .map(|(k, s)| CableStrand {
            left_attach: s.left,
            right_attach: s.right,
            row_y: origin.y + HEADER_HEIGHT + (k as f64 + 0.5) * ROW_HEIGHT,
            color: s.color,
            gauge: s.gauge,
            label: s.label,
        })
        .collect();

    CableBox {
        title,
        subtitle,
        origin,
        width,
        height,
        strands,
    }
}

/// `<type> · <length> m`, omitting whichever part is unset (empty when both).
fn cable_subtitle(meta: &CableMeta) -> String {
    let mut parts = Vec::new();
    if let Some(t) = &meta.r#type {
        parts.push(t.clone());
    }
    if let Some(l) = meta.length {
        parts.push(format!("{l} m"));
    }
    parts.join(" · ")
}

/// Render a port's pins as a comma-joined string, or `None` when unassigned.
fn format_pins(pins: &[Pin]) -> Option<String> {
    if pins.is_empty() {
        return None;
    }
    Some(
        pins.iter()
            .map(Pin::to_string)
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// The per-wire orthogonal path within a cable: out from each node's attach
/// edge to a shared channel x, then vertical, then in to the other attach.
/// `k` of `n` spreads the vertical segments so the bundle reads as parallel
/// strands rather than one overlapping line.
pub(super) fn cable_path(from: Point, to: Point, k: usize, n: usize) -> [Point; 4] {
    let spread = (k as f64 - (n as f64 - 1.0) / 2.0) * BUNDLE_SPACING;
    let channel_x = (from.x + to.x) / 2.0 + spread;
    [
        from,
        Point::new(channel_x, from.y),
        Point::new(channel_x, to.y),
        to,
    ]
}

/// The midpoint of a cable wire's vertical channel, where its annotation
/// (label + gauge) is placed. Shares the spread logic with [`cable_path`].
pub(super) fn cable_label_anchor(from: Point, to: Point, k: usize, n: usize) -> Point {
    let spread = (k as f64 - (n as f64 - 1.0) / 2.0) * BUNDLE_SPACING;
    let channel_x = (from.x + to.x) / 2.0 + spread;
    Point::new(channel_x, (from.y + to.y) / 2.0)
}
