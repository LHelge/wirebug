//! Shared rendering geometry: a world-space point and an axis-aligned box.
//! Both are pure presentation concepts (the DSL IR knows nothing about
//! where anything is drawn); [`Side`] is now authored in the view and
//! carried in the IR, so we re-export it here for the renderer's
//! convenience.

/// 2D point in SVG user-space units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const ORIGIN: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// An axis-aligned box in world coordinates: a drawn component, pin table
/// or cable box, without the rest of what the renderer knows about it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bounds {
    pub(crate) origin: Point,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// Slack for comparing computed edges: two boxes whose edges meet after
/// float arithmetic (`centre ± width / 2`) still count as touching.
const EPS: f64 = 1e-6;

impl Bounds {
    /// Whether the two boxes share interior area. Boxes that only touch
    /// along an edge or at a corner do not overlap.
    pub(crate) fn overlaps(&self, other: &Bounds) -> bool {
        self.origin.x < other.origin.x + other.width - EPS
            && other.origin.x < self.origin.x + self.width - EPS
            && self.origin.y < other.origin.y + other.height - EPS
            && other.origin.y < self.origin.y + self.height - EPS
    }
}

pub use crate::dsl::ir::Side;

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64, width: f64, height: f64) -> Bounds {
        Bounds {
            origin: Point::new(x, y),
            width,
            height,
        }
    }

    #[test]
    fn boxes_sharing_area_overlap() {
        let a = at(0.0, 0.0, 100.0, 50.0);
        assert!(a.overlaps(&at(90.0, 40.0, 100.0, 50.0)));
        assert!(at(90.0, 40.0, 100.0, 50.0).overlaps(&a));
        // One box wholly inside the other.
        assert!(a.overlaps(&at(10.0, 10.0, 20.0, 20.0)));
    }

    #[test]
    fn boxes_touching_along_an_edge_or_corner_do_not_overlap() {
        let a = at(0.0, 0.0, 100.0, 50.0);
        assert!(!a.overlaps(&at(100.0, 0.0, 100.0, 50.0)));
        assert!(!a.overlaps(&at(0.0, 50.0, 100.0, 50.0)));
        assert!(!a.overlaps(&at(100.0, 50.0, 10.0, 10.0)));
        // An edge that meets only after float rounding still touches.
        assert!(!a.overlaps(&at(100.0 + 1e-9, 0.0, 100.0, 50.0)));
        assert!(!a.overlaps(&at(100.0 - 1e-9, 0.0, 100.0, 50.0)));
    }

    #[test]
    fn separated_boxes_do_not_overlap() {
        let a = at(0.0, 0.0, 100.0, 50.0);
        assert!(!a.overlaps(&at(200.0, 0.0, 10.0, 10.0)));
        // Overlapping on one axis only is not enough.
        assert!(!a.overlaps(&at(50.0, 80.0, 10.0, 10.0)));
    }
}
