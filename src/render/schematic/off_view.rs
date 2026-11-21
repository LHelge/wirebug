//! Off-view references: where a listed port's wires go when the view does
//! not draw them.
//!
//! A schematic view draws a wire segment only when both its ends are placed
//! ports. A listed port whose wire lands on an end the view leaves out (an
//! unlisted port, an instance the view does not include, an own port with
//! no enclosure placement, the far side of a splice) would otherwise show
//! as a bare stub. It gets an [`OffView`] reference instead: a short stub
//! in the wire's colour running out from the port, and the far ends as
//! text at its end (`→ vcm.can2_h`), collapsed to `+N more` as room runs
//! out, the whole list in the text's tooltip.
//!
//! References never move a wire. The view is routed as if they did not
//! exist, and each reference is then fitted round what is drawn
//! ([`Placement::off_views`]): it takes the most detailed text that sits
//! clear of every wire, box, title, label, annotation, code and reference
//! already placed, and that keeps the view within `MAX_CANVAS`; where no
//! text fits it keeps its stub alone, and where not even the stub fits it
//! is left out. So `check`, which lays the view out the same way, reports
//! exactly what it would without references.

use std::fmt;

use super::draw::{CODE_HALF_HEIGHT, CodeAnchor};
use super::layout::{Connection, Grid, PlacedComponent, PlacedPort, Placement};
use super::{LABEL_INSET, PORT_RADIUS};
use crate::dsl::ir::{InstanceName, PortName, SpliceName, TypeName, WireColor, WireEnd};
use crate::render::Canvas;
use crate::render::geometry::{Bounds, Point, Side};
use crate::render::text::Font;

/// The reference's font (the `.off-view-label` rule), measured with the
/// shared estimator, which bounds the bundled font's widths: a text
/// measured as clear is clear as drawn.
const FONT_SIZE: f64 = 10.0;
const FONT: Font = Font::regular(FONT_SIZE);
/// Half the 4-unit `wire-casing` stroke: how far a wire's ink reaches
/// either side of its line.
const CASING_HALF: f64 = 2.0;
/// The white halo round the text (half its 3px stroke).
const HALO: f64 = 1.5;
/// Between the end of the drawn stub and the reference's text, and
/// between the pin annotation and the end of the stub.
const GAP: f64 = 3.0;
/// How far off the stub's line a text beside it is centred, nearest
/// first: clear of a wire running on along the line (its casing and the
/// text's halo), then also clear of that wire's code.
const BESIDE: [f64; 2] = [
    CASING_HALF + HALO + FONT_SIZE / 2.0,
    CODE_HALF_HEIGHT + 1.0 + FONT_SIZE / 2.0,
];
/// How far a leader runs along the lane, before the gap to its text.
const LEADER_RUN: f64 = 3.0;
/// The shortest stub worth drawing: one that reaches past the port's dot.
const MIN_STUB: f64 = PORT_RADIUS + 4.0;
/// A port label's font (the `.port-label` rule).
const LABEL_FONT_SIZE: f64 = 11.0;
const LABEL_FONT: Font = Font::regular(LABEL_FONT_SIZE);
/// The longest list a reference spells out, in characters (the arrow
/// aside); a longer one collapses to `+N more`.
const MAX_CHARS: usize = 32;

/// One end a listed port's undrawn wire reaches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FarEnd {
    /// A port of a child the view does not place: unlisted, or on an
    /// instance the view does not include.
    Port(WireEnd),
    /// The subject's own port, which the view does not place on an
    /// enclosure: the wire leaves the subject there.
    Leaves { port: PortName, subject: TypeName },
    /// A splice, and the ends its net joins that the view does not place.
    Splice {
        splice: SpliceName,
        beyond: Vec<WireEnd>,
    },
}

impl FarEnd {
    /// `end` as a far end of a wire in `subject`.
    pub(super) fn of(end: &WireEnd, subject: &TypeName) -> Self {
        match end {
            WireEnd::Own(port) => Self::Leaves {
                port: port.clone(),
                subject: subject.clone(),
            },
            WireEnd::Splice(splice) => Self::Splice {
                splice: splice.clone(),
                beyond: Vec::new(),
            },
            WireEnd::Child { .. } => Self::Port(end.clone()),
        }
    }

    /// The texts the end can be written as, most detailed first: a
    /// splice lists what its net joins beyond it as far as room allows,
    /// then `+N more`, then its name alone.
    fn texts(&self) -> Vec<String> {
        let Self::Splice { splice, beyond } = self else {
            return vec![self.to_string()];
        };
        let names: Vec<String> = beyond.iter().map(ToString::to_string).collect();
        let mut texts: Vec<String> = (1..=names.len())
            .rev()
            .map(|shown| format!("{splice} ({})", listed(&names, shown)))
            .collect();
        texts.push(splice.to_string());
        texts
    }
}

/// The most detailed text: the name a reader would write in a wire, a
/// mark on an own port saying the wire leaves the subject there, and a
/// splice's name with everything it joins beyond it.
impl fmt::Display for FarEnd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Port(end) => write!(f, "{end}"),
            Self::Leaves { port, subject } => write!(f, "{port} (leaves {subject})"),
            Self::Splice { splice, beyond } if beyond.is_empty() => write!(f, "{splice}"),
            Self::Splice { splice, beyond } => {
                let names: Vec<String> = beyond.iter().map(ToString::to_string).collect();
                write!(f, "{splice} ({})", names.join(", "))
            }
        }
    }
}

/// The first `shown` of `names`, then `+N more` for the rest.
fn listed(names: &[String], shown: usize) -> String {
    let mut listed = names[..shown].to_vec();
    if shown < names.len() {
        listed.push(format!("+{} more", names.len() - shown));
    }
    listed.join(", ")
}

/// A placed port's undrawn wires: every end they reach that the view does
/// not place, in wire order and then endpoint order, and the colour of the
/// first of them, which its stub is drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Undrawn {
    /// Never empty.
    pub(super) far: Vec<FarEnd>,
    pub(super) color: WireColor,
}

impl Undrawn {
    /// The texts the reference can read, most detailed first and without
    /// the arrow: one far end as [`FarEnd::texts`]; several listed as far
    /// as they go, then `+N more`, then `N more` naming none. Last, for a
    /// spot too cramped for any of them, nothing: the arrow alone says the
    /// wire goes on, and the tooltip where.
    fn texts(&self) -> Vec<String> {
        let mut texts = match self.far.as_slice() {
            [one] => one.texts(),
            far => {
                let names: Vec<String> = far.iter().map(ToString::to_string).collect();
                let mut texts: Vec<String> = (1..=names.len())
                    .rev()
                    .map(|shown| listed(&names, shown))
                    .collect();
                texts.push(format!("{} more", names.len()));
                texts
            }
        };
        texts.push(String::new());
        texts
    }

    /// Every far end in full, for the reference's tooltip.
    fn title(&self) -> String {
        let names: Vec<String> = self.far.iter().map(ToString::to_string).collect();
        names.join(", ")
    }
}

/// Which of [`Undrawn::texts`] a reference reads when nothing is in its
/// way: the most detailed one within [`MAX_CHARS`], else the last that
/// says something, which for several far ends names none of them
/// (`3 more`) and for one is the shortest way to name it.
fn preferred(texts: &[String]) -> usize {
    let saying = texts.len().saturating_sub(1);
    texts[..saying]
        .iter()
        .position(|text| text.chars().count() <= MAX_CHARS)
        .unwrap_or(saying.saturating_sub(1))
}

/// One placed port's off-view reference, laid out: the stub out from the
/// port and the text at its end, both along the way the port's wire would
/// leave it.
#[derive(Debug, Clone)]
pub(super) struct OffView {
    /// The included instance the port belongs to; `None` on the enclosure.
    pub(super) owner: Option<InstanceName>,
    pub(super) port: PortName,
    /// The way the reference runs out of the port: the side the port's
    /// texts are laid out for (the opposite one for an inverted enclosure
    /// port, which faces the interior).
    pub(super) side: Side,
    at: Point,
    texts: Vec<String>,
    /// Which of `texts` is drawn; `None` for a stub with no room for any.
    shown: Option<usize>,
    pub(super) title: String,
    pub(super) color: WireColor,
    /// How far out from the port the drawn stub ends, on its line.
    stub: f64,
    /// Where the text can go: first on the stub's line, past its end;
    /// then beside it, at the end of a leader (for a port whose drawn wire
    /// runs on along that line). A beside lane exists only where its text
    /// lies strictly nearer its own port's line than any other port's on
    /// the side.
    lanes: Vec<Lane>,
    /// Which of `lanes` the reference is drawn in.
    lane: usize,
}

/// One place for a reference's text: its centre line `across` off the
/// stub's line (toward [`OffView::beside`]), from `start` out from the
/// port. Beside the line (`across` > 0), a leader leaves the stub's line
/// `branch` out from the port, jogs across to the lane and runs along it
/// to the text.
#[derive(Debug, Clone, Copy)]
struct Lane {
    across: f64,
    branch: f64,
    start: f64,
}

impl OffView {
    /// The reference of `port` (of `component`, owned by `owner`) when it
    /// has undrawn wires, its text the most detailed one within
    /// [`MAX_CHARS`]. `drawn` is whether a drawn wire also leaves the port.
    fn new(
        owner: Option<&InstanceName>,
        component: &PlacedComponent,
        port: &PlacedPort,
        drawn: bool,
        grid: Grid,
    ) -> Option<Self> {
        let undrawn = port.off_view.as_ref()?;
        let step = grid.step();
        // A drawn wire leaving the port too turns at its routing stub's
        // tip, one step out, or runs on along it; the stub runs a step
        // past the tip, so it reads as running on rather than as the text
        // touching the turn.
        let least = if drawn { 2.0 * step } else { step };
        let half = FONT_SIZE / 2.0;
        let stub = least
            .max(MIN_STUB)
            .max(component.reach_beside(port, -half, half) + GAP);
        let on_line = Lane {
            across: 0.0,
            branch: stub,
            start: stub + GAP,
        };
        // A leader branches off past whatever sits beside the stub on the
        // lane's side, so it crosses no pin annotation, and its text starts
        // past whatever sits in the lane.
        let beside = BESIDE
            .into_iter()
            .filter(|&across| component.nearest_line_is_own(port, across))
            .map(|across| {
                let branch = MIN_STUB.max(component.reach_beside(port, 0.0, across + half) + GAP);
                let start = (branch + LEADER_RUN + GAP)
                    .max(component.reach_beside(port, across - half, across + half) + GAP);
                Lane {
                    across,
                    branch,
                    start,
                }
            });
        let texts = undrawn.texts();
        Some(Self {
            owner: owner.cloned(),
            port: port.port.clone(),
            side: port.label_side(),
            at: port.pos,
            shown: Some(preferred(&texts)),
            texts,
            title: undrawn.title(),
            color: undrawn.color.clone(),
            stub,
            lanes: [on_line].into_iter().chain(beside).collect(),
            lane: 0,
        })
    }

    /// Which way across the stub's line the beside lane lies: below a west
    /// or east stub (a pin annotation sits above it), left of a north one
    /// and right of a south one, in screen coordinates (+1 toward larger
    /// y on a west or east side, larger x on a north or south one).
    fn beside(side: Side) -> f64 {
        match side {
            Side::West | Side::East | Side::South => 1.0,
            Side::North => -1.0,
        }
    }

    /// The end the reference is drawn at, as the design names it.
    pub(super) fn end(&self) -> WireEnd {
        match &self.owner {
            Some(instance) => WireEnd::Child {
                instance: instance.clone(),
                port: self.port.clone(),
            },
            None => WireEnd::Own(self.port.clone()),
        }
    }

    /// The text drawn, if any: the arrow at the stub's end, pointing away
    /// from the port (after the list on a west side, before it elsewhere).
    pub(super) fn text(&self) -> Option<String> {
        let list = &self.texts[self.shown?];
        Some(match (self.side, list.is_empty()) {
            (Side::West, true) => "←".to_string(),
            (_, true) => "→".to_string(),
            (Side::West, false) => format!("{list} ←"),
            (_, false) => format!("→ {list}"),
        })
    }

    /// The drawn path, in the undrawn wire's colour: the stub, from the
    /// port out along its line; or, beside it, the leader, which follows
    /// the line to its branch, jogs across to the lane and runs along it
    /// to the text.
    pub(super) fn path(&self) -> Vec<Point> {
        let lane = self.lanes[self.lane];
        if self.lane == 0 {
            return vec![self.at, self.out(self.stub)];
        }
        let branch = self.out(lane.branch);
        vec![
            self.at,
            branch,
            self.across(branch, lane.across),
            self.across(self.out(lane.start - GAP), lane.across),
        ]
    }

    /// The ink of each run of [`Self::path`]: its casing either side of
    /// it, and no farther along it than it runs (the casing is cut square;
    /// at a bend the next run covers the corner).
    fn path_extents(&self) -> Vec<Bounds> {
        self.path()
            .windows(2)
            .map(|run| {
                let (a, b) = (run[0], run[1]);
                let (across_x, across_y) = if a.y == b.y {
                    (0.0, CASING_HALF)
                } else {
                    (CASING_HALF, 0.0)
                };
                Bounds {
                    origin: Point::new(a.x.min(b.x) - across_x, a.y.min(b.y) - across_y),
                    width: (a.x - b.x).abs() + 2.0 * across_x,
                    height: (a.y - b.y).abs() + 2.0 * across_y,
                }
            })
            .collect()
    }

    /// Whether the drawn path runs only where a drawn wire leaving the
    /// port along the stub's line, out to `run_end`, already covers it.
    fn hidden_by(&self, run_end: Point) -> bool {
        let on_line = match self.side {
            Side::West | Side::East => run_end.y == self.at.y,
            Side::North | Side::South => run_end.x == self.at.x,
        };
        let reach = self.distance_to(&Bounds {
            origin: run_end,
            width: 0.0,
            height: 0.0,
        });
        self.lane == 0 && on_line && reach >= self.stub
    }

    /// Where the text is anchored: level, on the stub's line, beside a
    /// west or east port; turned to read outward along a north or south
    /// stub (upward on the north, downward on the south).
    pub(super) fn anchor(&self) -> (Point, &'static str, f64) {
        let lane = self.lanes[self.lane];
        let at = self.across(self.out(lane.start), lane.across);
        match self.side {
            Side::West => (at, "end", 0.0),
            Side::East => (at, "start", 0.0),
            Side::North => (at, "start", -90.0),
            Side::South => (at, "start", 90.0),
        }
    }

    /// The point `distance` out from the port, along the reference.
    fn out(&self, distance: f64) -> Point {
        let Point { x, y } = self.at;
        match self.side {
            Side::West => Point::new(x - distance, y),
            Side::East => Point::new(x + distance, y),
            Side::North => Point::new(x, y - distance),
            Side::South => Point::new(x, y + distance),
        }
    }

    /// `point` moved `distance` toward the beside lane's side.
    fn across(&self, point: Point, distance: f64) -> Point {
        let shift = distance * Self::beside(self.side);
        match self.side {
            Side::West | Side::East => Point::new(point.x, point.y + shift),
            Side::North | Side::South => Point::new(point.x + shift, point.y),
        }
    }

    /// The area `from`..`to` out from the port, `half` either side of a
    /// line `across` off the stub's.
    fn band(&self, from: f64, to: f64, across: f64, half: f64) -> Bounds {
        let (a, b) = (
            self.across(self.out(from), across),
            self.across(self.out(to), across),
        );
        let (min_x, max_x) = (a.x.min(b.x), a.x.max(b.x));
        let (min_y, max_y) = (a.y.min(b.y), a.y.max(b.y));
        let (min_x, max_x, min_y, max_y) = match self.side {
            Side::West | Side::East => (min_x, max_x, min_y - half, max_y + half),
            Side::North | Side::South => (min_x - half, max_x + half, min_y, max_y),
        };
        Bounds {
            origin: Point::new(min_x, min_y),
            width: max_x - min_x,
            height: max_y - min_y,
        }
    }

    /// The area the text covers, if it has one: its measured width
    /// rounded up to a whole unit, so the canvas it widens stays whole.
    pub(super) fn extent(&self) -> Option<Bounds> {
        let width = FONT.width(&self.text()?).ceil();
        let lane = self.lanes[self.lane];
        Some(self.band(lane.start, lane.start + width, lane.across, FONT_SIZE / 2.0))
    }

    /// The area the stub's ink covers on its line, before any leader.
    fn stub_extent(&self) -> Bounds {
        self.band(0.0, self.stub, 0.0, CASING_HALF)
    }

    /// Everything the reference draws, for the canvas to hold.
    pub(super) fn extents(&self) -> impl Iterator<Item = Bounds> + use<> {
        self.path_extents().into_iter().chain(self.extent())
    }

    /// How far out from the port `area` starts, along the reference.
    fn distance_to(&self, area: &Bounds) -> f64 {
        match self.side {
            Side::West => self.at.x - (area.origin.x + area.width),
            Side::East => area.origin.x - self.at.x,
            Side::North => self.at.y - (area.origin.y + area.height),
            Side::South => area.origin.y - self.at.y,
        }
    }

    /// The least detailed text, the arrow alone.
    fn shortest(&self) -> usize {
        self.texts.len() - 1
    }
}

impl PlacedComponent {
    /// How far out from `port` the texts already beside its stub reach
    /// across the band `from`..`to` off the stub's line (toward
    /// [`OffView::beside`]): the pin annotations of the ports on its side
    /// that meet the band, and, on a box's north side, the box's title,
    /// each as the shared estimator measures it.
    fn reach_beside(&self, port: &PlacedPort, from: f64, to: f64) -> f64 {
        let side = port.label_side();
        let across = |b: &Bounds| match side {
            Side::West | Side::East => (b.origin.y, b.origin.y + b.height),
            Side::North | Side::South => (b.origin.x, b.origin.x + b.width),
        };
        let line = match side {
            Side::West | Side::East => port.pos.y,
            Side::North | Side::South => port.pos.x,
        };
        let sign = OffView::beside(side);
        let (lo, hi) = {
            let (a, b) = (line + sign * from, line + sign * to);
            (a.min(b), a.max(b))
        };
        let meets = |b: &Bounds| {
            let (b_lo, b_hi) = across(b);
            b_lo < hi && b_hi > lo
        };
        let out = |b: &Bounds| match side {
            Side::West => port.pos.x - b.origin.x,
            Side::East => b.origin.x + b.width - port.pos.x,
            Side::North => port.pos.y - b.origin.y,
            Side::South => b.origin.y + b.height - port.pos.y,
        };
        let pins = self
            .ports
            .iter()
            .filter(|p| p.label_side() == side)
            .filter_map(PlacedPort::pin_bound);
        let title = (!port.inverted && side == Side::North).then(|| self.title_bound());
        // Rounded up to a whole unit, so what is placed past it stays on
        // whole coordinates.
        pins.chain(title)
            .filter(meets)
            .map(|b| out(&b))
            .fold(0.0, f64::max)
            .ceil()
    }
}

impl PlacedComponent {
    /// Whether a text centred `across` off `port`'s line (toward
    /// [`OffView::beside`]) lies strictly nearer that line than the line
    /// of any other port on the side, so it reads as `port`'s.
    fn nearest_line_is_own(&self, port: &PlacedPort, across: f64) -> bool {
        let side = port.label_side();
        let line = |p: &PlacedPort| match side {
            Side::West | Side::East => p.pos.y,
            Side::North | Side::South => p.pos.x,
        };
        let text = line(port) + OffView::beside(side) * across;
        self.ports
            .iter()
            .filter(|p| p.label_side() == side && p.port != port.port)
            .all(|p| (line(p) - text).abs() > across)
    }
}

impl PlacedPort {
    /// The area the port's name covers (11px, `LABEL_INSET` in from the
    /// port, turned on a north or south side): inside its box, or, for an
    /// inverted enclosure port, outside the boundary.
    fn label_extent(&self) -> Bounds {
        let length = LABEL_FONT.width(&self.label);
        let half = LABEL_FONT_SIZE / 2.0;
        let Point { x, y } = self.pos;
        let (x0, y0, x1, y1) = match self.label_side() {
            Side::West => (
                x + LABEL_INSET,
                y - half,
                x + LABEL_INSET + length,
                y + half,
            ),
            Side::East => (
                x - LABEL_INSET - length,
                y - half,
                x - LABEL_INSET,
                y + half,
            ),
            Side::North => (
                x - half,
                y + LABEL_INSET,
                x + half,
                y + LABEL_INSET + length,
            ),
            Side::South => (
                x - half,
                y - LABEL_INSET - length,
                x + half,
                y - LABEL_INSET,
            ),
        };
        Bounds {
            origin: Point::new(x0, y0),
            width: x1 - x0,
            height: y1 - y0,
        }
    }
}

/// A view's routed wires (one per connection, in connection order) and
/// their codes: what its references are fitted round.
pub(super) struct Drawn<'a> {
    pub(super) connections: &'a [Connection<'a>],
    pub(super) wires: &'a [Vec<Point>],
    pub(super) codes: &'a [Option<CodeAnchor>],
}

/// Everything already drawn in a view that a reference must not cover,
/// as areas. Each box and title is kept with its component's index in the
/// placement, so a reference can pass its own.
struct Clutter {
    /// Each routed wire segment and each placed reference's stub, widened
    /// by its casing, with the ends of the connection it draws (a stub's
    /// port twice).
    wires: Vec<([WireEnd; 2], Bounds)>,
    boxes: Vec<(usize, Bounds)>,
    titles: Vec<(usize, Bounds)>,
    /// Pin annotations and port labels: texts a reference's text and
    /// leader keep off, which its stub may pass beside as a drawn wire
    /// would.
    marks: Vec<Bounds>,
    /// The wire codes, the text boxes, the enclosure's title and the texts
    /// of the references placed so far.
    texts: Vec<Bounds>,
}

impl Clutter {
    fn of(placement: &Placement, drawn: &Drawn<'_>) -> Self {
        let mut wires = Vec::new();
        for (connection, wire) in drawn.connections.iter().zip(drawn.wires) {
            for run in wire.windows(2) {
                wires.push((connection.ends(), casing(run[0], run[1])));
            }
        }
        let components = placement.components.values().enumerate();
        let enclosure = placement.enclosure();
        let mut texts: Vec<Bounds> = drawn
            .codes
            .iter()
            .zip(drawn.connections)
            .filter_map(|(code, connection)| Some(code.as_ref()?.extent(connection.color)))
            .collect();
        texts.extend(placement.texts.iter().map(|text| Bounds {
            origin: text.origin,
            width: text.width,
            height: text.height,
        }));
        texts.extend(enclosure.map(PlacedComponent::title_bound));
        Self {
            wires,
            boxes: components.clone().map(|(i, pc)| (i, pc.bounds())).collect(),
            titles: components.map(|(i, pc)| (i, pc.title_bound())).collect(),
            marks: placement
                .ports()
                .flat_map(|port| port.pin_bound().into_iter().chain([port.label_extent()]))
                .collect(),
            texts,
        }
    }

    /// What `reference`'s stub (of the component at `owner`, `None` on
    /// the enclosure) would run into: wires of other connections than its
    /// port's, other boxes and titles, codes and other references' texts.
    fn blocking_stub<'c>(
        &'c self,
        reference: &OffView,
        owner: Option<usize>,
    ) -> impl Iterator<Item = &'c Bounds> + use<'c> {
        let end = reference.end();
        let own = move |(i, _): &&(usize, Bounds)| Some(*i) != owner;
        let stub = reference.stub_extent();
        let wires = self
            .wires
            .iter()
            .filter(move |(ends, _)| !ends.contains(&end))
            .map(|(_, b)| b);
        let boxes = self.boxes.iter().filter(own).map(|(_, b)| b);
        let titles = self.titles.iter().filter(own).map(|(_, b)| b);
        wires
            .chain(boxes)
            .chain(titles)
            .chain(&self.texts)
            .filter(move |b| stub.overlaps(b))
    }

    /// Whether `area` sits clear of everything drawn but the wires that
    /// end at `own` (the port a leader branches off).
    fn clears(&self, area: &Bounds, own: Option<&WireEnd>) -> bool {
        let wires = self
            .wires
            .iter()
            .filter(|(ends, _)| own.is_none_or(|own| !ends.contains(own)))
            .map(|(_, b)| b);
        let boxes = self.boxes.iter().chain(&self.titles).map(|(_, b)| b);
        !wires
            .chain(boxes)
            .chain(&self.marks)
            .chain(&self.texts)
            .any(|b| area.overlaps(b))
    }

    /// Whether `reference`, as it would be drawn, is clear: its text of
    /// everything, and a leader of everything but its own port's wires.
    /// The stub on its line is cleared before ([`Self::blocking_stub`]).
    fn fits(&self, reference: &OffView) -> bool {
        let end = reference.end();
        let leader = reference.path_extents().into_iter().skip(1);
        let text_clear = reference
            .extent()
            .is_none_or(|text| self.clears(&text, None));
        text_clear && leader.into_iter().all(|run| self.clears(&run, Some(&end)))
    }

    /// Count `reference` as drawn, for the references fitted after it.
    fn add(&mut self, reference: &OffView) {
        let end = reference.end();
        for run in reference.path_extents() {
            self.wires.push(([end.clone(), end.clone()], run));
        }
        self.texts.extend(reference.extent());
    }
}

/// The area a wire run from `a` to `b` covers, its casing included.
fn casing(a: Point, b: Point) -> Bounds {
    Bounds {
        origin: Point::new(a.x.min(b.x) - CASING_HALF, a.y.min(b.y) - CASING_HALF),
        width: (a.x - b.x).abs() + 2.0 * CASING_HALF,
        height: (a.y - b.y).abs() + 2.0 * CASING_HALF,
    }
}

impl Placement {
    /// Every placed port's off-view reference, fitted round what is
    /// already `drawn`, in port order (boxes in include order, each one's
    /// ports west, east, north, south in authored order, then the
    /// enclosure's), each fitted against the ones before it.
    ///
    /// A reference's stub first: one running into a wire of another
    /// connection, another box or title, a code, a text box or an earlier
    /// reference is cut short of it, and keeps no text; one too short to
    /// reach past the port's dot is left out. Then its text, the most
    /// detailed rung of its ladder (from the one within `MAX_CHARS` down
    /// to the arrow alone), each tried on the stub's line and then beside
    /// it at the end of a leader, that sits clear of every wire, box,
    /// title, pin annotation, port label, code, text box and earlier
    /// reference (and whose leader does too, but for its own port's
    /// wires); with none, the stub stays alone. A stub alone that a drawn
    /// wire of its own port covers shows nothing, and the reference is
    /// left out. A rung is taken only if the view's `canvas` with it
    /// still fits `MAX_CANVAS`, or grows no larger than it was; a stub
    /// alone that does not is left out. The enclosure's outline is not in
    /// the way: a reference may cross it.
    pub(super) fn off_views(
        &self,
        drawn: &Drawn<'_>,
        grid: Grid,
        canvas: impl Fn(&[OffView]) -> Canvas,
    ) -> Vec<OffView> {
        let mut clutter = Clutter::of(self, drawn);
        let here: Vec<WireEnd> = drawn
            .connections
            .iter()
            .flat_map(Connection::ends)
            .collect();
        let components = self
            .components
            .iter()
            .enumerate()
            .map(|(i, (name, pc))| (Some(i), Some(name), pc))
            .chain(self.enclosure().map(|pc| (None, None, pc)));
        let mut fitted: Vec<OffView> = Vec::new();
        for (index, owner, pc) in components {
            for port in &pc.ports {
                let end = match owner {
                    Some(instance) => WireEnd::Child {
                        instance: instance.clone(),
                        port: port.port.clone(),
                    },
                    None => WireEnd::Own(port.port.clone()),
                };
                let Some(mut reference) = OffView::new(owner, pc, port, here.contains(&end), grid)
                else {
                    continue;
                };
                // Where the port's own drawn wires first run to, from it.
                let runs_out: Vec<Point> = drawn
                    .connections
                    .iter()
                    .zip(drawn.wires)
                    .filter(|(connection, _)| connection.ends().contains(&end))
                    .filter_map(|(_, wire)| match wire.as_slice() {
                        [first, next, ..] if *first == port.pos => Some(*next),
                        [.., before, last] if *last == port.pos => Some(*before),
                        _ => None,
                    })
                    .collect();
                let first = reference.shown.unwrap_or_default();
                let lanes = reference.lanes.len();
                let blocked = clutter
                    .blocking_stub(&reference, index)
                    .map(|area| reference.distance_to(area))
                    .fold(f64::INFINITY, f64::min);
                let rungs: Vec<(Option<usize>, usize)> = if blocked.is_finite() {
                    reference.stub = blocked - 1.0;
                    vec![(None, 0)]
                } else {
                    (first..=reference.shortest())
                        .flat_map(|shown| (0..lanes).map(move |lane| (Some(shown), lane)))
                        .chain([(None, 0)])
                        .collect()
                };
                if reference.stub < MIN_STUB {
                    continue;
                }
                let before = canvas(&fitted);
                let chosen = rungs.into_iter().find(|&(shown, lane)| {
                    let mut trial = reference.clone();
                    trial.shown = shown;
                    trial.lane = lane;
                    // A stub alone that a drawn wire of its own port covers
                    // shows nothing: no room.
                    let shows = shown.is_some() || !runs_out.iter().any(|&r| trial.hidden_by(r));
                    if !shows || !clutter.fits(&trial) {
                        return false;
                    }
                    fitted.push(trial);
                    let after = canvas(&fitted);
                    fitted.pop();
                    after.fits() || after.within(&before)
                });
                let Some((shown, lane)) = chosen else {
                    continue;
                };
                reference.shown = shown;
                reference.lane = lane;
                clutter.add(&reference);
                fitted.push(reference);
            }
        }
        fitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{Design, View};
    use crate::render::MAX_CANVAS;
    use crate::render::schematic::MIN_PORT_PITCH;
    use crate::render::schematic::draw::code_anchors;
    use crate::render::schematic::route::Router;
    use crate::render::schematic::tests::{design_from, view_of};

    /// `view` of `design` placed on `grid` and routed, and its references
    /// fitted on a canvas `canvas` measures.
    struct Fitted {
        placement: Placement,
        wires: Vec<Vec<Point>>,
        references: Vec<OffView>,
    }

    fn fit_on(
        design: &Design,
        view: &View,
        grid: f64,
        canvas: impl Fn(&[OffView]) -> Canvas,
    ) -> Fitted {
        let subject = design.get(&design.root).expect("root");
        let grid = Grid::new(grid);
        let placement = Placement::compute(design, subject, view, grid).expect("places");
        let (wires, references) = {
            let connections = placement.connections();
            let pairs: Vec<_> = connections.iter().map(|c| (c.a, c.b)).collect();
            let wires = Router::build(&placement, grid.step())
                .route_all(&pairs, grid.pitch())
                .expect("routes");
            let codes = code_anchors(&wires);
            let drawn = Drawn {
                connections: &connections,
                wires: &wires,
                codes: &codes,
            };
            let references = placement.off_views(&drawn, grid, canvas);
            (wires, references)
        };
        Fitted {
            placement,
            wires,
            references,
        }
    }

    /// As [`fit_on`], on a canvas that always fits.
    fn fit(design: &Design, view: &View, grid: f64) -> Fitted {
        fit_on(design, view, grid, |_| Canvas::new(0.0, 0.0))
    }

    /// Each reference as `port: text`, `-` for a stub alone.
    fn texts(fitted: &Fitted) -> Vec<String> {
        fitted
            .references
            .iter()
            .map(|r| format!("{}: {}", r.port, r.text().unwrap_or("-".to_string())))
            .collect()
    }

    /// A box with a port on every side, each wired to `far`, which no view
    /// includes, all on one of the box's two connectors, so each pin
    /// annotation names its designator and reaches out along the stub.
    fn four_sides() -> Design {
        design_from(
            r#"
component sys {
    a: Blk;
    far: Blk;
    wire red 1 [a.e, far.e];
    wire red 1 [a.w, far.w];
    wire red 1 [a.n, far.n];
    wire red 1 [a.s, far.s];
    component Blk {
        connector signals "Signals 4p" {
            pub port e "E" pin 1;
            pub port w "W" pin 2;
            pub port n "N" pin 3;
            pub port s "S" pin 4;
        }
        connector spare "Spare 1p" {
            pub port x "X" pin 1;
        }
    }
}
"#,
        )
    }

    fn four_sides_view() -> View {
        view_of(
            "sys",
            &[(
                "a",
                0.0,
                0.0,
                &[
                    ("e", Side::East),
                    ("w", Side::West),
                    ("n", Side::North),
                    ("s", Side::South),
                ],
            )],
        )
    }

    #[test]
    fn a_reference_runs_out_from_its_port_on_every_side_past_its_pin() {
        let design = four_sides();
        let fitted = fit(&design, &four_sides_view(), 15.0);
        let a = &fitted.placement.components[&InstanceName::from("a")];
        assert_eq!(
            texts(&fitted),
            ["w: far.w ←", "e: → far.e", "n: → far.n", "s: → far.s"]
        );
        let bounds = a.bounds();
        for reference in &fitted.references {
            let text = reference.extent().expect("a text");
            let outside = match reference.side {
                Side::West => text.origin.x + text.width < bounds.origin.x,
                Side::East => text.origin.x > bounds.origin.x + bounds.width,
                Side::North => text.origin.y + text.height < bounds.origin.y,
                Side::South => text.origin.y > bounds.origin.y + bounds.height,
            };
            assert!(outside, "{:?} {text:?}", reference.side);
            // On the stub's line, past its end.
            let path = reference.path();
            let (from, to) = (path[0], path[1]);
            let port = a.ports.iter().find(|p| p.port == reference.port).unwrap();
            assert_eq!(from, port.pos);
            assert!(!text.overlaps(&reference.stub_extent()));
            assert_eq!(
                reference.distance_to(&text),
                reference.distance_to(&Bounds {
                    origin: to,
                    width: 0.0,
                    height: 0.0,
                }) + GAP
            );
        }
    }

    #[test]
    fn neighbouring_references_stay_apart_at_the_minimum_pitch() {
        let design = design_from(
            r#"
component sys {
    a: Blk;
    wire red 1 [a.p1, gone.p];
    wire red 1 [a.p2, gone.p];
    wire red 1 [a.p3, gone.p];
    wire red 1 [a.q1, gone.p];
    wire red 1 [a.q2, gone.p];
    gone: Gone;
    component Blk {
        connector x1 "X1" {
            pub port p1 "P1" pin 1;
            pub port p2 "P2" pin 12;
            pub port p3 "P3" pin 3;
        }
        connector x2 "X2" {
            pub port q1 "Q1" pin 1;
            pub port q2 "Q2" pin 2;
        }
    }
    component Gone {
        pub port p "P";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[(
                "a",
                0.0,
                0.0,
                &[
                    ("p1", Side::East),
                    ("p2", Side::East),
                    ("p3", Side::East),
                    ("q1", Side::North),
                    ("q2", Side::North),
                ],
            )],
        );
        let fitted = fit(&design, &view, MIN_PORT_PITCH / 2.0);
        // Every reference has room for its name.
        assert_eq!(
            texts(&fitted),
            [
                "p1: → gone.p",
                "p2: → gone.p",
                "p3: → gone.p",
                "q1: → gone.p",
                "q2: → gone.p"
            ]
        );
        assert_clear(&fitted);
    }

    /// Assert that no reference of `fitted` lies over anything drawn: its
    /// text clear of every wire, box, title, pin annotation, port label,
    /// code and other reference, and its stub clear of every wire of
    /// another connection, every other box and title, every code and
    /// every other reference.
    fn assert_clear(fitted: &Fitted) {
        let placement = &fitted.placement;
        let connections = placement.connections();
        let codes = code_anchors(&fitted.wires);
        let runs: Vec<([WireEnd; 2], Bounds)> = connections
            .iter()
            .zip(&fitted.wires)
            .flat_map(|(c, wire)| {
                let ends = c.ends();
                wire.windows(2)
                    .map(move |run| (ends.clone(), casing(run[0], run[1])))
            })
            .collect();
        let boxes: Vec<(Option<&InstanceName>, Bounds)> = placement
            .components
            .iter()
            .flat_map(|(name, pc)| [(Some(name), pc.bounds()), (Some(name), pc.title_bound())])
            .collect();
        let marks: Vec<Bounds> = placement
            .ports()
            .flat_map(|p| p.pin_bound().into_iter().chain([p.label_extent()]))
            .collect();
        let code_areas: Vec<Bounds> = codes
            .iter()
            .zip(&connections)
            .filter_map(|(code, c)| Some(code.as_ref()?.extent(c.color)))
            .collect();
        for (i, reference) in fitted.references.iter().enumerate() {
            let others = fitted
                .references
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, r)| r);
            let other_areas: Vec<Bounds> = others.flat_map(OffView::extents).collect();
            let path = reference.path_extents();
            let stub = path[0];
            let end = reference.end();
            // It shows something: a text, a leader, or a stub no drawn wire
            // of its own port covers.
            let own_runs: Vec<Point> = connections
                .iter()
                .zip(&fitted.wires)
                .filter(|(c, _)| c.ends().contains(&end))
                .flat_map(|(_, wire)| [wire[1], wire[wire.len() - 2]])
                .collect();
            assert!(
                reference.text().is_some()
                    || reference.lane > 0
                    || !own_runs.iter().any(|&r| reference.hidden_by(r)),
                "{}: draws nothing visible",
                reference.port
            );
            // A leader is new ink: clear of everything but its own port's
            // wires.
            for leader in &path[1..] {
                let others = runs
                    .iter()
                    .filter(|(ends, _)| !ends.contains(&end))
                    .map(|(_, b)| b)
                    .chain(boxes.iter().map(|(_, b)| b))
                    .chain(&marks)
                    .chain(&code_areas)
                    .chain(&other_areas);
                for area in others {
                    assert!(
                        !leader.overlaps(area),
                        "{}: leader over {area:?}",
                        reference.port
                    );
                }
            }
            for (ends, run) in &runs {
                if !ends.contains(&end) {
                    assert!(!stub.overlaps(run), "{}: stub on a wire", reference.port);
                }
            }
            for (owner, area) in &boxes {
                if *owner != reference.owner.as_ref() {
                    assert!(!stub.overlaps(area), "{}: stub on a box", reference.port);
                }
            }
            for area in code_areas.iter().chain(&other_areas) {
                assert!(!stub.overlaps(area), "{}: stub on a text", reference.port);
            }
            let Some(text) = reference.extent() else {
                continue;
            };
            let everything = runs
                .iter()
                .map(|(_, b)| b)
                .chain(boxes.iter().map(|(_, b)| b))
                .chain(&marks)
                .chain(&code_areas)
                .chain(&other_areas);
            for area in everything {
                assert!(
                    !text.overlaps(area),
                    "{}: {text:?} over {area:?}",
                    reference.port
                );
            }
        }
    }

    /// Assert that every reference of `fitted` reads as its own port's:
    /// no element of it lies on the line of another port on its side of
    /// its box, and a text beside the stub is nearer its own port's line
    /// than any other.
    fn assert_own_lines(fitted: &Fitted) {
        let boxes = fitted
            .placement
            .components
            .iter()
            .map(|(name, pc)| (Some(name), pc))
            .chain(fitted.placement.enclosure().map(|pc| (None, pc)));
        for (owner, pc) in boxes {
            for reference in fitted
                .references
                .iter()
                .filter(|r| r.owner.as_ref() == owner)
            {
                let across = |b: &Bounds| match reference.side {
                    Side::West | Side::East => (b.origin.y, b.origin.y + b.height),
                    Side::North | Side::South => (b.origin.x, b.origin.x + b.width),
                };
                let line = |p: &PlacedPort| match reference.side {
                    Side::West | Side::East => p.pos.y,
                    Side::North | Side::South => p.pos.x,
                };
                let side_ports: Vec<&PlacedPort> = pc
                    .ports
                    .iter()
                    .filter(|p| p.label_side() == reference.side)
                    .collect();
                let own = side_ports
                    .iter()
                    .find(|p| p.port == reference.port)
                    .map(|p| line(p))
                    .expect("its port");
                let others: Vec<f64> = side_ports
                    .iter()
                    .filter(|p| p.port != reference.port)
                    .map(|p| line(p))
                    .collect();
                for element in reference.extents() {
                    let (lo, hi) = across(&element);
                    for &other in &others {
                        assert!(
                            !(lo < other && other < hi),
                            "{}: {element:?} on another port's line {other}",
                            reference.port
                        );
                    }
                }
                if let (true, Some(text)) = (reference.lane > 0, reference.extent()) {
                    let (lo, hi) = across(&text);
                    let centre = (lo + hi) / 2.0;
                    for &other in &others {
                        assert!(
                            (centre - own).abs() < (centre - other).abs(),
                            "{}: text at {centre} nearer {other} than its own {own}",
                            reference.port
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_reference_never_reads_as_a_neighbouring_port_s() {
        // `c.p0` and `c.p2` run straight across to `d`, so their texts
        // cannot follow their stubs, and on to `gone`; `c.p1` and `c.p3`
        // have no wire. Beside the stub only where the text is nearer its
        // own port's line: at the default grid, not at the finest.
        let design = design_from(
            r#"
component sys {
    c: Quad;
    d: Quad;
    gone: Quad;
    wire red 1 [c.p0, d.p0];
    wire blue 1 [c.p0, gone.p0];
    wire red 1 [c.p2, d.p2];
    wire blue 1 [c.p2, gone.p2];
    component Quad {
        pub port p0 "P0";
        pub port p1 "P1";
        pub port p2 "P2";
        pub port p3 "P3";
    }
}
"#,
        );
        let east = [
            ("p0", Side::East),
            ("p1", Side::East),
            ("p2", Side::East),
            ("p3", Side::East),
        ];
        let west = east.map(|(p, _)| (p, Side::West));
        let view = view_of("sys", &[("c", 0.0, 0.0, &east), ("d", 30.0, 0.0, &west)]);
        let at = |grid: f64| {
            let fitted = fit(&design, &view, grid);
            assert_clear(&fitted);
            assert_own_lines(&fitted);
            fitted
        };
        let default = at(15.0);
        assert_eq!(texts(&default), ["p0: → gone.p0", "p2: → gone.p2"]);
        assert!(default.references.iter().all(|r| r.lane > 0));
        // At grid 10 the lane clear of the wire's code would sit nearer
        // the next port, so only the arrow fits beside the code.
        assert_eq!(texts(&at(10.0)), ["p0: →", "p2: →"]);
        // At the finest grid no lane is nearer its own port's line, and a
        // stub alone would lie under the drawn wire: nothing to show.
        assert!(at(MIN_PORT_PITCH / 2.0).references.is_empty());
    }

    /// Two banks of eight ports wired across in reverse, so the wires
    /// cross in the channel between them, and every port also wired to
    /// `gone`, which the view leaves out.
    fn crossed() -> Design {
        let n = 8;
        let ports: String = (0..n)
            .map(|i| format!("pub port p{i} \"P{i}\";\n"))
            .collect();
        let wires: String = (0..n)
            .map(|i| {
                format!(
                    "wire red 1 [a.p{i}, b.p{}];\nwire black 1 [a.p{i}, gone.p];\nwire blue 1 [b.p{i}, gone.p];\n",
                    n - 1 - i
                )
            })
            .collect();
        design_from(&format!(
            "component sys {{\n a: Bank;\n b: Bank;\n gone: Gone;\n{wires}\n component Bank {{\n{ports}}}\n component Gone {{ pub port p \"P\"; }}\n}}\n"
        ))
    }

    #[test]
    fn a_reference_among_crossed_wires_gives_way_rather_than_cover_one() {
        let design = crossed();
        let east: Vec<(String, Side)> = (0..8).map(|i| (format!("p{i}"), Side::East)).collect();
        let west: Vec<(String, Side)> = (0..8).map(|i| (format!("p{i}"), Side::West)).collect();
        let east: Vec<(&str, Side)> = east.iter().map(|(p, s)| (p.as_str(), *s)).collect();
        let west: Vec<(&str, Side)> = west.iter().map(|(p, s)| (p.as_str(), *s)).collect();
        for (grid, apart) in [(15.0, 20.0), (10.0, 20.0), (MIN_PORT_PITCH / 2.0, 40.0)] {
            let view = view_of("sys", &[("a", 0.0, 0.0, &east), ("b", apart, 0.0, &west)]);
            let fitted = fit(&design, &view, grid);
            assert_clear(&fitted);
            assert_own_lines(&fitted);
            // Every port has an undrawn wire, but not every one has room:
            // some give way, down to an arrow or nothing.
            let all = texts(&fitted);
            let gave_way = all.len() < 16 || all.iter().any(|t| t.ends_with(['-', '→', '←']));
            assert!(gave_way, "{all:?}");
            // At the default pitch some still name their far end.
            if grid == 15.0 {
                assert!(all.iter().any(|t| t.ends_with("gone.p ←")), "{all:?}");
            }
        }
    }

    /// `a.s`, on a box above `b`, wired on to three ends no view includes.
    fn above() -> Design {
        design_from(
            r#"
component sys {
    a: Blk;
    b: Blk;
    wire red 1 [a.s, x1.p, x2.p, x3.p];
    x1: Blk;
    x2: Blk;
    x3: Blk;
    component Blk {
        pub port s "S";
        pub port p "P";
    }
}
"#,
        )
    }

    /// `a.s`'s reference with `b` placed `gap` grid steps below `a`.
    fn fit_above(gap: f64) -> Fitted {
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("s", Side::South)]),
                ("b", 0.0, gap, &[("p", Side::West)]),
            ],
        );
        let fitted = fit(&above(), &view, 20.0);
        assert_clear(&fitted);
        fitted
    }

    /// What `a.s` reads with `b` placed `gap` grid steps below `a`.
    fn reference_above(gap: f64) -> Vec<String> {
        texts(&fit_above(gap))
    }

    #[test]
    fn a_reference_takes_the_most_detailed_text_that_fits() {
        // `b`'s title is the first thing in the way: the more room above
        // it, the more the reference says.
        assert_eq!(reference_above(13.0), ["s: → x1.p, x2.p, x3.p"]);
        assert_eq!(reference_above(10.0), ["s: → 3 more"]);
        assert_eq!(reference_above(7.0), ["s: →"]);
    }

    #[test]
    fn a_reference_with_no_room_for_any_text_keeps_its_stub() {
        // Not even the arrow fits past the stub or at the end of a leader
        // before `b`'s title; the stub, which no drawn wire covers, still
        // shows.
        assert_eq!(reference_above(6.0), ["s: -"]);
        assert_eq!(fit_above(6.0).references[0].lane, 0);
    }

    #[test]
    fn a_reference_whose_stub_would_meet_something_is_cut_short_or_left_out() {
        // `b`'s title starts where the stub would end: there is no stub
        // worth drawing, so no reference.
        assert!(reference_above(5.0).is_empty());
    }

    #[test]
    fn a_wire_passing_in_front_of_the_port_cuts_the_stub_short_of_it() {
        // `c.s`–`d.n` runs straight down one step in front of `a`'s east
        // side, across the tip of `a.p`'s stub: the stub stops short of
        // it, with no room for text.
        let design = design_from(
            r#"
component sys {
    a: Blk;
    c: Blk;
    d: Blk;
    gone: Blk;
    wire red 1 [c.s, d.n];
    wire blue 1 [a.p, gone.p];
    component Blk {
        pub port p "P";
        pub port s "S";
        pub port n "N";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("c", 7.0, -10.0, &[("s", Side::South)]),
                ("d", 7.0, 10.0, &[("n", Side::North)]),
            ],
        );
        let fitted = fit(&design, &view, 15.0);
        assert_eq!(
            fitted.wires[0].iter().map(|p| p.x).collect::<Vec<_>>(),
            [105.0, 105.0]
        );
        assert_eq!(texts(&fitted), ["p: -"]);
        let path = fitted.references[0].path();
        let (from, to) = (path[0], path[1]);
        assert!(to.x - from.x < 15.0 - CASING_HALF, "{from:?} {to:?}");
        assert!(to.x - from.x >= MIN_STUB);
        assert_clear(&fitted);
    }

    #[test]
    fn a_wire_running_on_from_the_port_moves_the_text_beside_it() {
        // `a.p` is wired straight across to `b.p`, and on to `gone`: the
        // drawn wire runs along the reference's line, so its text goes
        // below it.
        let design = design_from(
            r#"
component sys {
    a: Blk;
    b: Blk;
    gone: Blk;
    wire red 1 [a.p, b.p];
    wire blue 1 [a.p, gone.p];
    component Blk {
        pub port p "P";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 30.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let fitted = fit(&design, &view, 15.0);
        assert_eq!(texts(&fitted), ["p: → gone.p"]);
        let reference = &fitted.references[0];
        assert_eq!(reference.lane, 1);
        let text = reference.extent().expect("a text");
        assert!(text.origin.y > reference.at.y, "{text:?}");
        assert_clear(&fitted);
    }

    #[test]
    fn a_reference_never_takes_the_canvas_past_its_limit() {
        let design = four_sides();
        // A canvas that is too large with any text longer than the arrow.
        let fitted = fit_on(&design, &four_sides_view(), 15.0, |references| {
            let longest = references
                .iter()
                .filter_map(OffView::text)
                .map(|t| t.chars().count())
                .max()
                .unwrap_or(0);
            let side = if longest > 1 { 2.0 * MAX_CANVAS } else { 100.0 };
            Canvas::new(side, side)
        });
        assert_eq!(texts(&fitted), ["w: ←", "e: →", "n: →", "s: →"]);
        // A view already past the limit may still draw a reference that
        // does not make it larger.
        let fitted = fit_on(&design, &four_sides_view(), 15.0, |_| {
            Canvas::new(2.0 * MAX_CANVAS, 100.0)
        });
        assert_eq!(texts(&fitted)[0], "w: far.w ←");
    }

    fn port(instance: &str, port: &str) -> WireEnd {
        WireEnd::Child {
            instance: InstanceName::from(instance),
            port: PortName::from(port),
        }
    }

    fn undrawn(far: Vec<FarEnd>) -> Undrawn {
        Undrawn {
            far,
            color: WireColor::from("red"),
        }
    }

    #[test]
    fn one_far_end_reads_as_the_name_written_in_a_wire() {
        let one = undrawn(vec![FarEnd::Port(port("vcm", "can2_h"))]);
        assert_eq!(one.texts(), ["vcm.can2_h", ""]);
        assert_eq!(one.title(), "vcm.can2_h");
    }

    #[test]
    fn an_own_port_says_the_wire_leaves_the_subject() {
        let own = FarEnd::of(
            &WireEnd::Own(PortName::from("out")),
            &TypeName::from("Loom"),
        );
        assert_eq!(own.to_string(), "out (leaves Loom)");
    }

    #[test]
    fn several_far_ends_collapse_as_room_runs_out() {
        let many = undrawn(vec![
            FarEnd::Port(port("vcm", "gnd")),
            FarEnd::Port(port("pump", "gnd")),
            FarEnd::Port(port("fan", "gnd")),
        ]);
        let texts = many.texts();
        assert_eq!(
            texts,
            [
                "vcm.gnd, pump.gnd, fan.gnd",
                "vcm.gnd, pump.gnd, +1 more",
                "vcm.gnd, +2 more",
                "3 more",
                "",
            ]
        );
        assert_eq!(preferred(&texts), 0);
        assert_eq!(many.title(), "vcm.gnd, pump.gnd, fan.gnd");
    }

    #[test]
    fn a_list_too_long_to_spell_out_lists_what_fits() {
        let names = ["front_battery.can_h", "rear_battery.can_h", "charger.can_h"];
        let many = undrawn(names.iter().map(|n| FarEnd::Port(port(n, "p"))).collect());
        let texts = many.texts();
        assert_eq!(texts[preferred(&texts)], "front_battery.can_h.p, +2 more");
    }

    #[test]
    fn when_not_even_the_first_end_fits_nothing_is_named() {
        let long = "a_very_long_instance_name_indeed";
        let many = undrawn(vec![
            FarEnd::Port(port(long, "p")),
            FarEnd::Port(port("b", "p")),
            FarEnd::Port(port("c", "p")),
        ]);
        let texts = many.texts();
        assert_eq!(texts[preferred(&texts)], "3 more");
    }

    #[test]
    fn one_long_end_is_still_named() {
        let long = "a_very_long_instance_name_indeed";
        let one = undrawn(vec![FarEnd::Port(port(long, "port"))]);
        let texts = one.texts();
        assert_eq!(texts[preferred(&texts)], format!("{long}.port"));
    }

    #[test]
    fn a_splice_names_itself_then_what_it_joins() {
        let splice = FarEnd::Splice {
            splice: SpliceName::from("sp_gnd"),
            beyond: vec![port("lamp", "gnd"), port("horn", "gnd")],
        };
        assert_eq!(
            splice.texts(),
            [
                "sp_gnd (lamp.gnd, horn.gnd)",
                "sp_gnd (lamp.gnd, +1 more)",
                "sp_gnd"
            ]
        );
        assert_eq!(splice.to_string(), "sp_gnd (lamp.gnd, horn.gnd)");
    }
}
