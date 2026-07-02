//! SVG emission: turning placed components, ports, and routed wires
//! into `svg` crate elements.

use svg::node::element::{Circle, Group, Polyline, Rectangle, Text};

use super::layout::{PlacedComponent, PlacedPort, PlacedText};
use super::{COMPONENT_TITLE_GAP, LABEL_INSET, PIN_INSET, PORT_RADIUS};
use crate::dsl::ir::{InstanceName, WireColor};
use crate::render::geometry::{Point, Side};

pub(super) fn render_component(cid: &InstanceName, pc: &PlacedComponent) -> Group {
    let rect = Rectangle::new()
        .set("x", pc.origin.x)
        .set("y", pc.origin.y)
        .set("width", pc.width)
        .set("height", pc.height);

    // Component title sits above the box (KiCad-style) so it doesn't
    // collide with rotated port labels that extend inward from the
    // top edge.
    let label = Text::new(pc.label.clone())
        .set("class", "component-label")
        .set("x", pc.origin.x + pc.width / 2.0)
        .set("y", pc.origin.y - COMPONENT_TITLE_GAP);

    let mut ports_group = Group::new().set("class", "ports");
    for port in &pc.ports {
        ports_group = ports_group.add(render_port(port));
    }

    Group::new()
        .set("class", "component")
        .set("data-component", cid.to_string())
        .add(rect)
        .add(label)
        .add(ports_group)
}

/// The subject's boundary box: a wrapper rectangle with the subject's own
/// ports on its edges, facing inward. Port label/pin placement is the same as
/// a component's — the label insets toward the interior (the schematic) and
/// the pin sits outside (the exterior), which reads as an inverted port.
pub(super) fn render_enclosure(pc: &PlacedComponent) -> Group {
    let rect = Rectangle::new()
        .set("x", pc.origin.x)
        .set("y", pc.origin.y)
        .set("width", pc.width)
        .set("height", pc.height);

    let label = Text::new(pc.label.clone())
        .set("class", "enclosure-label")
        .set("x", pc.origin.x + pc.width / 2.0)
        .set("y", pc.origin.y - COMPONENT_TITLE_GAP);

    let mut ports_group = Group::new().set("class", "ports");
    for port in &pc.ports {
        ports_group = ports_group.add(render_port(port));
    }

    Group::new()
        .set("class", "enclosure")
        .add(rect)
        .add(label)
        .add(ports_group)
}

pub(super) fn render_text_box(text: &PlacedText) -> Group {
    let rect = Rectangle::new()
        .set("x", text.origin.x)
        .set("y", text.origin.y)
        .set("width", text.width)
        .set("height", text.height)
        .set("rx", 4)
        .set("ry", 4);

    let label = Text::new(text.label.clone())
        .set("x", text.origin.x + text.width / 2.0)
        .set("y", text.origin.y + text.height / 2.0);

    Group::new()
        .set("class", "text-box")
        .set("data-text", text.name.clone())
        .add(rect)
        .add(label)
}

fn render_port(p: &PlacedPort) -> Group {
    let circle = Circle::new()
        .set("cx", p.pos.x)
        .set("cy", p.pos.y)
        .set("r", PORT_RADIUS);

    // An inverted boundary port faces the interior, so it labels like a
    // normal port on the opposite side: the name sits outside the boundary,
    // the pin number inside.
    let side = if p.inverted {
        p.side.opposite()
    } else {
        p.side
    };

    let label = text_with_placement(
        p.label.clone(),
        "port-label",
        inside_label_placement(side, p.pos),
    );

    let mut group = Group::new()
        .set("class", "port")
        .set("data-port", p.port.to_string())
        .add(circle)
        .add(label);

    if let Some(pin) = &p.pin {
        let pin_text =
            text_with_placement(pin.clone(), "port-pin", outside_pin_placement(side, p.pos));
        group = group.add(pin_text);
    }

    group
}

/// Placement for a text label: anchor position, text-anchor,
/// optional rotation, and optional dominant-baseline override.
/// Rotation is in degrees, clockwise.
struct LabelPlacement {
    x: f64,
    y: f64,
    anchor: &'static str,
    rotate: f64,
    baseline: Option<&'static str>,
}

fn text_with_placement(content: String, class: &'static str, lp: LabelPlacement) -> Text {
    let mut text = Text::new(content)
        .set("class", class)
        .set("text-anchor", lp.anchor)
        .set("x", lp.x)
        .set("y", lp.y);
    if let Some(baseline) = lp.baseline {
        text = text.set("dominant-baseline", baseline);
    }
    if lp.rotate != 0.0 {
        text = text.set(
            "transform",
            format!("rotate({} {} {})", lp.rotate, lp.x, lp.y),
        );
    }
    text
}

/// Port-name label placement. North/south labels are rotated 90° so
/// adjacent ports don't overlap horizontally. All sides use
/// `dominant-baseline="central"` so the text's cross-axis center
/// aligns with the port — no manual half-glyph fudge.
fn inside_label_placement(side: Side, pos: Point) -> LabelPlacement {
    match side {
        Side::West => LabelPlacement {
            x: pos.x + LABEL_INSET,
            y: pos.y,
            anchor: "start",
            rotate: 0.0,
            baseline: Some("central"),
        },
        Side::East => LabelPlacement {
            x: pos.x - LABEL_INSET,
            y: pos.y,
            anchor: "end",
            rotate: 0.0,
            baseline: Some("central"),
        },
        Side::North => LabelPlacement {
            x: pos.x,
            y: pos.y + LABEL_INSET,
            anchor: "start",
            rotate: 90.0,
            baseline: Some("central"),
        },
        Side::South => LabelPlacement {
            x: pos.x,
            y: pos.y - LABEL_INSET,
            anchor: "start",
            rotate: -90.0,
            baseline: Some("central"),
        },
    }
}

/// Pin-number label placement (outside the box). Kept horizontal on
/// all sides — pin numbers are short enough that they don't fight
/// each other.
fn outside_pin_placement(side: Side, pos: Point) -> LabelPlacement {
    match side {
        Side::West => LabelPlacement {
            x: pos.x - PIN_INSET,
            y: pos.y - 5.0,
            anchor: "end",
            rotate: 0.0,
            baseline: None,
        },
        Side::East => LabelPlacement {
            x: pos.x + PIN_INSET,
            y: pos.y - 5.0,
            anchor: "start",
            rotate: 0.0,
            baseline: None,
        },
        Side::North => LabelPlacement {
            x: pos.x + PIN_INSET,
            y: pos.y - 5.0,
            anchor: "start",
            rotate: 0.0,
            baseline: None,
        },
        Side::South => LabelPlacement {
            x: pos.x + PIN_INSET,
            y: pos.y + 13.0,
            anchor: "start",
            rotate: 0.0,
            baseline: None,
        },
    }
}

/// Emit a routed connector as a `<polyline>`. `path` is the ordered list
/// of points produced by [`super::route::Router::route`]. The wire's
/// authored color rides along as `data-color`, so a host stylesheet can
/// theme wires by color (`.wire[data-color="white"] { … }`) in embed mode.
pub(super) fn render_wire(path: &[Point], color: &WireColor) -> Polyline {
    let points = path
        .iter()
        .map(|p| format!("{},{}", p.x, p.y))
        .collect::<Vec<_>>()
        .join(" ");
    Polyline::new()
        .set("class", "wire")
        .set("data-color", color.as_str())
        .set("points", points)
}

/// Along-track clearance a code keeps from a crossing wire, so its halo
/// never visually cuts the crossed wire: half the code's rendered width
/// plus the halo.
const CROSSING_CLEARANCE: f64 = 14.0;
/// Along-track clearance between two codes (centre to centre), so codes on
/// parallel wires in a shared channel don't stack into a blur.
const CODE_CLEARANCE: f64 = 26.0;
/// How far off a segment's line an obstacle can sit and still block it —
/// just over half the haloed text height.
const OFF_LINE_TOLERANCE: f64 = 10.0;
/// The shortest free run worth placing a code in. Below this, the longest
/// segment's midpoint wins regardless of obstacles: a slightly broken
/// crossing beats a missing code.
const MIN_CODE_RUN: f64 = 24.0;

/// A point a wire's color code must keep clear of.
pub(super) struct CodeObstacle {
    at: Point,
    clearance: f64,
}

impl CodeObstacle {
    /// A perpendicular crossing with another wire (it sits on the wire
    /// itself, so it always blocks its surroundings).
    pub(super) fn crossing(at: Point) -> Self {
        Self {
            at,
            clearance: CROSSING_CLEARANCE,
        }
    }

    /// A code already placed on another wire (blocks only when that wire
    /// runs close enough to this one for the texts to collide).
    pub(super) fn code(at: Point) -> Self {
        Self {
            at,
            clearance: CODE_CLEARANCE,
        }
    }

    /// The span of `s ∈ [0, len]` along the axis-aligned segment `a→b`
    /// this obstacle blocks, or `None` when it sits too far off the
    /// segment's line to matter.
    fn blocked_span(&self, a: Point, b: Point, len: f64, vertical: bool) -> Option<(f64, f64)> {
        let (along, off) = if vertical {
            (
                (self.at.y - a.y) * (b.y - a.y).signum(),
                (self.at.x - a.x).abs(),
            )
        } else {
            (
                (self.at.x - a.x) * (b.x - a.x).signum(),
                (self.at.y - a.y).abs(),
            )
        };
        if off >= OFF_LINE_TOLERANCE {
            return None;
        }
        let (s0, s1) = (along - self.clearance, along + self.clearance);
        (s1 > 0.0 && s0 < len).then_some((s0.max(0.0), s1.min(len)))
    }
}

/// Where a wire's color code sits, and whether it reads along a vertical
/// run (rotated 90°).
pub(super) struct CodeAnchor {
    pub(super) at: Point,
    vertical: bool,
}

/// The anchor for a wire's color code: the midpoint of the longest
/// obstacle-free run along the routed polyline. When every free run is
/// shorter than [`MIN_CODE_RUN`], falls back to the longest segment's
/// midpoint. `None` only for a degenerate path.
pub(super) fn wire_code_anchor(path: &[Point], obstacles: &[CodeObstacle]) -> Option<CodeAnchor> {
    let mut best: Option<(f64, CodeAnchor)> = None;
    let mut fallback: Option<(f64, CodeAnchor)> = None;

    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b.x - a.x).abs() + (b.y - a.y).abs();
        if len <= 0.0 {
            continue;
        }
        let vertical = (b.x - a.x).abs() < (b.y - a.y).abs();
        let at = |s: f64| {
            let t = s / len;
            Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
        };

        if fallback.as_ref().is_none_or(|(l, _)| len > *l) {
            fallback = Some((
                len,
                CodeAnchor {
                    at: at(len / 2.0),
                    vertical,
                },
            ));
        }

        let mut blocked: Vec<(f64, f64)> = obstacles
            .iter()
            .filter_map(|o| o.blocked_span(a, b, len, vertical))
            .collect();
        blocked.sort_by(|x, y| x.0.total_cmp(&y.0));

        let mut gaps: Vec<(f64, f64)> = Vec::new();
        let mut cursor = 0.0;
        for (s0, s1) in blocked {
            if s0 > cursor {
                gaps.push((cursor, s0));
            }
            cursor = cursor.max(s1);
        }
        if cursor < len {
            gaps.push((cursor, len));
        }

        for (g0, g1) in gaps {
            let run = g1 - g0;
            if run >= MIN_CODE_RUN && best.as_ref().is_none_or(|(r, _)| run > *r) {
                best = Some((
                    run,
                    CodeAnchor {
                        at: at((g0 + g1) / 2.0),
                        vertical,
                    },
                ));
            }
        }
    }

    best.or(fallback).map(|(_, anchor)| anchor)
}

/// Every point where two routed wires cross perpendicularly: `result[i]`
/// holds the points at which other wires cross `wires[i]`. Both interiors
/// must be strict — segments merely touching at an endpoint (a chain's
/// shared port) don't count.
pub(super) fn wire_crossings(wires: &[Vec<Point>]) -> Vec<Vec<Point>> {
    let mut crossings = vec![Vec::new(); wires.len()];
    for i in 0..wires.len() {
        for j in i + 1..wires.len() {
            for si in wires[i].windows(2) {
                for sj in wires[j].windows(2) {
                    if let Some(p) = perpendicular_crossing((si[0], si[1]), (sj[0], sj[1])) {
                        crossings[i].push(p);
                        crossings[j].push(p);
                    }
                }
            }
        }
    }
    crossings
}

/// The interior crossing point of one horizontal and one vertical segment,
/// if the pair is perpendicular and actually crosses.
fn perpendicular_crossing(sa: (Point, Point), sb: (Point, Point)) -> Option<Point> {
    let horizontal = |s: &(Point, Point)| (s.1.y - s.0.y).abs() < (s.1.x - s.0.x).abs();
    let (h, v) = match (horizontal(&sa), horizontal(&sb)) {
        (true, false) => (sa, sb),
        (false, true) => (sb, sa),
        _ => return None,
    };
    let (hx0, hx1) = (h.0.x.min(h.1.x), h.0.x.max(h.1.x));
    let (vy0, vy1) = (v.0.y.min(v.1.y), v.0.y.max(v.1.y));
    let (x, y) = (v.0.x, h.0.y);
    (x > hx0 && x < hx1 && y > vy0 && y < vy1).then(|| Point::new(x, y))
}

/// The wire's color code (IEC 60757 where known, the authored name
/// otherwise), drawn at its computed anchor. The text is haloed
/// (`paint-order: stroke`), so sitting directly on the line breaks it
/// legibly, like a road label on a map; on a vertical run it rotates to
/// read along the wire.
pub(super) fn render_wire_code(anchor: &CodeAnchor, color: &WireColor) -> Text {
    let mut text = Text::new(color.code())
        .set("class", "wire-code")
        .set("x", anchor.at.x)
        .set("y", anchor.at.y)
        .set("dominant-baseline", "central");
    if anchor.vertical {
        text = text.set(
            "transform",
            format!("rotate(-90 {} {})", anchor.at.x, anchor.at.y),
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn unobstructed_code_sits_at_longest_segment_midpoint() {
        // 100-long horizontal run, then a 40-long vertical drop.
        let path = [p(0.0, 0.0), p(100.0, 0.0), p(100.0, 40.0)];
        let anchor = wire_code_anchor(&path, &[]).expect("anchor");
        assert_eq!((anchor.at.x, anchor.at.y), (50.0, 0.0));
        assert!(!anchor.vertical);
    }

    #[test]
    fn code_dodges_a_crossing_at_the_midpoint() {
        let path = [p(0.0, 0.0), p(100.0, 0.0)];
        // A crossing just left of centre blocks [26, 54]; the biggest free
        // run is [54, 100], so the code shifts right of the crossing.
        let obstacles = [CodeObstacle::crossing(p(40.0, 0.0))];
        let anchor = wire_code_anchor(&path, &obstacles).expect("anchor");
        assert_eq!((anchor.at.x, anchor.at.y), (77.0, 0.0));
    }

    #[test]
    fn fully_blocked_wire_falls_back_to_longest_segment_midpoint() {
        let path = [p(0.0, 0.0), p(30.0, 0.0)];
        // One crossing blankets the short wire; no free run clears
        // MIN_CODE_RUN, so the midpoint wins anyway.
        let obstacles = [CodeObstacle::crossing(p(15.0, 0.0))];
        let anchor = wire_code_anchor(&path, &obstacles).expect("anchor");
        assert_eq!((anchor.at.x, anchor.at.y), (15.0, 0.0));
    }

    #[test]
    fn far_off_line_obstacle_does_not_block() {
        let path = [p(0.0, 0.0), p(100.0, 0.0)];
        // A code on a distant parallel wire is no obstacle.
        let obstacles = [CodeObstacle::code(p(50.0, 40.0))];
        let anchor = wire_code_anchor(&path, &obstacles).expect("anchor");
        assert_eq!((anchor.at.x, anchor.at.y), (50.0, 0.0));
    }

    #[test]
    fn nearby_placed_code_repels_along_the_run() {
        let path = [p(0.0, 0.0), p(100.0, 0.0)];
        // A code on a wire hugging this one (same channel) pushes this
        // wire's code out of its clearance.
        let obstacles = [CodeObstacle::code(p(50.0, 4.0))];
        let anchor = wire_code_anchor(&path, &obstacles).expect("anchor");
        assert!((anchor.at.x - 50.0).abs() >= CODE_CLEARANCE / 2.0);
    }

    #[test]
    fn vertical_run_rotates_the_code() {
        let path = [p(0.0, 0.0), p(0.0, 80.0)];
        let anchor = wire_code_anchor(&path, &[]).expect("anchor");
        assert!(anchor.vertical);
        assert_eq!((anchor.at.x, anchor.at.y), (0.0, 40.0));
    }

    #[test]
    fn crossings_are_detected_per_wire() {
        let a = vec![p(0.0, 0.0), p(100.0, 0.0)];
        let b = vec![p(50.0, -50.0), p(50.0, 50.0)];
        let crossings = wire_crossings(&[a, b]);
        assert_eq!(crossings[0], vec![p(50.0, 0.0)]);
        assert_eq!(crossings[1], vec![p(50.0, 0.0)]);
    }

    #[test]
    fn touching_endpoints_are_not_crossings() {
        // `b` tees into `a`'s interior from below and stops on it; a
        // shared port, not a crossing.
        let a = vec![p(0.0, 0.0), p(100.0, 0.0)];
        let b = vec![p(50.0, 50.0), p(50.0, 0.0)];
        let crossings = wire_crossings(&[a, b]);
        assert!(crossings[0].is_empty());
        assert!(crossings[1].is_empty());
    }

    #[test]
    fn parallel_segments_never_cross() {
        let a = vec![p(0.0, 0.0), p(100.0, 0.0)];
        let b = vec![p(0.0, 10.0), p(100.0, 10.0)];
        let crossings = wire_crossings(&[a, b]);
        assert!(crossings[0].is_empty());
    }
}
