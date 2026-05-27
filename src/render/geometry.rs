//! Shared rendering geometry: a world-space point. The point is a pure
//! presentation concept (the DSL IR knows nothing about where anything is
//! drawn); [`Side`] is now authored in the view and carried in the IR, so
//! we re-export it here for the renderer's convenience.

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

pub use crate::dsl::ir::Side;
