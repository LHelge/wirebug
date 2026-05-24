//! Collapse routes into maximal segments and find shared OVG edges.
//!
//! A route's geometry is an alternating sequence of horizontal and
//! vertical [`Segment`]s. Each segment records the canonical OVG edges it
//! covers, so the per-edge order from §6.1 transfers onto segments in
//! §6.2. A segment is *shared* with another route exactly where they
//! cover a common edge.

use std::collections::HashMap;

use super::super::RawRoute;
use super::super::visibility::Ovg;
use super::EPS;
use crate::view::Point;

/// An OVG edge in canonical (low, high) node-id form.
pub(super) type Edge = (usize, usize);

pub(super) fn canon(u: usize, v: usize) -> Edge {
    if u <= v { (u, v) } else { (v, u) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Orientation {
    Horizontal,
    Vertical,
}

/// A maximal straight run of one route. `perp` is the fixed coordinate
/// (y for horizontal, x for vertical) — the value §6.2 solves for. The
/// run's extent along the other axis is derived on demand from its
/// neighbours' `perp` (`place::spans_of`), never stored, so it can't go
/// stale as the solve moves segments.
#[derive(Debug, Clone)]
pub(super) struct Segment {
    pub(super) orientation: Orientation,
    pub(super) perp: f64,
    pub(super) edges: Vec<Edge>,
    /// Touches a port connection point — its `perp` is pinned there.
    pub(super) is_end: bool,
}

/// Collapse one route's point path into maximal segments. The caller must
/// only pass routes with a non-empty node path (orthogonal throughout);
/// fallback straight routes are handled separately.
pub(super) fn segmentize(ovg: &Ovg, raw: &RawRoute) -> Vec<Segment> {
    let mut pts: Vec<Point> = Vec::with_capacity(raw.nodes.len() + 2);
    pts.push(raw.a);
    pts.extend(raw.nodes.iter().map(|&n| ovg.position(n)));
    pts.push(raw.b);

    let mut segments: Vec<Segment> = Vec::new();
    for i in 0..pts.len() - 1 {
        let (p, q) = (pts[i], pts[i + 1]);
        let (orientation, perp) = if (p.y - q.y).abs() < EPS {
            (Orientation::Horizontal, p.y)
        } else {
            (Orientation::Vertical, p.x)
        };
        // Step `i` runs between node `i-1` and node `i`; the stub steps at
        // either end (i == 0, i == nodes.len()) carry no OVG edge.
        let edge = (i >= 1 && i < raw.nodes.len()).then(|| canon(raw.nodes[i - 1], raw.nodes[i]));

        match segments.last_mut() {
            Some(seg) if seg.orientation == orientation && (seg.perp - perp).abs() < EPS => {
                seg.edges.extend(edge);
            }
            _ => segments.push(Segment {
                orientation,
                perp,
                edges: edge.into_iter().collect(),
                is_end: false,
            }),
        }
    }

    if let Some(first) = segments.first_mut() {
        first.is_end = true;
    }
    if let Some(last) = segments.last_mut() {
        last.is_end = true;
    }
    segments
}

/// Map each OVG edge used by two or more routes to the routes that use it.
pub(super) fn shared_edges(routes: &[Vec<usize>]) -> HashMap<Edge, Vec<usize>> {
    let mut map: HashMap<Edge, Vec<usize>> = HashMap::new();
    for (rid, nodes) in routes.iter().enumerate() {
        for w in nodes.windows(2) {
            let users = map.entry(canon(w[0], w[1])).or_default();
            if users.last() != Some(&rid) {
                users.push(rid);
            }
        }
    }
    map.retain(|_, users| users.len() >= 2);
    map
}

#[cfg(test)]
mod tests {
    use super::super::super::geometry::Rect;
    use super::*;

    /// Build an OVG whose nodes include all the given points, and a raw
    /// route through the points named by `node_pts` (between `a` and `b`).
    fn route_through(a: Point, node_pts: &[Point], b: Point) -> (Ovg, RawRoute) {
        let mut extra = vec![a, b];
        extra.extend_from_slice(node_pts);
        let ovg = Ovg::build(&[] as &[Rect], &extra);
        let nodes = node_pts.iter().map(|&p| ovg.node_at(p).unwrap()).collect();
        (ovg, RawRoute { a, b, nodes })
    }

    #[test]
    fn segmentize_splits_at_each_bend() {
        // a -> (10,0) -> (10,10) -> b : H, V, H.
        let a = Point::new(0.0, 0.0);
        let b = Point::new(20.0, 10.0);
        let (ovg, raw) = route_through(a, &[Point::new(10.0, 0.0), Point::new(10.0, 10.0)], b);
        let segs = segmentize(&ovg, &raw);

        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].orientation, Orientation::Horizontal);
        assert_eq!(segs[0].perp, 0.0);
        assert_eq!(segs[1].orientation, Orientation::Vertical);
        assert_eq!(segs[1].perp, 10.0);
        assert_eq!(segs[2].orientation, Orientation::Horizontal);
        assert_eq!(segs[2].perp, 10.0);
        assert!(segs[0].is_end && segs[2].is_end && !segs[1].is_end);
        // The single node-node step (10,0)->(10,10) is the middle segment.
        assert_eq!(segs[1].edges.len(), 1);
    }

    #[test]
    fn two_routes_sharing_a_channel_report_the_edge() {
        // Both routes traverse the node pair (10,0)-(20,0).
        let mids = [Point::new(10.0, 0.0), Point::new(20.0, 0.0)];
        let (ovg, r0) = route_through(Point::new(0.0, 0.0), &mids, Point::new(30.0, 0.0));
        let n0 = ovg.node_at(mids[0]).unwrap();
        let n1 = ovg.node_at(mids[1]).unwrap();
        let r1 = RawRoute {
            a: Point::new(0.0, 0.0),
            b: Point::new(30.0, 0.0),
            nodes: vec![n0, n1],
        };

        let shared = shared_edges(&[r0.nodes.clone(), r1.nodes.clone()]);
        assert!(shared.contains_key(&canon(n0, n1)));
        assert_eq!(shared[&canon(n0, n1)], vec![0, 1]);
    }

    #[test]
    fn unshared_edges_are_absent() {
        let mids = [Point::new(10.0, 0.0), Point::new(20.0, 0.0)];
        let (_, r0) = route_through(Point::new(0.0, 0.0), &mids, Point::new(30.0, 0.0));
        let shared = shared_edges(std::slice::from_ref(&r0.nodes));
        assert!(shared.is_empty());
    }
}
