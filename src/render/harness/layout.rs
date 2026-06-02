//! Harness layout: turn a view's harness includes into positioned connector
//! nodes (pin tables) plus a central spine of cable boxes, ready for the
//! bezier wire router in `draw`.
//!
//! A harness include names `instance.connector`; the node is that whole
//! connector, drawn as a table of pin rows at its authored `(x, y)`. The
//! renderer derives a vertical **spine** midway between the connectors; each
//! node faces the spine, declared cables stack along it as boxes, and wires
//! run pin → cable → pin (or pin → pin for loose wires). Each subject wire is
//! chain-decomposed into consecutive pairs (`[a, b, c]` → `a–b, b–c`); a pair
//! is kept only when both ends land on *included* connectors. Connectorless
//! ports, `Own` ends, and ends on excluded connectors drop silently.

use std::cmp::Ordering;
use std::collections::HashMap;

use indexmap::IndexMap;

use super::draw::wire_annotation;
use super::{
    CABLE_GAP, CHAR_WIDTH, HEADER_HEIGHT, MIN_NODE_WIDTH, NODE_PAD, PIN_COL_WIDTH, ROW_HEIGHT,
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
    /// Which edge this pin's cable leaves by — toward the connector its
    /// conductor reaches. Pins of one node can differ (a node that bridges
    /// both directions). Filled by [`HarnessLayout::face_pins`].
    pub(super) side: Side,
    /// Where cables attach: on this pin's `side` edge, at the row centre.
    /// Filled alongside `side`.
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
    /// The node's dominant pin side — a per-node summary of the per-pin
    /// [`PinRow::side`]s, used where a single facing is wanted.
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

/// A loose wire (one not in a declared cable), resolved to its two attach
/// points. Drawn as a single bezier with no box.
pub(super) struct LooseWire {
    pub(super) from: Point,
    pub(super) to: Point,
    pub(super) color: String,
    pub(super) gauge: f64,
    pub(super) label: Option<String>,
}

/// One conductor of a declared cable: it leads in from the left connector to
/// the box's left edge at `row_y`, crosses the box, and leads out from the
/// right edge to the right connector.
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

/// A declared `cable`, drawn WireViz-style as a labelled box on the spine,
/// one row per strand.
pub(super) struct CableBox {
    /// Cable label (or designator) shown as the title.
    pub(super) title: String,
    /// `<type> · <length> m · ×<count>`, omitting absent parts.
    pub(super) subtitle: String,
    pub(super) origin: Point,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) strands: Vec<CableStrand>,
}

impl CableBox {
    /// Move the box so its top sits at `top`, re-flowing the strand rows.
    fn move_top_to(&mut self, top: f64) {
        self.origin.y = top;
        self.place_rows();
    }

    /// Set each strand's `row_y` from the current `origin.y`.
    fn place_rows(&mut self) {
        for (k, strand) in self.strands.iter_mut().enumerate() {
            strand.row_y = self.origin.y + HEADER_HEIGHT + (k as f64 + 0.5) * ROW_HEIGHT;
        }
    }
}

/// The full harness layout: connector nodes in include order, a box per
/// declared cable (placed on the spine), and the loose wires between
/// connectors. The spine x is consumed during layout and not retained.
pub(super) struct HarnessLayout {
    pub(super) nodes: Vec<ConnectorNode>,
    pub(super) cable_boxes: Vec<CableBox>,
    pub(super) loose: Vec<LooseWire>,
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

        // The spine is the vertical line midway between the connectors. Each
        // pin attaches on the edge toward the connector its conductor reaches,
        // so a node bridging both directions sends each pin the short way
        // rather than forcing the whole table to one side.
        let spine_x = Self::spine_x(&nodes);
        Self::face_pins(&mut nodes, &raws, spine_x);

        // Declared cables draw as boxes on the spine; loose wires draw as
        // direct pin-to-pin beziers.
        let (tagged, loose_raws): (Vec<RawWire>, Vec<RawWire>) =
            raws.into_iter().partition(|r| r.cable.is_some());
        let cable_boxes = Self::build_cable_boxes(&nodes, subject, spine_x, tagged);
        let loose = loose_raws
            .into_iter()
            .map(|raw| LooseWire {
                from: nodes[raw.a].pins[raw.ra].attach,
                to: nodes[raw.b].pins[raw.rb].attach,
                color: raw.color,
                gauge: raw.gauge,
                label: raw.label,
            })
            .collect();
        HarnessLayout {
            nodes,
            cable_boxes,
            loose,
        }
    }

    /// The spine x: midway between the leftmost and rightmost node centres
    /// (0 when there are no nodes).
    fn spine_x(nodes: &[ConnectorNode]) -> f64 {
        let xs = nodes.iter().map(|n| n.centre().x);
        let min = xs.clone().fold(f64::INFINITY, f64::min);
        let max = xs.fold(f64::NEG_INFINITY, f64::max);
        if min.is_finite() {
            (min + max) / 2.0
        } else {
            0.0
        }
    }

    /// Give every pin the edge it should leave by: the side toward the
    /// connector its conductor reaches (East when that connector is to the
    /// right, West when to the left), voting across the pin's conductors. A
    /// pin with no conductor, or a left/right tie, falls back to the spine
    /// side. Each node's `facing` is set to the dominant side of its pins.
    fn face_pins(nodes: &mut [ConnectorNode], wires: &[RawWire], spine_x: f64) {
        // +1 east / -1 west, summed per (node, pin row) over its conductors.
        let mut votes: HashMap<(usize, usize), i32> = HashMap::new();
        for w in wires {
            let (ax, bx) = (nodes[w.a].centre().x, nodes[w.b].centre().x);
            *votes.entry((w.a, w.ra)).or_default() += if bx >= ax { 1 } else { -1 };
            *votes.entry((w.b, w.rb)).or_default() += if ax >= bx { 1 } else { -1 };
        }

        for (ni, node) in nodes.iter_mut().enumerate() {
            let cx = node.origin.x + node.width / 2.0;
            let spine_side = if cx < spine_x { Side::East } else { Side::West };
            let (left_x, right_x) = (node.origin.x, node.origin.x + node.width);
            let mut tally = 0i32;
            for (ri, row) in node.pins.iter_mut().enumerate() {
                let side = match votes.get(&(ni, ri)).copied().unwrap_or(0) {
                    v if v > 0 => Side::East,
                    v if v < 0 => Side::West,
                    _ => spine_side,
                };
                row.side = side;
                let edge_x = if side == Side::West { left_x } else { right_x };
                row.attach = Point::new(edge_x, row.y);
                tally += if side == Side::East { 1 } else { -1 };
            }
            node.facing = match tally.cmp(&0) {
                Ordering::Greater => Side::East,
                Ordering::Less => Side::West,
                Ordering::Equal => spine_side,
            };
        }
    }

    /// Group tagged conductors by their cable designator, lay out one box per
    /// cable on the spine, then push overlapping boxes apart vertically.
    fn build_cable_boxes(
        nodes: &[ConnectorNode],
        subject: &Instance,
        spine_x: f64,
        raws: Vec<RawWire>,
    ) -> Vec<CableBox> {
        let mut groups: IndexMap<CableName, Vec<RawWire>> = IndexMap::new();
        for mut raw in raws {
            let Some(name) = raw.cable.take() else {
                continue;
            };
            groups.entry(name).or_default().push(raw);
        }
        let mut boxes: Vec<CableBox> = groups
            .into_iter()
            .map(|(name, raws)| build_cable_box(nodes, subject, &name, spine_x, raws))
            .collect();

        // De-overlap along the spine: sort by centre y, then sweep top-down
        // pushing each box clear of the previous one's bottom edge.
        boxes.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y));
        let mut floor = f64::NEG_INFINITY;
        for b in &mut boxes {
            if b.origin.y < floor {
                b.move_top_to(floor);
            }
            floor = b.origin.y + b.height + CABLE_GAP;
        }
        boxes
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
        for w in &self.loose {
            grow(w.from);
            grow(w.to);
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
            Pin::display_list(&port.pins),
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
    let header_width =
        title.chars().count().max(subtitle.chars().count()) as f64 * CHAR_WIDTH + 2.0 * NODE_PAD;
    let width = body_width.max(header_width).max(MIN_NODE_WIDTH);
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
            side: Side::East,      // set in face_pins
            attach: Point::ORIGIN, // set in face_pins
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

/// Lay out one cable box for the conductors tagged with `name`. The box is
/// centred on the spine; its vertical centre is the centroid of its strands'
/// midpoints (the de-overlap pass in `build_cable_boxes` resolves collisions).
/// Strands are ordered top-to-bottom by their midpoint y, so the bundle fans
/// out monotonically and lead beziers cross as little as possible.
fn build_cable_box(
    nodes: &[ConnectorNode],
    subject: &Instance,
    name: &CableName,
    spine_x: f64,
    raws: Vec<RawWire>,
) -> CableBox {
    let meta = subject.cables.get(name);
    let title = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| name.to_string());

    struct Strand {
        left: Point,
        right: Point,
        color: String,
        gauge: f64,
        label: Option<String>,
    }
    let mut strands: Vec<Strand> = raws
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

    // The 1D occupancy step: order rows by each conductor's midpoint y.
    strands.sort_by(|a, b| {
        let ma = (a.left.y + a.right.y) / 2.0;
        let mb = (b.left.y + b.right.y) / 2.0;
        ma.total_cmp(&mb)
    });

    let subtitle = cable_subtitle(meta, strands.len());
    let n = strands.len().max(1) as f64;
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
    let origin = Point::new(spine_x - width / 2.0, cy - height / 2.0);

    let strands = strands
        .into_iter()
        .map(|s| CableStrand {
            left_attach: s.left,
            right_attach: s.right,
            row_y: 0.0, // set by place_rows below
            color: s.color,
            gauge: s.gauge,
            label: s.label,
        })
        .collect();

    let mut cable_box = CableBox {
        title,
        subtitle,
        origin,
        width,
        height,
        strands,
    };
    cable_box.place_rows();
    cable_box
}

/// `<type> · <length> m · ×<count>`, omitting the type/length parts that are
/// unset; the conductor count is always shown.
fn cable_subtitle(meta: Option<&CableMeta>, count: usize) -> String {
    let mut parts = Vec::new();
    if let Some(t) = meta.and_then(|m| m.r#type.as_ref()) {
        parts.push(t.clone());
    }
    if let Some(l) = meta.and_then(|m| m.length) {
        parts.push(format!("{l} m"));
    }
    parts.push(format!("×{count}"));
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{ConnectorRef, InstancePath, Port, TypeName, Visibility};

    fn strand(row_y: f64) -> CableStrand {
        CableStrand {
            left_attach: Point::ORIGIN,
            right_attach: Point::ORIGIN,
            row_y,
            color: "black".into(),
            gauge: 1.0,
            label: None,
        }
    }

    #[test]
    fn subtitle_shows_present_parts_and_always_the_count() {
        let full = CableMeta {
            label: None,
            r#type: Some("2-core".into()),
            length: Some(0.8),
        };
        assert_eq!(cable_subtitle(Some(&full), 3), "2-core · 0.8 m · ×3");

        let bare = CableMeta {
            label: None,
            r#type: None,
            length: None,
        };
        assert_eq!(cable_subtitle(Some(&bare), 2), "×2");
        assert_eq!(cable_subtitle(None, 1), "×1");
    }

    #[test]
    fn move_top_to_translates_and_reflows_rows() {
        let mut cb = CableBox {
            title: "c".into(),
            subtitle: String::new(),
            origin: Point::new(0.0, 0.0),
            width: 100.0,
            height: HEADER_HEIGHT + 2.0 * ROW_HEIGHT,
            strands: vec![strand(0.0), strand(0.0)],
        };
        cb.place_rows();
        let r0 = cb.strands[0].row_y;
        let r1 = cb.strands[1].row_y;
        assert!(r1 > r0, "rows stack downward");

        cb.move_top_to(100.0);
        assert_eq!(cb.origin.y, 100.0);
        assert_eq!(cb.strands[0].row_y, r0 + 100.0);
        assert_eq!(cb.strands[1].row_y, r1 + 100.0);
    }

    #[test]
    fn connector_node_width_includes_title() {
        let title = "Very Long Descriptive Instance Label";
        let connector = ConnectorName::from("j1");
        let instance = Instance {
            path: InstancePath::root(InstanceName::from("leaf")),
            type_name: TypeName::from("leaf"),
            label: Some(title.into()),
            ports: [(
                PortName::from("p"),
                Port {
                    name: PortName::from("p"),
                    label: "P".into(),
                    visibility: Visibility::Public,
                    connector: Some(ConnectorRef {
                        name: Some(connector.clone()),
                        part: "J1".into(),
                        index: 0,
                    }),
                    pins: vec![Pin(1)],
                },
            )]
            .into_iter()
            .collect(),
            children: IndexMap::new(),
            wires: Vec::new(),
            cables: IndexMap::new(),
            connectors: IndexMap::new(),
        };

        let node = build_node(&instance, &connector, 0.0, 0.0).expect("node");
        let expected = title.chars().count() as f64 * CHAR_WIDTH + 2.0 * NODE_PAD;
        assert!(node.width >= expected);
    }
}
