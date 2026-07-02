//! SVG emission for harness drawings: connector pin tables, cable boxes on
//! the spine, and the bezier wires that flex between them.

use svg::node::element::{Circle, Group, Line, Path, Rectangle, Text};

use super::bezier::{FLEX, flex};
use super::layout::{CableBox, ConnectorNode, LooseWire};
use super::{HEADER_HEIGHT, NODE_PAD, PIN_COL_WIDTH, PIN_DOT_RADIUS, ROW_HEIGHT};
use crate::dsl::ir::WireColor;
use crate::render::geometry::{Point, Side};

/// A `cable-wire` path with the given SVG path data, stroked in the
/// color's base. The full authored color rides along as `data-color`, so a
/// host stylesheet can theme strands by color in embed mode (where the
/// stroke default is gone).
fn wire_path(d: String, color: &WireColor) -> Path {
    Path::new()
        .set("class", "cable-wire")
        .set("stroke", color.css())
        .set("data-color", color.to_string())
        .set("d", d)
}

/// The tracer stripe of a two-tone wire: the same path as the core,
/// stroked in the tracer color with a dash pattern (set in the style
/// block), so the strand reads as base-with-stripe, WireViz-style.
fn tracer_path(d: String, color: &WireColor, tracer: &str) -> Path {
    Path::new()
        .set("class", "cable-wire-tracer")
        .set("stroke", tracer)
        .set("data-color", color.to_string())
        .set("d", d)
}

/// The black casing drawn under a strand — the same path, stroked wider —
/// so any core color, white included, reads against any background
/// (WireViz's trick). Every casing of a strand is emitted before its cores,
/// or a later casing would overdraw the core at the segment joins.
fn casing_path(d: String) -> Path {
    Path::new().set("class", "cable-wire-casing").set("d", d)
}

/// A connector as a titled pin table: header (instance + connector), then
/// one row per pin (number + label), with an attach dot on the facing edge.
pub(super) fn render_node(node: &ConnectorNode) -> Group {
    let (ox, oy) = (node.origin.x, node.origin.y);

    let outline = Rectangle::new()
        .set("x", ox)
        .set("y", oy)
        .set("width", node.width)
        .set("height", node.height);

    let header_bg = Rectangle::new()
        .set("class", "header")
        .set("x", ox)
        .set("y", oy)
        .set("width", node.width)
        .set("height", HEADER_HEIGHT);

    let title = Text::new(node.title.clone())
        .set("class", "connector-title")
        .set("x", ox + node.width / 2.0)
        .set("y", oy + HEADER_HEIGHT * 0.42);
    let subtitle = Text::new(node.subtitle.clone())
        .set("class", "connector-part")
        .set("x", ox + node.width / 2.0)
        .set("y", oy + HEADER_HEIGHT * 0.78);

    let mut group = Group::new()
        .set("class", "connector")
        .add(outline)
        .add(header_bg)
        .add(title)
        .add(subtitle);

    for (i, row) in node.pins.iter().enumerate() {
        let row_top = oy + HEADER_HEIGHT + i as f64 * ROW_HEIGHT;

        // Each pin's number column hugs the edge it leaves by (nearest its
        // cable); the label column fills the rest. Pins of one node can leave
        // by different edges, so this is decided per row, not per node.
        let facing_west = row.side == Side::West;
        let pin_col_x = if facing_west {
            ox + PIN_COL_WIDTH / 2.0
        } else {
            ox + node.width - PIN_COL_WIDTH / 2.0
        };
        let (label_x, label_anchor) = if facing_west {
            (ox + PIN_COL_WIDTH + NODE_PAD, "start")
        } else {
            (ox + node.width - PIN_COL_WIDTH - NODE_PAD, "end")
        };

        if i > 0 {
            group = group.add(
                Line::new()
                    .set("class", "row-sep")
                    .set("x1", ox)
                    .set("y1", row_top)
                    .set("x2", ox + node.width)
                    .set("y2", row_top),
            );
        }

        if let Some(pin) = &row.pin {
            group = group.add(
                Text::new(pin.clone())
                    .set("class", "pin-num")
                    .set("x", pin_col_x)
                    .set("y", row.y)
                    .set("dominant-baseline", "central"),
            );
        }
        group = group.add(
            Text::new(row.label.clone())
                .set("class", "pin-label")
                .set("text-anchor", label_anchor)
                .set("x", label_x)
                .set("y", row.y)
                .set("dominant-baseline", "central"),
        );
        group = group.add(
            Circle::new()
                .set("class", "pin-dot")
                .set("cx", row.attach.x)
                .set("cy", row.attach.y)
                .set("r", PIN_DOT_RADIUS),
        );
    }

    group
}

/// Nominal width of one half-twist in a braided (twisted-pair) box run.
/// The actual pitch stretches to divide the run into a whole, even count.
const TWIST_PITCH: f64 = 28.0;

/// The path data for one strand of a two-strand braid: chained
/// horizontally-flexed cubics (the same shape as [`flex`]) alternating
/// between the strand's own row and its partner's. The half-twist count is
/// even, so the strand exits the box on the row it entered and the
/// lead-out bezier stays untangled.
fn braid_d(from: Point, to_x: f64, other_y: f64) -> String {
    let width = to_x - from.x;
    let n = ((width / TWIST_PITCH) as usize).max(2) / 2 * 2;
    let step = width / n as f64;

    let mut d = format!("M{},{}", from.x, from.y);
    let (mut y, mut other) = (from.y, other_y);
    for i in 0..n {
        let x0 = from.x + i as f64 * step;
        let x1 = if i == n - 1 { to_x } else { x0 + step };
        d.push_str(&format!(
            " C{},{} {},{} {},{}",
            x0 + FLEX * step,
            y,
            x1 - FLEX * step,
            other,
            x1,
            other
        ));
        std::mem::swap(&mut y, &mut other);
    }
    d
}

/// A declared cable as a titled box with one coloured strand per row: each
/// strand flexes in from its left connector, runs across the box — straight,
/// or braided with its partner when the cable is a twisted pair — and
/// flexes out to its right connector.
pub(super) fn render_cable_box(cb: &CableBox) -> Group {
    let (ox, oy) = (cb.origin.x, cb.origin.y);

    let mut group = Group::new()
        .set("class", "connector cable-box")
        .add(
            Rectangle::new()
                .set("x", ox)
                .set("y", oy)
                .set("width", cb.width)
                .set("height", cb.height),
        )
        .add(
            Rectangle::new()
                .set("class", "header")
                .set("x", ox)
                .set("y", oy)
                .set("width", cb.width)
                .set("height", HEADER_HEIGHT),
        )
        .add(
            Text::new(cb.title.clone())
                .set("class", "connector-title")
                .set("x", ox + cb.width / 2.0)
                .set("y", oy + HEADER_HEIGHT * 0.42),
        );
    if !cb.subtitle.is_empty() {
        group = group.add(
            Text::new(cb.subtitle.clone())
                .set("class", "connector-part")
                .set("x", ox + cb.width / 2.0)
                .set("y", oy + HEADER_HEIGHT * 0.78),
        );
    }

    // The braid drawing needs a partner row to swap with, so it applies
    // to exactly two strands; a twisted cable with another conductor
    // count still draws straight rows.
    let braided = cb.twisted && cb.strands.len() == 2;

    let left_edge = Point::new(ox, 0.0);
    let right_edge = Point::new(ox + cb.width, 0.0);
    for (i, strand) in cb.strands.iter().enumerate() {
        let entry = Point::new(left_edge.x, strand.row_y);
        let exit = Point::new(right_edge.x, strand.row_y);

        let lead_in = flex(strand.left_attach, entry, FLEX).path_d();
        let run = if braided {
            braid_d(entry, exit.x, cb.strands[1 - i].row_y)
        } else {
            format!("M{},{} L{},{}", entry.x, entry.y, exit.x, exit.y)
        };
        let lead_out = flex(exit, strand.right_attach, FLEX).path_d();
        group = group
            .add(casing_path(lead_in.clone()))
            .add(casing_path(run.clone()))
            .add(casing_path(lead_out.clone()))
            .add(wire_path(lead_in.clone(), &strand.color))
            .add(wire_path(run.clone(), &strand.color))
            .add(wire_path(lead_out.clone(), &strand.color));
        if let Some(tracer) = strand.color.tracer() {
            group = group
                .add(tracer_path(lead_in, &strand.color, tracer))
                .add(tracer_path(run, &strand.color, tracer))
                .add(tracer_path(lead_out, &strand.color, tracer));
        }

        group = group.add(
            Text::new(wire_annotation(
                strand.label.as_deref(),
                strand.gauge,
                &strand.color,
            ))
            .set("class", "cable-label")
            .set("x", ox + cb.width / 2.0)
            .set("y", strand.row_y - 4.0),
        );
    }

    group
}

/// A loose wire (no cable): a single flexed bezier pin-to-pin, annotated at
/// its midpoint.
pub(super) fn render_loose(wire: &LooseWire) -> Group {
    let curve = flex(wire.from, wire.to, FLEX);
    let mid = curve.point_at(0.5);
    let mut group = Group::new()
        .set("class", "cable")
        .add(casing_path(curve.path_d()))
        .add(wire_path(curve.path_d(), &wire.color));
    if let Some(tracer) = wire.color.tracer() {
        group = group.add(tracer_path(curve.path_d(), &wire.color, tracer));
    }
    group.add(
        Text::new(wire_annotation(
            wire.label.as_deref(),
            wire.gauge,
            &wire.color,
        ))
        .set("class", "cable-label")
        .set("x", mid.x)
        .set("y", mid.y - 2.0),
    )
}

/// The text shown along a wire: `<label> · <gauge>mm² · <color code>`,
/// dropping the label part when the wire is unlabelled. The color code is
/// IEC 60757 where known (`render/color.rs`), so the strand stays
/// identifiable in grayscale print where the stroke color doesn't help.
/// `f64`'s shortest-round-trip formatting already drops a trailing `.0`
/// (`50.0` → `50`, `0.25` stays `0.25`).
pub(super) fn wire_annotation(label: Option<&str>, gauge: f64, color: &WireColor) -> String {
    let tail = format!("{gauge}mm² · {}", color.code());
    match label {
        Some(l) => format!("{l} · {tail}"),
        None => tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn braid_has_an_even_half_twist_count_and_exits_on_its_own_row() {
        let d = braid_d(Point::new(0.0, 10.0), 100.0, 30.0);
        assert!(d.starts_with("M0,10"));
        // 100 / 28 → 3, rounded down to 2 half-twists.
        assert_eq!(d.matches(" C").count(), 2);
        // Even count: the last segment lands back on the strand's own row
        // at the box's right edge.
        assert!(d.ends_with("100,10"), "{d}");
    }

    #[test]
    fn braid_alternates_between_the_two_rows() {
        let d = braid_d(Point::new(0.0, 10.0), 200.0, 30.0);
        // 200 / 28 → 7, rounded down to 6 half-twists, ~33.3 wide each:
        // the strand touches the partner row at every odd crossover.
        assert_eq!(d.matches(" C").count(), 6);
        assert!(d.contains(",30 "), "{d}");
        assert!(d.ends_with(",10"), "{d}");
    }

    #[test]
    fn short_braid_still_gets_two_half_twists() {
        let d = braid_d(Point::new(0.0, 10.0), 30.0, 30.0);
        assert_eq!(d.matches(" C").count(), 2);
        assert!(d.ends_with("30,10"), "{d}");
    }

    #[test]
    fn annotation_combines_label_gauge_and_color_code() {
        assert_eq!(
            wire_annotation(Some("HV+"), 50.0, &"orange".into()),
            "HV+ · 50mm² · OG"
        );
        assert_eq!(wire_annotation(None, 0.25, &"white".into()), "0.25mm² · WH");
        assert_eq!(
            wire_annotation(None, 1.0, &"chartreuse".into()),
            "1mm² · chartreuse"
        );
    }
}
