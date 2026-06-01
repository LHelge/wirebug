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
use crate::render::geometry::Point;

const EPS: f64 = 1e-6;
/// Cost of one proper wire crossing. Kept below a bend so bend count still
/// dominates, but far above ordinary length so crossings break equal-bend
/// ties before path length does.
const CROSSING_PENALTY: i64 = BEND_PENALTY / 10;
const SOFT_OVERLAP_PENALTY: i64 = BEND_PENALTY / 10;

/// Find a minimal-cost orthogonal route from a source connection point
/// (leaving along `start_dir`) to a target connection point (entered
/// along `goal_in_dir`), via the stub nodes `start_stub` / `goal_stub`.
/// Returns the base OVG node-id path (`start_stub` … `goal_stub`,
/// excluding the synthetic port nodes), or `None` if the graph offers no
/// route. Adds a crossing penalty for each segment that properly crosses
/// any already routed polyline in `avoid`, and an overlap penalty for
/// running along any soft guide line in `soft_avoid`.
#[allow(clippy::too_many_arguments)]
pub(super) fn find_route_avoiding(
    ovg: &Ovg,
    start_pos: Point,
    start_dir: Dir,
    start_stub: usize,
    goal_pos: Point,
    goal_in_dir: Dir,
    goal_stub: usize,
    avoid: &[Vec<Point>],
    soft_avoid: &[Vec<Point>],
) -> Option<Vec<usize>> {
    let search = Search {
        ovg,
        start_pos,
        start_dir,
        start_stub,
        goal_pos,
        goal_in_dir,
        goal_stub,
        avoid,
        soft_avoid,
        port_a: ovg.node_count(),
        port_b: ovg.node_count() + 1,
    };

    let (path, _cost) = astar(
        &(search.port_a, start_dir),
        |&(id, dir)| search.successors(id, dir),
        |&(id, dir)| search.heuristic(id, dir),
        |&(id, _)| id == search.port_b,
    )?;

    let nodes = path
        .iter()
        .map(|&(id, _)| id)
        .filter(|&id| id != search.port_a && id != search.port_b)
        .collect();
    Some(nodes)
}

struct Search<'a> {
    ovg: &'a Ovg,
    start_pos: Point,
    start_dir: Dir,
    start_stub: usize,
    goal_pos: Point,
    goal_in_dir: Dir,
    goal_stub: usize,
    avoid: &'a [Vec<Point>],
    soft_avoid: &'a [Vec<Point>],
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
                self.seg_cost(self.start_pos, to, dir, self.start_dir),
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
            .map(|&(nb, edir)| {
                (
                    (nb, edir),
                    self.seg_cost(from, self.ovg.position(nb), dir, edir),
                )
            })
            .collect();

        if id == self.goal_stub {
            out.push((
                (self.port_b, self.goal_in_dir),
                self.seg_cost(from, self.goal_pos, dir, self.goal_in_dir),
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

    fn seg_cost(&self, from: Point, to: Point, entry: Dir, step: Dir) -> i64 {
        let bend = if step == entry { 0 } else { BEND_PENALTY };
        len_scaled(from, to)
            + bend
            + CROSSING_PENALTY * crossing_count(from, to, self.avoid)
            + SOFT_OVERLAP_PENALTY * overlap_count(from, to, self.soft_avoid)
    }
}

fn len_scaled(a: Point, b: Point) -> i64 {
    (((a.x - b.x).abs() + (a.y - b.y).abs()) * SCALE).round() as i64
}

fn crossing_count(a: Point, b: Point, avoid: &[Vec<Point>]) -> i64 {
    avoid
        .iter()
        .flat_map(|route| route.windows(2))
        .filter(|w| orthogonally_intersects(a, b, w[0], w[1]))
        .count() as i64
}

fn overlap_count(a: Point, b: Point, avoid: &[Vec<Point>]) -> i64 {
    avoid
        .iter()
        .flat_map(|route| route.windows(2))
        .filter(|w| collinear_overlap(a, b, w[0], w[1]))
        .count() as i64
}

fn orthogonally_intersects(a: Point, b: Point, c: Point, d: Point) -> bool {
    let ab_horizontal = (a.y - b.y).abs() < EPS;
    let cd_horizontal = (c.y - d.y).abs() < EPS;
    if ab_horizontal == cd_horizontal {
        return false;
    }

    let (h0, h1, v0, v1) = if ab_horizontal {
        (a, b, c, d)
    } else {
        (c, d, a, b)
    };
    let h_y = h0.y;
    let v_x = v0.x;
    let h_lo = h0.x.min(h1.x);
    let h_hi = h0.x.max(h1.x);
    let v_lo = v0.y.min(v1.y);
    let v_hi = v0.y.max(v1.y);

    v_x >= h_lo - EPS && v_x <= h_hi + EPS && h_y >= v_lo - EPS && h_y <= v_hi + EPS
}

fn collinear_overlap(a: Point, b: Point, c: Point, d: Point) -> bool {
    let ab_horizontal = (a.y - b.y).abs() < EPS;
    let cd_horizontal = (c.y - d.y).abs() < EPS;
    if ab_horizontal != cd_horizontal {
        return false;
    }

    if ab_horizontal {
        if (a.y - c.y).abs() >= EPS {
            return false;
        }
        intervals_overlap(a.x, b.x, c.x, d.x)
    } else {
        if (a.x - c.x).abs() >= EPS {
            return false;
        }
        intervals_overlap(a.y, b.y, c.y, d.y)
    }
}

fn intervals_overlap(a0: f64, a1: f64, b0: f64, b1: f64) -> bool {
    let (a_lo, a_hi) = (a0.min(a1), a0.max(a1));
    let (b_lo, b_hi) = (b0.min(b1), b0.max(b1));
    a_hi.min(b_hi) - a_lo.max(b_lo) > EPS
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

#[cfg(test)]
mod tests {
    use super::super::collapse_collinear;
    use super::super::geometry::Rect;
    use super::*;

    /// Resolve a route to its collapsed polyline, the way the router does
    /// for rendering — prepend/append the port points, map nodes to
    /// positions, merge collinear runs.
    fn route_points(
        ovg: &Ovg,
        a: Point,
        da: Dir,
        sa: usize,
        b: Point,
        db: Dir,
        sb: usize,
    ) -> Vec<Point> {
        let nodes = find_route_avoiding(ovg, a, da, sa, b, db, sb, &[], &[]).expect("route");
        let mut pts = vec![a];
        pts.extend(nodes.iter().map(|&n| ovg.position(n)));
        pts.push(b);
        collapse_collinear(pts)
    }

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
        let path = route_points(&ovg, a, Dir::East, sa, b, Dir::East, sb);
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

        let path = route_points(&ovg, a, Dir::East, sa, b, Dir::East, sb);

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
        let box_rect = Rect::new(150.0, 0.0, 100.0, 100.0);
        let a = Point::new(100.0, 50.0);
        let a_stub = Point::new(116.0, 50.0);
        let b = Point::new(300.0, 50.0);
        let b_stub = Point::new(284.0, 50.0);
        let ovg = Ovg::build(&[box_rect], &[a, a_stub, b, b_stub]);
        let sa = ovg.node_at(a_stub).unwrap();
        let sb = ovg.node_at(b_stub).unwrap();

        let path = route_points(&ovg, a, Dir::East, sa, b, Dir::East, sb);
        // First step leaves `a` heading East (away from the box).
        assert!(path[1].x > a.x && (path[1].y - a.y).abs() < EPS);
        // Last step arrives at `b` from the West (heading East, inward).
        let penult = path[path.len() - 2];
        assert!(penult.x < b.x && (penult.y - b.y).abs() < EPS);
    }

    #[test]
    fn crossing_penalty_breaks_equal_bend_ties() {
        let a = Point::new(0.0, 0.0);
        let a_stub = Point::new(10.0, 0.0);
        let b = Point::new(100.0, 100.0);
        let b_stub = Point::new(90.0, 100.0);
        let avoid = vec![vec![Point::new(0.0, 50.0), Point::new(20.0, 50.0)]];
        let extra = [
            a,
            a_stub,
            b,
            b_stub,
            Point::new(10.0, 50.0),
            Point::new(90.0, 50.0),
        ];
        let ovg = Ovg::build(&[] as &[Rect], &extra);
        let sa = ovg.node_at(a_stub).unwrap();
        let sb = ovg.node_at(b_stub).unwrap();

        let nodes =
            find_route_avoiding(&ovg, a, Dir::East, sa, b, Dir::East, sb, &avoid, &[])
                .expect("route");
        let mut path = vec![a];
        path.extend(nodes.iter().map(|&n| ovg.position(n)));
        path.push(b);
        let path = collapse_collinear(path);

        assert!(
            !path
                .windows(2)
                .any(|w| orthogonally_intersects(w[0], w[1], avoid[0][0], avoid[0][1])),
            "route still crosses avoided segment: {path:?}"
        );
        assert!(
            path.iter().any(|p| (p.x - 90.0).abs() < EPS),
            "expected route to use the non-crossing dogleg: {path:?}"
        );
    }

    #[test]
    fn soft_overlap_penalty_breaks_equal_bend_ties() {
        let a = Point::new(0.0, 0.0);
        let a_stub = Point::new(10.0, 0.0);
        let b = Point::new(100.0, 100.0);
        let b_stub = Point::new(90.0, 100.0);
        let soft_avoid = vec![vec![Point::new(10.0, 0.0), Point::new(10.0, 100.0)]];
        let extra = [
            a,
            a_stub,
            b,
            b_stub,
            Point::new(10.0, 100.0),
            Point::new(90.0, 0.0),
        ];
        let ovg = Ovg::build(&[] as &[Rect], &extra);
        let sa = ovg.node_at(a_stub).unwrap();
        let sb = ovg.node_at(b_stub).unwrap();

        let nodes =
            find_route_avoiding(&ovg, a, Dir::East, sa, b, Dir::East, sb, &[], &soft_avoid)
                .expect("route");
        let mut path = vec![a];
        path.extend(nodes.iter().map(|&n| ovg.position(n)));
        path.push(b);
        let path = collapse_collinear(path);

        assert!(
            !path
                .windows(2)
                .any(|w| collinear_overlap(w[0], w[1], soft_avoid[0][0], soft_avoid[0][1])),
            "route still overlaps soft avoid line: {path:?}"
        );
        assert!(
            path.iter().any(|p| (p.x - 90.0).abs() < EPS),
            "expected route to use the non-overlapping dogleg: {path:?}"
        );
    }
}
