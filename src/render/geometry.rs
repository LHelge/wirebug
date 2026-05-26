//! Shared rendering geometry: a world-space point and the four sides of a
//! component box. These are presentation concepts, owned by the renderer
//! (the DSL IR knows nothing about where anything is drawn).

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

/// Which side of a component rectangle a port sits on, named by compass
/// direction. In SVG coordinates y grows downward, so North is the top
/// edge and South the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    West,
    East,
    North,
    South,
}
