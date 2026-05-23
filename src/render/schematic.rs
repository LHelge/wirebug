//! Rectangle-based SVG schematic renderer.
//!
//! Layout is straight-forward: each component becomes a box with ports
//! distributed evenly along the sides the view places them on, and
//! every connection becomes a Manhattan-routed polyline between two
//! ports. No collision avoidance — wires may cross boxes when the
//! manual layout doesn't leave room. That's an MVP limitation.

use std::collections::HashMap;

use indexmap::IndexMap;
use svg::Document;
use svg::node::element::{Circle, Group, Polyline, Rectangle, Style, Text};

use super::Renderer;
use crate::error::Result;
use crate::model::{ComponentId, Model, PortRef};
use crate::view::{ConnectorPortRef, Point, Side, View};

const MIN_WIDTH: f64 = 160.0;
const MIN_HEIGHT: f64 = 100.0;
const PORT_SPACING: f64 = 28.0;
const SIDE_MARGIN: f64 = 26.0;
const LABEL_INSET: f64 = 10.0;
const PIN_INSET: f64 = 6.0;
const PORT_RADIUS: f64 = 4.0;
const STEPOUT: f64 = 20.0;
const CHAR_WIDTH: f64 = 7.5;
const COMPONENT_TITLE_GAP: f64 = 8.0;
const SVG_MARGIN: f64 = 48.0;
const TITLE_GAP: f64 = 12.0;

const STYLE: &str = "\
.component rect { fill: white; stroke: black; stroke-width: 1.5; }
.component-label { font: bold 13px sans-serif; text-anchor: middle; }
.port circle { fill: black; }
.port-label { font: 11px sans-serif; }
.port-pin { font: italic 10px sans-serif; fill: #555; }
.wire { fill: none; stroke: black; stroke-width: 1.25; }
.title { font: bold 14px sans-serif; }\
";

/// SVG renderer for `kind: schematic` views.
#[derive(Default)]
pub struct SchematicRenderer;

impl Renderer for SchematicRenderer {
    fn render(&self, model: &Model, view: &View) -> Result<String> {
        let placement = Placement::compute(model, view);

        let mut doc = Document::new()
            .set("xmlns", "http://www.w3.org/2000/svg")
            .add(Style::new(STYLE));

        let viewbox = placement.viewbox(view.title.is_some());
        doc = doc.set(
            "viewBox",
            format!(
                "{} {} {} {}",
                viewbox.x, viewbox.y, viewbox.width, viewbox.height
            ),
        );

        if let Some(title) = &view.title {
            doc = doc.add(
                Text::new(title.clone())
                    .set("class", "title")
                    .set("x", viewbox.x + SVG_MARGIN)
                    .set("y", viewbox.y + SVG_MARGIN - TITLE_GAP),
            );
        }

        let mut components_group = Group::new().set("class", "components");
        for (cid, pc) in &placement.components {
            components_group = components_group.add(render_component(cid, pc));
        }
        doc = doc.add(components_group);

        let mut wires_group = Group::new().set("class", "wires");
        for connection in &model.connections {
            let Some(a) = placement.endpoint(&connection.from) else {
                continue;
            };
            let Some(b) = placement.endpoint(&connection.to) else {
                continue;
            };
            wires_group = wires_group.add(render_wire(a, b));
        }
        doc = doc.add(wires_group);

        Ok(doc.to_string())
    }
}

/// Geometry for a single component box and all its placed ports, in
/// world (SVG) coordinates.
struct PlacedComponent {
    origin: Point,
    width: f64,
    height: f64,
    label: String,
    ports: Vec<PlacedPort>,
    port_index: HashMap<ConnectorPortRef, usize>,
}

struct PlacedPort {
    cp: ConnectorPortRef,
    side: Side,
    pos: Point,
    pin: Option<String>,
    label: String,
}

/// Everything the renderer needs after walking the model + view once.
struct Placement {
    components: IndexMap<ComponentId, PlacedComponent>,
}

struct ViewBox {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Placement {
    fn compute(model: &Model, view: &View) -> Self {
        let mut components = IndexMap::new();
        let empty_layout = Default::default();

        for (cid, origin) in &view.layout {
            let Some(component) = model.components.get(cid) else {
                continue;
            };
            let port_layout = view.ports.get(cid).unwrap_or(&empty_layout);

            let (width, height) = box_dimensions(component, cid, port_layout);
            let label = component.label.clone().unwrap_or_else(|| cid.to_string());

            let mut ports = Vec::new();
            let mut port_index = HashMap::new();

            for (side, refs) in port_layout.sides() {
                let n = refs.len();
                for (k, cp) in refs.iter().enumerate() {
                    let pos = port_position(*origin, width, height, side, k, n);
                    let pin = component
                        .connectors
                        .get(&cp.connector)
                        .and_then(|c| c.ports.get(&cp.port))
                        .and_then(|p| p.clone());
                    let label = cp.port.to_string();
                    port_index.insert(cp.clone(), ports.len());
                    ports.push(PlacedPort {
                        cp: cp.clone(),
                        side,
                        pos,
                        pin,
                        label,
                    });
                }
            }

            components.insert(
                cid.clone(),
                PlacedComponent {
                    origin: *origin,
                    width,
                    height,
                    label,
                    ports,
                    port_index,
                },
            );
        }

        Self { components }
    }

    fn endpoint(&self, port: &PortRef) -> Option<&PlacedPort> {
        let comp = self.components.get(&port.component)?;
        let cp = ConnectorPortRef {
            connector: port.connector.clone(),
            port: port.port.clone(),
        };
        let idx = comp.port_index.get(&cp)?;
        Some(&comp.ports[*idx])
    }

    fn viewbox(&self, has_title: bool) -> ViewBox {
        if self.components.is_empty() {
            return ViewBox {
                x: 0.0,
                y: 0.0,
                width: MIN_WIDTH,
                height: MIN_HEIGHT,
            };
        }

        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for pc in self.components.values() {
            min_x = min_x.min(pc.origin.x);
            min_y = min_y.min(pc.origin.y);
            max_x = max_x.max(pc.origin.x + pc.width);
            max_y = max_y.max(pc.origin.y + pc.height);
        }

        let pad = SVG_MARGIN;
        let title_pad = if has_title { 20.0 } else { 0.0 };
        ViewBox {
            x: min_x - pad,
            y: min_y - pad - title_pad,
            width: (max_x - min_x) + 2.0 * pad,
            height: (max_y - min_y) + 2.0 * pad + title_pad,
        }
    }
}

fn box_dimensions(
    component: &crate::model::Component,
    cid: &ComponentId,
    layout: &crate::view::ComponentPortLayout,
) -> (f64, f64) {
    let label = component.label.as_deref().unwrap_or(cid.as_ref());
    let label_w = label.chars().count() as f64 * CHAR_WIDTH + 2.0 * SIDE_MARGIN;

    let top_w = side_required(layout.top.len());
    let bot_w = side_required(layout.bottom.len());
    let width = MIN_WIDTH.max(label_w).max(top_w).max(bot_w);

    let left_h = side_required(layout.left.len());
    let right_h = side_required(layout.right.len());

    // Top/bottom labels are drawn vertically (rotated 90°) so the
    // longest one dictates how far it reaches into the box.
    let top_label_h = vertical_label_extent(&layout.top);
    let bot_label_h = vertical_label_extent(&layout.bottom);
    let vertical_label_h = SIDE_MARGIN + top_label_h + bot_label_h + SIDE_MARGIN;

    let height = MIN_HEIGHT.max(left_h).max(right_h).max(vertical_label_h);

    (width, height)
}

fn side_required(n: usize) -> f64 {
    match n {
        0 => 0.0,
        _ => (n.saturating_sub(1)) as f64 * PORT_SPACING + 2.0 * SIDE_MARGIN,
    }
}

/// Vertical span (in user units) that a rotated label needs from the
/// box edge inward, given the longest port name on the side. Returns
/// 0 when the side has no ports.
fn vertical_label_extent(refs: &[ConnectorPortRef]) -> f64 {
    let max_chars = refs
        .iter()
        .map(|cp| cp.port.as_ref().chars().count())
        .max()
        .unwrap_or(0);
    if max_chars == 0 {
        0.0
    } else {
        LABEL_INSET + max_chars as f64 * CHAR_WIDTH
    }
}

fn port_position(origin: Point, w: f64, h: f64, side: Side, k: usize, n: usize) -> Point {
    let along = |span: f64| -> f64 {
        if n <= 1 {
            span / 2.0
        } else {
            (k as f64) * span / (n - 1) as f64
        }
    };

    let span_h = (w - 2.0 * SIDE_MARGIN).max(0.0);
    let span_v = (h - 2.0 * SIDE_MARGIN).max(0.0);

    match side {
        Side::Top => Point::new(origin.x + SIDE_MARGIN + along(span_h), origin.y),
        Side::Bottom => Point::new(origin.x + SIDE_MARGIN + along(span_h), origin.y + h),
        Side::Left => Point::new(origin.x, origin.y + SIDE_MARGIN + along(span_v)),
        Side::Right => Point::new(origin.x + w, origin.y + SIDE_MARGIN + along(span_v)),
    }
}

fn render_component(cid: &ComponentId, pc: &PlacedComponent) -> Group {
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

fn render_port(p: &PlacedPort) -> Group {
    let circle = Circle::new()
        .set("cx", p.pos.x)
        .set("cy", p.pos.y)
        .set("r", PORT_RADIUS);

    let label = text_with_placement(p.label.clone(), "port-label", inside_label_placement(p));

    let mut group = Group::new()
        .set("class", "port")
        .set("data-port", p.cp.to_string())
        .add(circle)
        .add(label);

    if let Some(pin) = &p.pin {
        let pin_text = text_with_placement(pin.clone(), "port-pin", outside_pin_placement(p));
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

/// Port-name label placement. Top/bottom labels are rotated 90° so
/// adjacent ports don't overlap horizontally. All sides use
/// `dominant-baseline="central"` so the text's cross-axis center
/// aligns with the port — no manual half-glyph fudge.
fn inside_label_placement(p: &PlacedPort) -> LabelPlacement {
    match p.side {
        Side::Left => LabelPlacement {
            x: p.pos.x + LABEL_INSET,
            y: p.pos.y,
            anchor: "start",
            rotate: 0.0,
            baseline: Some("central"),
        },
        Side::Right => LabelPlacement {
            x: p.pos.x - LABEL_INSET,
            y: p.pos.y,
            anchor: "end",
            rotate: 0.0,
            baseline: Some("central"),
        },
        Side::Top => LabelPlacement {
            x: p.pos.x,
            y: p.pos.y + LABEL_INSET,
            anchor: "start",
            rotate: 90.0,
            baseline: Some("central"),
        },
        Side::Bottom => LabelPlacement {
            x: p.pos.x,
            y: p.pos.y - LABEL_INSET,
            anchor: "start",
            rotate: -90.0,
            baseline: Some("central"),
        },
    }
}

/// Pin-number label placement (outside the box). Kept horizontal on
/// all sides — pin numbers are short enough that they don't fight
/// each other.
fn outside_pin_placement(p: &PlacedPort) -> LabelPlacement {
    match p.side {
        Side::Left => LabelPlacement {
            x: p.pos.x - PIN_INSET,
            y: p.pos.y - 5.0,
            anchor: "end",
            rotate: 0.0,
            baseline: None,
        },
        Side::Right => LabelPlacement {
            x: p.pos.x + PIN_INSET,
            y: p.pos.y - 5.0,
            anchor: "start",
            rotate: 0.0,
            baseline: None,
        },
        Side::Top => LabelPlacement {
            x: p.pos.x + PIN_INSET,
            y: p.pos.y - 5.0,
            anchor: "start",
            rotate: 0.0,
            baseline: None,
        },
        Side::Bottom => LabelPlacement {
            x: p.pos.x + PIN_INSET,
            y: p.pos.y + 13.0,
            anchor: "start",
            rotate: 0.0,
            baseline: None,
        },
    }
}

fn render_wire(a: &PlacedPort, b: &PlacedPort) -> Polyline {
    let path = manhattan_route(a.pos, side_normal(a.side), b.pos, side_normal(b.side));
    let points = path
        .iter()
        .map(|p| format!("{},{}", p.x, p.y))
        .collect::<Vec<_>>()
        .join(" ");
    Polyline::new().set("class", "wire").set("points", points)
}

fn side_normal(side: Side) -> (f64, f64) {
    match side {
        Side::Left => (-1.0, 0.0),
        Side::Right => (1.0, 0.0),
        Side::Top => (0.0, -1.0),
        Side::Bottom => (0.0, 1.0),
    }
}

/// Build a Manhattan-routed polyline between two ports, given their
/// outward normals. The result always starts at `a`, steps outward to
/// `a + na*STEPOUT`, takes one or two orthogonal jogs, steps in to
/// `b + nb*STEPOUT`, and ends at `b`.
fn manhattan_route(a: Point, na: (f64, f64), b: Point, nb: (f64, f64)) -> Vec<Point> {
    let sa = Point::new(a.x + na.0 * STEPOUT, a.y + na.1 * STEPOUT);
    let sb = Point::new(b.x + nb.0 * STEPOUT, b.y + nb.1 * STEPOUT);

    let mut path = vec![a, sa];

    let a_horiz = na.0 != 0.0;
    let b_horiz = nb.0 != 0.0;

    match (a_horiz, b_horiz) {
        (true, true) => {
            let mid = (sa.x + sb.x) / 2.0;
            path.push(Point::new(mid, sa.y));
            path.push(Point::new(mid, sb.y));
        }
        (false, false) => {
            let mid = (sa.y + sb.y) / 2.0;
            path.push(Point::new(sa.x, mid));
            path.push(Point::new(sb.x, mid));
        }
        (true, false) => {
            path.push(Point::new(sb.x, sa.y));
        }
        (false, true) => {
            path.push(Point::new(sa.x, sb.y));
        }
    }

    path.push(sb);
    path.push(b);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> (Model, View) {
        let model: Model = r#"
components:
  a:
    label: "Alpha"
    connectors:
      j:
        ports:
          out: "1"
  b:
    label: "Beta"
    connectors:
      j:
        ports:
          in: "1"
connections:
  - { from: a.j.out, to: b.j.in }
"#
        .parse()
        .unwrap();

        let view: View = r#"
kind: schematic
title: "Tiny"
layout:
  a: { x: 0, y: 0 }
  b: { x: 300, y: 0 }
ports:
  a:
    right: [j.out]
  b:
    left: [j.in]
"#
        .parse()
        .unwrap();

        (model, view)
    }

    #[test]
    fn tiny_render_contains_expected_fragments() {
        let (model, view) = tiny();
        let svg = SchematicRenderer.render(&model, &view).expect("renders");

        assert!(svg.contains("<svg"));
        assert!(svg.contains("viewBox="));
        assert!(svg.contains("Alpha"));
        assert!(svg.contains("Beta"));
        assert!(svg.contains("class=\"wire\""));
        assert!(svg.contains("class=\"component\""));
        assert!(svg.contains("class=\"port\""));
    }

    #[test]
    fn manhattan_route_horizontal_pair_has_z_bend() {
        let a = Point::new(100.0, 50.0);
        let b = Point::new(300.0, 80.0);
        let path = manhattan_route(a, (1.0, 0.0), b, (-1.0, 0.0));
        // a, sa, mid_a, mid_b, sb, b == 6 points
        assert_eq!(path.len(), 6);
        // Z-bend: the two middle points share an x.
        assert!((path[2].x - path[3].x).abs() < 1e-9);
    }

    #[test]
    fn manhattan_route_l_bend_for_mixed_normals() {
        let a = Point::new(100.0, 50.0);
        let b = Point::new(200.0, 200.0);
        let path = manhattan_route(a, (1.0, 0.0), b, (0.0, 1.0));
        // a, sa, corner, sb, b == 5 points
        assert_eq!(path.len(), 5);
    }

    #[test]
    fn port_position_distributes_evenly_on_a_side() {
        let origin = Point::ORIGIN;
        let w = 200.0;
        let h = 150.0;
        let first = port_position(origin, w, h, Side::Top, 0, 3);
        let last = port_position(origin, w, h, Side::Top, 2, 3);
        assert_eq!(first.y, 0.0);
        assert_eq!(last.y, 0.0);
        assert_eq!(first.x, SIDE_MARGIN);
        assert_eq!(last.x, w - SIDE_MARGIN);
    }

    #[test]
    fn unplaced_port_is_silently_dropped_from_wires() {
        let model: Model = r#"
components:
  a:
    connectors:
      j:
        ports:
          out: "1"
  b:
    connectors:
      j:
        ports:
          in: "1"
connections:
  - { from: a.j.out, to: b.j.in }
"#
        .parse()
        .unwrap();
        // View places `a` only; the connection's `b` endpoint is hidden.
        let view: View = r#"
kind: schematic
layout:
  a: { x: 0, y: 0 }
ports:
  a:
    right: [j.out]
"#
        .parse()
        .unwrap();

        let svg = SchematicRenderer.render(&model, &view).unwrap();
        // No wire group should contain a polyline since b isn't placed.
        assert!(!svg.contains("class=\"wire\""));
    }
}
