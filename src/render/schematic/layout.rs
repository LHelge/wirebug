//! Component/port placement: walking a model + view once to produce
//! positioned boxes and ports in SVG world coordinates.

use std::collections::HashMap;

use indexmap::IndexMap;

use super::{
    CHAR_WIDTH, LABEL_INSET, MIN_HEIGHT, MIN_WIDTH, PORT_SPACING, SIDE_MARGIN, SVG_MARGIN,
};
use crate::model::{Component, ComponentId, Model, PortRef};
use crate::view::{ComponentPortLayout, ConnectorPortRef, Point, Side, View};

/// Geometry for a single component box and all its placed ports, in
/// world (SVG) coordinates.
pub(super) struct PlacedComponent {
    pub(super) origin: Point,
    pub(super) width: f64,
    pub(super) height: f64,
    pub(super) label: String,
    pub(super) ports: Vec<PlacedPort>,
    port_index: HashMap<ConnectorPortRef, usize>,
}

pub(super) struct PlacedPort {
    pub(super) cp: ConnectorPortRef,
    pub(super) side: Side,
    pub(super) pos: Point,
    pub(super) pin: Option<String>,
    pub(super) label: String,
}

/// Everything the renderer needs after walking the model + view once.
pub(super) struct Placement {
    pub(super) components: IndexMap<ComponentId, PlacedComponent>,
}

pub(super) struct ViewBox {
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) width: f64,
    pub(super) height: f64,
}

impl Placement {
    pub(super) fn compute(model: &Model, view: &View) -> Self {
        let mut components = IndexMap::new();
        let empty_layout = ComponentPortLayout::default();

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
                        .lookup(&cp.connector, &cp.port)
                        .and_then(|info| info.pin)
                        .map(str::to_owned);
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

    pub(super) fn endpoint(&self, port: &PortRef) -> Option<&PlacedPort> {
        let comp = self.components.get(&port.component)?;
        let cp = ConnectorPortRef {
            connector: port.connector.clone(),
            port: port.port.clone(),
        };
        let idx = comp.port_index.get(&cp)?;
        Some(&comp.ports[*idx])
    }

    pub(super) fn viewbox(&self, has_title: bool) -> ViewBox {
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
    component: &Component,
    cid: &ComponentId,
    layout: &ComponentPortLayout,
) -> (f64, f64) {
    let label = component.label.as_deref().unwrap_or(cid.as_ref());
    let label_w = label.chars().count() as f64 * CHAR_WIDTH + 2.0 * SIDE_MARGIN;

    let top_w = side_required(layout.north.len());
    let bot_w = side_required(layout.south.len());
    let width = MIN_WIDTH.max(label_w).max(top_w).max(bot_w);

    let left_h = side_required(layout.west.len());
    let right_h = side_required(layout.east.len());

    // North/south labels are drawn vertically (rotated 90°) so the
    // longest one dictates how far it reaches into the box.
    let top_label_h = vertical_label_extent(&layout.north);
    let bot_label_h = vertical_label_extent(&layout.south);
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
        Side::North => Point::new(origin.x + SIDE_MARGIN + along(span_h), origin.y),
        Side::South => Point::new(origin.x + SIDE_MARGIN + along(span_h), origin.y + h),
        Side::West => Point::new(origin.x, origin.y + SIDE_MARGIN + along(span_v)),
        Side::East => Point::new(origin.x + w, origin.y + SIDE_MARGIN + along(span_v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_position_distributes_evenly_on_a_side() {
        let origin = Point::ORIGIN;
        let w = 200.0;
        let h = 150.0;
        let first = port_position(origin, w, h, Side::North, 0, 3);
        let last = port_position(origin, w, h, Side::North, 2, 3);
        assert_eq!(first.y, 0.0);
        assert_eq!(last.y, 0.0);
        assert_eq!(first.x, SIDE_MARGIN);
        assert_eq!(last.x, w - SIDE_MARGIN);
    }
}
