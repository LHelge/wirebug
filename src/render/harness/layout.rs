//! Harness layout: turn a view's harness includes into positioned connector
//! nodes (pin tables) plus a central spine of cable boxes, ready for the
//! bezier wire router in `draw`.
//!
//! A harness include names `instance.connector`; the node is drawn as a
//! table of pin rows at its authored `(x, y)`, **auto-scoped to the pins
//! that carry a conductor in this view** — a large control connector
//! shrinks to what the harness actually uses, and an include with no
//! conductors draws as a header-only box. The renderer derives a vertical
//! **spine** midway between the connectors; each node faces the spine,
//! declared cables stack along it as boxes, and wires run pin → cable → pin
//! (or pin → pin for loose wires). Each subject wire is chain-decomposed
//! into consecutive pairs (`[a, b, c]` → `a–b, b–c`); a pair is kept only
//! when both ends land on *included* connectors. Connectorless ports, `Own`
//! ends, and ends on excluded connectors drop silently.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use super::draw::wire_annotation;
use super::{
    BADGE_INSET, BADGE_SIZE, BRAID_SECTION, CABLE_GAP, CABLE_LABEL_PAD, CHAR_WIDTH, HEADER_HEIGHT,
    LABEL_CHAR_WIDTH, MIN_NODE_WIDTH, NODE_PAD, PIN_COL_WIDTH, ROW_HEIGHT, SVG_MARGIN,
};
use crate::dsl::ir::{
    CableMeta, CableName, ConnectorName, ConnectorPropertyValue, Design, Half, Instance,
    InstanceName, Pin, PortName, WireColor, WireEnd,
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
    /// The housing-half chip (`"M"`/`"F"`) of an inline-connector include;
    /// `None` for ordinary connectors.
    pub(super) badge: Option<&'static str>,
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
    pub(super) color: WireColor,
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
    pub(super) color: WireColor,
    pub(super) gauge: f64,
    pub(super) label: Option<String>,
    /// The strand's `twisted { }` group within its cable; strands sharing
    /// an index braid together (two-strand groups only).
    pub(super) group: Option<u32>,
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
            strand.row_y =
                self.origin.y + HEADER_HEIGHT + CABLE_LABEL_PAD + (k as f64 + 0.5) * ROW_HEIGHT;
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
    color: WireColor,
    gauge: f64,
    label: Option<String>,
    /// The declared cable this conductor belongs to, if any.
    cable: Option<CableName>,
    /// The conductor's `twisted { }` group within that cable.
    twisted_group: Option<u32>,
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

        // Auto-scope pre-pass: which ports of each included connector carry
        // a conductor in this view. Mirrors `locate` below, but keyed by
        // names — the nodes it scopes don't exist yet.
        let included: HashSet<NodeKey> = view
            .includes
            .iter()
            .filter_map(|inc| Some((inc.instance.clone(), inc.connector.clone()?)))
            .collect();
        let end_key = |end: &WireEnd| -> Option<(NodeKey, PortName)> {
            let WireEnd::Child { instance, port } = end else {
                return None;
            };
            let child = subject.children.get(instance).and_then(|p| design.get(p))?;
            let conn = child.ports.get(port)?.connector.as_ref()?.name.clone();
            let key = (instance.clone(), conn);
            included.contains(&key).then_some((key, port.clone()))
        };
        let mut wired: HashMap<NodeKey, HashSet<PortName>> = HashMap::new();
        for wire in &subject.wires {
            for pair in wire.endpoints.windows(2) {
                let (Some((ka, pa)), Some((kb, pb))) = (end_key(&pair[0]), end_key(&pair[1]))
                else {
                    continue;
                };
                if ka == kb {
                    continue; // a wire within one connector isn't a cable
                }
                wired.entry(ka).or_default().insert(pa);
                wired.entry(kb).or_default().insert(pb);
            }
        }

        let no_pins = HashSet::new();
        for inc in &view.includes {
            let Some(conn) = &inc.connector else { continue };
            let Some(child) = subject
                .children
                .get(&inc.instance)
                .and_then(|p| design.get(p))
            else {
                continue;
            };
            let visible = wired
                .get(&(inc.instance.clone(), conn.clone()))
                .unwrap_or(&no_pins);
            let Some(node) = build_node(child, conn, visible, inc.x * step, inc.y * step, inc.half)
            else {
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
            let conn = child.ports.get(port)?.connector.as_ref()?.name.clone();
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
                    twisted_group: wire.twisted_group,
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

    /// The spine x: centred in the *gap* between the left-hand and right-hand
    /// connector columns, so a cable box's lead-in and lead-out read equal
    /// even when the two sides differ in width (centring on the node *centres*
    /// skews the box toward the wider side). Nodes are split at the midpoint
    /// of their centres; the spine sits midway between the left column's
    /// rightmost edge and the right column's leftmost edge. Falls back to the
    /// centre midpoint when the split is degenerate — everything on one side,
    /// or columns that horizontally overlap (0 when there are no nodes).
    fn spine_x(nodes: &[ConnectorNode]) -> f64 {
        let centres = nodes.iter().map(|n| n.centre().x);
        let min = centres.clone().fold(f64::INFINITY, f64::min);
        let max = centres.fold(f64::NEG_INFINITY, f64::max);
        if !min.is_finite() {
            return 0.0;
        }
        let midpoint = (min + max) / 2.0;
        let left_edge = nodes
            .iter()
            .filter(|n| n.centre().x <= midpoint)
            .map(|n| n.origin.x + n.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let right_edge = nodes
            .iter()
            .filter(|n| n.centre().x > midpoint)
            .map(|n| n.origin.x)
            .fold(f64::INFINITY, f64::min);
        if right_edge >= left_edge && left_edge.is_finite() && right_edge.is_finite() {
            (left_edge + right_edge) / 2.0
        } else {
            midpoint
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
///
/// `half` is the housing half an inline-connector include selected: the
/// node's subtitle carries that half's part identity (each loom's drawing
/// shows the half crimped onto it) and its header gets an M/F chip.
fn build_node(
    child: &Instance,
    conn: &ConnectorName,
    visible: &HashSet<PortName>,
    cx: f64,
    cy: f64,
    half: Option<Half>,
) -> Option<ConnectorNode> {
    // `found` (the connector exists on this child, with its optional
    // description) comes from any port of the connector, so a fully
    // unwired include still draws (as a header-only box); the rows are
    // auto-scoped to the view's wired pins.
    let mut found: Option<Option<String>> = None;
    let mut rows: Vec<(PortName, Option<String>, String, Option<u32>)> = Vec::new();
    for (name, port) in &child.ports {
        let Some(cref) = &port.connector else {
            continue;
        };
        if &cref.name != conn {
            continue;
        }
        found.get_or_insert_with(|| cref.description.clone());
        if !visible.contains(name) {
            continue;
        }
        rows.push((
            name.clone(),
            Pin::display_list(&port.pins),
            port.label.clone(),
            port.pins.first().map(|p| p.0),
        ));
    }
    let description = found?;

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

    // An inline include's subtitle shows the selected half's part identity
    // instead of the (absent) connector description: `<designator> ·
    // <description> · <part>`, the part omitted when it just repeats the
    // description.
    let half_meta = half.and_then(|h| child.inline.as_ref()?.half(h));
    let badge = half_meta.and(half).map(Half::badge);
    let subtitle = match (half_meta, &description) {
        (Some(hm), _) => {
            let part = match hm.properties.get("part") {
                Some(ConnectorPropertyValue::Str(p)) if p != &hm.description => {
                    format!(" · {p}")
                }
                _ => String::new(),
            };
            format!("{conn} · {}{part}", hm.description)
        }
        (None, Some(d)) => format!("{conn} · {d}"),
        (None, None) => conn.to_string(),
    };

    let widest_label = rows.iter().map(|r| r.2.chars().count()).max().unwrap_or(0);
    let body_width = PIN_COL_WIDTH + widest_label as f64 * CHAR_WIDTH + 2.0 * NODE_PAD;
    // The badge chip sits in the header's top-right corner; give the header
    // room for it on both sides so the centred title stays clear of it.
    let badge_allowance = if badge.is_some() {
        2.0 * (BADGE_SIZE + BADGE_INSET)
    } else {
        0.0
    };
    let header_width = title.chars().count().max(subtitle.chars().count()) as f64 * CHAR_WIDTH
        + 2.0 * NODE_PAD
        + badge_allowance;
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
        badge,
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
        color: WireColor,
        gauge: f64,
        label: Option<String>,
        group: Option<u32>,
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
                group: raw.twisted_group,
            }
        })
        .collect();

    // The 1D occupancy step: order rows by each conductor's midpoint y —
    // except that a twisted group's strands must land in adjacent rows to
    // braid, so groups sort as one unit (by the group's centroid), members
    // by their own midpoint within it. The group id breaks unit-key ties:
    // two pairs with equal centroids (or a loose strand landing exactly on
    // a pair's centroid) would otherwise interleave by raw midpoint and
    // neither pair could braid.
    let midpoint = |s: &Strand| (s.left.y + s.right.y) / 2.0;
    let group_centroid: HashMap<u32, f64> = {
        let mut sums: HashMap<u32, (f64, f64)> = HashMap::new();
        for s in &strands {
            if let Some(g) = s.group {
                let e = sums.entry(g).or_insert((0.0, 0.0));
                e.0 += midpoint(s);
                e.1 += 1.0;
            }
        }
        sums.into_iter().map(|(g, (sum, n))| (g, sum / n)).collect()
    };
    strands.sort_by(|a, b| {
        let unit = |s: &Strand| s.group.map_or_else(|| midpoint(s), |g| group_centroid[&g]);
        unit(a)
            .total_cmp(&unit(b))
            .then_with(|| a.group.cmp(&b.group))
            .then(midpoint(a).total_cmp(&midpoint(b)))
    });

    let subtitle = cable_subtitle(meta, strands.len());
    let n = strands.len().max(1) as f64;
    let cy = strands
        .iter()
        .map(|s| (s.left.y + s.right.y) / 2.0)
        .sum::<f64>()
        / n;

    let annotation_widths: Vec<f64> = strands
        .iter()
        .map(|s| {
            wire_annotation(s.label.as_deref(), s.gauge, &s.color)
                .chars()
                .count() as f64
                * LABEL_CHAR_WIDTH
        })
        .collect();
    let header = title.chars().count().max(subtitle.chars().count()) as f64 * CHAR_WIDTH;
    // A braided pair lays out label zone · braid · label zone side by
    // side, so it needs room for all three; a straight row only its own
    // annotation. Take the widest requirement across rows.
    let pair_partner = braid_partners(strands.iter().map(|s| s.group));
    let content = strands
        .iter()
        .enumerate()
        .map(|(i, _)| match pair_partner[i] {
            Some(j) => {
                let (first, second) = (i.min(j), i.max(j));
                annotation_widths[first]
                    + annotation_widths[second]
                    + BRAID_SECTION
                    + 2.0 * NODE_PAD
            }
            None => annotation_widths[i],
        })
        .fold(0.0, f64::max);
    let width = (content.max(header) + 2.0 * NODE_PAD).max(MIN_NODE_WIDTH);
    let height = HEADER_HEIGHT + CABLE_LABEL_PAD + strands.len() as f64 * ROW_HEIGHT;
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
            group: s.group,
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

/// For each strand (by index), the index of its braid partner: the other
/// member of its `twisted { }` pair. The grammar guarantees two members
/// per group, but a hand-built design can violate that, so any other
/// size defensively gets `None` and draws straight.
pub(super) fn braid_partners(groups: impl Iterator<Item = Option<u32>>) -> Vec<Option<usize>> {
    let groups: Vec<Option<u32>> = groups.collect();
    let mut members: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, g) in groups.iter().enumerate() {
        if let Some(g) = g {
            members.entry(*g).or_default().push(i);
        }
    }
    groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let g = (*g)?;
            match *members[&g].as_slice() {
                [a, b] => Some(if i == a { b } else { a }),
                _ => None,
            }
        })
        .collect()
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
            group: None,
        }
    }

    /// Two twisted pairs whose row centroids tie exactly (as a real
    /// resolver loom's EX and SIN pairs can): the group id must break the
    /// tie so each pair still lands in adjacent rows and can braid, instead
    /// of interleaving by raw midpoint.
    #[test]
    fn tied_centroid_pairs_keep_their_strands_adjacent() {
        let design = crate::render::schematic::tests::design_from(
            r#"
component Root {
    component Left {
        connector j "J 4p" {
            pub port l1 "L1" pin 1;
            pub port l2 "L2" pin 2;
            pub port l3 "L3" pin 3;
            pub port l4 "L4" pin 4;
        }
    }

    component Right {
        connector j "J 4p" {
            pub port r1 "R1" pin 1;
            pub port r2 "R2" pin 2;
            pub port r3 "R3" pin 3;
            pub port r4 "R4" pin 4;
        }
    }

    left:  Left;
    right: Right;

    // Pair X spans rows (0, 2) and (3, 1); pair Y rows (1, 0) and (2, 3).
    // Both centroids sit at row 1.5, but the pairs interleave by midpoint.
    cable c "C" {
        twisted {
            wire red 0.5 "X+" [left.l1, right.r3];
            wire red 0.5 "X-" [left.l4, right.r2];
        }
        twisted {
            wire blue 0.5 "Y+" [left.l2, right.r1];
            wire blue 0.5 "Y-" [left.l3, right.r4];
        }
    }
}
"#,
        );
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == TypeName::from("Root"))
            .expect("root instance");
        let view = crate::dsl::ir::View {
            kind: crate::dsl::ir::ViewKind::Harness,
            title: "Harness".to_string(),
            grid: None,
            subject: TypeName::from("Root"),
            has_enclosure: false,
            enclosure: Vec::new(),
            includes: vec![
                crate::dsl::ir::Include {
                    instance: InstanceName::from("left"),
                    connector: Some(ConnectorName::from("j")),
                    half: None,
                    x: 0.0,
                    y: 0.0,
                    ports: Vec::new(),
                },
                crate::dsl::ir::Include {
                    instance: InstanceName::from("right"),
                    connector: Some(ConnectorName::from("j")),
                    half: None,
                    x: 30.0,
                    y: 0.0,
                    ports: Vec::new(),
                },
            ],
            texts: Vec::new(),
        };

        let layout = HarnessLayout::compute(&design, subject, &view, 20.0);
        let [cable] = layout.cable_boxes.as_slice() else {
            panic!("expected one cable box");
        };
        let partners = braid_partners(cable.strands.iter().map(|s| s.group));
        assert!(
            partners.iter().all(Option::is_some),
            "every strand in this design belongs to a pair"
        );
        for (i, partner) in partners.iter().enumerate() {
            if let Some(j) = partner {
                assert_eq!(
                    i.abs_diff(*j),
                    1,
                    "pair strands must sit in adjacent rows, got rows {i} and {j}"
                );
            }
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
                        name: connector.clone(),
                        description: Some("J1".into()),
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
            inline: None,
        };

        let visible: HashSet<PortName> = [PortName::from("p")].into_iter().collect();
        let node = build_node(&instance, &connector, &visible, 0.0, 0.0, None).expect("node");
        let expected = title.chars().count() as f64 * CHAR_WIDTH + 2.0 * NODE_PAD;
        assert!(node.width >= expected);
    }

    /// Auto-scope: a pin outside the view's wired set is dropped from the
    /// table; an empty set keeps the node as a header-only box.
    #[test]
    fn node_rows_scope_to_the_visible_set() {
        let connector = ConnectorName::from("j1");
        let port = |name: &str, pin: u32| {
            (
                PortName::from(name),
                Port {
                    name: PortName::from(name),
                    label: name.to_uppercase(),
                    visibility: Visibility::Public,
                    connector: Some(ConnectorRef {
                        name: connector.clone(),
                        description: Some("J1".into()),
                        index: 0,
                    }),
                    pins: vec![Pin(pin)],
                },
            )
        };
        let instance = Instance {
            path: InstancePath::root(InstanceName::from("leaf")),
            type_name: TypeName::from("leaf"),
            label: None,
            ports: [port("a", 1), port("b", 2), port("c", 3)]
                .into_iter()
                .collect(),
            children: IndexMap::new(),
            wires: Vec::new(),
            cables: IndexMap::new(),
            connectors: IndexMap::new(),
            inline: None,
        };

        let visible: HashSet<PortName> = [PortName::from("a"), PortName::from("c")]
            .into_iter()
            .collect();
        let node = build_node(&instance, &connector, &visible, 0.0, 0.0, None).expect("node");
        let rows: Vec<&str> = node.pins.iter().map(|r| r.port.as_str()).collect();
        assert_eq!(rows, ["a", "c"], "only wired pins, still in pin order");

        let node =
            build_node(&instance, &connector, &HashSet::new(), 0.0, 0.0, None).expect("node");
        assert!(node.pins.is_empty(), "unwired include is a header-only box");
        assert_eq!(node.height, HEADER_HEIGHT);
    }
}
