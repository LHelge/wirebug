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
//! The reference keeps routed wires off it by being an obstacle to the
//! router, but only when a wire would otherwise cross it, and only when
//! reserving it leaves every wire the view draws routable
//! ([`Settled::new`]). So a view routes as it did without references
//! unless one is in a wire's way, and a project that passes `check`
//! without them passes with them.

use std::fmt;

use super::PIN_CHAR_WIDTH;
use super::layout::{Connection, Grid, PlacedComponent, PlacedPort, Placement};
use super::route::Router;
use crate::dsl::ir::{InstanceName, PortName, SpliceName, TypeName, WireColor, WireEnd};
use crate::render::geometry::{Bounds, Point, Side};

/// The reference's font size (the `.off-view-label` rule), and its
/// estimated advance per character: the pin annotation's.
const FONT_SIZE: f64 = 10.0;
const CHAR_WIDTH: f64 = PIN_CHAR_WIDTH;
/// Half the 4-unit `wire-casing` stroke: how far a wire's ink reaches
/// either side of its line.
const CASING_HALF: f64 = 2.0;
/// The white halo round the text (half its 3px stroke).
const HALO: f64 = 1.5;
/// How far either side of a reference's line a wire's ink would touch
/// its text: half the text's height, its halo and half a casing.
const ZONE_HALF: f64 = FONT_SIZE / 2.0 + HALO + CASING_HALF;
/// Between the end of the drawn stub and the reference's text, and
/// between the pin annotation and the end of the stub.
const GAP: f64 = 3.0;
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
    /// Which of `texts` is drawn.
    shown: usize,
    pub(super) title: String,
    pub(super) color: WireColor,
    /// How far out from the port the drawn stub ends, and the text starts.
    stub: f64,
    start: f64,
    /// Where the space the reference keeps clear starts, out from the
    /// port: at the port, or, when a drawn wire also leaves the port,
    /// past the tip of its routing stub, which that wire turns at.
    keep_from: f64,
    step: f64,
    /// Whether the router routes around the reference.
    pub(super) reserved: bool,
    /// Whether even its shortest text has no room clear of what is around
    /// it, so it is never reserved.
    cramped: bool,
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
        let side = port.label_side();
        let step = grid.step();
        // A drawn wire leaving the port too turns at its routing stub's
        // tip, one step out; the stub runs a step past that, so it reads
        // as running on rather than as the text touching the turn.
        let keep_from = if drawn { step } else { 0.0 };
        let stub = (keep_from + step).max(reach(component, port) + GAP);
        let texts = undrawn.texts();
        Some(Self {
            owner: owner.cloned(),
            port: port.port.clone(),
            side,
            at: port.pos,
            shown: preferred(&texts),
            texts,
            title: undrawn.title(),
            color: undrawn.color.clone(),
            stub,
            start: stub + GAP,
            keep_from,
            step,
            reserved: false,
            cramped: false,
        })
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

    /// The text drawn: the arrow at the stub's end, pointing away from the
    /// port (after the list on a west side, before it elsewhere).
    pub(super) fn text(&self) -> String {
        let list = &self.texts[self.shown];
        match (self.side, list.is_empty()) {
            (Side::West, true) => "←".to_string(),
            (_, true) => "→".to_string(),
            (Side::West, false) => format!("{list} ←"),
            (_, false) => format!("→ {list}"),
        }
    }

    /// The drawn stub, from the port out to where the text starts.
    pub(super) fn stub(&self) -> [Point; 2] {
        [self.at, self.out(self.stub)]
    }

    /// Where the text is anchored: level, on the stub's line, beside a
    /// west or east port; turned to read outward along a north or south
    /// stub (upward on the north, downward on the south).
    pub(super) fn anchor(&self) -> (Point, &'static str, f64) {
        let at = self.out(self.start);
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

    fn text_width(&self) -> f64 {
        self.text().chars().count() as f64 * CHAR_WIDTH
    }

    /// The area `from`..`to` out from the port, `half` either side of the
    /// stub's line.
    fn band(&self, from: f64, to: f64, half: f64) -> Bounds {
        let (a, b) = (self.out(from), self.out(to));
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

    /// The area the text covers.
    pub(super) fn extent(&self) -> Bounds {
        let end = self.start + self.text_width();
        self.band(self.start, end, FONT_SIZE / 2.0)
    }

    /// The space a routed wire must keep out of: the stub and the text
    /// with its halo, widened by a wire's casing. It starts past the
    /// routing stub's tip when a drawn wire leaves the port too, so that
    /// wire may turn there.
    fn zone(&self) -> Bounds {
        let from = if self.keep_from > 0.0 {
            self.keep_from + CASING_HALF
        } else {
            0.0
        };
        let end = self.start + self.text_width();
        self.band(from, end + HALO, ZONE_HALF)
    }

    /// The obstacle the router routes around when the reference is
    /// reserved: from [`Self::keep_from`] to the first grid line clear of
    /// the text, and as many whole grid steps either side of the stub's
    /// line as cover [`Self::zone`] (one, at any grid of 8.5 or more), so
    /// every edge it adds to the visibility graph is a grid line and a
    /// wire running along one keeps clear of the text.
    fn reservation(&self) -> Bounds {
        let snap = |d: f64| (d / self.step).ceil() * self.step;
        let end = snap(self.start + self.text_width() + GAP);
        self.band(self.keep_from, end, snap(ZONE_HALF))
    }

    /// Whether `path` runs through the reference.
    fn crossed_by(&self, path: &[Point]) -> bool {
        let zone = self.zone();
        path.windows(2).any(|run| {
            let (a, b) = (run[0], run[1]);
            let segment = Bounds {
                origin: Point::new(a.x.min(b.x), a.y.min(b.y)),
                width: (a.x - b.x).abs(),
                height: (a.y - b.y).abs(),
            };
            zone.overlaps(&segment)
        })
    }

    /// The shortest of the texts.
    fn shortest(&self) -> usize {
        self.texts.len() - 1
    }
}

/// How far out from `port` the texts already beside its stub reach: the
/// pin annotations of the ports on its side whose band across the stub
/// meets the reference's, and, on a box's north side, the box's title.
fn reach(component: &PlacedComponent, port: &PlacedPort) -> f64 {
    let side = port.label_side();
    let across = |b: &Bounds| match side {
        Side::West | Side::East => (b.origin.y, b.origin.y + b.height),
        Side::North | Side::South => (b.origin.x, b.origin.x + b.width),
    };
    let line = match side {
        Side::West | Side::East => port.pos.y,
        Side::North | Side::South => port.pos.x,
    };
    let half = FONT_SIZE / 2.0;
    let meets = |b: &Bounds| {
        let (lo, hi) = across(b);
        lo < line + half && hi > line - half
    };
    let out = |b: &Bounds| match side {
        Side::West => port.pos.x - b.origin.x,
        Side::East => b.origin.x + b.width - port.pos.x,
        Side::North => port.pos.y - b.origin.y,
        Side::South => b.origin.y + b.height - port.pos.y,
    };
    let pins = component
        .ports
        .iter()
        .filter(|p| p.label_side() == side)
        .filter_map(PlacedPort::pin_extent);
    let title = (!port.inverted && side == Side::North).then(|| component.title_extent());
    pins.chain(title)
        .filter(meets)
        .map(|b| out(&b))
        .fold(0.0, f64::max)
}

/// What a reference's space must stay clear of: every other box's keepout,
/// title and pin annotations, the text boxes, every port and its routing
/// stub, and the enclosure's boundary.
struct Room {
    /// Each by the component it belongs to: its index in the placement, or
    /// `None` for the enclosure.
    keepouts: Vec<(Option<usize>, Bounds)>,
    texts: Vec<(Option<usize>, Bounds)>,
    points: Vec<(Option<usize>, Point)>,
    free: Vec<Bounds>,
    enclosure: Option<Bounds>,
}

impl Room {
    fn of(placement: &Placement, step: f64) -> Self {
        let components = placement
            .components
            .values()
            .enumerate()
            .map(|(i, pc)| (Some(i), pc))
            .chain(placement.enclosure().map(|pc| (None, pc)));
        let mut room = Self {
            keepouts: Vec::new(),
            texts: Vec::new(),
            points: Vec::new(),
            free: placement
                .texts
                .iter()
                .map(|text| Bounds {
                    origin: text.origin,
                    width: text.width,
                    height: text.height,
                })
                .collect(),
            enclosure: placement.enclosure_bounds(),
        };
        for (owner, pc) in components {
            if owner.is_some() {
                let b = pc.bounds();
                room.keepouts.push((
                    owner,
                    Bounds {
                        origin: Point::new(b.origin.x - step, b.origin.y - step),
                        width: b.width + 2.0 * step,
                        height: b.height + 2.0 * step,
                    },
                ));
                room.texts.push((owner, pc.title_extent()));
            }
            for port in &pc.ports {
                room.texts.extend(port.pin_extent().map(|b| (owner, b)));
                room.points.push((owner, port.pos));
                room.points.push((owner, port.stub(step)));
            }
        }
        room
    }

    /// Whether `space`, kept by a reference of `port` (of `owner`), is
    /// clear of everything it must not cover. The port itself and its
    /// stub's tip are the reference's own.
    fn fits(&self, space: &Bounds, owner: Option<usize>, port: &PlacedPort, step: f64) -> bool {
        let own = [port.pos, port.stub(step)];
        let others = |(o, b): &(Option<usize>, Bounds)| *o != owner && space.overlaps(b);
        let inside = |p: Point| {
            p.x > space.origin.x
                && p.x < space.origin.x + space.width
                && p.y > space.origin.y
                && p.y < space.origin.y + space.height
        };
        let within = self.enclosure.is_none_or(|e| {
            space.origin.x >= e.origin.x
                && space.origin.y >= e.origin.y
                && space.origin.x + space.width <= e.origin.x + e.width
                && space.origin.y + space.height <= e.origin.y + e.height
        });
        within
            && !self.keepouts.iter().any(others)
            && !self.texts.iter().any(others)
            && !self.free.iter().any(|b| space.overlaps(b))
            && !self
                .points
                .iter()
                .any(|&(o, p)| inside(p) && !(o == owner && own.contains(&p)))
    }
}

/// A view's routing settled together with its off-view references: the
/// router every wire is routed with, each reference as it is drawn, and
/// the routed wires, `None` when a wire has no route.
pub(super) struct Settled {
    pub(super) router: Router,
    pub(super) references: Vec<OffView>,
    pub(super) wires: Option<Vec<Vec<Point>>>,
}

impl Settled {
    /// Route `connections` (`placement`'s) and lay out the references.
    ///
    /// Each reference first takes the most detailed text whose space is
    /// clear of the boxes, texts, ports and other references around it,
    /// else its shortest, kept clear of nothing. The wires are routed as
    /// if there were no references; a reference a routed wire runs
    /// through is then reserved, made an obstacle, and the wires routed
    /// again, until no wire runs through an unreserved reference. A
    /// reservation that would leave some wire with no route, as the
    /// router's own graph says ([`Router::connects`]), yields: the
    /// reference is tried at its shortest text, and else stays where it
    /// is, unreserved. So reserving never takes a route away, and a view
    /// whose wires do not meet its references routes exactly as it would
    /// without them. A view that does not route at all reserves nothing.
    pub(super) fn new(placement: &Placement, connections: &[Connection<'_>], grid: Grid) -> Self {
        let step = grid.step();
        let mut references = references(placement, connections, grid);
        let base = Router::build(placement, step, &[]);
        let routes = |router: &Router| connections.iter().all(|c| router.connects(c.a, c.b));
        if !base.blocked_stubs(placement).is_empty() || !routes(&base) {
            return Self {
                router: base,
                references,
                wires: None,
            };
        }
        let pairs: Vec<_> = connections.iter().map(|c| (c.a, c.b)).collect();
        let mut router = base;
        let mut kept: Vec<Bounds> = Vec::new();
        let mut yielded: Vec<bool> = references.iter().map(|r| r.cramped).collect();
        loop {
            let Ok(wires) = router.route_all(&pairs, grid.pitch()) else {
                return Self {
                    router,
                    references,
                    wires: None,
                };
            };
            let crossed: Vec<usize> = (0..references.len())
                .filter(|&i| !references[i].reserved && !yielded[i])
                .filter(|&i| wires.iter().any(|w| references[i].crossed_by(w)))
                .collect();
            let mut grew = false;
            for i in crossed {
                let mut shown = vec![references[i].shown];
                if references[i].shortest() != references[i].shown {
                    shown.push(references[i].shortest());
                }
                for text in shown {
                    let mut candidate = references[i].clone();
                    candidate.shown = text;
                    kept.push(candidate.reservation());
                    let trial = Router::build(placement, step, &kept);
                    if routes(&trial) {
                        candidate.reserved = true;
                        references[i] = candidate;
                        router = trial;
                        grew = true;
                        break;
                    }
                    kept.pop();
                }
                if !references[i].reserved {
                    yielded[i] = true;
                    references[i].shown = references[i].shortest();
                }
            }
            if !grew {
                return Self {
                    router,
                    references,
                    wires: Some(wires),
                };
            }
        }
    }
}

/// Every placed port's reference, in placement order (boxes in include
/// order, then the enclosure), each at the most detailed text whose space
/// fits among the others ([`Room::fits`]), else at its shortest.
fn references(placement: &Placement, connections: &[Connection<'_>], grid: Grid) -> Vec<OffView> {
    let room = Room::of(placement, grid.step());
    let drawn: Vec<WireEnd> = connections.iter().flat_map(Connection::ends).collect();
    let components = placement
        .components
        .iter()
        .enumerate()
        .map(|(i, (name, pc))| (Some(i), Some(name), pc))
        .chain(placement.enclosure().map(|pc| (None, None, pc)));
    let mut taken: Vec<Bounds> = Vec::new();
    let mut references = Vec::new();
    for (index, owner, pc) in components {
        for port in &pc.ports {
            let end = match owner {
                Some(instance) => WireEnd::Child {
                    instance: instance.clone(),
                    port: port.port.clone(),
                },
                None => WireEnd::Own(port.port.clone()),
            };
            let Some(mut reference) = OffView::new(owner, pc, port, drawn.contains(&end), grid)
            else {
                continue;
            };
            let fitting = (reference.shown..reference.texts.len()).find(|&shown| {
                let mut trial = reference.clone();
                trial.shown = shown;
                room.fits(&trial.reservation(), index, port, grid.step())
                    && !taken.iter().any(|text| trial.extent().overlaps(text))
            });
            match fitting {
                Some(shown) => {
                    reference.shown = shown;
                    taken.push(reference.extent());
                }
                None => {
                    reference.shown = reference.shortest();
                    reference.cramped = true;
                }
            }
            references.push(reference);
        }
    }
    references
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{Design, View};
    use crate::render::schematic::MIN_PORT_PITCH;
    use crate::render::schematic::tests::{design_from, view_of};

    /// `view` of `design` placed on `grid`, and its routing settled.
    fn settle(design: &Design, view: &View, grid: f64) -> (Placement, Settled) {
        let subject = design.get(&design.root).expect("root");
        let placement = Placement::compute(design, subject, view, Grid::new(grid)).expect("places");
        let settled = {
            let connections = placement.connections();
            Settled::new(&placement, &connections, Grid::new(grid))
        };
        (placement, settled)
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
        let (placement, settled) = settle(&design, &four_sides_view(), 15.0);
        let a = &placement.components[&InstanceName::from("a")];
        let texts: Vec<(String, String)> = settled
            .references
            .iter()
            .map(|r| (r.port.to_string(), r.text()))
            .collect();
        assert_eq!(
            texts,
            [
                ("w".to_string(), "far.w ←".to_string()),
                ("e".to_string(), "→ far.e".to_string()),
                ("n".to_string(), "→ far.n".to_string()),
                ("s".to_string(), "→ far.s".to_string()),
            ]
        );
        let bounds = a.bounds();
        for reference in &settled.references {
            let text = reference.extent();
            let outside = match reference.side {
                Side::West => text.origin.x + text.width < bounds.origin.x,
                Side::East => text.origin.x > bounds.origin.x + bounds.width,
                Side::North => text.origin.y + text.height < bounds.origin.y,
                Side::South => text.origin.y > bounds.origin.y + bounds.height,
            };
            assert!(outside, "{:?} {text:?}", reference.side);
            // Clear of every pin annotation and of the box's title.
            for pin in a.ports.iter().filter_map(PlacedPort::pin_extent) {
                assert!(
                    !text.overlaps(&pin),
                    "{:?}: {text:?} {pin:?}",
                    reference.side
                );
            }
            assert!(!text.overlaps(&a.title_extent()), "{:?}", reference.side);
            // The stub runs from the port to the text.
            let [from, to] = reference.stub();
            let port = a.ports.iter().find(|p| p.port == reference.port).unwrap();
            assert_eq!(from, port.pos);
            assert!(!text.overlaps(&Bounds {
                origin: Point::new(from.x.min(to.x), from.y.min(to.y)),
                width: (from.x - to.x).abs(),
                height: (from.y - to.y).abs(),
            }));
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
        let (placement, settled) = settle(&design, &view, MIN_PORT_PITCH / 2.0);
        let a = &placement.components[&InstanceName::from("a")];
        let texts: Vec<Bounds> = settled.references.iter().map(OffView::extent).collect();
        assert_eq!(texts.len(), 5);
        for (i, text) in texts.iter().enumerate() {
            for other in &texts[i + 1..] {
                assert!(!text.overlaps(other), "{text:?} {other:?}");
            }
            for pin in a.ports.iter().filter_map(PlacedPort::pin_extent) {
                assert!(!text.overlaps(&pin), "{text:?} {pin:?}");
            }
        }
    }

    /// `a.p` wired to `b.p` straight across, and `a.q` to `gone`, which
    /// the view leaves out; with `crimped`, `a.p` is wired to `gone` too.
    fn across(crimped: bool) -> Design {
        let extra = if crimped {
            "wire blue 1 [a.p, gone.p];"
        } else {
            ""
        };
        design_from(&format!(
            r#"
component sys {{
    a: Blk;
    b: Blk;
    gone: Blk;
    wire red 1 [a.p, b.p];
    wire black 1 [a.q, gone.p];
    {extra}
    component Blk {{
        pub port p "P";
        pub port q "Q";
    }}
}}
"#
        ))
    }

    fn across_view() -> View {
        view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East), ("q", Side::East)]),
                ("b", 30.0, -1.0, &[("p", Side::West)]),
            ],
        )
    }

    /// The wires `view` routes to with no reference reserved.
    fn unreserved_wires(placement: &Placement, grid: f64) -> Vec<Vec<Point>> {
        let connections = placement.connections();
        let pairs: Vec<_> = connections.iter().map(|c| (c.a, c.b)).collect();
        Router::build(placement, grid, &[])
            .route_all(&pairs, 2.0 * grid)
            .expect("routes")
    }

    #[test]
    fn wires_clear_of_every_reference_route_as_if_there_were_none() {
        let design = across(false);
        let (placement, settled) = settle(&design, &across_view(), 15.0);
        assert_eq!(settled.references.len(), 1);
        assert!(!settled.references[0].reserved);
        let wires = settled.wires.expect("routes");
        assert_eq!(wires, unreserved_wires(&placement, 15.0));
        assert!(!settled.references[0].crossed_by(&wires[0]));
    }

    #[test]
    fn a_wire_that_would_run_through_a_reference_goes_round_it() {
        // `a.p`'s drawn wire runs straight on to `b.p`, through the
        // reference of its own undrawn wire to `gone`, until the reference
        // is reserved.
        let design = across(true);
        let (placement, settled) = settle(&design, &across_view(), 15.0);
        let before = unreserved_wires(&placement, 15.0);
        let p = settled
            .references
            .iter()
            .find(|r| r.port == PortName::from("p"))
            .expect("a.p has a reference");
        assert!(before.iter().any(|w| p.crossed_by(w)), "{before:?}");
        assert!(p.reserved);
        let wires = settled.wires.expect("routes");
        for reference in &settled.references {
            assert!(!wires.iter().any(|w| reference.crossed_by(w)), "{wires:?}");
        }
        // The drawn wire still leaves through its own stub.
        assert_eq!(wires[0][0], placement.connections()[0].a.pos);
    }

    #[test]
    fn a_reference_short_of_room_shortens_and_at_the_last_shows_its_arrow_alone() {
        let design = design_from(
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
        );
        let text_below = |gap: f64| {
            let view = view_of(
                "sys",
                &[
                    ("a", 0.0, 0.0, &[("s", Side::South)]),
                    ("b", 0.0, gap, &[("p", Side::West)]),
                ],
            );
            let (_, settled) = settle(&design, &view, 20.0);
            settled.references[0].text()
        };
        assert_eq!(text_below(20.0), "→ x1.p, x2.p, x3.p");
        assert_eq!(text_below(10.0), "→ 3 more");
        assert_eq!(text_below(8.0), "→");
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
