//! Component/port placement: walking a design's view once to produce
//! positioned boxes and ports in SVG world coordinates.
//!
//! The DSL view authors each include's ports explicitly: which side a port
//! sits on, and in what order (the `ports { }` block). A box shows exactly
//! the ports listed for it — that list is both the layout and the scope.
//! A wire segment is drawn only when both its ends are placed ports; a
//! listed port whose wire reaches an end the view leaves out records that
//! end ([`PlacedPort::off_view`]) for its off-view reference.

use std::collections::HashMap;

use indexmap::IndexMap;

use super::off_view::{FarEnd, OffView, Undrawn};
use super::pin::{PinAnnotation, PinOrientation};
use super::{
    CHAR_WIDTH, COMPONENT_TITLE_GAP, DEFAULT_GRID, LABEL_INSET, MIN_HEIGHT, MIN_PORT_PITCH,
    MIN_WIDTH, PIN_CANVAS_PAD, SVG_MARGIN, TEXT_BOX_HEIGHT, TEXT_BOX_MIN_WIDTH, TEXT_BOX_PAD_X,
};
use crate::dsl::ir::{
    Design, Instance, InstanceName, NetEnd, Port, PortName, View, WireColor, WireEnd,
};
use crate::error::ViewResult;
use crate::render::GridFault;
use crate::render::geometry::{Bounds, Point, Side};
use crate::render::text::Font;

/// A port's identity within a view: a port on an included child instance, or
/// one of the subject's own ports drawn on the enclosure boundary.
#[derive(Clone, PartialEq, Eq, Hash)]
enum PortKey {
    Child {
        instance: InstanceName,
        port: PortName,
    },
    Enclosure(PortName),
}

impl PortKey {
    /// The included instance the port belongs to; `None` on the enclosure.
    fn owner(&self) -> Option<&InstanceName> {
        match self {
            PortKey::Child { instance, .. } => Some(instance),
            PortKey::Enclosure(_) => None,
        }
    }
}

/// A resolved include: its world centre, the child instance it places, and
/// the authored port placements (side + order) to lay out for it.
type BoxEntry<'a> = (Point, &'a Instance, &'a [(PortName, Side)]);

/// The view's grid: one step in world units. Coordinates (component
/// centres) given in grid units reach world space by multiplying. Ports
/// sit two steps apart, centred on each side, and a box is always an even
/// number of steps — so its centre, and every port, lands on a grid line
/// for any port count.
#[derive(Clone, Copy)]
pub(super) struct Grid(f64);

impl Grid {
    pub(super) fn new(step: f64) -> Self {
        Self(step)
    }

    /// The grid `view` asks for (`DEFAULT_GRID` when it names none),
    /// validated: ports sit two steps apart, so that pitch must clear a
    /// port label or adjacent labels would overlap.
    pub(super) fn of_view(view: &View) -> Result<Self, GridFault> {
        let step = GridFault::positive_step(view, DEFAULT_GRID)?;
        let minimum = MIN_PORT_PITCH / 2.0;
        if step < minimum {
            return Err(GridFault::TooFine {
                grid: step,
                minimum,
            });
        }
        Ok(Self::new(step))
    }

    /// One grid step in world units — also the router's clearance.
    pub(super) fn step(self) -> f64 {
        self.0
    }

    fn to_world(self, units: f64) -> f64 {
        units * self.0
    }

    /// Round a world-space length up to an even number of grid steps, so
    /// half the box is still a whole step and centring keeps every port
    /// on a grid line.
    fn snap_box(self, v: f64) -> f64 {
        let quantum = 2.0 * self.0;
        (v / quantum).ceil() * quantum
    }

    /// Side margin (edge-to-first-port): two grid steps — a full port
    /// pitch, so ports sit at least their own spacing in from the corner.
    fn margin(self) -> f64 {
        2.0 * self.0
    }

    /// Spacing between consecutive ports on a side: two grid steps. The
    /// grid is validated so this pitch is at least `MIN_PORT_PITCH`, so
    /// adjacent port labels never collide. Parallel wires in a nudged
    /// channel are spread by this same pitch (see [`SchematicRenderer`]).
    pub(super) fn pitch(self) -> f64 {
        2.0 * self.0
    }

    /// How far the enclosure box stands off the wrapped child boxes — four
    /// steps (an even count, so the edges stay grid-aligned and routing has
    /// room between the children and the boundary).
    fn enclosure_inset(self) -> f64 {
        4.0 * self.0
    }
}

/// Geometry for a single component box and all its placed ports, in
/// world (SVG) coordinates.
pub(super) struct PlacedComponent {
    pub(super) origin: Point,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) label: String,
    pub(super) ports: Vec<PlacedPort>,
    /// Port name → index into `ports`. Port names are unique per
    /// component in the DSL, so the name alone keys a port.
    port_index: HashMap<PortName, usize>,
}

impl PlacedComponent {
    /// The component's drawn box.
    pub(super) fn bounds(&self) -> Bounds {
        Bounds {
            origin: self.origin,
            width: self.width,
            height: self.height,
        }
    }

    /// Move each north-side pin annotation that would meet the title out
    /// along its stub, past the title.
    fn clear_pins_of_title(&mut self) {
        let title = self.title_extent();
        for port in self.ports.iter_mut().filter(|p| p.side == Side::North) {
            if let Some(pin) = &mut port.pin {
                pin.clear_title(port.pos, &title);
            }
        }
    }

    /// The area the title above the box covers (bold 13px, centred), its
    /// width estimated at [`TITLE_CHAR_WIDTH`] per character.
    pub(super) fn title_extent(&self) -> Bounds {
        self.title_area(self.label.chars().count() as f64 * TITLE_CHAR_WIDTH)
    }

    /// The same area measured with the shared estimator, which bounds the
    /// bundled font's widths: what another text must keep clear of.
    pub(super) fn title_bound(&self) -> Bounds {
        self.title_area(TITLE_FONT.width(&self.label))
    }

    /// The area a title `width` long covers, centred above the box.
    fn title_area(&self, width: f64) -> Bounds {
        Bounds {
            origin: Point::new(
                self.origin.x + (self.width - width) / 2.0,
                self.origin.y - COMPONENT_TITLE_GAP - TITLE_ASCENT,
            ),
            width,
            height: TITLE_ASCENT + TITLE_DESCENT,
        }
    }
}

/// The title's estimated advance per character, and how far its glyphs
/// reach above and below its baseline: bold 13px sans-serif.
const TITLE_CHAR_WIDTH: f64 = 8.5;
const TITLE_FONT: Font = Font::bold(13.0);
const TITLE_ASCENT: f64 = 12.0;
const TITLE_DESCENT: f64 = 3.0;

pub(super) struct PlacedPort {
    pub(super) port: PortName,
    pub(super) side: Side,
    pub(super) pos: Point,
    /// The pin number(s) drawn outside the box, `None` for a port with no
    /// pins.
    pub(super) pin: Option<PinAnnotation>,
    pub(super) label: String,
    /// True for an enclosure port: it sits on the boundary facing *inward*,
    /// so its wire leaves toward the schematic interior, not outward.
    pub(super) inverted: bool,
    /// The ends its wires reach that the view does not draw, when it has
    /// any: what its off-view reference names.
    pub(super) off_view: Option<Undrawn>,
}

impl PlacedPort {
    /// The side the port's texts are laid out for: its own, or the
    /// opposite one for an inverted enclosure port, which faces the
    /// interior and so labels like a normal port on the opposite side.
    pub(super) fn label_side(&self) -> Side {
        if self.inverted {
            self.side.opposite()
        } else {
            self.side
        }
    }

    /// The area the pin annotation covers, if the port has one.
    pub(super) fn pin_extent(&self) -> Option<Bounds> {
        let pin = self.pin.as_ref()?;
        Some(pin.extent(self.label_side(), self.pos))
    }

    /// The pin annotation's area measured with the shared estimator
    /// ([`PinAnnotation::bound`]), if the port has one.
    pub(super) fn pin_bound(&self) -> Option<Bounds> {
        let pin = self.pin.as_ref()?;
        Some(pin.bound(self.label_side(), self.pos))
    }

    /// The tip of the port's routing stub: `step` out the way its wire
    /// leaves it (inward for an inverted enclosure port).
    pub(super) fn stub(&self, step: f64) -> Point {
        let Point { x, y } = self.pos;
        match self.label_side() {
            Side::West => Point::new(x - step, y),
            Side::East => Point::new(x + step, y),
            Side::North => Point::new(x, y - step),
            Side::South => Point::new(x, y + step),
        }
    }
}

pub(super) struct PlacedText {
    /// The deprecated named form's name, drawn as `data-text`.
    pub(super) name: Option<String>,
    pub(super) origin: Point,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) label: String,
}

/// Everything the renderer needs after walking the design's view once:
/// the placed boxes plus the wire segments to route between their ports.
pub(super) struct Placement {
    pub(super) components: IndexMap<InstanceName, PlacedComponent>,
    pub(super) texts: Vec<PlacedText>,
    /// The subject's boundary box with its inverted ports, when the view
    /// authors an `enclosure { }` block. Drawn and routed to, but never an
    /// obstacle (it's a container, not a component).
    enclosure: Option<PlacedComponent>,
    /// Chain-decomposed wire segments, both ends resolving to a placed
    /// port (see [`Placement::compute`]).
    connections: Vec<Segment>,
}

/// One chain-decomposed wire segment, keyed by its placed-port endpoints
/// and carrying the authored color of the wire it came from.
struct Segment {
    a: PortKey,
    b: PortKey,
    color: WireColor,
}

/// A segment resolved for routing and drawing: both placed ports plus the
/// wire's authored color (shown as a code annotation on the drawn wire).
pub(super) struct Connection<'a> {
    pub(super) a: &'a PlacedPort,
    pub(super) b: &'a PlacedPort,
    pub(super) color: &'a WireColor,
    /// The included instance each end (`a`, `b`) belongs to; `None` for a
    /// port on the enclosure.
    pub(super) owners: [Option<&'a InstanceName>; 2],
}

impl Connection<'_> {
    /// The segment's two ends as the design names them: `instance.port`,
    /// or the subject's own `port` for one on the enclosure.
    pub(super) fn ends(&self) -> [WireEnd; 2] {
        let end = |owner: Option<&InstanceName>, port: &PlacedPort| match owner {
            Some(instance) => WireEnd::Child {
                instance: instance.clone(),
                port: port.port.clone(),
            },
            None => WireEnd::Own(port.port.clone()),
        };
        [end(self.owners[0], self.a), end(self.owners[1], self.b)]
    }

    /// `connections` grouped into the electrical nets they draw: two
    /// connections are in one net when they share a placed port, directly
    /// or through other members — the segments of a multi-endpoint wire, a
    /// splice net's segments, two wires crimped into one port. Each net
    /// lists its members' indices in source order, and the nets come in the
    /// order of their first member.
    pub(super) fn nets(connections: &[Self]) -> Vec<Vec<usize>> {
        // Union-find over connection indices, joined through the first
        // connection seen at each port. The map is only looked up, never
        // iterated, so its order cannot leak into the result.
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let mut parent: Vec<usize> = (0..connections.len()).collect();
        let mut first_at: HashMap<WireEnd, usize> = HashMap::new();
        for (i, connection) in connections.iter().enumerate() {
            for end in connection.ends() {
                let other = *first_at.entry(end).or_insert(i);
                let (a, b) = (root(&mut parent, i), root(&mut parent, other));
                parent[a.max(b)] = a.min(b);
            }
        }
        let mut net_of_root: Vec<Option<usize>> = vec![None; connections.len()];
        let mut nets: Vec<Vec<usize>> = Vec::new();
        for i in 0..connections.len() {
            let r = root(&mut parent, i);
            match net_of_root[r] {
                Some(net) => nets[net].push(i),
                None => {
                    net_of_root[r] = Some(nets.len());
                    nets.push(vec![i]);
                }
            }
        }
        nets
    }
}

pub(super) struct ViewBox {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

/// The four sides of one box, each holding its ports in render order.
#[derive(Default)]
struct SidePorts<'a> {
    west: Vec<(&'a PortName, &'a Port)>,
    east: Vec<(&'a PortName, &'a Port)>,
    north: Vec<(&'a PortName, &'a Port)>,
    south: Vec<(&'a PortName, &'a Port)>,
}

impl<'a> SidePorts<'a> {
    fn push(&mut self, side: Side, entry: (&'a PortName, &'a Port)) {
        match side {
            Side::West => self.west.push(entry),
            Side::East => self.east.push(entry),
            Side::North => self.north.push(entry),
            Side::South => self.south.push(entry),
        }
    }

    fn sides(&self) -> [(Side, &Vec<(&'a PortName, &'a Port)>); 4] {
        [
            (Side::West, &self.west),
            (Side::East, &self.east),
            (Side::North, &self.north),
            (Side::South, &self.south),
        ]
    }
}

impl Placement {
    /// Walk `view` over `design`: resolve included child instances, read
    /// each one's authored port placements (side + order), decompose the
    /// subject's wires into segments between listed ports, then place boxes
    /// and ports on the grid.
    pub(super) fn compute(
        design: &Design,
        subject: &Instance,
        view: &crate::dsl::ir::View,
        grid: Grid,
    ) -> ViewResult<Self> {
        // Resolve each include to its child instance, world centre, and
        // authored port placements, preserving include order.
        let mut boxes: IndexMap<InstanceName, BoxEntry> = IndexMap::new();
        for inc in &view.includes {
            let Some(child_path) = subject.children.get(&inc.instance) else {
                continue;
            };
            let Some(child) = design.get(child_path) else {
                continue;
            };
            let centre = Point::new(grid.to_world(inc.x), grid.to_world(inc.y));
            boxes.insert(inc.instance.clone(), (centre, child, inc.ports.as_slice()));
        }

        // A port is shown iff the view lists it; that listing also fixes its
        // side. Build the lookup from the authored placements of every
        // resolved box, plus the subject's own ports placed on the enclosure.
        let mut side_of: HashMap<PortKey, Side> = HashMap::new();
        for (name, (_, _, ports)) in &boxes {
            for (port, side) in ports.iter() {
                side_of.insert(
                    PortKey::Child {
                        instance: name.clone(),
                        port: port.clone(),
                    },
                    *side,
                );
            }
        }
        for ep in &view.enclosure {
            side_of.insert(PortKey::Enclosure(ep.port.clone()), ep.side);
        }

        // Chain-decompose the subject's wires into segments whose ends both
        // resolve to a placed port — a listed port on an included box, or a
        // subject-own port listed in the enclosure. Ends on excluded
        // instances, unlisted ports, and own ports without an enclosure
        // placement drop silently.
        let mut connections: Vec<Segment> = Vec::new();
        for wire in &subject.wires {
            for segment in wire.segments() {
                let [from, to] = segment.ends;
                let (Some(a), Some(b)) = (endpoint_key(from), endpoint_key(to)) else {
                    continue;
                };
                if side_of.contains_key(&a) && side_of.contains_key(&b) {
                    connections.push(Segment {
                        a,
                        b,
                        color: wire.color.clone(),
                    });
                }
            }
        }

        // A splice is drawn as what it is electrically: one net joining the
        // far ends of its conductors (through any splice-to-splice links),
        // chained like a multi-endpoint rail between the ends the view
        // places, in wire order, each port once. A segment takes the color
        // of the conductor that brings its second end in.
        for net in subject.splice_nets() {
            let mut placed: Vec<(PortKey, &WireColor)> = Vec::new();
            for end in &net.ends {
                let Some(key) = endpoint_key(end.end) else {
                    continue;
                };
                if side_of.contains_key(&key) && !placed.iter().any(|(k, _)| k == &key) {
                    placed.push((key, &end.wire.color));
                }
            }
            for pair in placed.windows(2) {
                connections.push(Segment {
                    a: pair[0].0.clone(),
                    b: pair[1].0.clone(),
                    color: pair[1].1.clone(),
                });
            }
        }

        let pitch = grid.pitch();
        let mut components = IndexMap::new();

        for (name, (centre, inst, ports)) in &boxes {
            // Place ports onto their authored sides, in authored order.
            let mut side_ports = SidePorts::default();
            for (port_name, side) in ports.iter() {
                if let Some(port) = inst.ports.get(port_name) {
                    side_ports.push(*side, (port_name, port));
                }
            }

            let label = inst
                .label
                .clone()
                .unwrap_or_else(|| inst.type_name.to_string());
            let (width, height) = box_dimensions(&label, &side_ports, grid);
            let origin = Point::new(centre.x - width / 2.0, centre.y - height / 2.0);

            let mut ports = Vec::new();
            let mut port_index = HashMap::new();
            for (side, refs) in side_ports.sides() {
                let n = refs.len();
                let pins: Vec<_> = refs
                    .iter()
                    .map(|(_, port)| PinAnnotation::text(inst, port))
                    .collect();
                // A lone port has no neighbour for its text to reach.
                let spacing = if n > 1 { pitch } else { f64::INFINITY };
                let orientation = PinAnnotation::orientation(
                    side,
                    pins.iter().flatten().map(String::as_str),
                    spacing,
                );
                for (k, ((port_name, port), pin)) in refs.iter().zip(pins).enumerate() {
                    let pos = port_position(origin, width, height, side, k, n, pitch);
                    port_index.insert((*port_name).clone(), ports.len());
                    ports.push(PlacedPort {
                        port: (*port_name).clone(),
                        side,
                        pos,
                        pin: pin.map(|text| PinAnnotation::new(text, orientation)),
                        label: port.label.clone(),
                        inverted: false,
                        off_view: None,
                    });
                }
            }

            let mut component = PlacedComponent {
                origin,
                width,
                height,
                label,
                ports,
                port_index,
            };
            component.clear_pins_of_title();
            components.insert(name.clone(), component);
        }

        let texts = view
            .texts
            .iter()
            .map(|text| {
                let width = (text.label.chars().count() as f64 * CHAR_WIDTH + 2.0 * TEXT_BOX_PAD_X)
                    .max(TEXT_BOX_MIN_WIDTH);
                let height = TEXT_BOX_HEIGHT;
                let centre = Point::new(grid.to_world(text.x), grid.to_world(text.y));
                PlacedText {
                    name: text.name.clone(),
                    origin: Point::new(centre.x - width / 2.0, centre.y - height / 2.0),
                    width,
                    height,
                    label: text.label.clone(),
                }
            })
            .collect();

        let enclosure = Self::place_enclosure(subject, view, &components, grid);

        let mut placement = Self {
            components,
            texts,
            enclosure,
            connections,
        };
        for (key, undrawn) in placement.undrawn(subject) {
            if let Some(port) = placement.endpoint_mut(&key) {
                port.off_view = Some(undrawn);
            }
        }
        Ok(placement)
    }

    /// Every placed port whose wires reach ends the view does not place,
    /// with those ends, in wire order and then endpoint order.
    ///
    /// Along a wire, a placed port reaches the ends on either side of it up
    /// to the nearest placed one: `[a, x, y, b]` with only `a` and `b`
    /// placed gives each of them `x` and `y`, since neither segment to
    /// them is drawn. A splice net (taken at its first conductor in wire
    /// order) is drawn as a rail between the ports it places, so the ends
    /// it does not place are named once, beside the first port it places,
    /// as the splice its conductor lands on and the ends beyond it. A net
    /// placing one port and reaching nothing else still names its splice.
    fn undrawn(&self, subject: &Instance) -> IndexMap<PortKey, Undrawn> {
        let placed = |end: &WireEnd| endpoint_key(end).filter(|key| self.endpoint(key).is_some());
        let unplaced = |end: &&WireEnd| !matches!(end, WireEnd::Splice(_)) && placed(end).is_none();
        let mut undrawn: IndexMap<PortKey, Undrawn> = IndexMap::new();
        let mut add = |key: PortKey, far: FarEnd, color: &WireColor| {
            let entry = undrawn.entry(key).or_insert_with(|| Undrawn {
                far: Vec::new(),
                color: color.clone(),
            });
            if !entry.far.contains(&far) {
                entry.far.push(far);
            }
        };
        let nets = subject.splice_nets();
        let mut seen = vec![false; nets.len()];
        for wire in &subject.wires {
            let net = nets
                .iter()
                .position(|net| net.splices.iter().any(|splice| wire.ends_on(splice)));
            if let Some(n) = net {
                if std::mem::replace(&mut seen[n], true) {
                    continue;
                }
                let net = &nets[n];
                let mut ports: Vec<(PortKey, &NetEnd)> = Vec::new();
                let mut beyond: Vec<WireEnd> = Vec::new();
                for end in &net.ends {
                    match placed(end.end) {
                        Some(key) if !ports.iter().any(|(k, _)| k == &key) => {
                            ports.push((key, end))
                        }
                        Some(_) => {}
                        None if !beyond.contains(end.end) => beyond.push(end.end.clone()),
                        None => {}
                    }
                }
                let Some((key, first)) = ports.first() else {
                    continue;
                };
                if beyond.is_empty() && ports.len() > 1 {
                    continue;
                }
                let landing = match first.wire.other_end(first.end) {
                    Some(WireEnd::Splice(splice)) => Some(splice),
                    _ => net.splices.first().copied(),
                };
                if let Some(splice) = landing {
                    let far = FarEnd::Splice {
                        splice: splice.clone(),
                        beyond,
                    };
                    add(key.clone(), far, &first.wire.color);
                }
                continue;
            }
            for (i, end) in wire.endpoints.iter().enumerate() {
                let Some(key) = placed(end) else {
                    continue;
                };
                let mut before: Vec<&WireEnd> = wire.endpoints[..i]
                    .iter()
                    .rev()
                    .take_while(unplaced)
                    .collect();
                before.reverse();
                let after = wire.endpoints[i + 1..].iter().take_while(unplaced);
                for far in before.into_iter().chain(after) {
                    add(
                        key.clone(),
                        FarEnd::of(far, &subject.type_name),
                        &wire.color,
                    );
                }
            }
        }
        undrawn
    }

    /// Wrap the placed child boxes in the subject's boundary, placing each
    /// authored subject port on the named edge at its free-axis coordinate.
    /// `None` when the view has no `enclosure { }` block. When there are no
    /// child boxes, the authored enclosure ports seed a small boundary by
    /// themselves so a boundary-only view still renders.
    fn place_enclosure(
        subject: &Instance,
        view: &crate::dsl::ir::View,
        components: &IndexMap<InstanceName, PlacedComponent>,
        grid: Grid,
    ) -> Option<PlacedComponent> {
        if !view.has_enclosure {
            return None;
        }

        // Bounding box of every child, then folded to span each port's
        // free-axis coordinate so the edge always reaches its ports.
        let mut bounds: Option<(f64, f64, f64, f64)> = None;
        for pc in components.values() {
            grow_bounds(
                &mut bounds,
                pc.origin,
                Point::new(pc.origin.x + pc.width, pc.origin.y + pc.height),
            );
        }
        let (mut min_x, mut min_y, mut max_x, mut max_y) = bounds.unwrap_or((0.0, 0.0, 0.0, 0.0));
        for ep in &view.enclosure {
            let world = grid.to_world(ep.coord);
            match ep.side {
                Side::West | Side::East => {
                    min_y = min_y.min(world);
                    max_y = max_y.max(world);
                }
                Side::North | Side::South => {
                    min_x = min_x.min(world);
                    max_x = max_x.max(world);
                }
            }
        }

        let inset = grid.enclosure_inset();
        let (left, top) = (min_x - inset, min_y - inset);
        let (right, bottom) = (max_x + inset, max_y + inset);
        let origin = Point::new(left, top);
        let (width, height) = (right - left, bottom - top);

        let mut ports = Vec::new();
        let mut port_index = HashMap::new();
        for ep in &view.enclosure {
            let Some(port) = subject.ports.get(&ep.port) else {
                continue;
            };
            let free = grid.to_world(ep.coord);
            let pos = match ep.side {
                Side::West => Point::new(left, free),
                Side::East => Point::new(right, free),
                Side::North => Point::new(free, top),
                Side::South => Point::new(free, bottom),
            };
            port_index.insert(ep.port.clone(), ports.len());
            ports.push(PlacedPort {
                port: ep.port.clone(),
                side: ep.side,
                pos,
                pin: PinAnnotation::text(subject, port)
                    .map(|text| PinAnnotation::new(text, PinOrientation::Horizontal)),
                label: port.label.clone(),
                inverted: true,
                off_view: None,
            });
        }
        // Authored coordinates space enclosure ports freely, so each edge's
        // annotations are oriented by its closest pair (none for a lone
        // port) rather than the pitch.
        for side in [Side::West, Side::East, Side::North, Side::South] {
            let on_side = |p: &&PlacedPort| p.side == side;
            let mut along: Vec<f64> = ports
                .iter()
                .filter(on_side)
                .map(|p| match side {
                    Side::West | Side::East => p.pos.y,
                    Side::North | Side::South => p.pos.x,
                })
                .collect();
            along.sort_by(f64::total_cmp);
            let spacing = along
                .windows(2)
                .map(|w| w[1] - w[0])
                .fold(f64::INFINITY, f64::min);
            let orientation = PinAnnotation::orientation(
                side,
                ports
                    .iter()
                    .filter(on_side)
                    .filter_map(|p| Some(p.pin.as_ref()?.text.as_str())),
                spacing,
            );
            for placed in ports.iter_mut().filter(|p| p.side == side) {
                if let Some(pin) = &mut placed.pin {
                    pin.orientation = orientation;
                }
            }
        }

        let label = subject
            .label
            .clone()
            .unwrap_or_else(|| subject.type_name.to_string());

        Some(PlacedComponent {
            origin,
            width,
            height,
            label,
            ports,
            port_index,
        })
    }

    fn endpoint(&self, key: &PortKey) -> Option<&PlacedPort> {
        let (comp, port) = match key {
            PortKey::Child { instance, port } => (self.components.get(instance)?, port),
            PortKey::Enclosure(port) => (self.enclosure.as_ref()?, port),
        };
        let idx = comp.port_index.get(port)?;
        comp.ports.get(*idx)
    }

    fn endpoint_mut(&mut self, key: &PortKey) -> Option<&mut PlacedPort> {
        let (comp, port) = match key {
            PortKey::Child { instance, port } => (self.components.get_mut(instance)?, port),
            PortKey::Enclosure(port) => (self.enclosure.as_mut()?, port),
        };
        let idx = *comp.port_index.get(port)?;
        comp.ports.get_mut(idx)
    }

    /// The wire segments to route, each resolved to its two placed ports
    /// with the wire's color carried along for annotation.
    pub(super) fn connections(&self) -> Vec<Connection<'_>> {
        self.connections
            .iter()
            .filter_map(|s| {
                Some(Connection {
                    a: self.endpoint(&s.a)?,
                    b: self.endpoint(&s.b)?,
                    color: &s.color,
                    owners: [s.a.owner(), s.b.owner()],
                })
            })
            .collect()
    }

    /// Whether a segment the router will route (one of
    /// [`Self::connections`]) ends on `instance`'s `port`.
    pub(super) fn is_wired(&self, instance: &InstanceName, port: &PortName) -> bool {
        let is_this = |end: &PortKey| match end {
            PortKey::Child {
                instance: i,
                port: p,
            } => i == instance && p == port,
            PortKey::Enclosure(_) => false,
        };
        self.connections.iter().any(|s| {
            (is_this(&s.a) || is_this(&s.b))
                && self.endpoint(&s.a).is_some()
                && self.endpoint(&s.b).is_some()
        })
    }

    /// The bounding box of every placed component, in include order.
    pub(super) fn component_bounds(&self) -> impl Iterator<Item = Bounds> + '_ {
        self.components.values().map(PlacedComponent::bounds)
    }

    /// The subject's boundary box, if the view authors an `enclosure { }`.
    pub(super) fn enclosure(&self) -> Option<&PlacedComponent> {
        self.enclosure.as_ref()
    }

    /// The subject boundary as routing geometry. The router treats this as a
    /// soft guide to avoid overlapping the dashed enclosure, not as an
    /// obstacle: wires may still attach to enclosure ports.
    pub(super) fn enclosure_bounds(&self) -> Option<Bounds> {
        self.enclosure.as_ref().map(PlacedComponent::bounds)
    }

    /// Every placed port the router must reach: child ports, then the
    /// enclosure's inverted ports. The enclosure is a routing endpoint set
    /// but not an obstacle, so it is absent from [`Self::component_bounds`].
    pub(super) fn ports(&self) -> impl Iterator<Item = &PlacedPort> + '_ {
        self.components
            .values()
            .chain(self.enclosure.as_ref())
            .flat_map(|pc| pc.ports.iter())
    }

    /// The drawing's bounding box, padded for margins and an optional
    /// title. Encloses both the component boxes and every routed wire —
    /// wires can detour outside the boxes, so they must be measured too —
    /// and keeps every pin annotation and off-view reference inside it.
    pub(super) fn viewbox(
        &self,
        has_title: bool,
        wires: &[Vec<Point>],
        references: &[OffView],
    ) -> ViewBox {
        if self.components.is_empty() && self.texts.is_empty() && self.enclosure.is_none() {
            return ViewBox {
                x: 0.0,
                y: 0.0,
                width: MIN_WIDTH,
                height: MIN_HEIGHT,
            };
        }

        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for pc in self.components.values().chain(self.enclosure.as_ref()) {
            min_x = min_x.min(pc.origin.x);
            min_y = min_y.min(pc.origin.y);
            max_x = max_x.max(pc.origin.x + pc.width);
            max_y = max_y.max(pc.origin.y + pc.height);
        }
        for text in &self.texts {
            min_x = min_x.min(text.origin.x);
            min_y = min_y.min(text.origin.y);
            max_x = max_x.max(text.origin.x + text.width);
            max_y = max_y.max(text.origin.y + text.height);
        }

        for p in wires.iter().flatten() {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }

        // Enclosure port names sit *outside* the boundary, so they can reach
        // past the box bounds — measure them so the viewBox doesn't clip.
        if let Some(enc) = &self.enclosure {
            for port in &enc.ports {
                let reach = LABEL_INSET + port.label.chars().count() as f64 * CHAR_WIDTH;
                match port.side {
                    Side::West => min_x = min_x.min(port.pos.x - reach),
                    Side::East => max_x = max_x.max(port.pos.x + reach),
                    Side::North => min_y = min_y.min(port.pos.y - reach),
                    Side::South => max_y = max_y.max(port.pos.y + reach),
                }
            }
        }

        let pad = SVG_MARGIN;
        let title_pad = if has_title { 20.0 } else { 0.0 };
        let (mut left, mut top) = (min_x - pad, min_y - pad - title_pad);
        let (mut right, mut bottom) = (max_x + pad, max_y + pad);

        // A pin annotation outside a box can reach past everything measured
        // above (a long designator on an unwired port at the edge), and so
        // can an off-view reference. Each is kept `PIN_CANVAS_PAD` inside
        // the canvas, measured after the margin so a text the margin
        // already holds leaves the canvas as it was.
        let texts = self
            .ports()
            .filter_map(PlacedPort::pin_extent)
            .chain(references.iter().flat_map(OffView::extents));
        for extent in texts {
            left = left.min(extent.origin.x - PIN_CANVAS_PAD);
            top = top.min(extent.origin.y - PIN_CANVAS_PAD - title_pad);
            right = right.max(extent.origin.x + extent.width + PIN_CANVAS_PAD);
            bottom = bottom.max(extent.origin.y + extent.height + PIN_CANVAS_PAD);
        }

        ViewBox {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }
}

fn grow_bounds(bounds: &mut Option<(f64, f64, f64, f64)>, a: Point, b: Point) {
    match bounds {
        Some((min_x, min_y, max_x, max_y)) => {
            *min_x = (*min_x).min(a.x).min(b.x);
            *min_y = (*min_y).min(a.y).min(b.y);
            *max_x = (*max_x).max(a.x).max(b.x);
            *max_y = (*max_y).max(a.y).max(b.y);
        }
        None => {
            *bounds = Some((a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)));
        }
    }
}

/// A wire endpoint's port key: a child instance's port, or a subject-own port
/// on the enclosure boundary. Whether the key is actually drawn depends on it
/// being listed (present in `side_of`); see [`Placement::compute`]. A splice
/// has no port in a schematic (`None`): its conductors draw as the net they
/// join, between the ports at their far ends.
fn endpoint_key(end: &WireEnd) -> Option<PortKey> {
    match end {
        WireEnd::Child { instance, port } => Some(PortKey::Child {
            instance: instance.clone(),
            port: port.clone(),
        }),
        WireEnd::Own(port) => Some(PortKey::Enclosure(port.clone())),
        WireEnd::Splice(_) => None,
    }
}

/// The minimum box size (width, height) in world units that fits this
/// component's ports and label on the given grid. Both dimensions are
/// rounded up to an even number of steps (so the box centres on the grid),
/// and margins/port pitch are the same values [`port_position`] places
/// ports at — so the box always contains its ports.
fn box_dimensions(label: &str, sides: &SidePorts, grid: Grid) -> (f64, f64) {
    let (margin, pitch) = (grid.margin(), grid.pitch());

    let label_w = label.chars().count() as f64 * CHAR_WIDTH + 2.0 * margin;

    // Span a side needs for `n` ports: nothing when empty, otherwise the
    // pitch between ports plus a margin at each end.
    let side_extent = |n: usize| match n {
        0 => 0.0,
        _ => (n - 1) as f64 * pitch + 2.0 * margin,
    };

    let width = MIN_WIDTH
        .max(label_w)
        .max(side_extent(sides.north.len()))
        .max(side_extent(sides.south.len()));

    // North/south labels are drawn vertically (rotated 90°), each reaching
    // in only from the edge it sits on. The box must fit both without them
    // colliding, plus a margin of clearance between them.
    let top_label_h = vertical_label_extent(&sides.north);
    let bot_label_h = vertical_label_extent(&sides.south);
    let vertical_label_h = top_label_h + bot_label_h + margin;

    // No fixed height floor — the box need only fit its ports and any
    // north/south labels, but always at least one port with its margins.
    let height = side_extent(sides.west.len())
        .max(side_extent(sides.east.len()))
        .max(vertical_label_h)
        .max(2.0 * margin);

    (grid.snap_box(width), grid.snap_box(height))
}

/// Vertical span (in user units) that a rotated label needs from the box
/// edge inward, given the longest port name on the side. Returns 0 when
/// the side has no ports.
fn vertical_label_extent(refs: &[(&PortName, &Port)]) -> f64 {
    let max_chars = refs
        .iter()
        .map(|(_, port)| port.label.chars().count())
        .max()
        .unwrap_or(0);
    if max_chars == 0 {
        0.0
    } else {
        LABEL_INSET + max_chars as f64 * CHAR_WIDTH
    }
}

/// Place port `k` of `n` on `side`, the ports centred about the box centre
/// and a `pitch` apart. With an even `n` the ports straddle the centreline
/// (±1 step, ±3 steps, …); with an odd `n` the middle one sits on it. The
/// box is an even number of steps, so `pitch` is two steps, and the centre
/// is grid-aligned, every port lands on a grid line.
fn port_position(
    origin: Point,
    w: f64,
    h: f64,
    side: Side,
    k: usize,
    n: usize,
    pitch: f64,
) -> Point {
    let offset = (k as f64 - (n as f64 - 1.0) / 2.0) * pitch;

    match side {
        Side::North => Point::new(origin.x + w / 2.0 + offset, origin.y),
        Side::South => Point::new(origin.x + w / 2.0 + offset, origin.y + h),
        Side::West => Point::new(origin.x, origin.y + h / 2.0 + offset),
        Side::East => Point::new(origin.x + w, origin.y + h / 2.0 + offset),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::schematic::tests::{design_from, view_of};

    #[test]
    fn port_position_centres_ports_about_the_box() {
        let origin = Point::ORIGIN;
        let (h, pitch) = (200.0, 40.0);
        let mid = h / 2.0;

        let p0 = port_position(origin, 100.0, h, Side::West, 0, 2, pitch);
        let p1 = port_position(origin, 100.0, h, Side::West, 1, 2, pitch);
        assert_eq!(p0.x, 0.0);
        assert_eq!(p0.y, mid - pitch / 2.0);
        assert_eq!(p1.y, mid + pitch / 2.0);

        let q = port_position(origin, 100.0, h, Side::West, 1, 3, pitch);
        assert_eq!(q.y, mid);
    }

    #[test]
    fn ports_take_their_authored_side() {
        // Sides come straight from the view; the geometry just honours them.
        let design = design_from(
            r#"
component sys {
    a: blk;
    b: blk;
    wire red 1 [a.p, b.p];
    component blk {
        pub port p "P";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 10.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, Grid::new(20.0)).unwrap();

        let a_port = &placement.components[&InstanceName::from("a")].ports[0];
        let b_port = &placement.components[&InstanceName::from("b")].ports[0];
        assert_eq!(a_port.side, Side::East);
        assert_eq!(b_port.side, Side::West);
    }

    #[test]
    fn chain_net_yields_consecutive_segments() {
        // A three-endpoint net decomposes into two segments (a-b, b-c).
        let design = design_from(
            r#"
component sys {
    a: blk;
    b: blk;
    c: blk;
    wire red 1 [a.p, b.p, c.p];
    component blk {
        pub port p "P";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 10.0, 0.0, &[("p", Side::East)]),
                ("c", 20.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, Grid::new(20.0)).unwrap();
        assert_eq!(placement.connections().len(), 2);
    }

    #[test]
    fn connections_sharing_a_port_form_one_net_in_source_order() {
        // `a`–`b`–`c` is one rail; `d`–`e` is a net of its own; `f`'s wire
        // is crimped into `c.p` beside the rail's end, joining the rail.
        let design = design_from(
            r#"
component sys {
    a: blk;
    b: blk;
    c: blk;
    d: blk;
    e: blk;
    f: blk;
    wire red 1 [a.p, b.p, c.p];
    wire blue 1 [d.p, e.p];
    wire white 1 [f.p, c.p];
    component blk {
        pub port p "P";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 10.0, 0.0, &[("p", Side::East)]),
                ("c", 20.0, 0.0, &[("p", Side::West)]),
                ("d", 0.0, 10.0, &[("p", Side::East)]),
                ("e", 20.0, 10.0, &[("p", Side::West)]),
                ("f", 0.0, 20.0, &[("p", Side::East)]),
            ],
        );
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, Grid::new(20.0)).unwrap();
        let connections = placement.connections();
        assert_eq!(connections.len(), 4);
        assert_eq!(Connection::nets(&connections), [vec![0, 1, 3], vec![2]]);
    }

    /// `a`, `b`, `c` and `d` each run a conductor to splice `sp` or, for
    /// `d`, to the splice `far` that `sp` links to.
    fn spliced() -> Design {
        design_from(
            r#"
component sys {
    a: blk;
    b: blk;
    c: blk;
    d: blk;
    splice sp;
    splice far;
    wire black 1 [a.p, sp];
    wire brown 1 [sp, b.p];
    wire blue 1 [c.p, sp];
    wire red 1 [sp, far];
    wire green 1 [far, d.p];
    wire grey 1 [c.p, far];
    component blk {
        pub port p "P";
    }
}
"#,
        )
    }

    /// The connections a view including `includes` in a row, each showing
    /// its `p`, places: as `owner.port` pairs, with their color.
    fn connections_of(design: &Design, includes: &[&str]) -> Vec<String> {
        const SOUTH: &[(&str, Side)] = &[("p", Side::South)];
        let includes: Vec<_> = includes
            .iter()
            .enumerate()
            .map(|(k, name)| (*name, k as f64 * 10.0, 0.0, SOUTH))
            .collect();
        let view = view_of("sys", &includes);
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(design, subject, &view, Grid::new(20.0)).unwrap();
        placement
            .connections()
            .iter()
            .map(|c| {
                let [a, b] = c.ends();
                format!("{a}-{b} {}", c.color)
            })
            .collect()
    }

    #[test]
    fn a_splice_draws_as_one_net_between_the_ports_it_joins() {
        let design = spliced();
        // The far ends of every conductor of the net (through the link to
        // `far`), chained in wire order, each port once (`c.p` reaches the
        // net twice); a segment takes the color of the conductor bringing
        // its second end in.
        assert_eq!(
            connections_of(&design, &["a", "b", "c", "d"]),
            ["a.p-b.p brown", "b.p-c.p blue", "c.p-d.p green"]
        );
        // An end the view leaves out drops, and the chain closes over it.
        assert_eq!(connections_of(&design, &["a", "d"]), ["a.p-d.p green"]);
        assert!(connections_of(&design, &["a"]).is_empty());
    }

    /// Each placed port's undrawn far ends, as `owner.port: end, end`
    /// (`port` alone on the enclosure), with the stub's colour.
    fn undrawn_of(placement: &Placement) -> Vec<String> {
        let boxes = placement.components.iter().map(|(n, pc)| (Some(n), pc));
        boxes
            .chain(placement.enclosure().map(|pc| (None, pc)))
            .flat_map(|(owner, pc)| pc.ports.iter().map(move |port| (owner, port)))
            .filter_map(|(owner, port)| {
                let undrawn = port.off_view.as_ref()?;
                let far: Vec<String> = undrawn.far.iter().map(ToString::to_string).collect();
                let at = match owner {
                    Some(owner) => format!("{owner}.{}", port.port),
                    None => port.port.to_string(),
                };
                Some(format!("{at}: {} {}", far.join(", "), undrawn.color))
            })
            .collect()
    }

    #[test]
    fn a_placed_port_records_the_unplaced_ends_on_either_side_of_it() {
        // `a` and `c` are placed; along the rail, `a` reaches `x` and `y`
        // up to `c`, and `c` reaches back to the same two and on to `z`.
        let design = design_from(
            r#"
component sys {
    a: blk;
    c: blk;
    x: blk;
    y: blk;
    z: blk;
    wire red 1 [a.p, x.p, y.p, c.p, z.p];
    component blk {
        pub port p "P";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("c", 10.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, Grid::new(20.0)).unwrap();
        assert_eq!(
            undrawn_of(&placement),
            ["a.p: x.p, y.p red", "c.p: x.p, y.p, z.p red"]
        );
    }

    #[test]
    fn drawn_wires_and_unwired_ports_record_nothing() {
        // `a.p`–`b.p` is drawn; `b.spare` has no wire; `b.p`'s second
        // wire runs to `c`, which the view leaves out: only it is recorded,
        // in that wire's colour.
        let design = design_from(
            r#"
component sys {
    a: blk;
    b: pad;
    c: blk;
    wire red 1 [a.p, b.p];
    wire blue 1 [b.p, c.p];
    component blk {
        pub port p "P";
    }
    component pad {
        pub port p "P";
        pub port spare "Spare";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 10.0, 0.0, &[("p", Side::West), ("spare", Side::East)]),
            ],
        );
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, Grid::new(20.0)).unwrap();
        assert_eq!(undrawn_of(&placement), ["b.p: c.p blue"]);
    }

    #[test]
    fn an_own_port_off_the_enclosure_is_where_the_wire_leaves_the_subject() {
        let design = design_from(
            r#"
component sys {
    a: blk;
    pub port out "OUT";
    wire red 1 [a.p, out];
    component blk {
        pub port p "P";
    }
}
"#,
        );
        let subject = design.get(&design.root).unwrap();
        let bare = view_of("sys", &[("a", 0.0, 0.0, &[("p", Side::East)])]);
        let placement = Placement::compute(&design, subject, &bare, Grid::new(20.0)).unwrap();
        assert_eq!(undrawn_of(&placement), ["a.p: out (leaves sys) red"]);

        // Placed on an enclosure, the wire is drawn and nothing is left.
        let mut enclosed = bare;
        enclosed.has_enclosure = true;
        enclosed.enclosure.push(crate::dsl::ir::EnclosurePort {
            port: PortName::from("out"),
            side: Side::East,
            coord: 0.0,
        });
        let placement = Placement::compute(&design, subject, &enclosed, Grid::new(20.0)).unwrap();
        assert!(undrawn_of(&placement).is_empty());
    }

    #[test]
    fn a_splice_net_names_its_splice_and_the_ends_beyond_it_once() {
        let design = spliced();
        let subject = design.get(&design.root).unwrap();
        let placed = |includes: &[&str]| {
            const SOUTH: &[(&str, Side)] = &[("p", Side::South)];
            let includes: Vec<_> = includes
                .iter()
                .enumerate()
                .map(|(k, name)| (*name, k as f64 * 10.0, 0.0, SOUTH))
                .collect();
            let view = view_of("sys", &includes);
            let placement = Placement::compute(&design, subject, &view, Grid::new(20.0)).unwrap();
            undrawn_of(&placement)
        };
        // The rail `a`–`b` is drawn; the net's other ends are named once,
        // beside its first placed port, after the splice `a`'s conductor
        // lands on, in that conductor's colour.
        assert_eq!(placed(&["a", "b"]), ["a.p: sp (c.p, d.p) black"]);
        assert_eq!(placed(&["b"]), ["b.p: sp (a.p, c.p, d.p) brown"]);
        assert!(placed(&["a", "b", "c", "d"]).is_empty());
    }

    #[test]
    fn unlisted_port_is_hidden() {
        // `b.spare` isn't listed in the view, so it isn't placed.
        let design = design_from(
            r#"
component sys {
    a: blk;
    b: pad;
    wire red 1 [a.p, b.p];
    component blk {
        pub port p "P";
    }
    component pad {
        pub port p "P";
        pub port spare "Spare";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 10.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, Grid::new(20.0)).unwrap();
        let b = &placement.components[&InstanceName::from("b")];
        assert_eq!(b.ports.len(), 1);
        assert_eq!(b.ports[0].port, PortName::from("p"));
    }

    #[test]
    fn listed_unconnected_port_is_shown() {
        // `b.spare` is listed but never wired: it shows as a bare stub.
        let design = design_from(
            r#"
component sys {
    a: blk;
    b: pad;
    wire red 1 [a.p, b.p];
    component blk {
        pub port p "P";
    }
    component pad {
        pub port p "P";
        pub port spare "Spare";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 10.0, 0.0, &[("p", Side::West), ("spare", Side::East)]),
            ],
        );
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, Grid::new(20.0)).unwrap();
        let b = &placement.components[&InstanceName::from("b")];
        assert_eq!(b.ports.len(), 2);
        assert!(b.ports.iter().any(|p| p.port == PortName::from("spare")));
        // The unconnected port draws no wire.
        assert_eq!(placement.connections().len(), 1);
    }

    #[test]
    fn enclosure_draws_a_subject_wire_and_places_ports_on_its_edge() {
        // The wire `[a.p, out]` ends on the subject's own port `out`; listing
        // `out` in the enclosure un-drops it and places it on the east edge.
        let design = design_from(
            r#"
component sys {
    a: blk;
    pub port out "OUT";
    wire red 1 [a.p, out];
    component blk {
        pub port p "P";
    }
}

view schematic "T" {
    grid: 10;
    enclosure {
        out at (east, 0);
    }
    include a at (0, 0) ports { west: p; }
}
"#,
        );
        let subject = design.get(&design.root).unwrap();
        let view = &design.views[0];
        let placement = Placement::compute(&design, subject, view, Grid::new(10.0)).unwrap();

        // The own end now resolves to a drawn connection.
        assert_eq!(placement.connections().len(), 1);

        // The enclosure wraps the child and sits `out` on its (inverted) east edge.
        let enc = placement.enclosure().expect("enclosure placed");
        let port = &enc.ports[0];
        assert_eq!(port.port, PortName::from("out"));
        assert!(port.inverted);
        assert_eq!(port.side, Side::East);
        assert_eq!(port.pos.x, enc.origin.x + enc.width);
    }

    #[test]
    fn no_enclosure_block_leaves_no_boundary() {
        let design = design_from(
            r#"
component sys {
    a: blk;
    b: blk;
    wire red 1 [a.p, b.p];
    component blk {
        pub port p "P";
    }
}
"#,
        );
        let subject = design.get(&design.root).unwrap();
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                ("b", 10.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let placement = Placement::compute(&design, subject, &view, Grid::new(20.0)).unwrap();
        assert!(placement.enclosure().is_none());
    }

    #[test]
    fn empty_enclosure_block_wraps_child_boxes() {
        let design = design_from(
            r#"
component sys {
    a: blk;
    component blk {
        pub port p "P";
    }
}

view schematic "T" {
    grid: 10;
    enclosure {
    }
    include a at (0, 0);
}
"#,
        );
        let subject = design.get(&design.root).unwrap();
        let view = &design.views[0];
        let placement = Placement::compute(&design, subject, view, Grid::new(10.0)).unwrap();

        let child = placement.components.get(&InstanceName::from("a")).unwrap();
        let enc = placement.enclosure().expect("enclosure placed");
        assert!(enc.ports.is_empty());
        assert!(enc.origin.x < child.origin.x);
        assert!(enc.origin.y < child.origin.y);
        assert!(enc.origin.x + enc.width > child.origin.x + child.width);
        assert!(enc.origin.y + enc.height > child.origin.y + child.height);
    }

    #[test]
    fn enclosure_without_child_boxes_still_draws_boundary_ports() {
        let design = design_from(
            r#"
component sys {
    pub port a "A";
    pub port b "B";
    wire red 1 [a, b];
}

view schematic "T" {
    grid: 10;
    enclosure {
        a at (west, 0);
        b at (east, 2);
    }
}
"#,
        );
        let subject = design.get(&design.root).unwrap();
        let view = &design.views[0];
        let placement = Placement::compute(&design, subject, view, Grid::new(10.0)).unwrap();

        let enc = placement.enclosure().expect("enclosure placed");
        assert_eq!(enc.ports.len(), 2);
        assert!(enc.width > 0.0);
        assert!(enc.height > 0.0);
        assert_eq!(placement.connections().len(), 1);

        let vb = placement.viewbox(false, &[], &[]);
        assert!(vb.width > MIN_WIDTH);
        assert!(vb.height > 0.0);
    }

    #[test]
    fn ports_land_on_grid_lines() {
        let design = design_from(
            r#"
component sys {
    a: blk;
    b: blk;
    wire red 1 [a.p1, b.p];
    wire red 1 [a.p2, b.p];
    wire red 1 [a.p3, b.p];
    component blk {
        pub port p1 "P1";
        pub port p2 "P2";
        pub port p3 "P3";
        pub port p "P";
    }
}
"#,
        );
        let view = view_of(
            "sys",
            &[
                (
                    "a",
                    0.0,
                    5.0,
                    &[("p1", Side::East), ("p2", Side::East), ("p3", Side::East)],
                ),
                ("b", 12.0, 5.0, &[("p", Side::West)]),
            ],
        );
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, Grid::new(10.0)).unwrap();
        for port in placement.ports() {
            assert_eq!(port.pos.x % 10.0, 0.0, "x off-grid: {:?}", port.pos);
            assert_eq!(port.pos.y % 10.0, 0.0, "y off-grid: {:?}", port.pos);
        }
    }

    /// `sys`, wired to the outside through an HV and a signal connector
    /// that both have a pin 1, around `ecu`, which has two connectors with
    /// long designators, and `lamp`, which has one.
    fn two_connectors() -> Design {
        design_from(
            r#"
component sys {
    ecu: Ecu;
    lamp: Lamp;
    connector hv "HV 2p" {
        pub port hv_pos "HV+" pin 1;
    }
    connector sig "Signal 2p" {
        pub port wake "WAKE" pin 1;
    }
    wire red 1 [ecu.a, lamp.a];
    component Ecu {
        connector bms_signals "Signals 2p" {
            pub port a "A" pin 1;
            pub port b "B" pin 2;
        }
        connector power "Power 2p" {
            pub port c "C" pin 1;
        }
    }
    component Lamp {
        connector x1 "Lamp 2p" {
            pub port a "A" pin 1;
            pub port b "B" pin 2;
        }
    }
}

view schematic "T" {
    enclosure {
        hv_pos at (west, 0);
        wake at (0, south);
    }
    include ecu at (0, 0) ports { north: a, b; west: c; }
    include lamp at (14, 0) ports { south: a, b; }
}
"#,
        )
    }

    /// Each placed port's annotation as `port text turned`, box by box,
    /// then the enclosure's.
    fn pins_of(placement: &Placement) -> Vec<String> {
        placement
            .components
            .values()
            .chain(placement.enclosure())
            .flat_map(|pc| &pc.ports)
            .filter_map(|port| {
                let pin = port.pin.as_ref()?;
                let turned = pin.orientation == PinOrientation::Vertical;
                Some(format!("{} {} {turned}", port.port, pin.text))
            })
            .collect()
    }

    #[test]
    fn annotations_name_the_connector_on_boxes_and_the_enclosure_with_several() {
        let design = two_connectors();
        let subject = design.get(&design.root).unwrap();
        let view = &design.views[0];
        let placement = Placement::compute(&design, subject, view, Grid::new(15.0)).unwrap();
        assert_eq!(
            pins_of(&placement),
            [
                // A lone port's text has no neighbour to reach, and stays
                // level however long it is.
                "c power:1 false",
                "a bms_signals:1 true",
                "b bms_signals:2 true",
                "a 1 false",
                "b 2 false",
                "hv_pos hv:1 false",
                "wake sig:1 false",
            ]
        );
    }

    #[test]
    fn the_canvas_holds_a_long_annotation_and_leaves_a_short_one_to_the_margin() {
        let design = two_connectors();
        let subject = design.get(&design.root).unwrap();
        let mut view = view_of(
            "sys",
            &[("lamp", 0.0, 0.0, &[("a", Side::West), ("b", Side::East)])],
        );
        let grid = Grid::new(15.0);
        let short = Placement::compute(&design, subject, &view, grid).unwrap();
        let lamp = &short.components[&InstanceName::from("lamp")];
        let vb = short.viewbox(false, &[], &[]);
        assert_eq!(vb.x, lamp.origin.x - SVG_MARGIN);
        assert_eq!(vb.x + vb.width, lamp.origin.x + lamp.width + SVG_MARGIN);

        view.includes[0].instance = InstanceName::from("ecu");
        view.includes[0].ports = vec![(PortName::from("a"), Side::West)];
        let long = Placement::compute(&design, subject, &view, grid).unwrap();
        let vb = long.viewbox(false, &[], &[]);
        let ecu = &long.components[&InstanceName::from("ecu")];
        let west = ecu.ports[0].pin_extent().unwrap();
        // `bms_signals:1` reaches past the margin on the west; the east,
        // with no annotation, keeps the margin.
        assert_eq!(vb.x, west.origin.x - PIN_CANVAS_PAD);
        assert_eq!(vb.x + vb.width, ecu.origin.x + ecu.width + SVG_MARGIN);
    }

    #[test]
    fn a_single_connector_side_turns_when_a_bare_text_would_reach_the_next_port() {
        // The rule is the same on every component: `1,2,3` level would run
        // over the next port's stub at the default pitch, and so would a
        // two-digit pin on a grid of 10, so their sides turn; a side of
        // single digits stays level.
        let design = design_from(
            r#"
component sys {
    lamp: Lamp;
    component Lamp {
        connector x1 "Lamp 8p" {
            pub port a "A" pins [1, 2, 3];
            pub port b "B" pin 4;
            pub port c "C" pin 10;
            pub port d "D" pin 11;
            pub port e "E" pin 5;
            pub port f "F" pin 6;
        }
    }
}
"#,
        );
        let subject = design.get(&design.root).unwrap();
        let view = view_of(
            "sys",
            &[(
                "lamp",
                0.0,
                0.0,
                &[
                    ("a", Side::North),
                    ("b", Side::North),
                    ("c", Side::South),
                    ("d", Side::South),
                    ("e", Side::West),
                    ("f", Side::West),
                ],
            )],
        );
        let turned = |grid: f64| {
            let placement = Placement::compute(&design, subject, &view, Grid::new(grid)).unwrap();
            pins_of(&placement)
        };
        assert_eq!(
            turned(15.0),
            [
                "e 5 false",
                "f 6 false",
                "a 1,2,3 true",
                "b 4 true",
                "c 10 false",
                "d 11 false",
            ]
        );
        assert_eq!(
            turned(10.0),
            [
                "e 5 false",
                "f 6 false",
                "a 1,2,3 true",
                "b 4 true",
                "c 10 true",
                "d 11 true",
            ]
        );
    }

    #[test]
    fn north_pin_annotations_under_the_title_move_past_it_and_others_stay() {
        // The cell module's case: a box with several connectors whose
        // north pins sit under its centred title. The three under
        // "BmsSlave" move out past it; the fourth, right of the title,
        // keeps its place.
        let design = design_from(
            r#"
component sys {
    slave: BmsSlave;
    component BmsSlave {
        connector iso_up "ISO-SPI up 2p" {
            pub port a "A" pin 1;
            pub port b "B" pin 2;
        }
        connector iso_down "ISO-SPI down 2p" {
            pub port c "C" pin 1;
            pub port d "D" pin 2;
        }
    }
}
"#,
        );
        let subject = design.get(&design.root).unwrap();
        let view = view_of(
            "sys",
            &[(
                "slave",
                0.0,
                0.0,
                &[
                    ("a", Side::North),
                    ("b", Side::North),
                    ("c", Side::North),
                    ("d", Side::North),
                ],
            )],
        );
        let placement = Placement::compute(&design, subject, &view, Grid::new(15.0)).unwrap();
        let slave = &placement.components[&InstanceName::from("slave")];
        let title = slave.title_extent();
        let moved: Vec<bool> = slave
            .ports
            .iter()
            .map(|port| {
                let pin = port.pin.as_ref().unwrap();
                let extent = port.pin_extent().unwrap();
                assert!(!extent.overlaps(&title), "{}: {extent:?}", port.port);
                // Turned, reading down: the number ends at the anchor, the
                // end nearest the port.
                let at = pin.placement(Side::North, port.pos);
                assert_eq!((at.anchor, at.rotate), ("end", 90.0), "{}", port.port);
                assert_eq!(at.y, extent.origin.y + extent.height, "{}", port.port);
                let unmoved = PinAnnotation::new(pin.text.clone(), pin.orientation)
                    .extent(Side::North, port.pos);
                extent != unmoved
            })
            .collect();
        assert_eq!(moved, [true, true, true, false]);
    }
}
