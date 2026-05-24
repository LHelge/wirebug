//! Rectilinear geometry primitives for connector routing: axis-aligned
//! obstacle rectangles and the four compass directions a route travels.

use super::super::layout::Bounds;
use crate::view::{Point, Side};

/// Axis-aligned rectangle in SVG world coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Rect {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) w: f64,
    pub(super) h: f64,
}

impl From<Bounds> for Rect {
    fn from(b: Bounds) -> Rect {
        Rect::new(b.origin.x, b.origin.y, b.width, b.height)
    }
}

impl Rect {
    pub(super) fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    fn right(&self) -> f64 {
        self.x + self.w
    }

    fn bottom(&self) -> f64 {
        self.y + self.h
    }

    /// Grow the rectangle by `margin` on every side.
    pub(super) fn inflated(&self, margin: f64) -> Rect {
        Rect {
            x: self.x - margin,
            y: self.y - margin,
            w: self.w + 2.0 * margin,
            h: self.h + 2.0 * margin,
        }
    }

    /// True iff `p` lies strictly inside the rectangle. Points on an
    /// edge are *not* contained — obstacle edges are valid routing lines.
    pub(super) fn contains_point(&self, p: Point) -> bool {
        p.x > self.x && p.x < self.right() && p.y > self.y && p.y < self.bottom()
    }

    /// True iff the axis-aligned segment `a`–`b` passes through the
    /// rectangle's interior. A segment that merely runs along an edge
    /// (or touches a corner) does not block.
    pub(super) fn blocks_segment(&self, a: Point, b: Point) -> bool {
        if (a.y - b.y).abs() < f64::EPSILON {
            // Horizontal segment at y = a.y.
            let y = a.y;
            if y <= self.y || y >= self.bottom() {
                return false;
            }
            let lo = a.x.min(b.x);
            let hi = a.x.max(b.x);
            lo < self.right() && hi > self.x
        } else if (a.x - b.x).abs() < f64::EPSILON {
            // Vertical segment at x = a.x.
            let x = a.x;
            if x <= self.x || x >= self.right() {
                return false;
            }
            let lo = a.y.min(b.y);
            let hi = a.y.max(b.y);
            lo < self.bottom() && hi > self.y
        } else {
            // Routes are orthogonal; a diagonal never reaches here.
            false
        }
    }
}

/// A compass direction of travel. North is toward smaller y (SVG y grows
/// downward), matching [`Side`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Dir {
    North,
    South,
    East,
    West,
}

impl Dir {
    pub(super) fn opposite(self) -> Dir {
        match self {
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::East => Dir::West,
            Dir::West => Dir::East,
        }
    }

    /// Unit step `(dx, dy)` in SVG coordinates.
    pub(super) fn unit(self) -> (f64, f64) {
        match self {
            Dir::North => (0.0, -1.0),
            Dir::South => (0.0, 1.0),
            Dir::East => (1.0, 0.0),
            Dir::West => (-1.0, 0.0),
        }
    }
}

impl From<Side> for Dir {
    /// A port's outward normal: the direction a wire leaves the box.
    fn from(side: Side) -> Dir {
        match side {
            Side::North => Dir::North,
            Side::South => Dir::South,
            Side::East => Dir::East,
            Side::West => Dir::West,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit() -> Rect {
        Rect::new(10.0, 10.0, 20.0, 20.0) // covers (10,10)..(30,30)
    }

    #[test]
    fn inflated_grows_on_every_side() {
        let r = unit().inflated(5.0);
        assert_eq!(r, Rect::new(5.0, 5.0, 30.0, 30.0));
    }

    #[test]
    fn contains_point_excludes_edges() {
        let r = unit();
        assert!(r.contains_point(Point::new(20.0, 20.0)));
        assert!(!r.contains_point(Point::new(10.0, 20.0))); // on west edge
        assert!(!r.contains_point(Point::new(0.0, 0.0)));
    }

    #[test]
    fn segment_through_interior_blocks() {
        let r = unit();
        assert!(r.blocks_segment(Point::new(0.0, 20.0), Point::new(40.0, 20.0)));
        assert!(r.blocks_segment(Point::new(20.0, 0.0), Point::new(20.0, 40.0)));
    }

    #[test]
    fn segment_along_edge_does_not_block() {
        let r = unit();
        // Along the north edge (y == 10).
        assert!(!r.blocks_segment(Point::new(0.0, 10.0), Point::new(40.0, 10.0)));
        // Outside entirely.
        assert!(!r.blocks_segment(Point::new(0.0, 5.0), Point::new(40.0, 5.0)));
        // Touching the west edge from outside, ending on it.
        assert!(!r.blocks_segment(Point::new(0.0, 20.0), Point::new(10.0, 20.0)));
    }
}
