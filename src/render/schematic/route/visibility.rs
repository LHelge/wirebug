//! Orthogonal visibility graph — Wybrow, Marriott & Stuckey §4 (full
//! citation and link in the [`super`] module docs).
//!
//! Nodes sit at the intersections of the "interesting" x and y
//! coordinates — obstacle corners and port stubs — minus any point
//! strictly inside an obstacle. Edges join grid-adjacent nodes whose
//! connecting segment does not cross an obstacle interior.
//!
//! Adjacent-only stepping is sufficient *because every obstacle edge
//! coordinate is already a grid line*: a segment spanning an obstacle's
//! interior is exactly one between two adjacent columns/rows, so gating
//! adjacent edges on [`Rect::blocks_segment`] is enough to forbid it.

use std::collections::HashMap;

use super::geometry::{Dir, Rect};
use crate::render::geometry::Point;

const EPS: f64 = 1e-6;

/// The orthogonal visibility graph for one diagram.
pub(super) struct Ovg {
    nodes: Vec<Point>,
    /// Per-node outgoing edges as `(neighbour, direction of travel)`.
    adj: Vec<Vec<(usize, Dir)>>,
    xs: Vec<f64>,
    ys: Vec<f64>,
    grid: HashMap<(usize, usize), usize>,
}

impl Ovg {
    /// Build the graph from inflated obstacle rectangles and the extra
    /// interesting points (port connection points and their stubs).
    pub(super) fn build(obstacles: &[Rect], extra: &[Point]) -> Self {
        let mut xs: Vec<f64> = Vec::new();
        let mut ys: Vec<f64> = Vec::new();
        for r in obstacles {
            xs.push(r.x);
            xs.push(r.x + r.w);
            ys.push(r.y);
            ys.push(r.y + r.h);
        }
        for p in extra {
            xs.push(p.x);
            ys.push(p.y);
        }
        let xs = sorted_unique(xs);
        let ys = sorted_unique(ys);

        let mut nodes = Vec::new();
        let mut grid = HashMap::new();
        for (i, &x) in xs.iter().enumerate() {
            for (j, &y) in ys.iter().enumerate() {
                let p = Point::new(x, y);
                if obstacles.iter().any(|r| r.contains_point(p)) {
                    continue;
                }
                grid.insert((i, j), nodes.len());
                nodes.push(p);
            }
        }

        let mut adj = vec![Vec::new(); nodes.len()];
        for (&(i, j), &id) in &grid {
            // East / South neighbours; the reverse edge is added when the
            // neighbour is the active node, so iterate forward only.
            if let Some(&east) = grid.get(&(i + 1, j)) {
                let seg = (nodes[id], nodes[east]);
                if !obstacles.iter().any(|r| r.blocks_segment(seg.0, seg.1)) {
                    adj[id].push((east, Dir::East));
                    adj[east].push((id, Dir::West));
                }
            }
            if let Some(&south) = grid.get(&(i, j + 1)) {
                let seg = (nodes[id], nodes[south]);
                if !obstacles.iter().any(|r| r.blocks_segment(seg.0, seg.1)) {
                    adj[id].push((south, Dir::South));
                    adj[south].push((id, Dir::North));
                }
            }
        }

        Self {
            nodes,
            adj,
            xs,
            ys,
            grid,
        }
    }

    pub(super) fn position(&self, id: usize) -> Point {
        self.nodes[id]
    }

    pub(super) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(super) fn neighbours(&self, id: usize) -> &[(usize, Dir)] {
        &self.adj[id]
    }

    /// The node id at exactly this point, if one exists. Used to attach
    /// port stubs, whose coordinates are interesting points by
    /// construction.
    pub(super) fn node_at(&self, p: Point) -> Option<usize> {
        let i = self.xs.iter().position(|&v| (v - p.x).abs() < EPS)?;
        let j = self.ys.iter().position(|&v| (v - p.y).abs() < EPS)?;
        self.grid.get(&(i, j)).copied()
    }
}

fn sorted_unique(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < EPS);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_node_sits_inside_an_obstacle() {
        let obstacles = [Rect::new(0.0, 0.0, 100.0, 100.0)];
        let ovg = Ovg::build(&obstacles, &[]);
        // The four corners are the only interesting points; all lie on
        // the boundary, none strictly inside.
        for id in 0..ovg.nodes.len() {
            assert!(!obstacles[0].contains_point(ovg.position(id)));
        }
    }

    #[test]
    fn edge_across_obstacle_interior_is_absent() {
        // Two stubs left and right of a central box, sharing its mid-y.
        let obstacles = [Rect::new(40.0, 0.0, 40.0, 80.0)];
        let extra = [Point::new(0.0, 40.0), Point::new(120.0, 40.0)];
        let ovg = Ovg::build(&obstacles, &extra);

        let left = ovg.node_at(Point::new(0.0, 40.0)).expect("left node");
        // The left stub must not connect straight east through the box to
        // the box's far wall: its east neighbour (box west wall) shares
        // an edge segment, but no edge crosses the interior at y=40.
        let crosses_interior = (0..ovg.nodes.len()).any(|a| {
            ovg.neighbours(a)
                .iter()
                .any(|&(b, _)| obstacles[0].blocks_segment(ovg.position(a), ovg.position(b)))
        });
        assert!(!crosses_interior);
        // And the left stub does have at least one usable edge.
        assert!(!ovg.neighbours(left).is_empty());
    }
}
