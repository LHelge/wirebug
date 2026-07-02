//! SVG emission: turning placed components, ports, and routed wires
//! into `svg` crate elements.

use svg::node::element::{Circle, Group, Polyline, Rectangle, Text};

use super::layout::{PlacedComponent, PlacedPort, PlacedText};
use super::{COMPONENT_TITLE_GAP, LABEL_INSET, PIN_INSET, PORT_RADIUS};
use crate::dsl::ir::InstanceName;
use crate::render::color::iec_code;
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
pub(super) fn render_wire(path: &[Point], color: &str) -> Polyline {
    let points = path
        .iter()
        .map(|p| format!("{},{}", p.x, p.y))
        .collect::<Vec<_>>()
        .join(" ");
    Polyline::new()
        .set("class", "wire")
        .set("data-color", color)
        .set("points", points)
}

/// The wire's color code (IEC 60757 where known, the authored name
/// otherwise), centred on the longest segment of the routed polyline. The
/// text is haloed (`paint-order: stroke`), so sitting directly on the line
/// breaks it legibly, like a road label on a map. Vertical segments rotate
/// the code to read along the wire. `None` for a degenerate path.
pub(super) fn render_wire_code(path: &[Point], color: &str) -> Option<Text> {
    let length = |seg: &(Point, Point)| (seg.1.x - seg.0.x).abs() + (seg.1.y - seg.0.y).abs();
    let (a, b) = path
        .windows(2)
        .map(|w| (w[0], w[1]))
        .max_by(|p, q| length(p).total_cmp(&length(q)))?;
    let mid = Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);

    let mut text = Text::new(iec_code(color))
        .set("class", "wire-code")
        .set("x", mid.x)
        .set("y", mid.y)
        .set("dominant-baseline", "central");
    if (b.x - a.x).abs() < (b.y - a.y).abs() {
        text = text.set("transform", format!("rotate(-90 {} {})", mid.x, mid.y));
    }
    Some(text)
}
