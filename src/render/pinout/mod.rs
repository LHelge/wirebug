//! Connector pinout renderer.
//!
//! A `pinout` view documents connectors on the view subject itself. Each
//! `include <connector> at (x, y);` becomes a titled pin table rendered from
//! the harness side, optionally with a physical cavity face above the pin
//! table when the connector type carries a layout.

use svg::Document;
use svg::node::element::{Group, Line, Rectangle, Style, Text};

use crate::dsl::ir::{
    Connector, ConnectorLayout, ConnectorName, Design, Instance, Pin, PortName, View,
};
use crate::error::{Error, Result};
use crate::render::SvgMode;
use crate::render::stamp::{STAMP_HEIGHT, STAMP_INSET, stamp_element};

const DEFAULT_GRID: f64 = 20.0;
const HEADER_HEIGHT: f64 = 44.0;
const ROW_HEIGHT: f64 = 22.0;
const PIN_COL_WIDTH: f64 = 34.0;
const PAD_X: f64 = 10.0;
const MIN_WIDTH: f64 = 180.0;
const CAVITY_SIZE: f64 = 38.0;
const CAVITY_GAP: f64 = 6.0;
const FACE_PAD: f64 = 12.0;
const CHAR_WIDTH: f64 = 7.0;
const SVG_MARGIN: f64 = 48.0;
const TITLE_GAP: f64 = 12.0;

pub(super) const STYLE: &str = "\
.pinout rect { fill: white; stroke: black; stroke-width: 1.5; }
.pinout .header { fill: #f0f0f0; }
.pinout-title { font: bold 13px sans-serif; text-anchor: middle; }
.pinout-part { font: 10px sans-serif; text-anchor: middle; fill: #555; }
.row-sep { stroke: #ddd; stroke-width: 1; }
.pin-num { font: italic 10px sans-serif; fill: #555; text-anchor: middle; dominant-baseline: central; }
.pin-label { font: 11px sans-serif; dominant-baseline: central; }
.cavity { fill: white; stroke: black; stroke-width: 1.25; }
.cavity-pin { font: bold 11px sans-serif; text-anchor: middle; dominant-baseline: central; }
.cavity-label { font: 8px sans-serif; text-anchor: middle; dominant-baseline: central; fill: #555; }
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
        mode: SvgMode,
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

        let has_title = !view.title.is_empty() && mode.titled();
        let mut vb = viewbox(&tables, has_title);
        let manifest = mode.stamped().then_some(design.manifest.as_ref()).flatten();
        if manifest.is_some() {
            vb.height += STAMP_HEIGHT;
        }

        let mut doc = Document::new()
            .set("xmlns", "http://www.w3.org/2000/svg")
            .set(
                "viewBox",
                format!("{} {} {} {}", vb.x, vb.y, vb.width, vb.height),
            );
        if mode.is_embed() {
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
    face: Option<Face>,
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
        let face = Face::from_connector(connector, subject);

        let max_chars = std::iter::once(title.chars().count())
            .chain(std::iter::once(subtitle.chars().count()))
            .chain(
                rows.iter()
                    .map(|r| r.label.chars().count() + r.pin.chars().count()),
            )
            .max()
            .unwrap_or(0);
        let table_width =
            MIN_WIDTH.max(max_chars as f64 * CHAR_WIDTH + PIN_COL_WIDTH + 3.0 * PAD_X);
        let face_width = face.as_ref().map_or(0.0, |f| f.width() + 2.0 * FACE_PAD);
        let width = table_width.max(face_width);
        let row_count = rows.len().max(1);
        let face_height = face.as_ref().map_or(0.0, |f| f.height() + FACE_PAD);
        let height = HEADER_HEIGHT + face_height + row_count as f64 * ROW_HEIGHT;

        Self {
            title,
            subtitle,
            face,
            rows,
            x: cx - width / 2.0,
            y: cy - height / 2.0,
            width,
            height,
        }
    }
}

struct Face {
    rows: u32,
    cols: u32,
    cells: Vec<Cell>,
}

impl Face {
    fn from_connector(connector: &Connector, subject: &Instance) -> Option<Self> {
        match connector.layout.as_ref()? {
            ConnectorLayout::Grid(layout) => {
                let numbering = GridNumbering::from(layout.numbering.as_deref());
                let cells = numbering
                    .positions(layout.rows, layout.cols)
                    .into_iter()
                    .enumerate()
                    .map(|(index, (x, y))| {
                        let pin = (index + 1) as u32;
                        let port = connector
                            .pins
                            .iter()
                            .find(|binding| binding.pin == Pin(pin))
                            .and_then(|binding| subject.ports.get(&binding.port));
                        Cell {
                            pin: pin.to_string(),
                            label: port.map(|p| p.label.clone()),
                            x,
                            y,
                            size: None,
                        }
                    })
                    .collect();
                Some(Self {
                    rows: layout.rows,
                    cols: layout.cols,
                    cells,
                })
            }
            ConnectorLayout::Face(layout) => {
                let min_x = layout
                    .cavities
                    .iter()
                    .map(|cavity| cavity.x as u32)
                    .min()
                    .unwrap_or(0);
                let min_y = layout
                    .cavities
                    .iter()
                    .map(|cavity| cavity.y as u32)
                    .min()
                    .unwrap_or(0);
                let cells = layout
                    .cavities
                    .iter()
                    .map(|cavity| {
                        let port = connector
                            .pins
                            .iter()
                            .find(|binding| binding.pin == cavity.pin)
                            .and_then(|binding| subject.ports.get(&binding.port));
                        Cell {
                            pin: cavity.pin.to_string(),
                            label: port.map(|p| p.label.clone()),
                            x: (cavity.x as u32).saturating_sub(min_x),
                            y: (cavity.y as u32).saturating_sub(min_y),
                            size: cavity.size.clone(),
                        }
                    })
                    .collect::<Vec<_>>();
                let cols = cells.iter().map(Cell::span_right).max().unwrap_or(0);
                let rows = cells.iter().map(Cell::span_bottom).max().unwrap_or(0);
                Some(Self { rows, cols, cells })
            }
        }
    }

    fn width(&self) -> f64 {
        self.cols as f64 * CAVITY_SIZE + (self.cols.saturating_sub(1)) as f64 * CAVITY_GAP
    }

    fn height(&self) -> f64 {
        self.rows as f64 * CAVITY_SIZE + (self.rows.saturating_sub(1)) as f64 * CAVITY_GAP
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GridNumbering {
    RowMajor,
    OddEven,
    Clockwise,
    CounterClockwise,
}

impl GridNumbering {
    fn from(value: Option<&str>) -> Self {
        match value {
            Some("odd_even") => Self::OddEven,
            Some("clockwise") => Self::Clockwise,
            Some("counter_clockwise") => Self::CounterClockwise,
            _ => Self::RowMajor,
        }
    }

    fn positions(self, rows: u32, cols: u32) -> Vec<(u32, u32)> {
        match self {
            Self::RowMajor => row_major_positions(rows, cols),
            Self::OddEven => odd_even_positions(rows, cols),
            Self::Clockwise => spiral_positions(rows, cols, SpiralDirection::Clockwise),
            Self::CounterClockwise => counter_clockwise_positions(rows, cols),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum SpiralDirection {
    Clockwise,
    CounterClockwise,
}

fn row_major_positions(rows: u32, cols: u32) -> Vec<(u32, u32)> {
    (0..rows)
        .flat_map(|y| (0..cols).map(move |x| (x, y)))
        .collect()
}

fn odd_even_positions(rows: u32, cols: u32) -> Vec<(u32, u32)> {
    (0..cols)
        .flat_map(|x| (0..rows).map(move |y| (x, y)))
        .collect()
}

fn counter_clockwise_positions(rows: u32, cols: u32) -> Vec<(u32, u32)> {
    if rows <= 2 {
        return (0..rows)
            .flat_map(|y| {
                if y == 0 {
                    EitherRange::Descending((0..cols).rev())
                } else {
                    EitherRange::Ascending(0..cols)
                }
                .map(move |x| (x, y))
            })
            .collect();
    }

    if cols <= 2 {
        return odd_even_positions(rows, cols);
    }

    spiral_positions(rows, cols, SpiralDirection::CounterClockwise)
}

enum EitherRange<A, D> {
    Ascending(A),
    Descending(D),
}

impl<A, D> Iterator for EitherRange<A, D>
where
    A: Iterator<Item = u32>,
    D: Iterator<Item = u32>,
{
    type Item = u32;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Ascending(iter) => iter.next(),
            Self::Descending(iter) => iter.next(),
        }
    }
}

fn spiral_positions(rows: u32, cols: u32, direction: SpiralDirection) -> Vec<(u32, u32)> {
    if rows == 0 || cols == 0 {
        return Vec::new();
    }

    let mut positions = Vec::with_capacity(rows as usize * cols as usize);
    let (mut left, mut top) = (0, 0);
    let (mut right, mut bottom) = (cols - 1, rows - 1);

    while left <= right && top <= bottom {
        match direction {
            SpiralDirection::Clockwise => {
                positions.extend((left..=right).map(|x| (x, top)));
                positions.extend(((top + 1)..=bottom).map(|y| (right, y)));
                if top < bottom {
                    positions.extend((left..right).rev().map(|x| (x, bottom)));
                }
                if left < right {
                    positions.extend(((top + 1)..bottom).rev().map(|y| (left, y)));
                }
            }
            SpiralDirection::CounterClockwise => {
                positions.extend((top..=bottom).map(|y| (left, y)));
                positions.extend(((left + 1)..=right).map(|x| (x, bottom)));
                if left < right {
                    positions.extend((top..bottom).rev().map(|y| (right, y)));
                }
                if top < bottom {
                    positions.extend(((left + 1)..right).rev().map(|x| (x, top)));
                }
            }
        }

        left += 1;
        top += 1;
        right = match right.checked_sub(1) {
            Some(next) => next,
            None => break,
        };
        bottom = match bottom.checked_sub(1) {
            Some(next) => next,
            None => break,
        };
    }

    positions.truncate(rows as usize * cols as usize);
    positions
}

struct Cell {
    pin: String,
    label: Option<String>,
    x: u32,
    y: u32,
    size: Option<String>,
}

impl Cell {
    fn span(&self) -> u32 {
        match self.size.as_deref() {
            Some("large") => 2,
            _ => 1,
        }
    }

    fn span_right(&self) -> u32 {
        self.x + self.span()
    }

    fn span_bottom(&self) -> u32 {
        self.y + self.span()
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
    let part = match connector.properties.get("part") {
        Some(crate::dsl::ir::ConnectorPropertyValue::Str(part)) => Some(part.as_str()),
        _ => None,
    };
    match (connector.description.as_deref(), part) {
        (Some(d), Some(p)) => format!("{d} · {p}"),
        (Some(d), None) => d.to_string(),
        (None, Some(p)) => p.to_string(),
        (None, None) => String::new(),
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

    let rows_top = table.y
        + HEADER_HEIGHT
        + table
            .face
            .as_ref()
            .map_or(0.0, |face| face.height() + FACE_PAD);

    if let Some(face) = &table.face {
        group = group.add(render_face(
            face,
            table.x + (table.width - face.width()) / 2.0,
            table.y + HEADER_HEIGHT + FACE_PAD / 2.0,
        ));
    }

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
        let row_top = rows_top + i as f64 * ROW_HEIGHT;
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

fn render_face(face: &Face, x: f64, y: f64) -> Group {
    let mut group = Group::new().set("class", "pinout-face");
    for cell in &face.cells {
        let span = cell.span();
        let size = span as f64 * CAVITY_SIZE + (span.saturating_sub(1)) as f64 * CAVITY_GAP;
        let cx = x + cell.x as f64 * (CAVITY_SIZE + CAVITY_GAP);
        let cy = y + cell.y as f64 * (CAVITY_SIZE + CAVITY_GAP);
        group = group
            .add(
                Rectangle::new()
                    .set("class", "cavity")
                    .set("x", cx)
                    .set("y", cy)
                    .set("width", size)
                    .set("height", size),
            )
            .add(
                Text::new(cell.pin.clone())
                    .set("class", "cavity-pin")
                    .set("x", cx + size / 2.0)
                    .set("y", cy + size * 0.42),
            );
        if let Some(label) = &cell.label {
            group = group.add(
                Text::new(label.clone())
                    .set("class", "cavity-label")
                    .set("x", cx + size / 2.0)
                    .set("y", cy + size * 0.72),
            );
        }
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
                    half: None,
                    x: *x,
                    y: *y,
                    ports: Vec::new(),
                })
                .collect(),
            texts: Vec::new(),
        }
    }

    fn render(design: &Design, view: &View, mode: SvgMode) -> String {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == view.subject)
            .expect("subject instance");
        PinoutRenderer
            .render(design, subject, view, mode)
            .expect("renders")
    }

    #[test]
    fn renders_connector_pin_table() {
        let design = design_from(
            r#"
connector_type ampseal "AMPSEAL" {
    part: "TE 776164-1";
    layout grid {
        rows: 1;
        cols: 2;
        numbering: row_major;
    }
}
component m {
    connector x1: ampseal {
        pub port can_h "CAN H" pin 1;
        pub port can_l "CAN L" pin 2;
    }
}
"#,
        );
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );

        assert!(svg.contains("class=\"pinout\""));
        assert!(svg.contains("x1"));
        assert!(svg.contains("AMPSEAL"));
        assert!(svg.contains("TE 776164-1"));
        assert!(svg.contains("class=\"pinout-face\""));
        assert_eq!(svg.matches("class=\"cavity\"").count(), 2);
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
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Embed,
        );

        assert!(svg.contains("class=\"wirebug wirebug-pinout\""));
        assert!(!svg.contains("<style>"));
    }

    #[test]
    fn renders_explicit_face_layout_with_large_cavity() {
        let design = design_from(
            r#"
connector_type control "Control" {
    layout face {
        cavity 47 at (0, 0) size large;
        cavity 1 at (3, 0);
    }
}
component m {
    connector x1: control {
        pub port hv_aux "HV AUX" pin 47;
        pub port wake "WAKE" pin 1;
    }
}
"#,
        );
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );

        assert!(svg.contains("class=\"pinout-face\""));
        assert_eq!(svg.matches("class=\"cavity\"").count(), 2);
        assert!(svg.contains("47"));
        assert!(svg.contains("HV AUX"));
        assert!(svg.contains("WAKE"));
    }

    #[test]
    fn wide_face_layout_has_symmetric_side_padding() {
        let design = design_from(
            r#"
connector_type wide "Wide" {
    layout face {
        cavity 1 at (0, 0);
        cavity 2 at (5, 0);
    }
}
component m {
    connector x1: wide {
    }
}
"#,
        );
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == TypeName::from("m"))
            .expect("subject");
        let connector = subject
            .connectors
            .get(&ConnectorName::from("x1"))
            .expect("connector");

        let table = Table::new(connector, subject, 0.0, 0.0);
        let face = table.face.as_ref().expect("face");

        assert_eq!(table.width, face.width() + 2.0 * FACE_PAD);
    }

    #[test]
    fn explicit_face_layout_sizes_to_occupied_cavities() {
        let design = design_from(
            r#"
connector_type offset "Offset" {
    layout face {
        cavity 1 at (4, 2);
        cavity 2 at (6, 2);
    }
}
component m {
    connector x1: offset {
    }
}
"#,
        );
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == TypeName::from("m"))
            .expect("subject");
        let connector = subject
            .connectors
            .get(&ConnectorName::from("x1"))
            .expect("connector");
        let face = Face::from_connector(connector, subject).expect("face");

        assert_eq!(face.cols, 3);
        assert_eq!(face.rows, 1);
        assert_eq!(face.cells[0].x, 0);
        assert_eq!(face.cells[1].x, 2);
    }

    #[test]
    fn odd_even_grid_numbering_places_pin_pairs_in_columns() {
        let positions = GridNumbering::OddEven.positions(2, 4);

        assert_eq!(
            positions,
            vec![
                (0, 0),
                (0, 1),
                (1, 0),
                (1, 1),
                (2, 0),
                (2, 1),
                (3, 0),
                (3, 1)
            ]
        );
    }

    #[test]
    fn clockwise_grid_numbering_wraps_around_the_face() {
        let positions = GridNumbering::Clockwise.positions(2, 4);

        assert_eq!(
            positions,
            vec![
                (0, 0),
                (1, 0),
                (2, 0),
                (3, 0),
                (3, 1),
                (2, 1),
                (1, 1),
                (0, 1)
            ]
        );
    }

    #[test]
    fn counter_clockwise_grid_numbering_keeps_wide_sides_grouped() {
        let positions = GridNumbering::CounterClockwise.positions(2, 4);

        assert_eq!(
            positions,
            vec![
                (3, 0),
                (2, 0),
                (1, 0),
                (0, 0),
                (0, 1),
                (1, 1),
                (2, 1),
                (3, 1)
            ]
        );
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
                SvgMode::Standalone,
            )
            .expect_err("unknown connector");
        assert!(matches!(err, Error::UnknownConnector { .. }));
    }
}
