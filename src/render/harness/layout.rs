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
//! declared cables stack along it as boxes, and wires run pin → cable → pin.
//! A loose wire runs pin → pin, unless that curve would meet a cable box:
//! then it runs through a lane of its own on the spine, stacked with the
//! boxes, so it never passes behind one. Each subject wire is chain-decomposed
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

use super::bezier::{Cubic, FLEX, flex};
use super::draw::{shorten, wire_annotation};
use super::{
    BADGE_INSET, BADGE_SIZE, BRAID_SECTION, CABLE_GAP, CABLE_LABEL_PAD, CHAR_WIDTH, HEADER_HEIGHT,
    LABEL_CHAR_WIDTH, LANE_MIN_LEAD, MIN_NODE_WIDTH, NODE_PAD, PIN_COL_WIDTH, ROW_HEIGHT,
    SPLICE_DOT_RADIUS, SPLICE_LINE_HEIGHT, SVG_MARGIN,
};
use crate::dsl::ir::{
    CableMeta, CableName, ConnectorName, ConnectorPropertyValue, Design, Half, HarnessNode,
    HarnessScope, Instance, InstanceName, Pin, PortName, Screened, Shield, Splice, SpliceName,
    Wire, WireColor, WireEnd,
};
use crate::render::geometry::{Bounds, Point, Side};
use crate::render::lint::{DeclaredHalves, EndTarget, IncludeTarget, Segment};

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
/// points. Drawn as a single bezier pin to pin, unless that curve (or its
/// annotation) would meet a cable box: then, when the wire crosses the
/// spine, it runs through a [`Lane`] of its own there instead.
pub(super) struct LooseWire {
    pub(super) from: Point,
    pub(super) to: Point,
    pub(super) color: WireColor,
    pub(super) gauge: f64,
    pub(super) label: Option<String>,
    /// The wire's straight run on the spine, when it has one.
    pub(super) lane: Option<Lane>,
}

/// A loose wire's straight run through the spine: a row of the spine's
/// occupancy no cable box shares, centred on the spine and at least as wide
/// as the widest box there, so its flexed leads stay clear of every box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Lane {
    /// World y of the run.
    pub(super) y: f64,
    /// Where the run starts.
    pub(super) left: f64,
    /// Where the run ends.
    pub(super) right: f64,
    /// How many characters of the annotation the run has room for, when
    /// that is fewer than it has: it is cut short to fit.
    pub(super) fit: Option<usize>,
}

/// Height of a wire annotation's 9px text above its baseline, its white
/// halo included.
const LABEL_ASCENT: f64 = 9.0;
/// Depth of a wire annotation's text (and halo) below its baseline.
const LABEL_DESCENT: f64 = 2.0;
/// Half a harness wire's stroke, its black casing included: how near a
/// wire's centre line may come to a box before the stroke touches it.
const WIRE_HALF_WIDTH: f64 = 2.0;
/// Points sampled along each curve when testing it against a box.
const CURVE_SAMPLES: u32 = 64;

impl LooseWire {
    /// The curves the wire is drawn as, in order: one flex pin to pin, or,
    /// through its lane, a lead-in flex, the straight run (a flat cubic)
    /// and a lead-out flex — the shape a cable strand takes, with no box.
    pub(super) fn curves(&self) -> Vec<Cubic> {
        let Some(lane) = self.lane else {
            return vec![flex(self.from, self.to, FLEX)];
        };
        let (left, right) = if self.from.x <= self.to.x {
            (self.from, self.to)
        } else {
            (self.to, self.from)
        };
        let (entry, exit) = (
            Point::new(lane.left, lane.y),
            Point::new(lane.right, lane.y),
        );
        vec![
            flex(left, entry, FLEX),
            flex(entry, exit, FLEX),
            flex(exit, right, FLEX),
        ]
    }

    /// Where the wire's annotation is anchored (its text's centre on the
    /// baseline): over the middle of its lane's run, as a cable strand's is
    /// over its row, or over the midpoint of its single curve.
    pub(super) fn annotation_at(&self) -> Point {
        match self.lane {
            Some(lane) => Point::new((lane.left + lane.right) / 2.0, lane.y - 4.0),
            None => {
                let mid = flex(self.from, self.to, FLEX).point_at(0.5);
                Point::new(mid.x, mid.y - 2.0)
            }
        }
    }

    /// The annotation in full: `<label> · <gauge>mm² · <code>`.
    pub(super) fn full_annotation(&self) -> String {
        wire_annotation(self.label.as_deref(), self.gauge, &self.color)
    }

    /// The annotation as drawn: in full, or cut short to the room its
    /// lane's run has.
    pub(super) fn annotation(&self) -> String {
        let full = self.full_annotation();
        match self.lane.and_then(|lane| lane.fit) {
            Some(chars) => shorten(&full, chars),
            None => full,
        }
    }

    /// The box the drawn annotation's text covers, halo included.
    pub(super) fn annotation_bounds(&self) -> Bounds {
        let at = self.annotation_at();
        let width = self.annotation().chars().count() as f64 * LABEL_CHAR_WIDTH;
        Bounds {
            origin: Point::new(at.x - width / 2.0, at.y - LABEL_ASCENT),
            width,
            height: LABEL_ASCENT + LABEL_DESCENT,
        }
    }

    /// Whether the wire as it stands — its stroke or its annotation —
    /// meets `cable`'s box, and so would read as one of its conductors (or
    /// be hidden by it).
    fn meets(&self, cable: &CableBox) -> bool {
        let reach = Bounds {
            origin: Point::new(
                cable.origin.x - WIRE_HALF_WIDTH,
                cable.origin.y - WIRE_HALF_WIDTH,
            ),
            width: cable.width + 2.0 * WIRE_HALF_WIDTH,
            height: cable.height + 2.0 * WIRE_HALF_WIDTH,
        };
        let inside = |p: Point| {
            p.x > reach.origin.x
                && p.x < reach.origin.x + reach.width
                && p.y > reach.origin.y
                && p.y < reach.origin.y + reach.height
        };
        self.curves().iter().any(|curve| {
            (0..=CURVE_SAMPLES)
                .any(|i| inside(curve.point_at(f64::from(i) / f64::from(CURVE_SAMPLES))))
        }) || self.annotation_bounds().overlaps(&cable.bounds())
    }

    /// The width the full annotation needs on a lane's run.
    fn annotation_width(&self) -> f64 {
        self.full_annotation().chars().count() as f64 * LABEL_CHAR_WIDTH + 2.0 * NODE_PAD
    }

    /// How wide a lane on the spine at `spine_x` the wire wants: as wide
    /// as the widest box there (`widest`, so its leads clear every box) or
    /// as its annotation needs, whichever is wider — but no wider than the
    /// room between its two ends allows, short of [`LANE_MIN_LEAD`] each
    /// side, where the annotation is cut short instead.
    fn lane_width(&self, spine_x: f64, widest: f64) -> f64 {
        let (left, right) = (self.from.x.min(self.to.x), self.from.x.max(self.to.x));
        let room = (2.0 * ((spine_x - left).min(right - spine_x) - LANE_MIN_LEAD)).max(0.0);
        widest.max(self.annotation_width().min(room))
    }

    /// Give the wire its lane at height `y`, `width` wide and centred on
    /// the spine at `spine_x`, its annotation cut short if it needs more.
    fn take_lane(&mut self, y: f64, spine_x: f64, width: f64) {
        let fit = (self.annotation_width() > width).then(|| {
            ((width - 2.0 * NODE_PAD) / LABEL_CHAR_WIDTH)
                .floor()
                .max(1.0) as usize
        });
        self.lane = Some(Lane {
            y,
            left: spine_x - width / 2.0,
            right: spine_x + width / 2.0,
            fit,
        });
    }
}

/// A loose wire whose ends lie either side of the spine: it can take a lane
/// there if its direct curve would meet a cable box.
struct Crossing {
    /// The wire, by index into the layout's loose wires.
    wire: usize,
    /// Its [`RawWire::order`].
    order: usize,
    /// The nodes it runs between, in order: the wires between one pair
    /// share a slot.
    pair: [AnchorNode; 2],
}

/// The midpoint y of a conductor between `a` and `b`: what the spine's
/// occupancy orders rows by and centres a stack of rows on.
fn midpoint_y(a: Point, b: Point) -> f64 {
    (a.y + b.y) / 2.0
}

/// The lanes of the laned loose wires between one pair of nodes: one row
/// each, stacked like a cable box's rows (ordered by midpoint y, centred on
/// their centroid) but with no box round them.
struct LooseSlot {
    /// The wires, as indices into the layout's loose wires, top to bottom.
    wires: Vec<usize>,
    /// World y of the slot's top edge.
    top: f64,
}

impl LooseSlot {
    /// A slot per pair of nodes among `laned` (in source order), each with
    /// its first wire's [`RawWire::order`], placed where its lanes' midpoints
    /// centre it, before the spine is stacked.
    fn gather<'c>(
        loose: &[LooseWire],
        laned: impl Iterator<Item = &'c Crossing>,
    ) -> Vec<(usize, Self)> {
        let mut pairs: IndexMap<[AnchorNode; 2], (usize, Vec<usize>)> = IndexMap::new();
        for wire in laned {
            pairs
                .entry(wire.pair)
                .or_insert_with(|| (wire.order, Vec::new()))
                .1
                .push(wire.wire);
        }
        let midpoint = |i: &usize| midpoint_y(loose[*i].from, loose[*i].to);
        pairs
            .into_values()
            .map(|(first, mut wires)| {
                wires.sort_by(|a, b| midpoint(a).total_cmp(&midpoint(b)));
                let centre = wires.iter().map(midpoint).sum::<f64>() / wires.len() as f64;
                let mut slot = Self { wires, top: 0.0 };
                slot.top = centre - slot.height() / 2.0;
                (first, slot)
            })
            .collect()
    }

    fn height(&self) -> f64 {
        self.wires.len() as f64 * ROW_HEIGHT
    }

    /// World y of the `k`th lane from the top.
    fn lane_y(&self, k: usize) -> f64 {
        self.top + (k as f64 + 0.5) * ROW_HEIGHT
    }
}

/// Something that takes a stretch of the spine: a cable box, or the lanes
/// of a [`LooseSlot`]. They are stacked along it together, so none
/// overlaps another.
enum Occupant<'a> {
    Cable(&'a mut CableBox),
    Loose(&'a mut LooseSlot),
}

impl Occupant<'_> {
    fn top(&self) -> f64 {
        match self {
            Self::Cable(cable) => cable.origin.y,
            Self::Loose(slot) => slot.top,
        }
    }

    fn height(&self) -> f64 {
        match self {
            Self::Cable(cable) => cable.height,
            Self::Loose(slot) => slot.height(),
        }
    }

    fn move_top_to(&mut self, top: f64) {
        match self {
            Self::Cable(cable) => cable.move_top_to(top),
            Self::Loose(slot) => slot.top = top,
        }
    }

    /// De-overlap along the spine: sort by top edge (ties in source order,
    /// by the first conductor each occupant carries), then sweep top-down
    /// pushing each occupant clear of the previous one's bottom edge.
    fn stack(mut occupants: Vec<(usize, Self)>) {
        occupants.sort_by(|(first_a, a), (first_b, b)| {
            a.top().total_cmp(&b.top()).then(first_a.cmp(first_b))
        });
        let mut floor = f64::NEG_INFINITY;
        for (_, occupant) in &mut occupants {
            if occupant.top() < floor {
                occupant.move_top_to(floor);
            }
            floor = occupant.top() + occupant.height() + CABLE_GAP;
        }
    }
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
    /// The box's outline.
    pub(super) fn bounds(&self) -> Bounds {
        Bounds {
            origin: self.origin,
            width: self.width,
            height: self.height,
        }
    }

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
    /// The node this end draws in.
    fn node(&self) -> HarnessNode<'d> {
        HarnessNode::Connector(self.instance, self.connector)
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
        let node = HarnessNode::of_end(design, subject, end)?;
        Self::at(design, subject, end, node)
    }

    /// `end`, which lands on `node` ([`HarnessNode::of_end`]), with the
    /// child instance and port of an end on a connector.
    fn at(
        design: &'d Design,
        subject: &'d Instance,
        end: &'d WireEnd,
        node: HarnessNode<'d>,
    ) -> Option<Self> {
        match (node, end) {
            (HarnessNode::Splice(splice), _) => Some(Self::Splice { end, splice }),
            (HarnessNode::Connector(instance, connector), WireEnd::Child { port, .. }) => {
                let child = subject.children.get(instance).and_then(|p| design.get(p))?;
                Some(Self::Connector(ConnectorEnd {
                    end,
                    instance,
                    connector,
                    port,
                    child,
                }))
            }
            (HarnessNode::Connector(..), _) => None,
        }
    }

    /// The end as the design names it.
    pub(super) fn end(&self) -> &'d WireEnd {
        match self {
            Self::Connector(connector) => connector.end,
            Self::Splice { end, .. } => end,
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
                let [from_node, to_node] = HarnessNode::between(design, subject, segment.ends)?;
                let [from, to] = segment.ends;
                let from = HarnessEnd::at(design, subject, from, from_node)?;
                let to = HarnessEnd::at(design, subject, to, to_node)?;
                Some(Self {
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

/// The node an [`Anchor`] is on: a connector node or a splice, by index.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum AnchorNode {
    Connector(usize),
    Splice(usize),
}

impl Anchor {
    fn node(self) -> AnchorNode {
        match self {
            Self::Pin { node, .. } => AnchorNode::Connector(node),
            Self::Splice(splice) => AnchorNode::Splice(splice),
        }
    }
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
    /// The conductor's place among those drawn, in wire order: the
    /// source-order tie-break along the spine.
    order: usize,
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
        let mut node_index: HashMap<HarnessNode, usize> = HashMap::new();
        let mut splice_index: HashMap<&SpliceName, usize> = HashMap::new();

        // The conductors this view draws: those whose two ends' nodes (a
        // connector or a splice) are both included.
        let scope = HarnessScope::new(design, subject, view);
        let kept: Vec<HarnessConductor> = HarnessConductor::all(design, subject)
            .filter(|c| scope.draws(c.ends.each_ref().map(HarnessEnd::end)))
            .collect();

        // The shields of the cables drawn: the overall one of each, and a
        // pair's own when the pair is drawn. A cable none of whose
        // conductors is drawn draws no box, and so none of its shields.
        let shields: Vec<DrawnShield> = subject
            .cables
            .iter()
            .flat_map(|(cable, meta)| {
                meta.shields()
                    .map(move |(screened, shield)| (cable, screened, shield))
            })
            .filter(|&(cable, screened, _)| scope.draws_screen(cable, screened))
            .map(|(cable, screened, shield)| DrawnShield {
                cable,
                screened,
                shield,
                ends: shield
                    .ends()
                    .iter()
                    .filter(|end| scope.draws_drain(cable, screened, end))
                    .filter_map(|end| match HarnessEnd::of(design, subject, end)? {
                        HarnessEnd::Connector(connector) => Some(connector),
                        // A drain lands on a pin: resolve rejects a splice.
                        HarnessEnd::Splice { .. } => None,
                    })
                    .collect(),
            })
            .collect();

        // Auto-scope: which ports of each included connector carry a kept
        // conductor or a drawn shield's drain.
        let mut wired: HashMap<HarnessNode, HashSet<PortName>> = HashMap::new();
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
            match HarnessNode::of_include(subject, inc) {
                Some(key @ HarnessNode::Connector(instance, conn)) => {
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
                Some(HarnessNode::Splice(name)) => {
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
                order: raws.len(),
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

        // Declared cables draw as boxes on the spine; loose wires as direct
        // pin-to-pin beziers, except that one crossing the spine whose
        // curve would meet a box takes a lane there instead, sharing the
        // spine's occupancy with the boxes.
        let (tagged, loose_raws): (Vec<RawWire>, Vec<RawWire>) =
            raws.into_iter().partition(|r| r.cable.is_some());
        let (mut cable_boxes, box_firsts) =
            Self::build_cable_boxes(&anchors, subject, spine_x, tagged, marks);
        let mut loose = Vec::new();
        let mut crossing = Vec::new();
        for raw in loose_raws {
            let (from, to) = (anchors.attach(raw.a), anchors.attach(raw.b));
            if from.x.min(to.x) < spine_x && spine_x < from.x.max(to.x) {
                let mut pair = [raw.a.node(), raw.b.node()];
                pair.sort();
                crossing.push(Crossing {
                    wire: loose.len(),
                    order: raw.order,
                    pair,
                });
            }
            loose.push(LooseWire {
                from,
                to,
                color: raw.color,
                gauge: raw.gauge,
                label: raw.label,
                lane: None,
            });
        }
        Self::occupy_spine(
            spine_x,
            &mut cable_boxes,
            &box_firsts,
            &mut loose,
            &crossing,
        );
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

    /// Group tagged conductors by their cable designator and lay out one
    /// box per cable on the spine, in the order the cables first appear.
    /// Each box comes with its first conductor's
    /// [`RawWire::order`]; [`Self::occupy_spine`] stacks them.
    fn build_cable_boxes(
        anchors: &Anchors,
        subject: &Instance,
        spine_x: f64,
        raws: Vec<RawWire>,
        mut marks: HashMap<&CableName, Vec<ShieldMark>>,
    ) -> (Vec<CableBox>, Vec<usize>) {
        let mut groups: IndexMap<CableName, Vec<RawWire>> = IndexMap::new();
        for mut raw in raws {
            let Some(name) = raw.cable.take() else {
                continue;
            };
            groups.entry(name).or_default().push(raw);
        }
        groups
            .into_iter()
            .map(|(name, raws)| {
                let first = raws.first().map_or(0, |raw| raw.order);
                let shields = marks.remove(&name).unwrap_or_default();
                let cable_box = build_cable_box(anchors, subject, &name, spine_x, raws, shields);
                (cable_box, first)
            })
            .unzip()
    }

    /// Share the spine between the cable boxes (each with its first
    /// conductor's order, `box_firsts`) and the loose wires that need a
    /// lane there. A wire that crosses the spine (`crossing`) takes a lane
    /// when its direct curve, or its annotation, would meet a box; a lane
    /// stacked onto the spine can push boxes onto another wire's curve, so
    /// this repeats — boxes back at their own places, the laned wires'
    /// slots stacked with them, the remaining curves tested against the
    /// boxes as they now stand — until no further wire needs a lane. Wires
    /// only ever move from direct to laned, so it ends. Leaves the boxes in
    /// their order down the spine.
    fn occupy_spine(
        spine_x: f64,
        cable_boxes: &mut [CableBox],
        box_firsts: &[usize],
        loose: &mut [LooseWire],
        crossing: &[Crossing],
    ) {
        let homes: Vec<f64> = cable_boxes.iter().map(|cable| cable.origin.y).collect();
        let mut laned = vec![false; crossing.len()];
        let slots = loop {
            for (cable, &top) in cable_boxes.iter_mut().zip(&homes) {
                cable.move_top_to(top);
            }
            let laned_wires = crossing
                .iter()
                .zip(&laned)
                .filter_map(|(wire, &laned)| laned.then_some(wire));
            let (slot_firsts, mut slots): (Vec<usize>, Vec<LooseSlot>) =
                LooseSlot::gather(loose, laned_wires).into_iter().unzip();
            let boxes = cable_boxes
                .iter_mut()
                .zip(box_firsts)
                .map(|(cable, &first)| (first, Occupant::Cable(cable)));
            let lanes = slots
                .iter_mut()
                .zip(slot_firsts)
                .map(|(slot, first)| (first, Occupant::Loose(slot)));
            Occupant::stack(boxes.chain(lanes).collect());

            let mut more = false;
            for (wire, laned) in crossing.iter().zip(&mut laned) {
                if !*laned
                    && cable_boxes
                        .iter()
                        .any(|cable| loose[wire.wire].meets(cable))
                {
                    *laned = true;
                    more = true;
                }
            }
            if !more {
                break slots;
            }
        };
        cable_boxes.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y));

        let widest = cable_boxes
            .iter()
            .map(|cable| cable.width)
            .fold(0.0, f64::max);
        // The lanes of one slot share its widest lane's width: an upper lane
        // wider than the one below would have the lower one's leads cross
        // its run. Its wires run between the same two nodes, so the same
        // room between their ends caps them all.
        for slot in &slots {
            let width = slot
                .wires
                .iter()
                .map(|&i| loose[i].lane_width(spine_x, widest))
                .fold(0.0, f64::max);
            for (k, &i) in slot.wires.iter().enumerate() {
                loose[i].take_lane(slot.lane_y(k), spine_x, width);
            }
        }
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
            if let Some(lane) = w.lane {
                grow(Point::new(lane.left, lane.y));
                grow(Point::new(lane.right, lane.y));
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
    let midpoint = |s: &Strand| midpoint_y(s.left, s.right);
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
    let cy = strands.iter().map(midpoint).sum::<f64>() / n;

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

    fn loose(from: Point, to: Point, lane: Option<Lane>) -> LooseWire {
        LooseWire {
            from,
            to,
            color: "white".into(),
            gauge: 0.5,
            label: Some("WAKE".into()),
            lane,
        }
    }

    #[test]
    fn a_wire_in_a_lane_leads_in_runs_straight_and_leads_out_whichever_way_it_runs() {
        let lane = Lane {
            y: 50.0,
            left: 100.0,
            right: 200.0,
            fit: None,
        };
        let (west, east) = (Point::new(0.0, 0.0), Point::new(300.0, 80.0));
        // Authored east to west: drawn left to right all the same.
        let [lead_in, run, lead_out] = loose(east, west, Some(lane)).curves()[..] else {
            panic!("three curves");
        };
        assert_eq!(lead_in.p0, west);
        assert_eq!(lead_in.p3, Point::new(100.0, 50.0));
        assert_eq!(run.p3, Point::new(200.0, 50.0));
        assert!(
            [run.p0, run.c1, run.c2, run.p3].iter().all(|p| p.y == 50.0),
            "the run is straight"
        );
        assert_eq!(lead_out.p3, east);
        assert_eq!(
            loose(east, west, Some(lane)).annotation_at(),
            Point::new(150.0, 46.0)
        );

        let direct = loose(west, east, None);
        assert_eq!(direct.curves(), [flex(west, east, FLEX)]);
    }

    /// Two loose wires between nodes 0 and 1 (one with a long label) and
    /// one between nodes 0 and 2, all straight through a box on the spine:
    /// the long label widens its own slot's lanes, both of them, and no
    /// other slot's.
    #[test]
    fn a_slot_shares_one_lane_width_and_widens_no_other_slot() {
        let mut cable = CableBox {
            title: "c".into(),
            subtitle: String::new(),
            origin: Point::new(-50.0, 0.0),
            width: 100.0,
            height: HEADER_HEIGHT + CABLE_LABEL_PAD + ROW_HEIGHT,
            strands: vec![strand(0)],
            shielded: false,
            shielded_pairs: Vec::new(),
            drains: Vec::new(),
        };
        cable.place_rows();
        let wire = |y: f64, label: &str| LooseWire {
            label: Some(label.into()),
            ..loose(Point::new(-400.0, y), Point::new(400.0, y), None)
        };
        let mut wires = [
            wire(60.0, "A LONG LABEL THAT NEEDS FAR MORE ROOM THAN THE BOX"),
            wire(70.0, "S1"),
            wire(80.0, "S2"),
        ];
        let between = |a: usize, b: usize| [AnchorNode::Connector(a), AnchorNode::Connector(b)];
        let crossing = [
            Crossing {
                wire: 0,
                order: 1,
                pair: between(0, 1),
            },
            Crossing {
                wire: 1,
                order: 2,
                pair: between(0, 1),
            },
            Crossing {
                wire: 2,
                order: 3,
                pair: between(0, 2),
            },
        ];
        HarnessLayout::occupy_spine(0.0, &mut [cable], &[0], &mut wires, &crossing);

        let span = |w: &LooseWire| w.lane.map(|l| l.right - l.left).expect("a lane");
        assert!(span(&wires[0]) > 100.0, "the long label widens its lane");
        assert_eq!(span(&wires[1]), span(&wires[0]), "and its slot mate's");
        assert_eq!(
            span(&wires[2]),
            wires[2].annotation_width().max(100.0),
            "the other slot keeps its own width"
        );
        assert!(span(&wires[2]) < span(&wires[0]));
    }

    #[test]
    fn spine_occupants_tied_on_their_top_stack_in_source_order() {
        let mut later = LooseSlot {
            wires: vec![0],
            top: 10.0,
        };
        let mut earlier = LooseSlot {
            wires: vec![1, 2],
            top: 10.0,
        };
        Occupant::stack(vec![
            (5, Occupant::Loose(&mut later)),
            (2, Occupant::Loose(&mut earlier)),
        ]);
        assert_eq!(earlier.top, 10.0, "the earlier conductor keeps its place");
        assert_eq!(later.top, 10.0 + 2.0 * ROW_HEIGHT + CABLE_GAP);
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
