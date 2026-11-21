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
//! into consecutive pairs (`[a, b, c]` → `a–b, b–c`), its
//! [`HarnessConductor`]s; a pair is kept only when both ends land on
//! *included* connectors. Connectorless ports, `Own` ends, and ends on
//! excluded connectors drop silently from the drawing; `check` reports a
//! conductor that no harness view of its component keeps. A drawn cable's
//! shields draw with it: a mark on its box, and a drain row with a line to
//! each terminating pin on an included connector (a shield alone never puts
//! a cable box in a view). An include naming one of the subject's splices
//! places that joint: a dot its conductors converge on, which a conductor
//! with a splice end is drawn to like to a connector's pin.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use super::draw::wire_annotation;
use super::{
    BADGE_INSET, BADGE_SIZE, BRAID_SECTION, CABLE_GAP, CABLE_LABEL_PAD, CHAR_WIDTH, HEADER_HEIGHT,
    LABEL_CHAR_WIDTH, MIN_NODE_WIDTH, NODE_PAD, PIN_COL_WIDTH, ROW_HEIGHT, SPLICE_DOT_RADIUS,
    SPLICE_LINE_HEIGHT, SVG_MARGIN,
};
use crate::dsl::ir::{
    CableMeta, CableName, ConnectorName, ConnectorPropertyValue, Design, Half, Instance,
    InstanceName, Pin, PortName, Screened, Shield, Splice, SpliceName, Wire, WireColor, WireEnd,
};
use crate::render::geometry::{Bounds, Point, Side};
use crate::render::lint::{DeclaredHalves, EndTarget, IncludeTarget, Segment};

/// What a conductor end lands on in a harness view, and what an include
/// places there: an instance's connector, or one of the subject's splices.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum NodeKey<'d> {
    Connector(&'d InstanceName, &'d ConnectorName),
    Splice(&'d SpliceName),
}

impl<'d> NodeKey<'d> {
    /// What `include` places in a view of `subject`: an instance's
    /// connector, a splice (an include naming one alone), or nothing a
    /// harness draws.
    fn of(subject: &'d Instance, include: &'d crate::dsl::ir::Include) -> Option<Self> {
        match &include.connector {
            Some(connector) => Some(Self::Connector(&include.instance, connector)),
            None => subject
                .splices
                .get_key_value(&SpliceName::from(include.instance.as_str()))
                .map(|(name, _)| Self::Splice(name)),
        }
    }
}

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
    /// Index of the include that placed this node, in `view.includes`.
    pub(super) include: usize,
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

/// A splice drawn as a joint: a dot its conductors converge on, with its
/// name, label and part beneath.
pub(super) struct SpliceNode {
    /// Index of the include that placed this splice, in `view.includes`.
    pub(super) include: usize,
    /// Where the conductors meet: the include's authored point.
    pub(super) centre: Point,
    /// The text lines under the dot, top to bottom: the name, then the
    /// label and the part when the splice has them.
    pub(super) lines: Vec<SpliceLine>,
    /// The joint's box, dot and text together: what the view's overlap
    /// check measures.
    pub(super) bounds: Bounds,
}

/// One text line of a [`SpliceNode`].
pub(super) struct SpliceLine {
    pub(super) kind: SpliceText,
    pub(super) text: String,
}

/// What a [`SpliceLine`] says, which picks its SVG class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SpliceText {
    Name,
    Label,
    Part,
}

impl SpliceNode {
    /// The joint for `splice`, placed by include number `include` with its
    /// dot at world `centre`.
    fn new(include: usize, splice: &Splice, centre: Point) -> Self {
        let lines: Vec<SpliceLine> = [
            Some((SpliceText::Name, splice.name.to_string())),
            splice.label.clone().map(|l| (SpliceText::Label, l)),
            splice.part.clone().map(|p| (SpliceText::Part, p)),
        ]
        .into_iter()
        .flatten()
        .map(|(kind, text)| SpliceLine { kind, text })
        .collect();
        let widest = lines
            .iter()
            .map(|l| l.text.chars().count() as f64 * CHAR_WIDTH)
            .fold(2.0 * SPLICE_DOT_RADIUS, f64::max);
        let width = widest + 2.0 * NODE_PAD;
        let height = 2.0 * SPLICE_DOT_RADIUS + lines.len() as f64 * SPLICE_LINE_HEIGHT + NODE_PAD;
        let bounds = Bounds {
            origin: Point::new(centre.x - width / 2.0, centre.y - SPLICE_DOT_RADIUS),
            width,
            height,
        };
        Self {
            include,
            centre,
            lines,
            bounds,
        }
    }

    /// The baseline of text line `i`, counted from the top.
    pub(super) fn line_y(&self, i: usize) -> f64 {
        self.centre.y + SPLICE_DOT_RADIUS + (i as f64 + 1.0) * SPLICE_LINE_HEIGHT
    }
}

impl ConnectorNode {
    /// The pin table's box, header included.
    pub(super) fn bounds(&self) -> Bounds {
        Bounds {
            origin: self.origin,
            width: self.width,
            height: self.height,
        }
    }

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
    /// The box row the strand runs in, counted from the top.
    pub(super) row: usize,
}

/// A shield's drain in a cable box: a row of its own (the screen running
/// through the cable), led out to each pin of an included connector the
/// drain is terminated on. A shield with no such pin has no row.
pub(super) struct DrainRow {
    pub(super) screened: Screened,
    /// The shield's label, or what it screens when it has none.
    pub(super) label: String,
    /// The terminating pins' attach points.
    pub(super) attaches: Vec<Point>,
    /// The box row the drain runs in, counted from the top.
    pub(super) row: usize,
    /// World y of that row.
    pub(super) row_y: f64,
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
    /// Whether the cable has an overall screen, marked on the box.
    pub(super) shielded: bool,
    /// The drawn twisted pairs that have a screen of their own, by group.
    pub(super) shielded_pairs: Vec<u32>,
    /// The drains of the cable's drawn shields, each in a row of its own.
    pub(super) drains: Vec<DrainRow>,
}

impl CableBox {
    /// Move the box so its top sits at `top`, re-flowing the rows.
    fn move_top_to(&mut self, top: f64) {
        self.origin.y = top;
        self.place_rows();
    }

    /// Set each strand's and drain's `row_y` from the current `origin.y`
    /// and its row.
    fn place_rows(&mut self) {
        let first = self.origin.y + HEADER_HEIGHT + CABLE_LABEL_PAD + ROW_HEIGHT / 2.0;
        let row_y = |row: usize| first + row as f64 * ROW_HEIGHT;
        for strand in &mut self.strands {
            strand.row_y = row_y(strand.row);
        }
        for drain in &mut self.drains {
            drain.row_y = row_y(drain.row);
        }
    }

    /// The rows `group`'s pair runs in, its drain's included: the first
    /// and the last. `None` when no strand of the pair is in the box.
    pub(super) fn pair_rows(&self, group: u32) -> Option<(f64, f64)> {
        let strands = self
            .strands
            .iter()
            .filter(|s| s.group == Some(group))
            .map(|s| s.row_y);
        let drains = self
            .drains
            .iter()
            .filter(|d| d.screened == Screened::Pair(group))
            .map(|d| d.row_y);
        let mut rows = strands.chain(drains);
        let first = rows.next()?;
        Some(rows.fold((first, first), |(top, bottom), y| {
            (top.min(y), bottom.max(y))
        }))
    }
}

/// The full harness layout: connector nodes in include order, a box per
/// declared cable (placed on the spine), and the loose wires between
/// connectors. The spine x is consumed during layout and not retained.
pub(super) struct HarnessLayout {
    pub(super) nodes: Vec<ConnectorNode>,
    /// The splices the view includes, in include order.
    pub(super) splices: Vec<SpliceNode>,
    pub(super) cable_boxes: Vec<CableBox>,
    pub(super) loose: Vec<LooseWire>,
    /// The conductors drawn, cabled and loose alike, in wire order: what
    /// `check` counts as covered by this view.
    pub(super) drawn: Vec<Segment>,
}

/// A wire end on a connector of one of the subject's children: an end a
/// harness view can draw, once it includes that connector.
pub(super) struct ConnectorEnd<'d> {
    end: &'d WireEnd,
    pub(super) instance: &'d InstanceName,
    pub(super) connector: &'d ConnectorName,
    pub(super) port: &'d PortName,
    /// The child instance the connector is on.
    pub(super) child: &'d Instance,
}

impl<'d> ConnectorEnd<'d> {
    /// The connector of the child `instance`'s `port`, the wire end `end`;
    /// `None` for a port on no connector.
    fn of(
        design: &'d Design,
        subject: &'d Instance,
        end: &'d WireEnd,
        instance: &'d InstanceName,
        port: &'d PortName,
    ) -> Option<Self> {
        let child = subject.children.get(instance).and_then(|p| design.get(p))?;
        let connector = &child.ports.get(port)?.connector.as_ref()?.name;
        Some(Self {
            end,
            instance,
            connector,
            port,
            child,
        })
    }

    /// The node this end draws in.
    fn node(&self) -> NodeKey<'d> {
        NodeKey::Connector(self.instance, self.connector)
    }

    /// How a harness view can include this end's connector. An inline
    /// connector is included through a half it declares, either one
    /// drawing it; one that declares no half cannot be included at all.
    pub(super) fn target(&self) -> EndTarget<'d> {
        let Some(inline) = &self.child.inline else {
            return EndTarget::Include(IncludeTarget::Connector {
                instance: self.instance,
                connector: self.connector,
            });
        };
        match DeclaredHalves::of(inline) {
            Some(halves) => EndTarget::Include(IncludeTarget::Inline {
                instance: self.instance,
                halves,
            }),
            None => EndTarget::HalflessInline(self.instance),
        }
    }
}

/// A conductor end a harness view can draw: on a connector of one of the
/// subject's children, or on one of the subject's splices.
pub(super) enum HarnessEnd<'d> {
    Connector(ConnectorEnd<'d>),
    Splice {
        end: &'d WireEnd,
        splice: &'d SpliceName,
    },
}

impl<'d> HarnessEnd<'d> {
    /// Where `end` lands in a harness view of `subject`, or `None` for an
    /// end no harness view can draw: the subject's own port, or a port on
    /// no connector.
    fn of(design: &'d Design, subject: &'d Instance, end: &'d WireEnd) -> Option<Self> {
        match end {
            WireEnd::Own(_) => None,
            WireEnd::Child { instance, port } => {
                ConnectorEnd::of(design, subject, end, instance, port).map(Self::Connector)
            }
            WireEnd::Splice(splice) => Some(Self::Splice { end, splice }),
        }
    }

    /// The end as the design names it.
    pub(super) fn end(&self) -> &'d WireEnd {
        match self {
            Self::Connector(connector) => connector.end,
            Self::Splice { end, .. } => end,
        }
    }

    /// The node this end draws in.
    fn node(&self) -> NodeKey<'d> {
        match self {
            Self::Connector(connector) => connector.node(),
            Self::Splice { splice, .. } => NodeKey::Splice(splice),
        }
    }

    /// The include that draws this end's node.
    pub(super) fn target(&self) -> EndTarget<'d> {
        match self {
            Self::Connector(connector) => connector.target(),
            Self::Splice { splice, .. } => EndTarget::Include(IncludeTarget::Splice(splice)),
        }
    }
}

/// One conductor a harness view of the subject can draw: a chain segment of
/// a subject wire (`[a, b, c]` → `a–b`, `b–c`) between connectors of two
/// different children, or a wire with a splice end (a splice's conductor
/// runs from a connector or from another splice).
pub(super) struct HarnessConductor<'d> {
    pub(super) segment: Segment,
    pub(super) wire: &'d Wire,
    pub(super) ends: [HarnessEnd<'d>; 2],
}

impl<'d> HarnessConductor<'d> {
    /// Every conductor of `subject`, in wire order. A segment with an end on
    /// no connector and no splice is not a harness conductor, and neither is
    /// one within a single connector: that is a jumper inside the housing,
    /// not a cable.
    pub(super) fn all(design: &'d Design, subject: &'d Instance) -> impl Iterator<Item = Self> {
        subject.wires.iter().enumerate().flat_map(move |(w, wire)| {
            wire.segments().filter_map(move |segment| {
                let [from, to] = segment.ends;
                let from = HarnessEnd::of(design, subject, from)?;
                let to = HarnessEnd::of(design, subject, to)?;
                (from.node() != to.node()).then_some(Self {
                    segment: Segment {
                        wire: w,
                        index: segment.index,
                    },
                    wire,
                    ends: [from, to],
                })
            })
        })
    }
}

/// A shield of a cable the view draws: the cable's overall screen, or the
/// screen of one of its pairs the view draws.
struct DrawnShield<'d> {
    cable: &'d CableName,
    screened: Screened,
    shield: &'d Shield,
    /// The shield's ends on connectors the view includes: where its drain
    /// is drawn to.
    ends: Vec<ConnectorEnd<'d>>,
}

/// A drawn shield as its cable box takes it, once the pins it lands on have
/// their attach points.
struct ShieldMark {
    screened: Screened,
    label: Option<String>,
    attaches: Vec<Point>,
}

/// Where a conductor attaches in the layout: a pin row of a connector node,
/// or a splice's dot.
#[derive(Clone, Copy)]
enum Anchor {
    Pin { node: usize, row: usize },
    Splice(usize),
}

/// The laid-out nodes a conductor can attach to.
struct Anchors<'a> {
    nodes: &'a [ConnectorNode],
    splices: &'a [SpliceNode],
}

impl Anchors<'_> {
    /// The x a conductor end at `anchor` comes from, which the pin at its
    /// other end faces toward.
    fn centre_x(&self, anchor: Anchor) -> f64 {
        match anchor {
            Anchor::Pin { node, .. } => self.nodes[node].centre().x,
            Anchor::Splice(splice) => self.splices[splice].centre.x,
        }
    }

    /// Where a conductor at `anchor` is drawn to: the pin's attach point
    /// (once [`HarnessLayout::face_pins`] has set it), or the splice's dot.
    fn attach(&self, anchor: Anchor) -> Point {
        match anchor {
            Anchor::Pin { node, row } => self.nodes[node].pins[row].attach,
            Anchor::Splice(splice) => self.splices[splice].centre,
        }
    }
}

/// A pending connection between two anchors, before facing (and thus the
/// pins' attach points) is known.
struct RawWire {
    a: Anchor,
    b: Anchor,
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
        let mut splices = Vec::new();
        let mut node_index: HashMap<NodeKey, usize> = HashMap::new();
        let mut splice_index: HashMap<&SpliceName, usize> = HashMap::new();

        // The conductors this view draws: those whose two ends' nodes (a
        // connector or a splice) are both included.
        let included: HashSet<NodeKey> = view
            .includes
            .iter()
            .filter_map(|inc| NodeKey::of(subject, inc))
            .collect();
        let kept: Vec<HarnessConductor> = HarnessConductor::all(design, subject)
            .filter(|c| c.ends.iter().all(|end| included.contains(&end.node())))
            .collect();

        // The shields of the cables drawn: the overall one of each, and a
        // pair's own when the pair is drawn. A cable none of whose
        // conductors is drawn draws no box, and so none of its shields.
        let drawn_cables: HashSet<&CableName> =
            kept.iter().filter_map(|c| c.wire.cable.as_ref()).collect();
        let drawn_pairs: HashSet<(&CableName, u32)> = kept
            .iter()
            .filter_map(|c| Some((c.wire.cable.as_ref()?, c.wire.twisted_group?)))
            .collect();
        let shields: Vec<DrawnShield> = subject
            .cables
            .iter()
            .filter(|(name, _)| drawn_cables.contains(name))
            .flat_map(|(cable, meta)| {
                meta.shields()
                    .map(move |(screened, shield)| (cable, screened, shield))
            })
            .filter(|&(cable, screened, _)| match screened {
                Screened::Cable => true,
                Screened::Pair(group) => drawn_pairs.contains(&(cable, group)),
            })
            .map(|(cable, screened, shield)| DrawnShield {
                cable,
                screened,
                shield,
                ends: shield
                    .ends()
                    .iter()
                    .filter_map(|end| match HarnessEnd::of(design, subject, end)? {
                        HarnessEnd::Connector(connector) => Some(connector),
                        // A drain lands on a pin: resolve rejects a splice.
                        HarnessEnd::Splice { .. } => None,
                    })
                    .filter(|end| included.contains(&end.node()))
                    .collect(),
            })
            .collect();

        // Auto-scope: which ports of each included connector carry a kept
        // conductor or a drawn shield's drain.
        let mut wired: HashMap<NodeKey, HashSet<PortName>> = HashMap::new();
        let conductor_ends = kept
            .iter()
            .flat_map(|c| &c.ends)
            .filter_map(|end| match end {
                HarnessEnd::Connector(connector) => Some(connector),
                HarnessEnd::Splice { .. } => None,
            });
        let drain_ends = shields.iter().flat_map(|s| &s.ends);
        for end in conductor_ends.chain(drain_ends) {
            wired
                .entry(end.node())
                .or_default()
                .insert(end.port.clone());
        }

        let no_pins = HashSet::new();
        for (i, inc) in view.includes.iter().enumerate() {
            let centre = Point::new(inc.x * step, inc.y * step);
            match NodeKey::of(subject, inc) {
                Some(key @ NodeKey::Connector(instance, conn)) => {
                    let Some(child) = subject.children.get(instance).and_then(|p| design.get(p))
                    else {
                        continue;
                    };
                    let visible = wired.get(&key).unwrap_or(&no_pins);
                    let Some(node) = build_node(i, child, conn, visible, centre, inc.half) else {
                        continue;
                    };
                    node_index.insert(key, nodes.len());
                    nodes.push(node);
                }
                Some(NodeKey::Splice(name)) => {
                    splice_index.insert(name, splices.len());
                    splices.push(SpliceNode::new(i, &subject.splices[name], centre));
                }
                None => {}
            }
        }

        // A connector end's (node, pin row).
        let locate_pin = |end: &ConnectorEnd| -> Option<(usize, usize)> {
            let &node = node_index.get(&end.node())?;
            let row = nodes[node].pins.iter().position(|r| &r.port == end.port)?;
            Some((node, row))
        };
        // Where a conductor end attaches: its pin's row, or its splice.
        let locate = |end: &HarnessEnd| -> Option<Anchor> {
            match end {
                HarnessEnd::Connector(connector) => {
                    locate_pin(connector).map(|(node, row)| Anchor::Pin { node, row })
                }
                HarnessEnd::Splice { splice, .. } => {
                    splice_index.get(splice).copied().map(Anchor::Splice)
                }
            }
        };

        let mut raws: Vec<RawWire> = Vec::new();
        let mut drawn = Vec::new();
        for conductor in kept {
            let [from, to] = &conductor.ends;
            let (Some(a), Some(b)) = (locate(from), locate(to)) else {
                continue;
            };
            let wire = conductor.wire;
            raws.push(RawWire {
                a,
                b,
                color: wire.color.clone(),
                gauge: wire.gauge,
                label: wire.label.clone(),
                cable: wire.cable.clone(),
                twisted_group: wire.twisted_group,
            });
            drawn.push(conductor.segment);
        }

        // Each drawn shield's drain pins, as (node, pin row).
        let drain_pins: Vec<Vec<(usize, usize)>> = shields
            .iter()
            .map(|shield| shield.ends.iter().filter_map(locate_pin).collect())
            .collect();

        // The spine is the vertical line midway between the connectors and
        // splices. Each pin attaches on the edge toward the connector or
        // splice its conductor reaches, so a node bridging both directions
        // sends each pin the short way rather than forcing the whole table
        // to one side.
        let spine_x = Self::spine_x(
            nodes
                .iter()
                .map(ConnectorNode::bounds)
                .chain(splices.iter().map(|s| s.bounds)),
        );
        Self::face_pins(&mut nodes, &splices, &raws, spine_x);
        let anchors = Anchors {
            nodes: &nodes,
            splices: &splices,
        };

        // A drain pin carries no conductor, so it faces the spine, where
        // its cable box is.
        let mut marks: HashMap<&CableName, Vec<ShieldMark>> = HashMap::new();
        for (shield, pins) in shields.iter().zip(drain_pins) {
            let attaches = pins
                .into_iter()
                .map(|(n, r)| nodes[n].pins[r].attach)
                .collect();
            marks.entry(shield.cable).or_default().push(ShieldMark {
                screened: shield.screened,
                label: shield.shield.label.clone(),
                attaches,
            });
        }

        // Declared cables draw as boxes on the spine; loose wires draw as
        // direct pin-to-pin beziers.
        let (tagged, loose_raws): (Vec<RawWire>, Vec<RawWire>) =
            raws.into_iter().partition(|r| r.cable.is_some());
        let cable_boxes = Self::build_cable_boxes(&anchors, subject, spine_x, tagged, marks);
        let loose = loose_raws
            .into_iter()
            .map(|raw| LooseWire {
                from: anchors.attach(raw.a),
                to: anchors.attach(raw.b),
                color: raw.color,
                gauge: raw.gauge,
                label: raw.label,
            })
            .collect();
        HarnessLayout {
            nodes,
            splices,
            cable_boxes,
            loose,
            drawn,
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
    fn spine_x(nodes: impl Iterator<Item = Bounds>) -> f64 {
        let nodes: Vec<Bounds> = nodes.collect();
        let centre = |b: &Bounds| b.origin.x + b.width / 2.0;
        let centres = nodes.iter().map(centre);
        let min = centres.clone().fold(f64::INFINITY, f64::min);
        let max = centres.fold(f64::NEG_INFINITY, f64::max);
        if !min.is_finite() {
            return 0.0;
        }
        let midpoint = (min + max) / 2.0;
        let left_edge = nodes
            .iter()
            .filter(|n| centre(n) <= midpoint)
            .map(|n| n.origin.x + n.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let right_edge = nodes
            .iter()
            .filter(|n| centre(n) > midpoint)
            .map(|n| n.origin.x)
            .fold(f64::INFINITY, f64::min);
        if right_edge >= left_edge && left_edge.is_finite() && right_edge.is_finite() {
            (left_edge + right_edge) / 2.0
        } else {
            midpoint
        }
    }

    /// Give every pin the edge it should leave by: the side toward the
    /// connector or splice its conductor reaches (East when that is to the
    /// right, West when to the left), voting across the pin's conductors. A
    /// pin with no conductor, or a left/right tie, falls back to the spine
    /// side. Each node's `facing` is set to the dominant side of its pins.
    fn face_pins(
        nodes: &mut [ConnectorNode],
        splices: &[SpliceNode],
        wires: &[RawWire],
        spine_x: f64,
    ) {
        // +1 east / -1 west, summed per (node, pin row) over its conductors.
        let mut votes: HashMap<(usize, usize), i32> = HashMap::new();
        {
            let anchors = Anchors { nodes, splices };
            for w in wires {
                let (ax, bx) = (anchors.centre_x(w.a), anchors.centre_x(w.b));
                for (anchor, here, there) in [(w.a, ax, bx), (w.b, bx, ax)] {
                    if let Anchor::Pin { node, row } = anchor {
                        *votes.entry((node, row)).or_default() +=
                            if there >= here { 1 } else { -1 };
                    }
                }
            }
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
        anchors: &Anchors,
        subject: &Instance,
        spine_x: f64,
        raws: Vec<RawWire>,
        mut marks: HashMap<&CableName, Vec<ShieldMark>>,
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
            .map(|(name, raws)| {
                let shields = marks.remove(&name).unwrap_or_default();
                build_cable_box(anchors, subject, &name, spine_x, raws, shields)
            })
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
        let boxes = self
            .nodes
            .iter()
            .map(ConnectorNode::bounds)
            .chain(self.splices.iter().map(|s| s.bounds));
        for b in boxes {
            grow(b.origin);
            grow(Point::new(b.origin.x + b.width, b.origin.y + b.height));
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
/// `conn`, placed by include number `include` and centred at world
/// `centre`. Returns `None` if the connector has no ports on this instance
/// (already reported by resolve as unknown).
///
/// `half` is the housing half an inline-connector include selected: the
/// node's subtitle carries that half's part identity (each loom's drawing
/// shows the half crimped onto it) and its header gets an M/F chip.
fn build_node(
    include: usize,
    child: &Instance,
    conn: &ConnectorName,
    visible: &HashSet<PortName>,
    centre: Point,
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

    let origin = Point::new(centre.x - width / 2.0, centre.y - height / 2.0);
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
        include,
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
/// out monotonically and lead beziers cross as little as possible. A drawn
/// shield's drain takes a row of its own: a pair's right below the pair, the
/// cable's overall one last.
fn build_cable_box(
    anchors: &Anchors,
    subject: &Instance,
    name: &CableName,
    spine_x: f64,
    raws: Vec<RawWire>,
    shields: Vec<ShieldMark>,
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
            let p1 = anchors.attach(raw.a);
            let p2 = anchors.attach(raw.b);
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

    // Rows top to bottom: the strands in their order, each terminated
    // pair's drain after the pair's last strand, the cable's drain last.
    let shielded = shields.iter().any(|s| s.screened == Screened::Cable);
    let shielded_pairs: Vec<u32> = shields
        .iter()
        .filter_map(|s| match s.screened {
            Screened::Pair(group) => Some(group),
            Screened::Cable => None,
        })
        .collect();
    let mut pending: Vec<ShieldMark> = shields
        .into_iter()
        .filter(|s| !s.attaches.is_empty())
        .collect();
    let mut order: Vec<Row> = Vec::new();
    for (i, strand) in strands.iter().enumerate() {
        order.push(Row::Strand(i));
        let pair_ends = strand
            .group
            .filter(|&g| strands.get(i + 1).and_then(|next| next.group) != Some(g));
        if let Some(group) = pair_ends
            && let Some(at) = pending
                .iter()
                .position(|s| s.screened == Screened::Pair(group))
        {
            order.push(Row::Drain(pending.remove(at)));
        }
    }
    order.extend(pending.into_iter().map(Row::Drain));
    let mut rows = vec![0; strands.len()];
    let mut drains = Vec::new();
    for (row, entry) in order.into_iter().enumerate() {
        match entry {
            Row::Strand(i) => rows[i] = row,
            Row::Drain(mark) => drains.push(DrainRow {
                screened: mark.screened,
                label: mark.label.unwrap_or_else(|| match mark.screened {
                    Screened::Cable => "Shield".to_string(),
                    Screened::Pair(_) => "Pair shield".to_string(),
                }),
                attaches: mark.attaches,
                row,
                row_y: 0.0, // set by place_rows below
            }),
        }
    }

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
    let drain_widths = drains
        .iter()
        .map(|d| d.label.chars().count() as f64 * LABEL_CHAR_WIDTH);
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
        .chain(drain_widths)
        .fold(0.0, f64::max);
    let width = (content.max(header) + 2.0 * NODE_PAD).max(MIN_NODE_WIDTH);
    let row_count = strands.len() + drains.len();
    let height = HEADER_HEIGHT + CABLE_LABEL_PAD + row_count as f64 * ROW_HEIGHT;
    let origin = Point::new(spine_x - width / 2.0, cy - height / 2.0);

    let strands = strands
        .into_iter()
        .zip(rows)
        .map(|(s, row)| CableStrand {
            left_attach: s.left,
            right_attach: s.right,
            row_y: 0.0, // set by place_rows below
            color: s.color,
            gauge: s.gauge,
            label: s.label,
            group: s.group,
            row,
        })
        .collect();

    let mut cable_box = CableBox {
        title,
        subtitle,
        origin,
        width,
        height,
        strands,
        shielded,
        shielded_pairs,
        drains,
    };
    cable_box.place_rows();
    cable_box
}

/// One row of a cable box, while the rows are being ordered.
enum Row {
    /// A strand, by its index in the sorted strands.
    Strand(usize),
    Drain(ShieldMark),
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
/// unset; the conductor count is always shown, and `shielded` follows it
/// when the cable has an overall screen.
fn cable_subtitle(meta: Option<&CableMeta>, count: usize) -> String {
    let mut parts = Vec::new();
    if let Some(t) = meta.and_then(|m| m.r#type.as_ref()) {
        parts.push(t.clone());
    }
    if let Some(length) = meta.and_then(|m| m.length) {
        parts.push(length.to_string());
    }
    parts.push(format!("×{count}"));
    if meta.is_some_and(|m| m.shield.is_some()) {
        parts.push("shielded".to_string());
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{
        ConnectorRef, InstancePath, Length, LengthUnit, PartMeta, Port, TypeName, Visibility,
    };

    fn strand(row: usize) -> CableStrand {
        CableStrand {
            left_attach: Point::ORIGIN,
            right_attach: Point::ORIGIN,
            row_y: 0.0,
            color: "black".into(),
            gauge: 1.0,
            label: None,
            group: None,
            row,
        }
    }

    /// A bare cable's metadata: no label, type, length or shield.
    fn bare_meta() -> CableMeta {
        CableMeta {
            label: None,
            r#type: None,
            length: None,
            shield: None,
            twisted: Vec::new(),
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
            span: crate::dsl::span::Span::new(crate::dsl::span::FileId(0), 0, 0),
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
                    span: crate::dsl::span::Span::new(crate::dsl::span::FileId(0), 0, 0),
                },
                crate::dsl::ir::Include {
                    instance: InstanceName::from("right"),
                    connector: Some(ConnectorName::from("j")),
                    half: None,
                    x: 30.0,
                    y: 0.0,
                    ports: Vec::new(),
                    span: crate::dsl::span::Span::new(crate::dsl::span::FileId(0), 0, 0),
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
            r#type: Some("2-core".into()),
            length: Length::new(0.8, LengthUnit::Metre),
            ..bare_meta()
        };
        assert_eq!(cable_subtitle(Some(&full), 3), "2-core · 0.8 m · ×3");

        // A length authored in millimetres shows in metres, exactly.
        let jumper = CableMeta {
            length: Length::new(150.0, LengthUnit::Millimetre),
            ..bare_meta()
        };
        assert_eq!(cable_subtitle(Some(&jumper), 2), "0.15 m · ×2");

        assert_eq!(cable_subtitle(Some(&bare_meta()), 2), "×2");
        assert_eq!(cable_subtitle(None, 1), "×1");
    }

    #[test]
    fn subtitle_says_a_cable_with_an_overall_screen_is_shielded() {
        let screened = CableMeta {
            r#type: Some("CAN".into()),
            shield: Some(Shield {
                label: None,
                termination: crate::dsl::ir::ShieldTermination::NoPin,
                span: crate::dsl::span::Span::new(crate::dsl::span::FileId(0), 0, 0),
            }),
            ..bare_meta()
        };
        assert_eq!(cable_subtitle(Some(&screened), 2), "CAN · ×2 · shielded");

        // A pair's own screen is marked round the pair, not in the title.
        let pair_only = CableMeta {
            twisted: vec![crate::dsl::ir::TwistedMeta {
                shield: screened.shield.clone(),
            }],
            ..bare_meta()
        };
        assert_eq!(cable_subtitle(Some(&pair_only), 2), "×2");
    }

    #[test]
    fn move_top_to_translates_and_reflows_rows() {
        let mut cb = CableBox {
            title: "c".into(),
            subtitle: String::new(),
            origin: Point::new(0.0, 0.0),
            width: 100.0,
            height: HEADER_HEIGHT + 3.0 * ROW_HEIGHT,
            strands: vec![strand(0), strand(2)],
            shielded: true,
            shielded_pairs: Vec::new(),
            drains: vec![DrainRow {
                screened: Screened::Cable,
                label: "Shield".into(),
                attaches: Vec::new(),
                row: 1,
                row_y: 0.0,
            }],
        };
        cb.place_rows();
        let r0 = cb.strands[0].row_y;
        let r1 = cb.strands[1].row_y;
        let drain = cb.drains[0].row_y;
        assert!(r0 < drain && drain < r1, "rows stack downward, by row");
        assert_eq!(drain - r0, ROW_HEIGHT);

        cb.move_top_to(100.0);
        assert_eq!(cb.origin.y, 100.0);
        assert_eq!(cb.strands[0].row_y, r0 + 100.0);
        assert_eq!(cb.strands[1].row_y, r1 + 100.0);
        assert_eq!(cb.drains[0].row_y, drain + 100.0);
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
            part: PartMeta::default(),
            splices: IndexMap::new(),
        };

        let visible: HashSet<PortName> = [PortName::from("p")].into_iter().collect();
        let node =
            build_node(0, &instance, &connector, &visible, Point::ORIGIN, None).expect("node");
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
            part: PartMeta::default(),
            splices: IndexMap::new(),
        };

        let visible: HashSet<PortName> = [PortName::from("a"), PortName::from("c")]
            .into_iter()
            .collect();
        let node =
            build_node(0, &instance, &connector, &visible, Point::ORIGIN, None).expect("node");
        let rows: Vec<&str> = node.pins.iter().map(|r| r.port.as_str()).collect();
        assert_eq!(rows, ["a", "c"], "only wired pins, still in pin order");

        let node = build_node(
            0,
            &instance,
            &connector,
            &HashSet::new(),
            Point::ORIGIN,
            None,
        )
        .expect("node");
        assert!(node.pins.is_empty(), "unwired include is a header-only box");
        assert_eq!(node.height, HEADER_HEIGHT);
    }
}
