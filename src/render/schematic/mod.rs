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
use crate::error::{Error, Result};
use crate::model::Model;
use crate::view::View;

use layout::{Grid, Placement};
use route::Router;

pub(super) const MIN_WIDTH: f64 = 160.0;
pub(super) const MIN_HEIGHT: f64 = 100.0;
/// Tightest spacing between adjacent ports at which their labels and pin
/// numbers still clear each other (labels are 11px, pins 10px). Ports sit
/// two grid steps apart, so the grid step must be at least half this; a
/// finer grid errors rather than overlapping labels.
pub(super) const MIN_PORT_PITCH: f64 = 15.0;
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
        let step = view.grid_step();
        if step <= 0.0 {
            return Err(Error::NonPositiveGrid { grid: step });
        }
        // Ports sit two steps apart, so that pitch must clear a port
        // label or adjacent labels would overlap.
        let min_step = MIN_PORT_PITCH / 2.0;
        if step < min_step {
            return Err(Error::GridTooSmall {
                grid: step,
                minimum: min_step,
            });
        }
        let grid = Grid::new(step);
        let placement = Placement::compute(model, view, grid)?;

        // Route before sizing the canvas: wires can detour outside the
        // component bounds (e.g. a bundle dropping below two south-facing
        // ports), so the viewBox has to enclose them too.
        let router = Router::build(&placement, step);
        let pairs: Vec<_> = model
            .connections
            .iter()
            .filter_map(|c| Some((placement.endpoint(&c.from)?, placement.endpoint(&c.to)?)))
            .collect();
        let wires = router.route_all(&pairs, step);

        let mut doc = Document::new()
            .set("xmlns", "http://www.w3.org/2000/svg")
            .add(Style::new(STYLE));

        let viewbox = placement.viewbox(view.title.is_some(), &wires);
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

        let mut wires_group = Group::new().set("class", "wires");
        for polyline in &wires {
            wires_group = wires_group.add(draw::render_wire(polyline));
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
    fn grid_finer_than_min_port_pitch_errors() {
        let model: Model = r#"
components:
  a:
    connectors: { j: { ports: { out: "1" } } }
connections: []
"#
        .parse()
        .unwrap();
        // Pitch is 2 steps, so a step of 5 gives pitch 10 < MIN_PORT_PITCH.
        let view: View = r#"
kind: schematic
grid: 5
layout:
  a: { x: 0, y: 0 }
ports:
  a: { east: [j.out] }
"#
        .parse()
        .unwrap();

        assert!(matches!(
            SchematicRenderer.render(&model, &view),
            Err(Error::GridTooSmall { .. })
        ));
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
