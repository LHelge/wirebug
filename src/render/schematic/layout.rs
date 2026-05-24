//! Component/port placement: walking a model + view once to produce
//! positioned boxes and ports in SVG world coordinates.

use std::collections::HashMap;

use indexmap::IndexMap;

use super::{CHAR_WIDTH, LABEL_INSET, MIN_HEIGHT, MIN_WIDTH, SVG_MARGIN};
use crate::error::{Error, Result};
use crate::model::{Component, ComponentId, Model, PortRef};
use crate::view::{ComponentPortLayout, ConnectorPortRef, Point, Side, View};

/// Tolerance for comparing an author-supplied box size against the
/// derived minimum, in world units.
const TOL: f64 = 1e-6;

/// The view's grid: one step in world units. Coordinates (component
/// centres) and sizes given in grid units reach world space by
/// multiplying. Ports sit two steps apart, centred on each side, and a box
/// is always an even number of steps — so its centre, and every port,
/// lands on a grid line for any port count.
#[derive(Clone, Copy)]
pub(super) struct Grid(f64);

impl Grid {
    pub(super) fn new(step: f64) -> Self {
        Self(step)
    }

    fn step(self) -> f64 {
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
    /// Keeps the box an even number of steps, so centring stays on-grid.
    fn margin(self) -> f64 {
        2.0 * self.0
    }

    /// Spacing between consecutive ports on a side: two grid steps. The
    /// grid is validated so this pitch is at least `MIN_PORT_PITCH`, so
    /// adjacent port labels never collide.
    fn pitch(self) -> f64 {
        2.0 * self.0
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
    port_index: HashMap<ConnectorPortRef, usize>,
}

pub(super) struct PlacedPort {
    pub(super) cp: ConnectorPortRef,
    pub(super) side: Side,
    pub(super) pos: Point,
    pub(super) pin: Option<String>,
    pub(super) label: String,
}

/// Everything the renderer needs after walking the model + view once.
pub(super) struct Placement {
    pub(super) components: IndexMap<ComponentId, PlacedComponent>,
}

pub(super) struct ViewBox {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

/// A component's axis-aligned box in world coordinates — the geometry the
/// router needs, without the rest of a `PlacedComponent`. Keeps "a box is
/// origin + size" a fact of `layout`, not of whoever consumes it.
pub(super) struct Bounds {
    pub(super) origin: Point,
    pub(super) width: f64,
    pub(super) height: f64,
}

impl Placement {
    pub(super) fn compute(model: &Model, view: &View, grid: Grid) -> Result<Self> {
        let mut components = IndexMap::new();
        let empty_layout = ComponentPortLayout::default();
        let pitch = grid.pitch();

        for (cid, bx) in &view.layout {
            let Some(component) = model.components.get(cid) else {
                continue;
            };
            let port_layout = view.ports.get(cid).unwrap_or(&empty_layout);

            let (min_w, min_h) = box_dimensions(component, cid, port_layout, grid);
            let width = resolve_size(cid, "width", bx.width, min_w, grid)?;
            let height = resolve_size(cid, "height", bx.height, min_h, grid)?;
            // `bx.x`/`bx.y` are the box centre in grid units. Box sizes are
            // even multiples of the step, so the top-left origin stays on
            // the grid and every port lands on a grid line.
            let centre = Point::new(grid.to_world(bx.x), grid.to_world(bx.y));
            let origin = Point::new(centre.x - width / 2.0, centre.y - height / 2.0);
            let label = component.label.clone().unwrap_or_else(|| cid.to_string());

            let mut ports = Vec::new();
            let mut port_index = HashMap::new();

            for (side, refs) in port_layout.sides() {
                let n = refs.len();
                for (k, cp) in refs.iter().enumerate() {
                    let pos = port_position(origin, width, height, side, k, n, pitch);
                    let pin = component
                        .lookup(&cp.connector, &cp.port)
                        .and_then(|info| info.pin)
                        .map(str::to_owned);
                    let label = cp.port.to_string();
                    port_index.insert(cp.clone(), ports.len());
                    ports.push(PlacedPort {
                        cp: cp.clone(),
                        side,
                        pos,
                        pin,
                        label,
                    });
                }
            }

            components.insert(
                cid.clone(),
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

        Ok(Self { components })
    }

    pub(super) fn endpoint(&self, port: &PortRef) -> Option<&PlacedPort> {
        let comp = self.components.get(&port.component)?;
        let cp = ConnectorPortRef {
            connector: port.connector.clone(),
            port: port.port.clone(),
        };
        let idx = comp.port_index.get(&cp)?;
        Some(&comp.ports[*idx])
    }

    /// The bounding box of every placed component, in layout order.
    pub(super) fn component_bounds(&self) -> impl Iterator<Item = Bounds> + '_ {
        self.components.values().map(|pc| Bounds {
            origin: pc.origin,
            width: pc.width,
            height: pc.height,
        })
    }

    /// Every placed port across all components, in layout order.
    pub(super) fn ports(&self) -> impl Iterator<Item = &PlacedPort> + '_ {
        self.components.values().flat_map(|pc| pc.ports.iter())
    }

    /// The drawing's bounding box, padded for margins and an optional
    /// title. Encloses both the component boxes and every routed wire —
    /// wires can detour outside the boxes, so they must be measured too.
    pub(super) fn viewbox(&self, has_title: bool, wires: &[Vec<Point>]) -> ViewBox {
        if self.components.is_empty() {
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

        for pc in self.components.values() {
            min_x = min_x.min(pc.origin.x);
            min_y = min_y.min(pc.origin.y);
            max_x = max_x.max(pc.origin.x + pc.width);
            max_y = max_y.max(pc.origin.y + pc.height);
        }

        for p in wires.iter().flatten() {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
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

/// The minimum box size (width, height) in world units that fits this
/// component's ports and label on the given grid. Both dimensions are
/// rounded up to an even number of steps (so the box centres on the grid),
/// and margins/port pitch are the same values [`port_position`] places
/// ports at — so the box always contains its ports.
fn box_dimensions(
    component: &Component,
    cid: &ComponentId,
    layout: &ComponentPortLayout,
    grid: Grid,
) -> (f64, f64) {
    let (margin, pitch) = (grid.margin(), grid.pitch());

    let label = component.label.as_deref().unwrap_or(cid.as_ref());
    let label_w = label.chars().count() as f64 * CHAR_WIDTH + 2.0 * margin;

    // Span a side needs for `n` ports: nothing when empty, otherwise the
    // pitch between ports plus a margin at each end.
    let side_extent = |n: usize| match n {
        0 => 0.0,
        _ => (n - 1) as f64 * pitch + 2.0 * margin,
    };

    let width = MIN_WIDTH
        .max(label_w)
        .max(side_extent(layout.north.len()))
        .max(side_extent(layout.south.len()));

    // North/south labels are drawn vertically (rotated 90°), each reaching
    // in only from the edge it sits on. The box must fit both without them
    // colliding, plus a margin of clearance between them.
    let top_label_h = vertical_label_extent(&layout.north);
    let bot_label_h = vertical_label_extent(&layout.south);
    let vertical_label_h = top_label_h + bot_label_h + margin;

    // No fixed height floor — the box need only fit its ports and any
    // north/south labels, but always at least one port with its margins.
    let height = side_extent(layout.west.len())
        .max(side_extent(layout.east.len()))
        .max(vertical_label_h)
        .max(2.0 * margin);

    (grid.snap_box(width), grid.snap_box(height))
}

/// Resolve a box dimension: an author-supplied size (in grid units) must
/// be at least the derived minimum, otherwise its ports wouldn't fit. An
/// omitted size falls back to that minimum. The result is rounded up to
/// an even number of steps so the box still centres on the grid.
fn resolve_size(
    cid: &ComponentId,
    axis: &'static str,
    given: Option<f64>,
    minimum: f64,
    grid: Grid,
) -> Result<f64> {
    match given {
        None => Ok(minimum),
        Some(units) => {
            let world = grid.to_world(units);
            if world + TOL < minimum {
                return Err(Error::ComponentBoxTooSmall {
                    component: cid.to_string(),
                    axis,
                    given: units,
                    minimum: minimum / grid.step(),
                });
            }
            Ok(grid.snap_box(world))
        }
    }
}

/// Vertical span (in user units) that a rotated label needs from the
/// box edge inward, given the longest port name on the side. Returns
/// 0 when the side has no ports.
fn vertical_label_extent(refs: &[ConnectorPortRef]) -> f64 {
    let max_chars = refs
        .iter()
        .map(|cp| cp.port.as_ref().chars().count())
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

    fn model_with_ports() -> Model {
        r#"
components:
  a:
    label: "A"
    connectors:
      j:
        ports: { p1: "1", p2: "2", p3: "3" }
connections: []
"#
        .parse()
        .expect("model parses")
    }

    #[test]
    fn port_position_centres_ports_about_the_box() {
        let origin = Point::ORIGIN;
        let (h, pitch) = (200.0, 40.0);
        let mid = h / 2.0;

        // Two ports straddle the centreline, half a pitch either side.
        let p0 = port_position(origin, 100.0, h, Side::West, 0, 2, pitch);
        let p1 = port_position(origin, 100.0, h, Side::West, 1, 2, pitch);
        assert_eq!(p0.x, 0.0);
        assert_eq!(p0.y, mid - pitch / 2.0);
        assert_eq!(p1.y, mid + pitch / 2.0);

        // Three ports: the middle one sits on the centreline.
        let q = port_position(origin, 100.0, h, Side::West, 1, 3, pitch);
        assert_eq!(q.y, mid);
    }

    #[test]
    fn ports_land_on_grid_lines() {
        let grid = Grid::new(10.0);
        let model = model_with_ports();
        let view: View = r#"
kind: schematic
grid: 10
layout:
  a: { x: 2, y: 3 }
ports:
  a: { east: [j.p1, j.p2, j.p3] }
"#
        .parse()
        .unwrap();

        let placement = Placement::compute(&model, &view, grid).expect("places");
        for port in placement.ports() {
            assert_eq!(port.pos.x % 10.0, 0.0, "x off-grid: {:?}", port.pos);
            assert_eq!(port.pos.y % 10.0, 0.0, "y off-grid: {:?}", port.pos);
        }
    }

    #[test]
    fn even_port_box_centres_on_the_grid() {
        // The point of the 2-step pitch: an even number of ports still
        // leaves the box an even number of steps, so its centre — the
        // layout coordinate — lands exactly on a grid line, and the ports
        // stay on grid lines too.
        let grid = Grid::new(20.0);
        let model = model_with_ports();
        let view: View = r#"
kind: schematic
grid: 20
layout:
  a: { x: 5, y: 5 }
ports:
  a: { west: [j.p1, j.p2] }
"#
        .parse()
        .unwrap();

        let placement = Placement::compute(&model, &view, grid).expect("places");
        let pc = placement.components.values().next().unwrap();
        // Box centre equals the specified centre (5,5) * step 20.
        assert_eq!(pc.origin.x + pc.width / 2.0, 100.0);
        assert_eq!(pc.origin.y + pc.height / 2.0, 100.0);
        for port in placement.ports() {
            assert_eq!(port.pos.x % 20.0, 0.0);
            assert_eq!(port.pos.y % 20.0, 0.0);
        }
    }

    #[test]
    fn omitted_size_derives_from_port_count() {
        // Grid 40 so three ports clearly exceed the title/text minimum
        // (MIN_HEIGHT snapped up to an even step count).
        let grid = Grid::new(40.0);
        let model = model_with_ports();
        let view: View = r#"
kind: schematic
grid: 40
layout:
  a: { x: 0, y: 0 }
ports:
  a: { west: [j.p1, j.p2, j.p3] }
"#
        .parse()
        .unwrap();

        let placement = Placement::compute(&model, &view, grid).expect("places");
        let pc = placement.components.values().next().unwrap();
        // 3 west ports => 2*pitch + 2*margin = 160 + 160 = 320 (margin is
        // a full pitch, 2 steps = 80 at grid 40).
        assert_eq!(pc.height, 320.0);
    }

    #[test]
    fn explicit_size_below_minimum_errors() {
        let grid = Grid::new(10.0);
        let model = model_with_ports();
        // height: 1 grid unit (10 world) can't hold three west ports.
        let view: View = r#"
kind: schematic
grid: 10
layout:
  a: { x: 0, y: 0, height: 1 }
ports:
  a: { west: [j.p1, j.p2, j.p3] }
"#
        .parse()
        .unwrap();

        assert!(matches!(
            Placement::compute(&model, &view, grid),
            Err(Error::ComponentBoxTooSmall { axis: "height", .. })
        ));
    }
}
