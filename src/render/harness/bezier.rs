//! Cubic-bezier math for harness wire routing — pure geometry, no SVG state
//! beyond the path string it can emit.
//!
//! Every harness wire segment is one horizontally-flexed cubic: the control
//! points share each endpoint's `y` but are pulled a fraction of the
//! horizontal span toward the other end, so the curve leaves each end
//! travelling horizontally before bending vertically. That is the
//! cable-flex look WireViz draws.

use crate::render::geometry::Point;

/// Fraction of the horizontal span each control point is pulled toward the
/// far end. Kept in `(0, 1)`, so control points stay between the endpoints in
/// `x` and the whole curve lives inside the endpoints' bounding box (a cubic
/// lies within the convex hull of its control points) — no viewbox overshoot.
pub(super) const FLEX: f64 = 0.4;

/// A cubic bezier as its four control points: start, two handles, end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Cubic {
    pub(super) p0: Point,
    pub(super) c1: Point,
    pub(super) c2: Point,
    pub(super) p3: Point,
}

/// A horizontally-flexed cubic from `from` to `to`. `flex` is the fraction of
/// the horizontal span (`to.x - from.x`) each handle is pulled inward; the
/// handles keep their endpoint's `y`.
pub(super) fn flex(from: Point, to: Point, flex: f64) -> Cubic {
    let dx = to.x - from.x;
    Cubic {
        p0: from,
        c1: Point::new(from.x + flex * dx, from.y),
        c2: Point::new(to.x - flex * dx, to.y),
        p3: to,
    }
}

impl Cubic {
    /// The SVG path data (`M … C …`) for this single segment.
    pub(super) fn path_d(&self) -> String {
        format!(
            "M{},{} C{},{} {},{} {},{}",
            self.p0.x, self.p0.y, self.c1.x, self.c1.y, self.c2.x, self.c2.y, self.p3.x, self.p3.y
        )
    }

    /// The point on the curve at parameter `t` (0 = start, 1 = end). Used to
    /// anchor a loose wire's annotation at its midpoint.
    pub(super) fn point_at(&self, t: f64) -> Point {
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        Point::new(
            a * self.p0.x + b * self.c1.x + c * self.c2.x + d * self.p3.x,
            a * self.p0.y + b * self.c1.y + c * self.c2.y + d * self.p3.y,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flex_keeps_endpoints_and_pulls_handles_horizontally() {
        let c = flex(Point::new(0.0, 0.0), Point::new(100.0, 40.0), 0.4);
        assert_eq!(c.p0, Point::new(0.0, 0.0));
        assert_eq!(c.p3, Point::new(100.0, 40.0));
        // Handles share their endpoint's y and sit 40% of the span inward.
        assert_eq!(c.c1, Point::new(40.0, 0.0));
        assert_eq!(c.c2, Point::new(60.0, 40.0));
    }

    #[test]
    fn handles_stay_within_the_endpoint_span() {
        // 0 < flex < 1 keeps both handles' x between the endpoints, so the
        // curve never overshoots its bounding box.
        let c = flex(Point::new(10.0, 5.0), Point::new(90.0, 80.0), FLEX);
        for x in [c.c1.x, c.c2.x] {
            assert!((10.0..=90.0).contains(&x), "handle x {x} escaped the span");
        }
    }

    #[test]
    fn degenerate_vertical_segment_has_collinear_handles() {
        // Same x on both ends: dx == 0, so handles sit directly above/below.
        let c = flex(Point::new(20.0, 0.0), Point::new(20.0, 50.0), 0.4);
        assert_eq!(c.c1, Point::new(20.0, 0.0));
        assert_eq!(c.c2, Point::new(20.0, 50.0));
    }

    #[test]
    fn midpoint_is_centred_for_a_symmetric_curve() {
        let c = flex(Point::new(0.0, 0.0), Point::new(100.0, 100.0), 0.4);
        let mid = c.point_at(0.5);
        assert!((mid.x - 50.0).abs() < 1e-9);
        assert!((mid.y - 50.0).abs() < 1e-9);
    }
}
