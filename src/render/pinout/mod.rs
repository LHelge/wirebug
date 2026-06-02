//! Connector pinout renderer.
//!
//! A `pinout` view documents connectors on the view subject itself. Each
//! `include <connector> at (x, y);` becomes a titled pin table rendered from
//! the harness side. Physical cavity-face layouts land in a later slice; this
//! renderer gives the model a useful visual endpoint now.

use svg::Document;
use svg::node::element::{Group, Line, Rectangle, Style, Text};

use crate::dsl::ir::{Connector, ConnectorName, Design, Instance, PortName, View};
use crate::error::{Error, Result};
use crate::render::stamp::{STAMP_HEIGHT, STAMP_INSET, stamp_element};

const DEFAULT_GRID: f64 = 20.0;
const HEADER_HEIGHT: f64 = 44.0;
const ROW_HEIGHT: f64 = 22.0;
const PIN_COL_WIDTH: f64 = 34.0;
const PAD_X: f64 = 10.0;
const MIN_WIDTH: f64 = 180.0;
const CHAR_WIDTH: f64 = 7.0;
const SVG_MARGIN: f64 = 48.0;
const TITLE_GAP: f64 = 12.0;

const STYLE: &str = "\
.pinout rect { fill: white; stroke: black; stroke-width: 1.5; }
.pinout .header { fill: #f0f0f0; }
.pinout-title { font: bold 13px sans-serif; text-anchor: middle; }
.pinout-part { font: 10px sans-serif; text-anchor: middle; fill: #555; }
.row-sep { stroke: #ddd; stroke-width: 1; }
.pin-num { font: italic 10px sans-serif; fill: #555; text-anchor: middle; dominant-baseline: central; }
.pin-label { font: 11px sans-serif; dominant-baseline: central; }
.title { font: bold 14px sans-serif; }
.stamp { font: 10px sans-serif; fill: #666; text-anchor: end; }\
";

/// SVG renderer for `kind: pinout` views.
#[derive(Default)]
pub struct PinoutRenderer;

impl PinoutRenderer {
    pub(super) fn render(
        &self,
        design: &Design,
        subject: &Instance,
        view: &View,
        embed: bool,
    ) -> Result<String> {
        let step = view.grid.unwrap_or(DEFAULT_GRID);
        if step <= 0.0 {
            return Err(Error::NonPositiveGrid { grid: step });
        }

        let mut tables = Vec::new();
        for inc in &view.includes {
            let name = inc.instance.as_str();
            let connector_name = ConnectorName::from(name);
            let connector =
                subject
                    .connectors
                    .get(&connector_name)
                    .ok_or_else(|| Error::UnknownConnector {
                        subject: subject.type_name.to_string(),
                        connector: name.to_string(),
                    })?;
            tables.push(Table::new(connector, subject, inc.x * step, inc.y * step));
        }

        let has_title = !view.title.is_empty();
        let mut vb = viewbox(&tables, has_title);
        let manifest = (!embed).then_some(design.manifest.as_ref()).flatten();
        if manifest.is_some() {
            vb.height += STAMP_HEIGHT;
        }

        let mut doc = Document::new()
            .set("xmlns", "http://www.w3.org/2000/svg")
            .set(
                "viewBox",
                format!("{} {} {} {}", vb.x, vb.y, vb.width, vb.height),
            );
        if embed {
            doc = doc.set("class", "wirebug wirebug-pinout");
        } else {
            doc = doc.add(Style::new(STYLE));
        }

        if has_title {
            doc = doc.add(
                Text::new(view.title.clone())
                    .set("class", "title")
                    .set("x", vb.x + SVG_MARGIN)
                    .set("y", vb.y + SVG_MARGIN - TITLE_GAP),
            );
        }

        if let Some(manifest) = manifest {
            doc = doc.add(stamp_element(
                manifest,
                vb.x + vb.width - STAMP_INSET,
                vb.y + vb.height - STAMP_INSET,
            ));
        }

        let mut group = Group::new().set("class", "pinouts");
        for table in &tables {
            group = group.add(render_table(table));
        }
        Ok(doc.add(group).to_string())
    }
}

struct Table {
    title: String,
    subtitle: String,
    rows: Vec<Row>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Table {
    fn new(connector: &Connector, subject: &Instance, cx: f64, cy: f64) -> Self {
        let title = connector.name.to_string();
        let subtitle = connector_subtitle(connector);
        let rows = connector
            .pins
            .iter()
            .map(|pin| {
                let port = subject.ports.get(&pin.port);
                Row {
                    pin: pin.pin.to_string(),
                    label: port_label(&pin.port, port),
                }
            })
            .collect::<Vec<_>>();

        let max_chars = std::iter::once(title.chars().count())
            .chain(std::iter::once(subtitle.chars().count()))
            .chain(
                rows.iter()
                    .map(|r| r.label.chars().count() + r.pin.chars().count()),
            )
            .max()
            .unwrap_or(0);
        let width = MIN_WIDTH.max(max_chars as f64 * CHAR_WIDTH + PIN_COL_WIDTH + 3.0 * PAD_X);
        let row_count = rows.len().max(1);
        let height = HEADER_HEIGHT + row_count as f64 * ROW_HEIGHT;

        Self {
            title,
            subtitle,
            rows,
            x: cx - width / 2.0,
            y: cy - height / 2.0,
            width,
            height,
        }
    }
}

#[derive(Clone)]
struct Row {
    pin: String,
    label: String,
}

#[derive(Clone, Copy)]
struct ViewBox {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn connector_subtitle(connector: &Connector) -> String {
    match connector.properties.get("part") {
        Some(crate::dsl::ir::ConnectorPropertyValue::Str(part)) => {
            format!("{} · {}", connector.description, part)
        }
        _ => connector.description.clone(),
    }
}

fn port_label(name: &PortName, port: Option<&crate::dsl::ir::Port>) -> String {
    match port {
        Some(port) if port.label == name.as_str() => port.label.clone(),
        Some(port) => format!("{} · {}", name, port.label),
        None => name.to_string(),
    }
}

fn render_table(table: &Table) -> Group {
    let mut group = Group::new()
        .set("class", "pinout")
        .add(
            Rectangle::new()
                .set("x", table.x)
                .set("y", table.y)
                .set("width", table.width)
                .set("height", table.height),
        )
        .add(
            Rectangle::new()
                .set("class", "header")
                .set("x", table.x)
                .set("y", table.y)
                .set("width", table.width)
                .set("height", HEADER_HEIGHT),
        )
        .add(
            Text::new(table.title.clone())
                .set("class", "pinout-title")
                .set("x", table.x + table.width / 2.0)
                .set("y", table.y + HEADER_HEIGHT * 0.42),
        )
        .add(
            Text::new(table.subtitle.clone())
                .set("class", "pinout-part")
                .set("x", table.x + table.width / 2.0)
                .set("y", table.y + HEADER_HEIGHT * 0.78),
        );

    let rows = if table.rows.is_empty() {
        vec![Row {
            pin: String::new(),
            label: "No pins assigned".to_string(),
        }]
    } else {
        table
            .rows
            .iter()
            .map(|r| Row {
                pin: r.pin.clone(),
                label: r.label.clone(),
            })
            .collect()
    };

    for (i, row) in rows.iter().enumerate() {
        let row_top = table.y + HEADER_HEIGHT + i as f64 * ROW_HEIGHT;
        let row_y = row_top + ROW_HEIGHT / 2.0;
        if i > 0 {
            group = group.add(
                Line::new()
                    .set("class", "row-sep")
                    .set("x1", table.x)
                    .set("y1", row_top)
                    .set("x2", table.x + table.width)
                    .set("y2", row_top),
            );
        }
        group = group
            .add(
                Text::new(row.pin.clone())
                    .set("class", "pin-num")
                    .set("x", table.x + PIN_COL_WIDTH / 2.0)
                    .set("y", row_y),
            )
            .add(
                Text::new(row.label.clone())
                    .set("class", "pin-label")
                    .set("x", table.x + PIN_COL_WIDTH + PAD_X)
                    .set("y", row_y),
            );
    }

    group
}

fn viewbox(tables: &[Table], has_title: bool) -> ViewBox {
    if tables.is_empty() {
        return ViewBox {
            x: 0.0,
            y: 0.0,
            width: SVG_MARGIN * 2.0,
            height: SVG_MARGIN * 2.0,
        };
    }
    let min_x = tables.iter().map(|t| t.x).fold(f64::INFINITY, f64::min);
    let min_y = tables.iter().map(|t| t.y).fold(f64::INFINITY, f64::min);
    let max_x = tables
        .iter()
        .map(|t| t.x + t.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = tables
        .iter()
        .map(|t| t.y + t.height)
        .fold(f64::NEG_INFINITY, f64::max);

    let title_space = if has_title {
        TITLE_GAP + ROW_HEIGHT
    } else {
        0.0
    };
    ViewBox {
        x: min_x - SVG_MARGIN,
        y: min_y - SVG_MARGIN - title_space,
        width: max_x - min_x + 2.0 * SVG_MARGIN,
        height: max_y - min_y + 2.0 * SVG_MARGIN + title_space,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::ir::{Include, InstanceName, TypeName, ViewKind};
    use crate::render::schematic::tests::design_from;

    fn pinout_view(subject: &str, includes: &[(&str, f64, f64)]) -> View {
        View {
            kind: ViewKind::Pinout,
            title: "Pinout".to_string(),
            grid: None,
            subject: TypeName::from(subject),
            has_enclosure: false,
            enclosure: Vec::new(),
            includes: includes
                .iter()
                .map(|(connector, x, y)| Include {
                    instance: InstanceName::from(*connector),
                    connector: None,
                    x: *x,
                    y: *y,
                    ports: Vec::new(),
                })
                .collect(),
            texts: Vec::new(),
        }
    }

    fn render(design: &Design, view: &View, embed: bool) -> String {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        PinoutRenderer
            .render(design, subject, view, embed)
            .expect("renders")
    }

    #[test]
    fn renders_connector_pin_table() {
        let design = design_from(
            r#"
connector_type ampseal "AMPSEAL" { part: "TE 776164-1"; }
component m {
    pub port can_h "CAN H";
    pub port can_l "CAN L";
    connector x1: ampseal {
        pin 1 = can_h;
        pin 2 = can_l;
    }
}
"#,
        );
        let svg = render(&design, &pinout_view("m", &[("x1", 0.0, 0.0)]), false);

        assert!(svg.contains("class=\"pinout\""));
        assert!(svg.contains("x1"));
        assert!(svg.contains("AMPSEAL"));
        assert!(svg.contains("TE 776164-1"));
        assert!(svg.contains("can_h · CAN H"));
        assert!(svg.contains("can_l · CAN L"));
    }

    #[test]
    fn embed_mode_tags_root_and_omits_style() {
        let design = design_from(
            r#"
component m {
    connector x1 "Legacy 1p" { pub port p "P" pin 1; }
}
"#,
        );
        let svg = render(&design, &pinout_view("m", &[("x1", 0.0, 0.0)]), true);

        assert!(svg.contains("class=\"wirebug wirebug-pinout\""));
        assert!(!svg.contains("<style>"));
    }

    #[test]
    fn unknown_connector_errors_at_render_time() {
        let design = design_from("component m { }\n");
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == TypeName::from("m"))
            .expect("subject");
        let err = PinoutRenderer
            .render(
                &design,
                subject,
                &pinout_view("m", &[("x1", 0.0, 0.0)]),
                false,
            )
            .expect_err("unknown connector");
        assert!(matches!(err, Error::UnknownConnector { .. }));
    }
}
