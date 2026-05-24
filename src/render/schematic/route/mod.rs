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
//!
//! The nudging stage (§6, separating wires that share a channel) is not
//! implemented yet; parallel wires may overlap.

mod astar;
mod geometry;
mod visibility;

use geometry::{Dir, Rect};
use visibility::Ovg;

use super::layout::{PlacedPort, Placement};
use crate::view::Point;

/// How far routes stay clear of component boxes. Also the length of the
/// stub segment by which a wire leaves its port before joining the graph.
const CLEARANCE: f64 = 16.0;
/// World units → fixed-point cost. Keeps A\* costs integral and `Ord`.
const SCALE: f64 = 100.0;
/// Cost of one bend, in scaled units. Chosen far larger than any
/// plausible path length so fewer bends always wins, with length as the
/// tie-break (matching the paper's bends-take-precedence ordering).
const BEND_PENALTY: i64 = 100_000_000;

/// A routing engine for one rendered diagram. Build once, then route
/// every connection against the shared visibility graph.
pub(super) struct Router {
    ovg: Ovg,
}

impl Router {
    pub(super) fn build(placement: &Placement) -> Self {
        let obstacles: Vec<Rect> = placement
            .components
            .values()
            .map(|pc| Rect::new(pc.origin.x, pc.origin.y, pc.width, pc.height).inflated(CLEARANCE))
            .collect();

        // Each port contributes its connection point and its stub (one
        // clearance out along the normal) as interesting points, so the
        // stub always lands exactly on a grid node.
        let mut extra = Vec::new();
        for pc in placement.components.values() {
            for port in &pc.ports {
                extra.push(port.pos);
                extra.push(stub(port));
            }
        }

        Self {
            ovg: Ovg::build(&obstacles, &extra),
        }
    }

    /// Route a single connection. Falls back to a direct segment if the
    /// graph offers no route (should not happen — the outer region is
    /// always open — but never drop a connection).
    pub(super) fn route(&self, a: &PlacedPort, b: &PlacedPort) -> Vec<Point> {
        let out_a = Dir::from(a.side);
        let in_b = Dir::from(b.side).opposite();

        let routed = match (self.ovg.node_at(stub(a)), self.ovg.node_at(stub(b))) {
            (Some(stub_a), Some(stub_b)) => {
                astar::find_route(&self.ovg, a.pos, out_a, stub_a, b.pos, in_b, stub_b)
            }
            _ => None,
        };

        routed.unwrap_or_else(|| vec![a.pos, b.pos])
    }
}

/// The point one [`CLEARANCE`] outward from a port along its normal.
fn stub(p: &PlacedPort) -> Point {
    let (dx, dy) = Dir::from(p.side).unit();
    Point::new(p.pos.x + dx * CLEARANCE, p.pos.y + dy * CLEARANCE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Model;
    use crate::view::View;

    /// Regression for the whole point of this module: a connection whose
    /// endpoints share a y would, under the old `manhattan_route`, run
    /// straight through the box sitting between them. The routed path
    /// must detour around it.
    #[test]
    fn route_detours_around_an_intervening_component() {
        let model: Model = r#"
components:
  a:
    connectors: { j: { ports: { out: "1" } } }
  b:
    connectors: { j: { ports: { p: "1" } } }
  c:
    connectors: { j: { ports: { in: "1" } } }
connections:
  - { from: a.j.out, to: c.j.in }
"#
        .parse()
        .unwrap();
        // `a` and `c` face each other across `b`, which is placed dead
        // centre between them.
        let view: View = r#"
kind: schematic
layout:
  a: { x: 0, y: 0 }
  b: { x: 200, y: 0 }
  c: { x: 400, y: 0 }
ports:
  a: { east: [j.out] }
  c: { west: [j.in] }
"#
        .parse()
        .unwrap();

        let placement = Placement::compute(&model, &view);
        let router = Router::build(&placement);
        let conn = &model.connections[0];
        let a = placement.endpoint(&conn.from).expect("a placed");
        let c = placement.endpoint(&conn.to).expect("c placed");

        let path = router.route(a, c);

        // `b`'s drawn box (un-inflated). The route must clear it.
        let b = Rect::new(200.0, 0.0, 160.0, 100.0);
        assert!(path.len() > 2, "expected a detour, got {path:?}");
        assert!(
            !path.windows(2).any(|w| b.blocks_segment(w[0], w[1])),
            "route crosses component b: {path:?}"
        );
        assert_eq!(*path.first().unwrap(), a.pos);
        assert_eq!(*path.last().unwrap(), c.pos);
    }
}
