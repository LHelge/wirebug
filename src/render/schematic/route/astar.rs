//! A\* over the orthogonal visibility graph — Wybrow, Marriott &
//! Stuckey §5 (full citation and link in the [`super`] module docs).
//!
//! The search state is `(node, direction-of-entry)` so that bends can be
//! counted: a step that changes direction costs [`BEND_PENALTY`] on top
//! of the segment length. Costs are fixed-point `i64` (world units ×
//! [`SCALE`]) so they are totally ordered without a float-ordering crate.
//!
//! The two connection points are modelled as synthetic nodes, each with
//! a single edge to its stub node on the obstacle-clearance boundary.
//! This forces the first segment out of the source and the last segment
//! into the target along the correct port normals.

use pathfinding::prelude::astar;

use super::geometry::Dir;
use super::visibility::Ovg;
use super::{BEND_PENALTY, SCALE};
use crate::view::Point;

const EPS: f64 = 1e-6;

/// Find a minimal-cost orthogonal route from a source connection point
/// (leaving along `start_dir`) to a target connection point (entered
/// along `goal_in_dir`), via the stub nodes `start_stub` / `goal_stub`.
/// Returns the polyline including both connection points, or `None` if
/// the graph offers no route.
pub(super) fn find_route(
    ovg: &Ovg,
    start_pos: Point,
    start_dir: Dir,
    start_stub: usize,
    goal_pos: Point,
    goal_in_dir: Dir,
    goal_stub: usize,
) -> Option<Vec<Point>> {
    let search = Search {
        ovg,
        start_pos,
        start_dir,
        start_stub,
        goal_pos,
        goal_in_dir,
        goal_stub,
        port_a: ovg.node_count(),
        port_b: ovg.node_count() + 1,
    };

    let (path, _cost) = astar(
        &(search.port_a, start_dir),
        |&(id, dir)| search.successors(id, dir),
        |&(id, dir)| search.heuristic(id, dir),
        |&(id, _)| id == search.port_b,
    )?;

    let pts = path.iter().map(|&(id, _)| search.pos(id)).collect();
    Some(collapse(pts))
}

struct Search<'a> {
    ovg: &'a Ovg,
    start_pos: Point,
    start_dir: Dir,
    start_stub: usize,
    goal_pos: Point,
    goal_in_dir: Dir,
    goal_stub: usize,
    port_a: usize,
    port_b: usize,
}

impl Search<'_> {
    fn pos(&self, id: usize) -> Point {
        if id == self.port_a {
            self.start_pos
        } else if id == self.port_b {
            self.goal_pos
        } else {
            self.ovg.position(id)
        }
    }

    fn successors(&self, id: usize, dir: Dir) -> Vec<((usize, Dir), i64)> {
        if id == self.port_a {
            let to = self.ovg.position(self.start_stub);
            return vec![(
                (self.start_stub, self.start_dir),
                seg_cost(self.start_pos, to, dir, self.start_dir),
            )];
        }
        if id == self.port_b {
            return Vec::new();
        }

        let from = self.pos(id);
        let mut out: Vec<((usize, Dir), i64)> = self
            .ovg
            .neighbours(id)
            .iter()
            .map(|&(nb, edir)| ((nb, edir), seg_cost(from, self.ovg.position(nb), dir, edir)))
            .collect();

        if id == self.goal_stub {
            out.push((
                (self.port_b, self.goal_in_dir),
                seg_cost(from, self.goal_pos, dir, self.goal_in_dir),
            ));
        }
        out
    }

    fn heuristic(&self, id: usize, dir: Dir) -> i64 {
        let p = self.pos(id);
        let dx = self.goal_pos.x - p.x;
        let dy = self.goal_pos.y - p.y;
        len_scaled(p, self.goal_pos) + BEND_PENALTY * min_bends(dx, dy, dir)
    }
}

fn seg_cost(from: Point, to: Point, entry: Dir, step: Dir) -> i64 {
    let bend = if step == entry { 0 } else { BEND_PENALTY };
    len_scaled(from, to) + bend
}

fn len_scaled(a: Point, b: Point) -> i64 {
    (((a.x - b.x).abs() + (a.y - b.y).abs()) * SCALE).round() as i64
}

/// A lower bound on the bends still needed to reach the goal from a node
/// heading `dir`, in free space (admissible — obstacles and the final
/// heading can only add bends). See paper Fig. 2(b).
fn min_bends(dx: f64, dy: f64, dir: Dir) -> i64 {
    let need_x = if dx > EPS {
        Some(Dir::East)
    } else if dx < -EPS {
        Some(Dir::West)
    } else {
        None
    };
    let need_y = if dy > EPS {
        Some(Dir::South)
    } else if dy < -EPS {
        Some(Dir::North)
    } else {
        None
    };

    match (need_x, need_y) {
        (None, None) => 0,
        (Some(n), None) | (None, Some(n)) => {
            if dir == n {
                0
            } else if dir == n.opposite() {
                2
            } else {
                1
            }
        }
        (Some(nx), Some(ny)) => {
            if dir == nx || dir == ny {
                1
            } else {
                2
            }
        }
    }
}

/// Drop coincident points and merge consecutive collinear segments so
/// the polyline strictly alternates horizontal and vertical runs.
fn collapse(pts: Vec<Point>) -> Vec<Point> {
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
    use super::super::geometry::Rect;
    use super::*;

    fn segment_hits(rect: &Rect, path: &[Point]) -> bool {
        path.windows(2).any(|w| rect.blocks_segment(w[0], w[1]))
    }

    #[test]
    fn clear_line_of_sight_routes_straight() {
        let a = Point::new(100.0, 50.0);
        let a_stub = Point::new(116.0, 50.0);
        let b = Point::new(300.0, 50.0);
        let b_stub = Point::new(284.0, 50.0);
        let ovg = Ovg::build(&[], &[a, a_stub, b, b_stub]);
        let sa = ovg.node_at(a_stub).unwrap();
        let sb = ovg.node_at(b_stub).unwrap();

        // a faces East (outward), b faces West (outward) -> enters East.
        let path = find_route(&ovg, a, Dir::East, sa, b, Dir::East, sb).expect("route");
        assert_eq!(path, vec![a, b]);
    }

    #[test]
    fn obstacle_between_ports_forces_detour() {
        let box_rect = Rect::new(150.0, 0.0, 100.0, 100.0);
        let a = Point::new(100.0, 50.0);
        let a_stub = Point::new(116.0, 50.0);
        let b = Point::new(300.0, 50.0);
        let b_stub = Point::new(284.0, 50.0);
        let ovg = Ovg::build(&[box_rect], &[a, a_stub, b, b_stub]);
        let sa = ovg.node_at(a_stub).unwrap();
        let sb = ovg.node_at(b_stub).unwrap();

        let path = find_route(&ovg, a, Dir::East, sa, b, Dir::East, sb).expect("route");

        assert!(path.len() > 2, "detour should bend: {path:?}");
        assert!(
            !segment_hits(&box_rect, &path),
            "route crosses box: {path:?}"
        );
        assert_eq!(*path.first().unwrap(), a);
        assert_eq!(*path.last().unwrap(), b);
    }

    #[test]
    fn route_leaves_and_enters_along_port_normals() {
        let a = Point::new(100.0, 50.0);
        let a_stub = Point::new(116.0, 50.0);
        let b = Point::new(300.0, 50.0);
        let b_stub = Point::new(284.0, 50.0);
        let ovg = Ovg::build(&[], &[a, a_stub, b, b_stub]);
        let sa = ovg.node_at(a_stub).unwrap();
        let sb = ovg.node_at(b_stub).unwrap();

        let path = find_route(&ovg, a, Dir::East, sa, b, Dir::East, sb).expect("route");
        // First step leaves `a` heading East (away from the box).
        assert!(path[1].x > a.x && (path[1].y - a.y).abs() < EPS);
        // Last step arrives at `b` from the West (heading East, inward).
        let penult = path[path.len() - 2];
        assert!(penult.x < b.x && (penult.y - b.y).abs() < EPS);
    }
}
