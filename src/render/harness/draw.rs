//! SVG emission for harness drawings: connector pin tables and the cable
//! bundles between them.

use svg::node::element::{Circle, Group, Line, Polyline, Rectangle, Text};

use super::layout::{Cable, CableBox, ConnectorNode, cable_label_anchor, cable_path};
use super::{HEADER_HEIGHT, NODE_PAD, PIN_COL_WIDTH, PIN_DOT_RADIUS, ROW_HEIGHT};
use crate::render::geometry::{Point, Side};

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

    // The pin column runs along the facing edge; the label column fills the
    // rest. The numbers sit nearest the cables.
    let facing_west = node.facing == Side::West;
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

    for (i, row) in node.pins.iter().enumerate() {
        let row_top = oy + HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
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

/// A cable as a bundle of colored, gauged, optionally-labelled wires.
pub(super) fn render_cable(cable: &Cable) -> Group {
    let mut group = Group::new().set("class", "cable");
    let n = cable.wires.len();
    for (k, w) in cable.wires.iter().enumerate() {
        let pts = cable_path(w.from, w.to, k, n);
        let points = pts
            .iter()
            .map(|p| format!("{},{}", p.x, p.y))
            .collect::<Vec<_>>()
            .join(" ");
        group = group.add(
            Polyline::new()
                .set("class", "cable-wire")
                .set("stroke", w.color.clone())
                .set("points", points),
        );

        let anchor = cable_label_anchor(w.from, w.to, k, n);
        group = group.add(
            Text::new(wire_annotation(w.label.as_deref(), w.gauge))
                .set("class", "cable-label")
                .set("x", anchor.x)
                .set("y", anchor.y - 2.0),
        );
    }
    group
}

/// A declared cable as a titled box with one coloured strand per row: each
/// strand leads in from its left connector, runs straight across the box, and
/// leads out to its right connector.
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

    let left_edge = ox;
    let right_edge = ox + cb.width;
    for strand in &cb.strands {
        let lead_in = cable_path(
            strand.left_attach,
            Point::new(left_edge, strand.row_y),
            0,
            1,
        );
        let through = [
            Point::new(left_edge, strand.row_y),
            Point::new(right_edge, strand.row_y),
        ];
        let lead_out = cable_path(
            Point::new(right_edge, strand.row_y),
            strand.right_attach,
            0,
            1,
        );

        for pts in [lead_in.as_slice(), through.as_slice(), lead_out.as_slice()] {
            let points = pts
                .iter()
                .map(|p| format!("{},{}", p.x, p.y))
                .collect::<Vec<_>>()
                .join(" ");
            group = group.add(
                Polyline::new()
                    .set("class", "cable-wire")
                    .set("stroke", strand.color.clone())
                    .set("points", points),
            );
        }

        group = group.add(
            Text::new(wire_annotation(strand.label.as_deref(), strand.gauge))
                .set("class", "cable-label")
                .set("x", ox + cb.width / 2.0)
                .set("y", strand.row_y - 4.0),
        );
    }

    group
}

/// The text shown along a wire: `<label> · <gauge>mm²`, or just the gauge
/// when the wire is unlabelled.
pub(super) fn wire_annotation(label: Option<&str>, gauge: f64) -> String {
    let gauge = format!("{}mm²", trim_float(gauge));
    match label {
        Some(l) => format!("{l} · {gauge}"),
        None => gauge,
    }
}

/// Format a gauge without a trailing `.0` (so `50.0` reads `50`, `0.25`
/// stays `0.25`).
fn trim_float(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotation_combines_label_and_gauge() {
        assert_eq!(wire_annotation(Some("HV+"), 50.0), "HV+ · 50mm²");
        assert_eq!(wire_annotation(None, 0.25), "0.25mm²");
    }
}
