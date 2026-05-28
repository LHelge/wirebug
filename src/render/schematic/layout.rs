//! Component/port placement: walking a design's view once to produce
//! positioned boxes and ports in SVG world coordinates.
//!
//! The DSL view authors each include's ports explicitly: which side a port
//! sits on, and in what order (the `ports { }` block). A box shows exactly
//! the ports listed for it — that list is both the layout and the scope.
//! A wire segment is drawn only when both its ends are listed ports on
//! included boxes; an unlisted or own end drops silently, so a listed port
//! with no surviving wire shows as a bare labelled stub.

use std::collections::HashMap;

use indexmap::IndexMap;

use super::{
    CHAR_WIDTH, LABEL_INSET, MIN_HEIGHT, MIN_WIDTH, SVG_MARGIN, TEXT_BOX_HEIGHT,
    TEXT_BOX_MIN_WIDTH, TEXT_BOX_PAD_X,
};
use crate::dsl::ir::{Design, Instance, InstanceName, Pin, Port, PortName};
use crate::error::Result;
use crate::render::geometry::{Point, Side};

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
    /// adjacent port labels never collide.
    fn pitch(self) -> f64 {
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

pub(super) struct PlacedPort {
    pub(super) port: PortName,
    pub(super) side: Side,
    pub(super) pos: Point,
    pub(super) pin: Option<String>,
    pub(super) label: String,
    /// True for an enclosure port: it sits on the boundary facing *inward*,
    /// so its wire leaves toward the schematic interior, not outward.
    pub(super) inverted: bool,
}

pub(super) struct PlacedText {
    pub(super) name: String,
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
    connections: Vec<(PortKey, PortKey)>,
}

pub(super) struct ViewBox {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

/// A component's axis-aligned box in world coordinates — the geometry the
/// router needs, without the rest of a `PlacedComponent`.
pub(super) struct Bounds {
    pub(super) origin: Point,
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
    ) -> Result<Self> {
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
        let mut connections: Vec<(PortKey, PortKey)> = Vec::new();
        for wire in &subject.wires {
            for pair in wire.endpoints.windows(2) {
                let (a, b) = (endpoint_key(&pair[0]), endpoint_key(&pair[1]));
                if side_of.contains_key(&a) && side_of.contains_key(&b) {
                    connections.push((a, b));
                }
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
                for (k, (port_name, port)) in refs.iter().enumerate() {
                    let pos = port_position(origin, width, height, side, k, n, pitch);
                    port_index.insert((*port_name).clone(), ports.len());
                    ports.push(PlacedPort {
                        port: (*port_name).clone(),
                        side,
                        pos,
                        pin: format_pins(&port.pins),
                        label: port.label.clone(),
                        inverted: false,
                    });
                }
            }

            components.insert(
                name.clone(),
                PlacedComponent {
                    origin,
                    width,
                    height,
                    label,
                    ports,
                    port_index,
                },
            );
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

        Ok(Self {
            components,
            texts,
            enclosure,
            connections,
        })
    }

    /// Wrap the placed child boxes in the subject's boundary, placing each
    /// authored subject port on the named edge at its free-axis coordinate.
    /// `None` when the view has no `enclosure { }` block or no boxes to wrap.
    fn place_enclosure(
        subject: &Instance,
        view: &crate::dsl::ir::View,
        components: &IndexMap<InstanceName, PlacedComponent>,
        grid: Grid,
    ) -> Option<PlacedComponent> {
        if view.enclosure.is_empty() || components.is_empty() {
            return None;
        }

        // Bounding box of every child, then folded to span each port's
        // free-axis coordinate so the edge always reaches its ports.
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for pc in components.values() {
            min_x = min_x.min(pc.origin.x);
            min_y = min_y.min(pc.origin.y);
            max_x = max_x.max(pc.origin.x + pc.width);
            max_y = max_y.max(pc.origin.y + pc.height);
        }
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
                pin: format_pins(&port.pins),
                label: port.label.clone(),
                inverted: true,
            });
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
        Some(&comp.ports[*idx])
    }

    /// The wire segments to route, each resolved to its two placed ports.
    pub(super) fn connection_pairs(&self) -> Vec<(&PlacedPort, &PlacedPort)> {
        self.connections
            .iter()
            .filter_map(|(a, b)| Some((self.endpoint(a)?, self.endpoint(b)?)))
            .collect()
    }

    /// The bounding box of every placed component, in include order.
    pub(super) fn component_bounds(&self) -> impl Iterator<Item = Bounds> + '_ {
        self.components.values().map(|pc| Bounds {
            origin: pc.origin,
            width: pc.width,
            height: pc.height,
        })
    }

    /// The subject's boundary box, if the view authors an `enclosure { }`.
    pub(super) fn enclosure(&self) -> Option<&PlacedComponent> {
        self.enclosure.as_ref()
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
    /// wires can detour outside the boxes, so they must be measured too.
    pub(super) fn viewbox(&self, has_title: bool, wires: &[Vec<Point>]) -> ViewBox {
        if self.components.is_empty() && self.texts.is_empty() {
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
        ViewBox {
            x: min_x - pad,
            y: min_y - pad - title_pad,
            width: (max_x - min_x) + 2.0 * pad,
            height: (max_y - min_y) + 2.0 * pad + title_pad,
        }
    }
}

/// A wire endpoint's port key: a child instance's port, or a subject-own port
/// on the enclosure boundary. Whether the key is actually drawn depends on it
/// being listed (present in `side_of`); see [`Placement::compute`].
fn endpoint_key(end: &crate::dsl::ir::WireEnd) -> PortKey {
    match end {
        crate::dsl::ir::WireEnd::Child { instance, port } => PortKey::Child {
            instance: instance.clone(),
            port: port.clone(),
        },
        crate::dsl::ir::WireEnd::Own(port) => PortKey::Enclosure(port.clone()),
    }
}

/// Render a port's pins as a comma-joined string, or `None` when it has
/// no pin assignment.
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
    blk a;
    blk b;
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
    blk a;
    blk b;
    blk c;
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
        assert_eq!(placement.connection_pairs().len(), 2);
    }

    #[test]
    fn unlisted_port_is_hidden() {
        // `b.spare` isn't listed in the view, so it isn't placed.
        let design = design_from(
            r#"
component sys {
    blk a;
    pad b;
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
    blk a;
    pad b;
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
        assert_eq!(placement.connection_pairs().len(), 1);
    }

    #[test]
    fn enclosure_draws_a_subject_wire_and_places_ports_on_its_edge() {
        // The wire `[a.p, out]` ends on the subject's own port `out`; listing
        // `out` in the enclosure un-drops it and places it on the east edge.
        let design = design_from(
            r#"
component sys {
    blk a;
    pub port out "OUT";
    wire red 1 [a.p, out];
    component blk {
        pub port p "P";
    }
}

view schematic "T" {
    grid 10;
    enclosure {
        out at (east, 0);
    }
    include a at (0, 0) ports { west: p; };
}
"#,
        );
        let subject = design.get(&design.root).unwrap();
        let view = &design.views[0];
        let placement = Placement::compute(&design, subject, view, Grid::new(10.0)).unwrap();

        // The own end now resolves to a drawn connection.
        assert_eq!(placement.connection_pairs().len(), 1);

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
    blk a;
    blk b;
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
    fn ports_land_on_grid_lines() {
        let design = design_from(
            r#"
component sys {
    blk a;
    blk b;
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
}
