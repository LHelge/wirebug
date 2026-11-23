//! Object-avoiding orthogonal connector routing.
//!
//! Implements the first two stages of the algorithm behind libavoid:
//!
//! > Michael Wybrow, Kim Marriott, and Peter J. Stuckey.
//! > "Orthogonal Connector Routing." In *Graph Drawing (GD 2009)*,
//! > LNCS 5849, pp. 219–231. Springer, 2010.
//! > <https://people.eng.unimelb.edu.au/pstuckey/papers/gd09.pdf>
//!
//! 1. [`visibility`] builds the orthogonal visibility graph of §4 — a
//!    non-uniform grid tailored to the obstacle geometry.
//! 2. [`astar`] finds a minimal-cost route (bends first, then length)
//!    through that graph for each connection, per §5.
//! 3. [`nudge`] separates wires that share a channel (§6): ordering them
//!    so a bundle fans out without crossing itself, then placing every
//!    segment via a separation-constraint solver.
//!
//! Routing is therefore done as a batch ([`Router::route_all`]) so the
//! nudging pass can see every connector at once.

mod astar;
mod geometry;
mod nudge;
mod visibility;

use geometry::{Dir, Rect};
use pathfinding::prelude::bfs;
use visibility::Ovg;

use super::layout::{PlacedPort, Placement};
use crate::render::geometry::Point;

const EPS: f64 = 1e-6;
/// World units → fixed-point cost. Keeps A\* costs integral and `Ord`.
const SCALE: f64 = 100.0;
/// Cost of one bend, in scaled units. Chosen far larger than any
/// plausible path length so fewer bends always wins, with length as the
/// tie-break (matching the paper's bends-take-precedence ordering).
const BEND_PENALTY: i64 = 100_000_000;

/// One connector's route through the visibility graph, before nudging:
/// the two port connection points plus the interior OVG node-id path
/// (`start_stub` … `goal_stub`).
pub(super) struct RawRoute {
    pub(super) a: Point,
    pub(super) b: Point,
    pub(super) nodes: Vec<usize>,
}

/// A routing engine for one rendered diagram. Build once, then route
/// every connection against the shared visibility graph.
pub(super) struct Router {
    ovg: Ovg,
    /// The keepouts: each part of a component's footprint (its box with
    /// its title row), inflated by the clearance.
    obstacles: Vec<Rect>,
    /// The component each of `obstacles` belongs to, by its index in
    /// include order.
    owners: Vec<usize>,
    soft_avoid: Vec<Vec<Point>>,
    /// How far routes stay clear of component boxes, and the length of
    /// the stub a wire leaves its port by — one grid step.
    clearance: f64,
}

impl Router {
    pub(super) fn build(placement: &Placement, clearance: f64) -> Self {
        let (owners, obstacles): (Vec<usize>, Vec<Rect>) = placement
            .obstacles()
            .map(|(owner, part)| (owner, Rect::from(part).inflated(clearance)))
            .unzip();

        // Each port contributes its connection point and its stub (one
        // clearance out along the normal) as interesting points, so the
        // stub always lands exactly on a grid node.
        let mut extra = Vec::new();
        for port in placement.ports() {
            extra.push(port.pos);
            extra.push(port.stub(clearance));
        }

        let soft_avoid = placement
            .enclosure_bounds()
            .map(|b| outline(Rect::from(b)))
            .into_iter()
            .collect();

        Self {
            ovg: Ovg::build(&obstacles, &extra),
            obstacles,
            owners,
            soft_avoid,
            clearance,
        }
    }

    /// Every wired child port of `placement` whose stub falls inside
    /// another component's keepout — the router's own precondition for
    /// attaching a port, read from the obstacles [`Self::route_all`] routes
    /// around. Enclosure ports are not checked: the enclosure stands four
    /// grid steps off the child boxes, so their inward stubs always clear
    /// every keepout.
    pub(super) fn blocked_stubs<'p>(&self, placement: &'p Placement) -> Vec<BlockedStub<'p>> {
        let mut blocked = Vec::new();
        for (owner, (name, component)) in placement.components.iter().enumerate() {
            for port in &component.ports {
                if !placement.is_wired(name, &port.port) {
                    continue;
                }
                let at = port.stub(self.clearance);
                let swallowed = self
                    .owners
                    .iter()
                    .zip(&self.obstacles)
                    .find(|&(&i, keepout)| i != owner && keepout.contains_point(at));
                if let Some((&blocker, _)) = swallowed {
                    blocked.push(BlockedStub {
                        owner,
                        port,
                        blocker,
                    });
                }
            }
        }
        blocked
    }

    /// Whether [`Self::route_all`] finds a route between `a` and `b`: both
    /// stubs have a graph node and the graph connects them. The search
    /// only weighs bends, crossings and guide overlaps, never forbids an
    /// edge, so it fails exactly when the stubs are disconnected — walled
    /// in by keepouts that meet.
    pub(super) fn connects(&self, a: &PlacedPort, b: &PlacedPort) -> bool {
        let (Some(from), Some(to)) = (
            self.ovg.node_at(a.stub(self.clearance)),
            self.ovg.node_at(b.stub(self.clearance)),
        ) else {
            return false;
        };
        bfs(
            &from,
            |&node| self.ovg.neighbours(node).iter().map(|&(next, _)| next),
            |&node| node == to,
        )
        .is_some()
    }

    /// Route a single connection through the graph, or `None` when it has
    /// no route.
    fn route_one_avoiding(
        &self,
        a: &PlacedPort,
        b: &PlacedPort,
        avoid: &[Vec<Point>],
    ) -> Option<RawRoute> {
        let stub_a = self.ovg.node_at(a.stub(self.clearance))?;
        let stub_b = self.ovg.node_at(b.stub(self.clearance))?;
        let nodes = astar::find_route_avoiding(
            &self.ovg,
            a.pos,
            out_dir(a),
            stub_a,
            b.pos,
            out_dir(b).opposite(),
            stub_b,
            avoid,
            &self.soft_avoid,
        )?;

        Some(RawRoute {
            a: a.pos,
            b: b.pos,
            nodes,
        })
    }

    /// Route every connection, then nudge shared channels apart (paper
    /// §6). Returns one polyline per input pair, in order, and the pairs
    /// left on one line for lack of room ([`Routed`]). `gap` is the
    /// minimum spacing between parallel wires in a shared channel — the
    /// port pitch (two grid steps), so a nudged bundle stays grid-aligned
    /// and matches the spacing of the ports it fans out from.
    ///
    /// Fails on the first pair with no route, naming it by its index in
    /// `pairs`: the router sees only placed ports, so the caller, which
    /// knows the wire each pair came from, names its ends.
    pub(super) fn route_all(
        &self,
        pairs: &[(&PlacedPort, &PlacedPort)],
        gap: f64,
    ) -> Result<Routed, Unrouted> {
        let mut raws = Vec::with_capacity(pairs.len());
        let mut routed = Vec::with_capacity(pairs.len());
        for (pair, &(a, b)) in pairs.iter().enumerate() {
            let raw = self
                .route_one_avoiding(a, b, &routed)
                .ok_or(Unrouted { pair })?;
            routed.push(self.raw_points(&raw));
            raws.push(raw);
        }
        Ok(nudge::run(&self.ovg, &self.obstacles, &raws, gap))
    }

    fn raw_points(&self, raw: &RawRoute) -> Vec<Point> {
        let mut pts = Vec::with_capacity(raw.nodes.len() + 2);
        pts.push(raw.a);
        pts.extend(raw.nodes.iter().map(|&n| self.ovg.position(n)));
        pts.push(raw.b);
        pts
    }
}

/// What [`Router::route_all`] routes: one polyline per input pair, in
/// order, and the pairs (lower index first) a channel had no room to hold
/// apart, which run on one line there.
pub(super) struct Routed {
    pub(super) wires: Vec<Vec<Point>>,
    pub(super) together: Vec<(usize, usize)>,
}

/// The connection [`Router::route_all`] found no route for.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Unrouted {
    /// Index of the connection in the pairs passed to `route_all`.
    pub(super) pair: usize,
}

/// A wired port whose wire cannot leave it: its stub lies strictly inside
/// another component's keepout, so the visibility graph has no node there
/// and [`Router::route_all`] would fail with [`Unrouted`].
pub(super) struct BlockedStub<'p> {
    /// Index of the port's own component, in `component_bounds` order.
    pub(super) owner: usize,
    pub(super) port: &'p PlacedPort,
    /// Index of the component whose keepout swallows the stub.
    pub(super) blocker: usize,
}

fn outline(r: Rect) -> Vec<Point> {
    vec![
        Point::new(r.x, r.y),
        Point::new(r.x + r.w, r.y),
        Point::new(r.x + r.w, r.y + r.h),
        Point::new(r.x, r.y + r.h),
        Point::new(r.x, r.y),
    ]
}

/// The direction a wire leaves a port: outward from its box, or inward for an
/// inverted enclosure port (which faces the schematic interior).
fn out_dir(p: &PlacedPort) -> Dir {
    let dir = Dir::from(p.side);
    if p.inverted { dir.opposite() } else { dir }
}

/// Drop coincident points and merge consecutive collinear segments so
/// the polyline strictly alternates horizontal and vertical runs.
pub(super) fn collapse_collinear(pts: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(pts.len());
    for p in pts {
        match out.last() {
            Some(last) if (last.x - p.x).abs() < EPS && (last.y - p.y).abs() < EPS => {}
            _ => out.push(p),
        }
    }
    if out.len() <= 2 {
        return out;
    }

    let mut merged = Vec::with_capacity(out.len());
    merged.push(out[0]);
    for window in out.windows(3) {
        let a = merged[merged.len() - 1];
        let b = window[1];
        let c = window[2];
        let collinear = ((a.y - b.y).abs() < EPS && (b.y - c.y).abs() < EPS)
            || ((a.x - b.x).abs() < EPS && (b.x - c.x).abs() < EPS);
        if !collinear {
            merged.push(b);
        }
    }
    merged.push(out[out.len() - 1]);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::geometry::Side;
    use crate::render::schematic::layout::Grid;
    use crate::render::schematic::tests::{design_from, view_of};

    #[test]
    fn inverted_port_stubs_inward() {
        use crate::dsl::ir::PortName;
        // A normal west port stubs west (x decreases); an inverted boundary
        // port faces the interior, so its stub goes east.
        let port = PlacedPort {
            port: PortName::from("x"),
            side: Side::West,
            pos: Point::new(100.0, 50.0),
            pin: None,
            label: "X".to_string(),
            inverted: true,
            off_view: None,
            lead: 0.0,
        };
        let s = port.stub(10.0);
        assert_eq!(s.x, 110.0);
        assert_eq!(s.y, 50.0);
    }

    /// Regression for the whole point of this module: a connection whose
    /// endpoints share a y would, under the old `manhattan_route`, run
    /// straight through the box sitting between them. The routed path
    /// must detour around it.
    #[test]
    fn route_detours_around_an_intervening_component() {
        // `a` and `c` face each other across `b` — unwired, so it shows no
        // ports, but is still a box routes must avoid — whose centre sits
        // dead between them. `b` is the second include.
        let design = design_from(
            r#"
component sys {
    a: node;
    b: node;
    c: node;
    wire red 1 [a.p, c.p];
    component node {
        pub port p "P";
    }
}
"#,
        );
        // grid 1 keeps grid units equal to world units; `compute` bypasses
        // the renderer's grid-floor check.
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, &[("p", Side::East)]),
                // `b` has no listed ports — a bare box routes must avoid.
                ("b", 200.0, 0.0, &[]),
                ("c", 400.0, 0.0, &[("p", Side::West)]),
            ],
        );
        let step = 1.0;

        let subject = design.get(&design.root).unwrap();
        let placement =
            Placement::compute(&design, subject, &view, Grid::new(step)).expect("places");
        let router = Router::build(&placement, step);
        let connections = placement.connections();
        let (a, c) = (connections[0].a, connections[0].b);

        // `b`'s drawn box (un-inflated) — taken from the placement, since
        // its world position depends on the centre-based layout.
        let b = Rect::from(placement.components[1].bounds());

        let path = router
            .route_all(&[(a, c)], step)
            .expect("routes")
            .wires
            .remove(0);

        assert!(path.len() > 2, "expected a detour, got {path:?}");
        assert!(
            !path.windows(2).any(|w| b.blocks_segment(w[0], w[1])),
            "route crosses component b: {path:?}"
        );
        assert_eq!(*path.first().unwrap(), a.pos);
        assert_eq!(*path.last().unwrap(), c.pos);
    }

    #[test]
    fn route_fails_when_a_stub_is_inside_an_obstacle() {
        use crate::dsl::ir::PortName;

        let obstacle = Rect::new(0.0, 0.0, 100.0, 100.0);
        // `b` and `c` (below) sit clear of the obstacle, their stubs on
        // graph nodes; `a`'s stub lies inside it.
        let clear = [(150.0, 50.0), (140.0, 50.0), (150.0, 80.0), (140.0, 80.0)]
            .map(|(x, y)| Point::new(x, y));
        let router = Router {
            ovg: Ovg::build(&[obstacle], &clear),
            obstacles: vec![obstacle],
            owners: vec![0],
            soft_avoid: Vec::new(),
            clearance: 10.0,
        };
        let a = PlacedPort {
            port: PortName::from("a"),
            side: Side::East,
            pos: Point::new(50.0, 50.0),
            pin: None,
            label: "A".to_string(),
            inverted: false,
            off_view: None,
            lead: 0.0,
        };
        let b = PlacedPort {
            port: PortName::from("b"),
            side: Side::West,
            pos: Point::new(150.0, 50.0),
            pin: None,
            label: "B".to_string(),
            inverted: false,
            off_view: None,
            lead: 0.0,
        };

        assert!(router.route_one_avoiding(&a, &b, &[]).is_none());
        // `route_all` names the failing pair by its position.
        let c = PlacedPort {
            port: PortName::from("c"),
            side: Side::West,
            pos: Point::new(150.0, 80.0),
            pin: None,
            label: "C".to_string(),
            inverted: false,
            off_view: None,
            lead: 0.0,
        };
        assert_eq!(
            router.route_all(&[(&b, &c), (&a, &b)], 10.0).err(),
            Some(Unrouted { pair: 1 })
        );
    }

    #[test]
    fn route_all_penalizes_crossings_with_previous_routes() {
        use crate::dsl::ir::PortName;

        let mk_port = |name: &str, side, x, y| PlacedPort {
            port: PortName::from(name),
            side,
            pos: Point::new(x, y),
            pin: None,
            label: name.to_string(),
            inverted: false,
            off_view: None,
            lead: 0.0,
        };
        let ports = [
            mk_port("a", Side::East, 0.0, 50.0),
            mk_port("b", Side::West, 30.0, 50.0),
            mk_port("c", Side::East, 0.0, 0.0),
            mk_port("d", Side::West, 100.0, 100.0),
        ];
        let extra = [
            Point::new(0.0, 50.0),
            Point::new(10.0, 50.0),
            Point::new(20.0, 50.0),
            Point::new(30.0, 50.0),
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(90.0, 100.0),
            Point::new(100.0, 100.0),
        ];
        let router = Router {
            ovg: Ovg::build(&[], &extra),
            obstacles: Vec::new(),
            owners: Vec::new(),
            soft_avoid: Vec::new(),
            clearance: 10.0,
        };

        let wires = router
            .route_all(&[(&ports[0], &ports[1]), (&ports[2], &ports[3])], 10.0)
            .expect("routes")
            .wires;

        assert_eq!(
            wires[0],
            vec![Point::new(0.0, 50.0), Point::new(30.0, 50.0)]
        );
        assert!(
            wires[1].iter().any(|p| (p.x - 90.0).abs() < EPS),
            "second route should take the non-crossing dogleg: {:?}",
            wires[1]
        );
    }

    /// Paths of a wire that must go round a box to reach a port on its
    /// far side: two mirror-image routes of equal cost.
    fn route_round_the_far_box(vertical: bool) -> Vec<Point> {
        let design = design_from(
            r#"
component sys {
    near: node;
    far: node;
    wire red 1 [near.p, far.p];
    component node {
        pub port p "P";
    }
}
"#,
        );
        let (side, at) = if vertical {
            (Side::South, (0.0, 8.0))
        } else {
            (Side::East, (12.0, 0.0))
        };
        let view = view_of(
            "sys",
            &[
                ("near", 0.0, 0.0, &[("p", side)]),
                ("far", at.0, at.1, &[("p", side)]),
            ],
        );
        let step = 20.0;
        let subject = design.get(&design.root).unwrap();
        let placement =
            Placement::compute(&design, subject, &view, Grid::new(step)).expect("places");
        let router = Router::build(&placement, step);
        let connections = placement.connections();
        let pair = (connections[0].a, connections[0].b);
        router
            .route_all(&[pair], 2.0 * step)
            .expect("routes")
            .wires
            .remove(0)
    }

    /// A rail and a wire that both have to pass between two boxes two
    /// steps apart: one line clear of both, which the two share. Nudging
    /// them the port pitch apart used to put one inside a box.
    #[test]
    fn wires_sharing_a_gap_with_room_for_one_stay_out_of_the_boxes() {
        let design = design_from(
            r#"
component sys {
    a: node;
    b: node;
    c: node;
    wire red 1 [a.p, b.p, c.p];
    wire black 1 [a.q, b.q];
    component node {
        pub port p "P";
        pub port q "Q";
    }
}
"#,
        );
        const SIDES: &[(&str, Side)] = &[("p", Side::West), ("q", Side::East)];
        let view = view_of(
            "sys",
            &[
                ("a", 0.0, 0.0, SIDES),
                ("b", 10.0, 0.0, SIDES),
                ("c", 20.0, 0.0, SIDES),
            ],
        );
        let grid = Grid::new(20.0);
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, grid).expect("places");
        let router = Router::build(&placement, grid.step());
        let connections = placement.connections();
        let pairs: Vec<_> = connections.iter().map(|c| (c.a, c.b)).collect();

        let routed = router.route_all(&pairs, grid.pitch()).expect("routes");

        let boxes: Vec<Rect> = placement
            .components
            .values()
            .map(|pc| Rect::from(pc.bounds()))
            .collect();
        for wire in &routed.wires {
            assert!(
                !wire
                    .windows(2)
                    .any(|w| boxes.iter().any(|b| b.blocks_segment(w[0], w[1]))),
                "a wire runs through a box: {wire:?}"
            );
        }
        // Both runs of the rail and the wire share the line between `a`
        // and `b`: every pair of them is named as left together.
        assert_eq!(routed.together, [(0, 1), (0, 2), (1, 2)]);
    }

    /// The tie between two equally good routes is settled by geometry, not
    /// by the order a hash map happened to hand the graph over in — which
    /// differs between every map built, in one process as across processes.
    #[test]
    fn a_tie_between_mirror_routes_is_settled_the_same_way_every_time() {
        for vertical in [true, false] {
            let first = route_round_the_far_box(vertical);
            for _ in 0..32 {
                assert_eq!(route_round_the_far_box(vertical), first);
            }
        }
    }

    #[test]
    fn a_tied_wire_goes_round_the_box_on_the_east() {
        let stacked = route_round_the_far_box(true);
        assert!(stacked.iter().any(|p| p.x > 0.0), "{stacked:?}");
        assert!(stacked.iter().all(|p| p.x >= 0.0), "{stacked:?}");
    }

    /// Over a box a wire has the title row to clear as well, so the way
    /// under it is the shorter one: no tie.
    #[test]
    fn a_wire_goes_under_a_box_rather_than_over_its_title() {
        let side_by_side = route_round_the_far_box(false);
        assert!(side_by_side.iter().any(|p| p.y > 0.0), "{side_by_side:?}");
        assert!(side_by_side.iter().all(|p| p.y >= 0.0), "{side_by_side:?}");
    }

    #[test]
    fn a_north_wire_bends_above_its_box_s_title_row() {
        // `near.p` leaves north and has to come back down to `far.p`, also
        // on a north side: the run between them lies a step above both
        // title rows, never in one.
        let design = design_from(
            r#"
component sys {
    near: node;
    far: node;
    wire red 1 [near.p, far.p];
    component node {
        pub port p "P";
    }
}
"#,
        );
        const NORTH: &[(&str, Side)] = &[("p", Side::North)];
        let view = view_of(
            "sys",
            &[("near", 0.0, 0.0, NORTH), ("far", 12.0, 0.0, NORTH)],
        );
        let grid = Grid::new(20.0);
        let subject = design.get(&design.root).unwrap();
        let placement = Placement::compute(&design, subject, &view, grid).expect("places");
        let router = Router::build(&placement, grid.step());
        let connections = placement.connections();
        let pair = (connections[0].a, connections[0].b);

        let wire = router
            .route_all(&[pair], grid.pitch())
            .expect("routes")
            .wires
            .remove(0);

        let top = placement.components[0].origin.y;
        let row = placement.components[0].title_row;
        assert_eq!(row.height, 20.0, "one step holds the title on this grid");
        let run: Vec<f64> = wire[1..wire.len() - 1].iter().map(|p| p.y).collect();
        assert_eq!(run, [top - 40.0, top - 40.0], "{wire:?}");
    }
}
