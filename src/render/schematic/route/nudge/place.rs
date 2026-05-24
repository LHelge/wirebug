//! §6.2 — final placement of segments.
//!
//! Two independent passes: the horizontal pass solves the y of every
//! horizontal segment, the vertical pass the x of every vertical
//! segment, each treating the other axis as fixed (the paper runs them
//! separately for exactly this reason). Within a pass each segment is a
//! VPSC variable pulled toward its routed position; shared channels add
//! separation constraints in the §6.1 order so the wires fan apart, and
//! every box adds a wall constraint so a nudged segment can't be pushed
//! back inside the clearance it routed around. Port-touching segments are
//! pinned.

use super::super::RawRoute;
use super::super::geometry::Rect;
use super::EPS;
use super::order::Channel;
use super::segments::{Orientation, Segment};
use super::vpsc::{self, Constraint, Var};
use crate::view::Point;

/// Weight that pins a port-touching segment to its connection point.
const PIN_WEIGHT: f64 = 1e6;
/// Weight of an (immovable) obstacle boundary.
const WALL_WEIGHT: f64 = 1e9;

/// Solve both axes, writing each segment's final `perp`. The horizontal
/// pass runs first and its result is applied before the vertical pass:
/// a vertical segment's span depends on the `y` its horizontal neighbours
/// just took. `obstacles` are the clearance-inflated component rectangles.
pub(super) fn place(
    shapes: &mut [Option<Vec<Segment>>],
    channels: &[Channel],
    raws: &[RawRoute],
    obstacles: &[Rect],
    gap: f64,
) {
    let horizontal = solve_axis(
        shapes,
        channels,
        raws,
        obstacles,
        Orientation::Horizontal,
        gap,
    );
    apply(shapes, &horizontal);
    let vertical = solve_axis(
        shapes,
        channels,
        raws,
        obstacles,
        Orientation::Vertical,
        gap,
    );
    apply(shapes, &vertical);
}

/// Write solved perpendicular coordinates back onto their segments.
fn apply(shapes: &mut [Option<Vec<Segment>>], assignment: &[(usize, usize, f64)]) {
    for &(ri, si, perp) in assignment {
        if let Some(segs) = shapes[ri].as_mut() {
            segs[si].perp = perp;
        }
    }
}

/// Solve one axis, returning the new `perp` for every segment of
/// `orientation` as `(route, segment, perp)`. Pure: reads the current
/// segments, writes nothing.
fn solve_axis(
    shapes: &[Option<Vec<Segment>>],
    channels: &[Channel],
    raws: &[RawRoute],
    obstacles: &[Rect],
    orientation: Orientation,
    gap: f64,
) -> Vec<(usize, usize, f64)> {
    // Each segment's extent along its parallel axis, from its neighbours'
    // current `perp`. A segment of this orientation runs between two of
    // the *other* orientation, whose `perp` this pass leaves fixed — so
    // these spans are stable for the whole solve.
    let spans: Vec<Option<Vec<(f64, f64)>>> = shapes
        .iter()
        .enumerate()
        .map(|(ri, shape)| {
            shape
                .as_ref()
                .map(|segs| spans_of(segs, raws[ri].a, raws[ri].b))
        })
        .collect();

    // One VPSC variable per segment of this orientation.
    let mut vars = Vec::new();
    let mut owner = Vec::new(); // var index -> (route, segment)
    let mut var_of = std::collections::HashMap::new();
    for (ri, shape) in shapes.iter().enumerate() {
        let Some(segs) = shape else { continue };
        for (si, seg) in segs.iter().enumerate() {
            if seg.orientation == orientation {
                var_of.insert((ri, si), vars.len());
                vars.push(Var {
                    desired: seg.perp,
                    weight: if seg.is_end { PIN_WEIGHT } else { 1.0 },
                });
                owner.push((ri, si));
            }
        }
    }

    let mut cons = Vec::new();

    // Separation constraints: consecutive routes in each channel's order
    // whose segments actually overlap along the channel.
    for ch in channels.iter().filter(|c| c.orientation == orientation) {
        let seg_in_channel = |r: usize| -> Option<usize> {
            shapes[r].as_ref()?.iter().position(|s| {
                s.orientation == orientation && s.edges.iter().any(|e| ch.edges.contains(e))
            })
        };

        let mut prev: Option<(usize, usize)> = None; // (route, segment index)
        for &r in &ch.order {
            let Some(si) = seg_in_channel(r) else {
                continue;
            };
            if let Some((pr, psi)) = prev
                && spans_overlap(span_at(&spans, pr, psi), span_at(&spans, r, si))
            {
                cons.push(Constraint {
                    left: var_of[&(pr, psi)],
                    right: var_of[&(r, si)],
                    gap,
                });
            }
            prev = Some((r, si));
        }
    }

    // Wall constraints: keep every interior segment on the side of each
    // box it routed around, so nudging can't push it into the clearance.
    // Each boundary becomes an immovable variable.
    for (vi, &(ri, si)) in owner.iter().enumerate() {
        let Some(segs) = shapes[ri].as_ref() else {
            continue;
        };
        let seg = &segs[si];
        if seg.is_end {
            continue; // anchored at a port on the box edge — leave it.
        }
        let (lo, hi) = span_at(&spans, ri, si);
        for r in obstacles {
            if let Some((wall_pos, seg_below_wall)) = wall_for(seg.perp, lo, hi, orientation, r) {
                let wall = vars.len();
                vars.push(Var {
                    desired: wall_pos,
                    weight: WALL_WEIGHT,
                });
                // `seg_below_wall` means the segment sits at a smaller
                // coordinate than the wall, i.e. `wall - seg >= 0`.
                cons.push(if seg_below_wall {
                    Constraint {
                        left: vi,
                        right: wall,
                        gap: 0.0,
                    }
                } else {
                    Constraint {
                        left: wall,
                        right: vi,
                        gap: 0.0,
                    }
                });
            }
        }
    }

    let positions = vpsc::solve(&vars, &cons);
    owner
        .iter()
        .enumerate()
        .map(|(vi, &(ri, si))| (ri, si, positions[vi]))
        .collect()
}

/// The precomputed `(lo, hi)` span of segment `si` in route `ri`. Indices
/// come from `owner`/`var_of`, so the route always has a span list.
fn span_at(spans: &[Option<Vec<(f64, f64)>>], ri: usize, si: usize) -> (f64, f64) {
    spans[ri].as_ref().map_or((0.0, 0.0), |s| s[si])
}

/// Each segment's `(lo, hi)` extent along its parallel axis, derived from
/// its neighbours' `perp` and the route's port points at the two ends.
fn spans_of(segs: &[Segment], a: Point, b: Point) -> Vec<(f64, f64)> {
    let n = segs.len();
    (0..n)
        .map(|i| {
            let left = if i == 0 {
                port_parallel(a, segs[i].orientation)
            } else {
                segs[i - 1].perp
            };
            let right = if i == n - 1 {
                port_parallel(b, segs[i].orientation)
            } else {
                segs[i + 1].perp
            };
            (left.min(right), left.max(right))
        })
        .collect()
}

/// If a segment of `orientation` at `perp`, spanning `[lo, hi]` along its
/// parallel axis, crosses obstacle `r`, return the boundary it must not
/// cross and whether the segment currently lies *below* (smaller
/// coordinate than) that boundary.
fn wall_for(
    perp: f64,
    lo: f64,
    hi: f64,
    orientation: Orientation,
    r: &Rect,
) -> Option<(f64, bool)> {
    let (near, far, blo, bhi) = match orientation {
        // Horizontal: perp = y, parallel = x; box spans [r.x, r.x+w] in x.
        Orientation::Horizontal => (r.y, r.y + r.h, r.x, r.x + r.w),
        // Vertical: perp = x, parallel = y; box spans [r.y, r.y+h] in y.
        Orientation::Vertical => (r.x, r.x + r.w, r.y, r.y + r.h),
    };

    let overlaps = lo.max(blo) < hi.min(bhi) - EPS;
    if !overlaps {
        return None;
    }

    if perp <= near + EPS {
        Some((near, true)) // stay at/above the near edge
    } else if perp >= far - EPS {
        Some((far, false)) // stay at/below the far edge
    } else {
        // Already inside (routing shouldn't allow this) — push to nearer.
        let to_near = perp - near;
        let to_far = far - perp;
        if to_near <= to_far {
            Some((near, true))
        } else {
            Some((far, false))
        }
    }
}

/// The coordinate of a port point along a segment's parallel axis.
fn port_parallel(p: Point, orientation: Orientation) -> f64 {
    match orientation {
        Orientation::Horizontal => p.x,
        Orientation::Vertical => p.y,
    }
}

fn spans_overlap(a: (f64, f64), b: (f64, f64)) -> bool {
    a.1.min(b.1) - a.0.max(b.0) > EPS
}

/// Rebuild a route's polyline from its solved segments: the port points
/// bracket the corners, each corner being where two adjacent segments
/// (one horizontal, one vertical) meet.
pub(super) fn rebuild(raw: &RawRoute, segs: &[Segment]) -> Vec<Point> {
    let mut pts = vec![raw.a];
    for pair in segs.windows(2) {
        pts.push(corner(&pair[0], &pair[1]));
    }
    pts.push(raw.b);
    super::collapse_collinear(pts)
}

fn corner(s0: &Segment, s1: &Segment) -> Point {
    match s0.orientation {
        // horizontal then vertical: x from the vertical, y from the horizontal
        Orientation::Horizontal => Point::new(s1.perp, s0.perp),
        // vertical then horizontal
        Orientation::Vertical => Point::new(s0.perp, s1.perp),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::super::segments::Edge;
    use super::*;

    // Box (already clearance-inflated) covering x∈[0,200], y∈[0,100].
    fn box_rect() -> Rect {
        Rect::new(0.0, 0.0, 200.0, 100.0)
    }

    #[test]
    fn segment_below_a_box_is_walled_below_it() {
        let (pos, below) =
            wall_for(110.0, 0.0, 200.0, Orientation::Horizontal, &box_rect()).expect("wall");
        assert_eq!(pos, 100.0); // the box's bottom edge
        assert!(!below); // segment must stay ≥ the wall
    }

    #[test]
    fn segment_above_a_box_is_walled_above_it() {
        let (pos, below) =
            wall_for(-10.0, 0.0, 200.0, Orientation::Horizontal, &box_rect()).expect("wall");
        assert_eq!(pos, 0.0); // the box's top edge
        assert!(below); // segment must stay ≤ the wall
    }

    #[test]
    fn segment_clear_of_a_box_in_x_needs_no_wall() {
        // Span 300..400 is past the box in x, so no parallel-axis overlap.
        assert!(wall_for(110.0, 300.0, 400.0, Orientation::Horizontal, &box_rect()).is_none());
    }

    /// A route shaped like `⊐`: a vertical stub, a horizontal run at `y`
    /// carrying `edge`, then a vertical stub. The middle segment is the
    /// one a shared channel nudges; `pinned` marks it port-anchored.
    fn channel_route(y: f64, edge: Edge, pinned: bool) -> Vec<Segment> {
        vec![
            Segment {
                orientation: Orientation::Vertical,
                perp: 0.0,
                edges: vec![],
                is_end: true,
            },
            Segment {
                orientation: Orientation::Horizontal,
                perp: y,
                edges: vec![edge],
                is_end: pinned,
            },
            Segment {
                orientation: Orientation::Vertical,
                perp: 100.0,
                edges: vec![],
                is_end: true,
            },
        ]
    }

    fn raw() -> RawRoute {
        RawRoute {
            a: Point::new(0.0, 0.0),
            b: Point::new(100.0, 20.0),
            nodes: vec![],
        }
    }

    fn solved_perp(out: &[(usize, usize, f64)], route: usize, seg: usize) -> f64 {
        out.iter()
            .find(|&&(ri, si, _)| ri == route && si == seg)
            .map(|&(_, _, p)| p)
            .expect("segment was solved")
    }

    #[test]
    fn overlapping_channel_segments_spread_by_the_nudge_gap() {
        let edge: Edge = (1, 2);
        let shapes = vec![
            Some(channel_route(10.0, edge, false)),
            Some(channel_route(10.0, edge, false)),
        ];
        let channels = vec![Channel {
            orientation: Orientation::Horizontal,
            order: vec![0, 1],
            edges: HashSet::from([edge]),
        }];

        let gap = 7.0;
        let out = solve_axis(
            &shapes,
            &channels,
            &[raw(), raw()],
            &[],
            Orientation::Horizontal,
            gap,
        );
        let (y0, y1) = (solved_perp(&out, 0, 1), solved_perp(&out, 1, 1));

        assert!((y1 - y0 - gap).abs() < EPS, "gap was {}", y1 - y0);
        assert!(
            ((y0 + y1) / 2.0 - 10.0).abs() < EPS,
            "not centred: {y0}, {y1}"
        );
    }

    #[test]
    fn a_pinned_channel_segment_holds_while_its_neighbour_yields() {
        let edge: Edge = (1, 2);
        let shapes = vec![
            Some(channel_route(10.0, edge, true)), // route 0 pinned at its port
            Some(channel_route(10.0, edge, false)),
        ];
        let channels = vec![Channel {
            orientation: Orientation::Horizontal,
            order: vec![0, 1],
            edges: HashSet::from([edge]),
        }];

        let gap = 6.0;
        let out = solve_axis(
            &shapes,
            &channels,
            &[raw(), raw()],
            &[],
            Orientation::Horizontal,
            gap,
        );
        let (y0, y1) = (solved_perp(&out, 0, 1), solved_perp(&out, 1, 1));

        assert!((y0 - 10.0).abs() < 1e-3, "pinned segment moved to {y0}");
        assert!((y1 - 16.0).abs() < 1e-3, "free segment didn't yield: {y1}");
    }
}
