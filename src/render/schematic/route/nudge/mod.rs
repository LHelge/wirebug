//! Connector nudging — paper §6.
//!
//! Routing (§4–5) sends every connector through the same visibility
//! graph, so wires that run along a common channel land exactly on top
//! of one another. This pass pulls them apart for legibility:
//!
//! - `segments` collapses each route into maximal H/V segments and finds
//!   the OVG edges shared by two or more connectors.
//! - `order` (§6.1) fixes the order of connectors within each shared
//!   channel so a bundle fans out without crossing itself.
//! - `place` (§6.2) assigns every segment its final coordinate via the
//!   `vpsc` separation-constraint solver: shared segments are nudged a
//!   minimum distance apart, port-touching segments stay pinned.

mod order;
mod place;
mod segments;
mod vpsc;

use super::RawRoute;
use super::collapse_collinear;
use super::geometry::Rect;
use super::visibility::Ovg;
use crate::render::geometry::Point;
use segments::Segment;

/// Coordinate tolerance shared across the nudge submodules.
pub(super) const EPS: f64 = 1e-6;

/// Run the full §6 pipeline, returning one finished polyline per route in
/// the input order. `gap` is the minimum spacing between parallel wires
/// in a shared channel (the view's grid step).
pub(super) fn run(ovg: &Ovg, obstacles: &[Rect], raws: &[RawRoute], gap: f64) -> Vec<Vec<Point>> {
    let node_paths: Vec<Vec<usize>> = raws.iter().map(|r| r.nodes.clone()).collect();

    let mut shapes: Vec<Option<Vec<Segment>>> = raws
        .iter()
        .map(|r| Some(segments::segmentize(ovg, r)))
        .collect();

    let channels = order::channels(ovg, &node_paths);
    place::place(&mut shapes, &channels, raws, obstacles, gap);

    raws.iter()
        .zip(shapes.iter())
        .map(|(r, shape)| match shape {
            Some(segs) => place::rebuild(r, segs),
            None => collapse_collinear(vec![r.a, r.b]),
        })
        .collect()
}
