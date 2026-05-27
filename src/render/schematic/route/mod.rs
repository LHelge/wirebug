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
/// (`start_stub` … `goal_stub`). An empty `nodes` means the fallback
/// straight segment.
pub(super) struct RawRoute {
    pub(super) a: Point,
    pub(super) b: Point,
    pub(super) nodes: Vec<usize>,
}

/// A routing engine for one rendered diagram. Build once, then route
/// every connection against the shared visibility graph.
pub(super) struct Router {
    ovg: Ovg,
    obstacles: Vec<Rect>,
    /// How far routes stay clear of component boxes, and the length of
    /// the stub a wire leaves its port by — one grid step.
    clearance: f64,
}

impl Router {
    pub(super) fn build(placement: &Placement, clearance: f64) -> Self {
        let obstacles: Vec<Rect> = placement
            .component_bounds()
            .map(|b| Rect::from(b).inflated(clearance))
            .collect();

        // Each port contributes its connection point and its stub (one
        // clearance out along the normal) as interesting points, so the
        // stub always lands exactly on a grid node.
        let mut extra = Vec::new();
        for port in placement.ports() {
            extra.push(port.pos);
            extra.push(stub(port, clearance));
        }

        Self {
            ovg: Ovg::build(&obstacles, &extra),
            obstacles,
            clearance,
        }
    }

    /// Route a single connection through the graph. Falls back to an
    /// empty node path (drawn as a straight segment) if the graph offers
    /// no route — never drops a connection.
    pub(super) fn route_one(&self, a: &PlacedPort, b: &PlacedPort) -> RawRoute {
        let out_a = out_dir(a);
        let in_b = out_dir(b).opposite();

        let nodes = match (
            self.ovg.node_at(stub(a, self.clearance)),
            self.ovg.node_at(stub(b, self.clearance)),
        ) {
            (Some(stub_a), Some(stub_b)) => {
                astar::find_route(&self.ovg, a.pos, out_a, stub_a, b.pos, in_b, stub_b)
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };

        RawRoute {
            a: a.pos,
            b: b.pos,
            nodes,
        }
    }

    /// Route every connection, then nudge shared channels apart (paper
    /// §6). Returns one polyline per input pair, in order. `gap` is the
    /// minimum spacing between parallel wires in a shared channel — the
    /// view's grid step, so a nudged bundle stays grid-aligned.
    pub(super) fn route_all(
        &self,
        pairs: &[(&PlacedPort, &PlacedPort)],
        gap: f64,
    ) -> Vec<Vec<Point>> {
        let raws: Vec<RawRoute> = pairs.iter().map(|(a, b)| self.route_one(a, b)).collect();
        nudge::run(&self.ovg, &self.obstacles, &raws, gap)
    }
}

/// The direction a wire leaves a port: outward from its box, or inward for an
/// inverted enclosure port (which faces the schematic interior).
fn out_dir(p: &PlacedPort) -> Dir {
    let dir = Dir::from(p.side);
    if p.inverted { dir.opposite() } else { dir }
}

/// The point one `clearance` along a port's leaving direction — its stub,
/// which always lands on a grid node.
fn stub(p: &PlacedPort, clearance: f64) -> Point {
    let (dx, dy) = out_dir(p).unit();
    Point::new(p.pos.x + dx * clearance, p.pos.y + dy * clearance)
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

    let mut merged = vec![out[0]];
    for i in 1..out.len() - 1 {
        let a = *merged.last().unwrap();
        let b = out[i];
        let c = out[i + 1];
        let collinear = ((a.y - b.y).abs() < EPS && (b.y - c.y).abs() < EPS)
            || ((a.x - b.x).abs() < EPS && (b.x - c.x).abs() < EPS);
        if !collinear {
            merged.push(b);
        }
    }
    merged.push(*out.last().unwrap());
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
        };
        let s = stub(&port, 10.0);
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
    node a;
    node b;
    node c;
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
        let pairs = placement.connection_pairs();
        let (a, c) = pairs[0];

        // `b`'s drawn box (un-inflated) — taken from the placement, since
        // its world position depends on the centre-based layout.
        let b = Rect::from(placement.component_bounds().nth(1).expect("b placed"));

        let path = router.route_all(&[(a, c)], step).remove(0);

        assert!(path.len() > 2, "expected a detour, got {path:?}");
        assert!(
            !path.windows(2).any(|w| b.blocks_segment(w[0], w[1])),
            "route crosses component b: {path:?}"
        );
        assert_eq!(*path.first().unwrap(), a.pos);
        assert_eq!(*path.last().unwrap(), c.pos);
    }
}
