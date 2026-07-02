//! SVG emission for harness drawings: connector pin tables, cable boxes on
//! the spine, and the bezier wires that flex between them.

use svg::node::element::{Circle, Group, Line, Path, Rectangle, Text};

use super::bezier::{FLEX, flex};
use super::layout::{CableBox, ConnectorNode, LooseWire};
use super::{HEADER_HEIGHT, NODE_PAD, PIN_COL_WIDTH, PIN_DOT_RADIUS, ROW_HEIGHT};
use crate::render::color::iec_code;
use crate::render::geometry::{Point, Side};

/// A `cable-wire` path with the given SVG path data, stroked in `color`.
/// The color also rides along as `data-color`, so a host stylesheet can
/// theme strands by color in embed mode (where the stroke default is gone).
fn wire_path(d: String, color: &str) -> Path {
    Path::new()
        .set("class", "cable-wire")
        .set("stroke", color)
        .set("data-color", color)
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

/// A declared cable as a titled box with one coloured strand per row: each
/// strand flexes in from its left connector, runs straight across the box, and
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

    let left_edge = Point::new(ox, 0.0);
    let right_edge = Point::new(ox + cb.width, 0.0);
    for strand in &cb.strands {
        let entry = Point::new(left_edge.x, strand.row_y);
        let exit = Point::new(right_edge.x, strand.row_y);

        let lead_in = flex(strand.left_attach, entry, FLEX).path_d();
        let run = format!("M{},{} L{},{}", entry.x, entry.y, exit.x, exit.y);
        let lead_out = flex(exit, strand.right_attach, FLEX).path_d();
        group = group
            .add(casing_path(lead_in.clone()))
            .add(casing_path(run.clone()))
            .add(casing_path(lead_out.clone()))
            .add(wire_path(lead_in, &strand.color))
            .add(wire_path(run, &strand.color))
            .add(wire_path(lead_out, &strand.color));

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
    Group::new()
        .set("class", "cable")
        .add(casing_path(curve.path_d()))
        .add(wire_path(curve.path_d(), &wire.color))
        .add(
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
pub(super) fn wire_annotation(label: Option<&str>, gauge: f64, color: &str) -> String {
    let tail = format!("{gauge}mm² · {}", iec_code(color));
    match label {
        Some(l) => format!("{l} · {tail}"),
        None => tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_combines_label_gauge_and_color_code() {
        assert_eq!(
            wire_annotation(Some("HV+"), 50.0, "orange"),
            "HV+ · 50mm² · OG"
        );
        assert_eq!(wire_annotation(None, 0.25, "white"), "0.25mm² · WH");
        assert_eq!(
            wire_annotation(None, 1.0, "chartreuse"),
            "1mm² · chartreuse"
        );
    }
}
