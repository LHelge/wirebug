//! Rectangle-based SVG schematic renderer.
//!
//! Each component becomes a box with ports distributed evenly along the
//! sides the view places them on, and every connection becomes an
//! orthogonal polyline between two ports.
//!
//! - `layout` turns a model + view into positioned boxes and ports.
//! - `draw` emits the SVG for those boxes, ports, and wires.
//! - `route` finds object-avoiding orthogonal paths for the wires.

mod draw;
mod layout;
mod route;

use svg::Document;
use svg::node::element::{Group, Style, Text};

use super::Renderer;
use crate::error::Result;
use crate::model::Model;
use crate::view::View;

use layout::Placement;
use route::Router;

pub(super) const MIN_WIDTH: f64 = 160.0;
pub(super) const MIN_HEIGHT: f64 = 100.0;
pub(super) const PORT_SPACING: f64 = 28.0;
pub(super) const SIDE_MARGIN: f64 = 26.0;
pub(super) const LABEL_INSET: f64 = 10.0;
pub(super) const PIN_INSET: f64 = 6.0;
pub(super) const PORT_RADIUS: f64 = 4.0;
pub(super) const CHAR_WIDTH: f64 = 7.5;
pub(super) const COMPONENT_TITLE_GAP: f64 = 8.0;
pub(super) const SVG_MARGIN: f64 = 48.0;
pub(super) const TITLE_GAP: f64 = 12.0;

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
            components_group = components_group.add(draw::render_component(cid, pc));
        }
        doc = doc.add(components_group);

        let router = Router::build(&placement);
        let mut wires_group = Group::new().set("class", "wires");
        for connection in &model.connections {
            let Some(a) = placement.endpoint(&connection.from) else {
                continue;
            };
            let Some(b) = placement.endpoint(&connection.to) else {
                continue;
            };
            wires_group = wires_group.add(draw::render_wire(&router.route(a, b)));
        }
        doc = doc.add(wires_group);

        Ok(doc.to_string())
    }
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
    east: [j.out]
  b:
    west: [j.in]
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
    east: [j.out]
"#
        .parse()
        .unwrap();

        let svg = SchematicRenderer.render(&model, &view).unwrap();
        // No wire group should contain a polyline since b isn't placed.
        assert!(!svg.contains("class=\"wire\""));
    }
}
