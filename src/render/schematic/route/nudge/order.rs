//! §6.1 — ordering connectors within shared channels.
//!
//! All shared edges that lie on one grid line form a *channel*. The
//! routes through a channel are ordered along it so that, when §6.2
//! nudges them apart, crossings are minimal.
//!
//! Scope note: the paper orders an arbitrary tree of shared edges via
//! pseudo-direction + split points. The orthogonal routes this renderer
//! produces share *collinear* channels (straight bundles), for which the
//! crossing-minimal order is derived from where each route enters the
//! channel, then oriented by the shared travel direction. A connector
//! entering earlier along an eastbound/northbound channel is stacked at
//! the smaller perpendicular coordinate; westbound/southbound channels
//! reverse that stack so the bundle stays on the same side of travel.

use std::collections::HashSet;

use super::super::geometry::Dir;
use super::super::visibility::Ovg;
use super::EPS;
use super::segments::{Edge, Orientation, canon, shared_edges};

/// A maximal set of collinear shared edges, with the routes that run
/// along it ordered for minimal crossings.
pub(super) struct Channel {
    pub(super) orientation: Orientation,
    /// Route ids in stacking order along the channel.
    pub(super) order: Vec<usize>,
    pub(super) edges: HashSet<Edge>,
}

/// Group every shared edge into its channel and order the routes within.
pub(super) fn channels(ovg: &Ovg, routes: &[Vec<usize>]) -> Vec<Channel> {
    let shared = shared_edges(routes);

    // Bucket shared edges by the grid line they lie on.
    let mut buckets: Vec<Channel> = Vec::new();
    let mut index: std::collections::HashMap<(Orientation, i64), usize> = Default::default();
    for (&edge, users) in &shared {
        let (orientation, perp) = edge_line(ovg, edge);
        let key = (orientation, (perp / EPS).round() as i64);
        let ci = *index.entry(key).or_insert_with(|| {
            buckets.push(Channel {
                orientation,
                order: Vec::new(),
                edges: HashSet::new(),
            });
            buckets.len() - 1
        });
        buckets[ci].edges.insert(edge);
        for &r in users {
            if !buckets[ci].order.contains(&r) {
                buckets[ci].order.push(r);
            }
        }
    }

    for ch in &mut buckets {
        let orientation = ch.orientation;
        let edges = &ch.edges;
        ch.order.sort_by(|&x, &y| {
            entry_key(ovg, &routes[x], edges, orientation)
                .partial_cmp(&entry_key(ovg, &routes[y], edges, orientation))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(x.cmp(&y))
        });
        if channel_direction(ovg, routes, &ch.order, edges, orientation)
            .is_some_and(reverses_stack_order)
        {
            ch.order.reverse();
        }
    }

    buckets
}

/// Orientation and perpendicular coordinate of a single edge's line.
fn edge_line(ovg: &Ovg, edge: Edge) -> (Orientation, f64) {
    let (p, q) = (ovg.position(edge.0), ovg.position(edge.1));
    if (p.y - q.y).abs() < EPS {
        (Orientation::Horizontal, p.y)
    } else {
        (Orientation::Vertical, p.x)
    }
}

/// Where a route enters the channel: the smallest coordinate, along the
/// channel, of any node it shares with the channel. Routes that enter
/// further along are stacked later, keeping a bundle crossing-free.
fn entry_key(ovg: &Ovg, nodes: &[usize], edges: &HashSet<Edge>, orientation: Orientation) -> f64 {
    let mut best = f64::INFINITY;
    for w in nodes.windows(2) {
        if edges.contains(&canon(w[0], w[1])) {
            for &n in w {
                let p = ovg.position(n);
                let par = match orientation {
                    Orientation::Horizontal => p.x,
                    Orientation::Vertical => p.y,
                };
                best = best.min(par);
            }
        }
    }
    best
}

fn channel_direction(
    ovg: &Ovg,
    routes: &[Vec<usize>],
    route_ids: &[usize],
    edges: &HashSet<Edge>,
    orientation: Orientation,
) -> Option<Dir> {
    let mut dir = None;
    for &route_id in route_ids {
        let nodes = &routes[route_id];
        let Some(next) = route_direction_in_channel(ovg, nodes, edges, orientation) else {
            continue;
        };
        if dir.is_some_and(|dir| dir != next) {
            return None;
        }
        dir = Some(next);
    }
    dir
}

fn route_direction_in_channel(
    ovg: &Ovg,
    nodes: &[usize],
    edges: &HashSet<Edge>,
    orientation: Orientation,
) -> Option<Dir> {
    for w in nodes.windows(2) {
        if !edges.contains(&canon(w[0], w[1])) {
            continue;
        }
        let (a, b) = (ovg.position(w[0]), ovg.position(w[1]));
        return Some(match orientation {
            Orientation::Horizontal if b.x > a.x => Dir::East,
            Orientation::Horizontal => Dir::West,
            Orientation::Vertical if b.y > a.y => Dir::South,
            Orientation::Vertical => Dir::North,
        });
    }
    None
}

fn reverses_stack_order(dir: Dir) -> bool {
    matches!(dir, Dir::South | Dir::West)
}

#[cfg(test)]
mod tests {
    use super::super::super::geometry::Rect;
    use super::*;
    use crate::render::geometry::Point;

    #[test]
    fn collinear_shared_edges_form_one_channel_ordered_by_entry() {
        // Grid line y = 0 with nodes at x = 0,10,20,30.
        let pts = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(20.0, 0.0),
            Point::new(30.0, 0.0),
        ];
        let ovg = Ovg::build(&[] as &[Rect], &pts);
        let id = |p: Point| ovg.node_at(p).unwrap();
        let (n0, n1, n2, n3) = (id(pts[0]), id(pts[1]), id(pts[2]), id(pts[3]));

        // Route 1 enters the channel further right (at x=10) than route 0
        // (at x=0); both run along y=0, sharing edge (10,20).
        let routes = vec![vec![n0, n1, n2], vec![n1, n2, n3]];
        let chans = channels(&ovg, &routes);

        assert_eq!(chans.len(), 1);
        assert_eq!(chans[0].orientation, Orientation::Horizontal);
        assert_eq!(chans[0].order, vec![0, 1]);
    }

    #[test]
    fn southbound_vertical_channel_reverses_entry_order() {
        // The top route enters the shared vertical run first, but for a
        // southbound channel it must be nudged to the right of the lower
        // route so the two doglegs do not cross.
        let pts = [
            Point::new(0.0, 0.0),
            Point::new(0.0, 10.0),
            Point::new(0.0, 20.0),
            Point::new(0.0, 30.0),
        ];
        let ovg = Ovg::build(&[] as &[Rect], &pts);
        let id = |p: Point| ovg.node_at(p).unwrap();
        let (n0, n1, n2, n3) = (id(pts[0]), id(pts[1]), id(pts[2]), id(pts[3]));

        let routes = vec![vec![n0, n1, n2], vec![n1, n2, n3]];
        let chans = channels(&ovg, &routes);

        assert_eq!(chans.len(), 1);
        assert_eq!(chans[0].orientation, Orientation::Vertical);
        assert_eq!(chans[0].order, vec![1, 0]);
    }
}
