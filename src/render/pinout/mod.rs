//! Connector pinout renderer.
//!
//! A `pinout` view documents connectors on the view subject itself. Each
//! `include <connector> at (x, y);` becomes a pin table headed by the
//! component and the connector, its rows sorted by pin, each listing what
//! is crimped into the cavity — colour, gauge and far end of every
//! conductor, and any screen's drain — from the connector's
//! [`PinTable`](crate::export::PinTable). When the connector type carries a
//! layout, its physical cavity face sits in a panel of its own on top of
//! the table. A face is always drawn from the harness side (the language's
//! convention for `layout`; it has no other) and says so in a caption.

mod fit;
mod rows;

use svg::Document;
use svg::node::element::{Group, Line, Path, Rectangle, Style, Text, Title};

use crate::dsl::ir::{
    Connector, ConnectorLayout, ConnectorName, Design, Include, Instance, Pin, View, WireColor,
};
use crate::error::{ViewErrorKind, ViewResult};
use crate::export::{PinTable, PinTables};
use crate::render::geometry::{Bounds, Point};
use crate::render::lint::{IncludeBox, ViewLayout};
use crate::render::stamp::{STAMP_HEIGHT, STAMP_INSET, stamp_element};
use crate::render::{Canvas, GridFault, SvgMode};
pub(super) use fit::MIN_LABEL_SIZE;
use fit::{FittedLabel, Font};
use rows::{Entry, Row};

/// The caption under every cavity face.
const FACE_CAPTION: &str = "Viewed from harness side";
/// The band under a face that holds [`FACE_CAPTION`].
const CAPTION_HEIGHT: f64 = 18.0;
/// Clear space between a cavity label and its cavity's border, each side.
const CAVITY_LABEL_INSET: f64 = 3.0;
const DEFAULT_GRID: f64 = 20.0;
const HEADER_HEIGHT: f64 = 44.0;
const HEADER_LINE_HEIGHT: f64 = 14.0;
/// A row's height with one entry (or none).
const ROW_HEIGHT: f64 = 22.0;
/// What each entry past a row's first adds to it: a double crimp's second
/// conductor is a sub-row of its pin's row.
const SUB_ROW_HEIGHT: f64 = 18.0;
const PIN_COL_WIDTH: f64 = 34.0;
const PAD_X: f64 = 10.0;
/// Room between a port's label and its note.
const NOTE_GAP: f64 = 6.0;
/// The narrowest each wire column is drawn, so a table whose pins carry
/// no conductor still reads as having the columns, empty.
const WIRE_MIN_WIDTH: f64 = 40.0;
const GAUGE_MIN_WIDTH: f64 = 36.0;
const FAR_MIN_WIDTH: f64 = 60.0;
/// The widest the far-end column grows; past it, far ends lose detail and
/// then are cut, rather than widen the table without bound.
const FAR_MAX_WIDTH: f64 = 280.0;
/// A wire colour's swatch, and the room between it and the colour's code.
const SWATCH_WIDTH: f64 = 16.0;
const SWATCH_HEIGHT: f64 = 9.0;
const SWATCH_GAP: f64 = 4.0;
/// What the wire column says of a pin no conductor reaches.
const UNWIRED: &str = "–";
/// What the wire column says of a screen's drain.
const DRAIN: &str = "drain";
const NO_PINS: &str = "No pins assigned";
const CAVITY_SIZE: f64 = 38.0;
const CAVITY_GAP: f64 = 6.0;
const FACE_PAD: f64 = 12.0;
const SVG_MARGIN: f64 = 48.0;
const TITLE_GAP: f64 = 12.0;

/// The fonts of the stylesheet's table text, which sizes are estimated
/// in: keep each in step with its rule in [`STYLE`].
const TITLE_FONT: Font = Font::bold(13.0);
const HEADER_LINE_FONT: Font = Font::regular(10.0);
const CAPTION_FONT: Font = Font::regular(10.0);
const LABEL_FONT: Font = Font::regular(11.0);
const NOTE_FONT: Font = Font::regular(9.0);
const WIRE_FONT: Font = Font::regular(10.0);

pub(super) const STYLE: &str = "\
.pinout rect { fill: white; stroke: black; stroke-width: 1.5; }
.pinout .header { fill: #f0f0f0; }
.pinout-title { font: bold 13px sans-serif; text-anchor: middle; }
.pinout-part { font: 10px sans-serif; text-anchor: middle; fill: #555; }
.row-sep { stroke: #ddd; stroke-width: 1; }
.pin-num { font: italic 10px sans-serif; fill: #555; text-anchor: middle; dominant-baseline: central; }
.pin-label { font: 11px sans-serif; dominant-baseline: central; }
.pin-note { font: italic 9px sans-serif; fill: #555; text-anchor: end; dominant-baseline: central; }
.pin-wire { font: 10px sans-serif; dominant-baseline: central; }
.pin-drain { font-style: italic; fill: #555; }
.pin-unwired { fill: #999; }
.pin-gauge { font: 10px sans-serif; dominant-baseline: central; }
.pin-far { font: 10px sans-serif; dominant-baseline: central; }
.wire-swatch { stroke: #333; stroke-width: 0.75; }
.wire-swatch-tracer { stroke: none; }
.wire-sep { stroke: #e4e4e4; stroke-width: 1; stroke-dasharray: 2 2; }
.cavity { fill: white; stroke: black; stroke-width: 1.25; }
.pinout rect.cavity-unused { fill: #f2f2f2; stroke: #999; stroke-width: 1; stroke-dasharray: 3 2; }
.cavity-pin { font: bold 11px sans-serif; text-anchor: middle; dominant-baseline: central; }
.cavity-pin-unused { font-weight: normal; fill: #999; }
.cavity-label { font: 8px sans-serif; text-anchor: middle; dominant-baseline: central; fill: #555; }
.pinout-caption { font: italic 10px sans-serif; text-anchor: middle; dominant-baseline: central; fill: #555; }
.title { font: bold 14px sans-serif; }
.stamp { font: 10px sans-serif; fill: #666; text-anchor: end; }\
";

/// SVG renderer for `kind: pinout` views.
#[derive(Default)]
pub struct PinoutRenderer;

impl PinoutRenderer {
    /// `view` drawn against `subject`, its rows read from `tables`, the
    /// design's pin tables (built once for every view of a run).
    pub(super) fn render<'d>(
        &self,
        design: &'d Design,
        tables: &PinTables<'d>,
        subject: &'d Instance,
        view: &View,
        mode: SvgMode,
    ) -> ViewResult<String> {
        let step = GridFault::positive_step(view, DEFAULT_GRID)?;
        let tables = Table::all(design, tables, subject, view, step)?;

        let has_title = mode.shows_title(view);
        let vb = viewbox(design, view, &tables, mode);
        let manifest = mode.stamp(design);

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

    /// Each include's box (its table and face panel), sized as
    /// [`Self::render`] sizes it from the same `tables`, and the canvas of
    /// the standalone SVG, or only the fault for a non-positive grid. An
    /// include naming no connector on the subject is skipped, and leaves no
    /// canvas: rendering reports it.
    pub(super) fn layout<'d>(
        &self,
        design: &'d Design,
        tables: &PinTables<'d>,
        subject: &'d Instance,
        view: &View,
    ) -> ViewLayout {
        let step = match GridFault::positive_step(view, DEFAULT_GRID) {
            Ok(step) => step,
            Err(fault) => return ViewLayout::bad_grid(fault),
        };
        let built: Vec<Option<Table>> = view
            .includes
            .iter()
            .map(|inc| Table::of_include(design, tables, subject, inc, step).ok())
            .collect();
        let boxes = built
            .iter()
            .enumerate()
            .filter_map(|(include, table)| {
                Some(IncludeBox {
                    include,
                    bounds: table.as_ref()?.bounds(),
                })
            })
            .collect();
        let canvas = built.into_iter().collect::<Option<Vec<_>>>().map(|tables| {
            let vb = viewbox(design, view, &tables, SvgMode::Standalone);
            Canvas::new(vb.width, vb.height)
        });
        ViewLayout {
            boxes,
            canvas,
            ..ViewLayout::default()
        }
    }
}

/// One include's drawing: the pin table (header, then rows) and, when the
/// connector type has a layout, the face's panel on top of it. Each is as
/// wide as its own content, and both are centred on the include's point.
struct Table<'d> {
    header: Header,
    face: Option<Face>,
    rows: Vec<Row<'d>>,
    columns: Columns,
    /// The table: header and rows.
    frame: Bounds,
    /// The face's panel, its bottom edge on the table's top edge.
    face_frame: Option<Bounds>,
}

impl<'d> Table<'d> {
    /// One table per include of `view`, in include order, on a grid of
    /// `step` world units; fails on an include naming no connector of
    /// `subject`.
    fn all(
        design: &'d Design,
        tables: &PinTables<'d>,
        subject: &'d Instance,
        view: &View,
        step: f64,
    ) -> ViewResult<Vec<Self>> {
        view.includes
            .iter()
            .map(|inc| Self::of_include(design, tables, subject, inc, step))
            .collect()
    }

    /// The table `inc` places: of `subject`'s connector it names, from that
    /// connector's pin table in `tables`.
    fn of_include(
        design: &'d Design,
        tables: &PinTables<'d>,
        subject: &'d Instance,
        inc: &Include,
        step: f64,
    ) -> ViewResult<Self> {
        let name = inc.instance.as_str();
        let table = tables
            .of(subject, &ConnectorName::from(name))
            .ok_or_else(|| ViewErrorKind::UnknownConnector {
                subject: subject.type_name.to_string(),
                connector: name.to_string(),
            })?;
        Ok(Self::new(design, table, inc, step))
    }

    /// The drawing of `table`, centred where `inc` places it on a grid of
    /// `step` world units: its [`Header`], its rows in pin order, and its
    /// face's panel on top.
    fn new(design: &Design, table: &PinTable<'d>, inc: &Include, step: f64) -> Self {
        let (cx, cy) = (inc.x * step, inc.y * step);
        let header = Header::new(design, table.instance, table.connector);
        let rows = Row::all(table);
        let face = Face::from_connector(table.connector, table.instance);
        let columns = Columns::of(&rows);

        let width = columns.width().max(header.width());
        let height = header.height() + rows.iter().map(Row::height).sum::<f64>().max(ROW_HEIGHT);
        let face_size = face
            .as_ref()
            .map(|face| (face.panel_width(), face.band_height()));
        let face_height = face_size.map_or(0.0, |(_, height)| height);
        let top = cy - (face_height + height) / 2.0;

        Self {
            header,
            face,
            rows,
            columns,
            frame: Bounds {
                origin: Point::new(cx - width / 2.0, top + face_height),
                width,
                height,
            },
            face_frame: face_size.map(|(width, height)| Bounds {
                origin: Point::new(cx - width / 2.0, top),
                width,
                height,
            }),
        }
    }

    /// The whole include's box: the table and the face's panel.
    fn bounds(&self) -> Bounds {
        let Some(face) = self.face_frame else {
            return self.frame;
        };
        let left = self.frame.origin.x.min(face.origin.x);
        let right = (self.frame.origin.x + self.frame.width).max(face.origin.x + face.width);
        Bounds {
            origin: Point::new(left, face.origin.y),
            width: right - left,
            height: face.height + self.frame.height,
        }
    }
}

impl Row<'_> {
    /// The row's height: one entry's, and a sub-row for each further one.
    fn height(&self) -> f64 {
        ROW_HEIGHT + self.entries.len().saturating_sub(1) as f64 * SUB_ROW_HEIGHT
    }

    /// The centre line of each of the row's entries, from the row's top.
    fn entry_offset(index: usize) -> f64 {
        ROW_HEIGHT / 2.0 + index as f64 * SUB_ROW_HEIGHT
    }
}

/// The width of each column's content after the pin's, as wide as the
/// widest cell in it (the far end's capped at [`FAR_MAX_WIDTH`]), each
/// [`PAD_X`] clear of the column before it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Columns {
    label: f64,
    wire: f64,
    gauge: f64,
    far: f64,
}

impl Columns {
    fn of(rows: &[Row]) -> Self {
        let entries = || rows.iter().flat_map(|row| &row.entries);
        let label = if rows.is_empty() {
            LABEL_FONT.width(NO_PINS)
        } else {
            Self::widest(rows.iter().map(|row| {
                let note = row
                    .note
                    .as_ref()
                    .map_or(0.0, |note| NOTE_GAP + NOTE_FONT.width(&note.to_string()));
                LABEL_FONT.width(&row.label) + note
            }))
        };
        let wire = Self::widest(entries().map(|entry| match entry {
            Entry::Conductor { color, .. } => {
                SWATCH_WIDTH + SWATCH_GAP + WIRE_FONT.width(&color.code())
            }
            Entry::Drain { .. } => WIRE_FONT.width(DRAIN),
        }));
        let gauge = Self::widest(
            entries()
                .filter_map(|entry| entry.gauge())
                .map(|gauge| WIRE_FONT.width(&gauge)),
        );
        // Capped, the column narrows again to the widest far end as it is
        // drawn there, since a far end past the cap loses detail.
        let cap = Self::widest(entries().map(|entry| WIRE_FONT.width(entry.far().full())))
            .min(FAR_MAX_WIDTH);
        let far = Self::widest(entries().map(|entry| {
            let fitted = entry.far().fit(WIRE_FONT, cap);
            WIRE_FONT.width(&fitted.text)
        }));
        Self {
            label,
            wire: wire.max(WIRE_MIN_WIDTH),
            gauge: gauge.max(GAUGE_MIN_WIDTH),
            far: far.max(FAR_MIN_WIDTH),
        }
    }

    /// The widest of `widths`, 0 for none.
    fn widest(widths: impl Iterator<Item = f64>) -> f64 {
        widths.fold(0.0, f64::max)
    }

    /// Where each column's content starts, from the table's left edge.
    fn label_x(&self) -> f64 {
        PIN_COL_WIDTH + PAD_X
    }

    fn wire_x(&self) -> f64 {
        self.label_x() + self.label + PAD_X
    }

    fn gauge_x(&self) -> f64 {
        self.wire_x() + self.wire + PAD_X
    }

    fn far_x(&self) -> f64 {
        self.gauge_x() + self.gauge + PAD_X
    }

    /// The width the columns take, padding on the right included.
    fn width(&self) -> f64 {
        self.far_x() + self.far + PAD_X
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
                        let pin = Pin((index + 1) as u32);
                        let port = connector
                            .pins
                            .iter()
                            .find(|binding| binding.pin == pin)
                            .and_then(|binding| subject.ports.get(&binding.port));
                        Cell {
                            pin,
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
                            pin: cavity.pin,
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

    /// The height of the face's panel: padding above and below the face,
    /// and the caption band under it.
    fn band_height(&self) -> f64 {
        self.height() + FACE_PAD + CAPTION_HEIGHT
    }

    /// The width of the face's panel: the face with [`FACE_PAD`] each
    /// side, or the caption's, whichever is wider — never the table's.
    fn panel_width(&self) -> f64 {
        (self.width() + 2.0 * FACE_PAD).max(CAPTION_FONT.width(FACE_CAPTION) + 2.0 * PAD_X)
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

/// One cavity of a face: its pin, the label of the port bound to it
/// (`None` for a cavity no port uses), and its place in cavity steps.
struct Cell {
    pin: Pin,
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

    /// The cavity's side, in world units.
    fn side(&self) -> f64 {
        let span = self.span();
        span as f64 * CAVITY_SIZE + (span.saturating_sub(1)) as f64 * CAVITY_GAP
    }

    /// Whether a port is bound to the cavity.
    fn is_used(&self) -> bool {
        self.label.is_some()
    }

    /// The port's label as it is drawn inside the cavity: fitted between
    /// its borders, [`CAVITY_LABEL_INSET`] clear of each.
    fn fitted_label(&self) -> Option<FittedLabel> {
        let label = self.label.as_deref()?;
        Some(FittedLabel::new(
            label,
            self.side() - 2.0 * CAVITY_LABEL_INSET,
        ))
    }
}

#[derive(Clone, Copy)]
struct ViewBox {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// A table's header: a bold title naming the component and the connector,
/// then a line for each further thing there is to say, none of them empty.
/// [`HEADER_HEIGHT`] holds the title and one line; each line past the
/// first adds [`HEADER_LINE_HEIGHT`].
struct Header {
    title: String,
    lines: Vec<String>,
}

impl Header {
    /// The header of `connector`'s table on `subject`: the component and
    /// the designator, then the housing's description and part. A pinout
    /// documents a component type, so its title names the subject by its
    /// label only when the type is placed once in `design`; a label would
    /// name one placement of several, so a type placed more than once (or
    /// a placement with no label) is named by its type. The wires belong
    /// to a placement, so a type placed more than once gets a line naming
    /// the one whose wires the table lists.
    fn new(design: &Design, subject: &Instance, connector: &Connector) -> Self {
        let placements = design.placements_of(&subject.type_name).count();
        let component = match &subject.label {
            Some(label) if placements <= 1 => label.clone(),
            _ => subject.type_name.to_string(),
        };
        let part = match connector.properties.get("part") {
            Some(crate::dsl::ir::ConnectorPropertyValue::Str(part)) => Some(part.as_str()),
            _ => None,
        };
        let housing: Vec<&str> = connector
            .description
            .as_deref()
            .into_iter()
            .chain(part)
            .collect();
        let wires = (placements > 1).then(|| {
            format!(
                "Wires shown for {}, one of {placements} placements",
                subject.path
            )
        });
        Self {
            title: format!("{component} · {}", connector.name),
            lines: (!housing.is_empty())
                .then(|| housing.join(" · "))
                .into_iter()
                .chain(wires)
                .collect(),
        }
    }

    fn height(&self) -> f64 {
        HEADER_HEIGHT + self.lines.len().saturating_sub(1) as f64 * HEADER_LINE_HEIGHT
    }

    /// The width the header's widest text needs, [`PAD_X`] clear each side.
    fn width(&self) -> f64 {
        let lines = self.lines.iter().map(|line| HEADER_LINE_FONT.width(line));
        TITLE_FONT.width(&self.title).max(lines.fold(0.0, f64::max)) + 2.0 * PAD_X
    }
}

fn render_table(table: &Table) -> Group {
    let mut group = Group::new().set("class", "pinout");
    if let (Some(face), Some(panel)) = (&table.face, &table.face_frame) {
        let centre = panel.origin.x + panel.width / 2.0;
        group = group
            .add(rectangle(panel).set("class", "face-panel"))
            .add(render_face(
                face,
                centre - face.width() / 2.0,
                panel.origin.y + FACE_PAD / 2.0,
            ))
            .add(
                Text::new(FACE_CAPTION)
                    .set("class", "pinout-caption")
                    .set("x", centre)
                    .set("y", panel.origin.y + panel.height - CAPTION_HEIGHT / 2.0),
            );
    }

    let frame = &table.frame;
    let (left, top) = (frame.origin.x, frame.origin.y);
    let right = left + frame.width;
    group = group
        .add(rectangle(frame))
        .add(
            Rectangle::new()
                .set("class", "header")
                .set("x", left)
                .set("y", top)
                .set("width", frame.width)
                .set("height", table.header.height()),
        )
        .add(
            Text::new(table.header.title.clone())
                .set("class", "pinout-title")
                .set("x", left + frame.width / 2.0)
                .set("y", top + HEADER_HEIGHT * 0.42),
        );
    for (i, line) in table.header.lines.iter().enumerate() {
        group = group.add(
            Text::new(line.clone())
                .set("class", "pinout-part")
                .set("x", left + frame.width / 2.0)
                .set(
                    "y",
                    top + HEADER_HEIGHT * 0.78 + i as f64 * HEADER_LINE_HEIGHT,
                ),
        );
    }

    let rows_top = top + table.header.height();
    if table.rows.is_empty() {
        return group.add(
            Text::new(NO_PINS)
                .set("class", "pin-label")
                .set("x", left + table.columns.label_x())
                .set("y", rows_top + ROW_HEIGHT / 2.0),
        );
    }
    let mut row_top = rows_top;
    for (i, row) in table.rows.iter().enumerate() {
        if i > 0 {
            group = group.add(
                Line::new()
                    .set("class", "row-sep")
                    .set("x1", left)
                    .set("y1", row_top)
                    .set("x2", right)
                    .set("y2", row_top),
            );
        }
        group = group.add(render_row(row, &table.columns, left, right, row_top));
        row_top += row.height();
    }
    group
}

/// `row`, its top at `top`, in a table spanning `left` to `right`: the pin
/// and the port's label centred on the row's height, its note against the
/// label column's right edge, then one sub-row per entry, a dotted rule
/// between two, or a dash for a pin no conductor reaches.
fn render_row(row: &Row, columns: &Columns, left: f64, right: f64, top: f64) -> Group {
    let middle = top + row.height() / 2.0;
    let label_x = left + columns.label_x();
    let mut group = Group::new()
        .set("class", "pin-row")
        .add(
            Text::new(row.pin.map(|pin| pin.to_string()).unwrap_or_default())
                .set("class", "pin-num")
                .set("x", left + PIN_COL_WIDTH / 2.0)
                .set("y", middle),
        )
        .add(
            Text::new(row.label.clone())
                .set("class", "pin-label")
                .set("x", label_x)
                .set("y", middle),
        );
    if let Some(note) = &row.note {
        group = group.add(
            Text::new(note.to_string())
                .set("class", note.class())
                // The anchor rides as an attribute too, for embed mode,
                // where the built-in stylesheet is absent.
                .set("text-anchor", "end")
                .set("x", label_x + columns.label)
                .set("y", middle),
        );
    }
    let wire_x = left + columns.wire_x();
    if row.entries.is_empty() {
        return group.add(
            Text::new(UNWIRED)
                .set("class", "pin-wire pin-unwired")
                .set("x", wire_x)
                .set("y", middle),
        );
    }
    for (i, entry) in row.entries.iter().enumerate() {
        let y = top + Row::entry_offset(i);
        if i > 0 {
            let rule = y - SUB_ROW_HEIGHT / 2.0;
            group = group.add(
                Line::new()
                    .set("class", "wire-sep")
                    .set("x1", wire_x - PAD_X / 2.0)
                    .set("y1", rule)
                    .set("x2", right)
                    .set("y2", rule),
            );
        }
        group = group.add(render_entry(entry, columns, left, y));
    }
    group
}

/// One entry of a row on its centre line `y`: a conductor's swatch and
/// colour code, gauge and far end, or a drain and where it goes.
fn render_entry(entry: &Entry, columns: &Columns, left: f64, y: f64) -> Group {
    let wire_x = left + columns.wire_x();
    let mut group = Group::new().set("class", "pin-entry");
    group = match entry {
        Entry::Conductor { color, .. } => group.add(swatch(color, wire_x, y)).add(
            Text::new(color.code())
                .set("class", "pin-wire")
                .set("x", wire_x + SWATCH_WIDTH + SWATCH_GAP)
                .set("y", y),
        ),
        Entry::Drain { .. } => group.add(
            Text::new(DRAIN)
                .set("class", "pin-wire pin-drain")
                .set("x", wire_x)
                .set("y", y),
        ),
    };
    if let Some(gauge) = entry.gauge() {
        group = group.add(
            Text::new(gauge)
                .set("class", "pin-gauge")
                .set("x", left + columns.gauge_x())
                .set("y", y),
        );
    }
    let far = entry.far().fit(WIRE_FONT, columns.far);
    let mut text = Text::new(far.text.clone())
        .set("class", "pin-far")
        .set("x", left + columns.far_x())
        .set("y", y);
    if far.text != entry.far().full() {
        text = text.add(Title::new(entry.far().full()));
    }
    group.add(text)
}

/// A wire colour's swatch, its left edge at `x` and centred on `y`: a
/// small bar filled with the colour, a two-tone colour's tracer as a stripe
/// along its middle.
///
/// The paint is the harness renderer's for the same colour —
/// [`WireColor::css`] for the base, [`WireColor::tracer`] for the stripe —
/// and the authored colour rides along as `data-color` in every mode, as
/// on a harness strand, so a host stylesheet can restyle it in embed mode.
/// A `<path>`, not a `<rect>`: the `.pinout rect` rule would paint over a
/// rectangle's fill.
fn swatch(color: &WireColor, x: f64, y: f64) -> Group {
    let top = y - SWATCH_HEIGHT / 2.0;
    let bar =
        |top: f64, height: f64| format!("M{x},{top} h{SWATCH_WIDTH} v{height} h-{SWATCH_WIDTH} Z");
    let mut group = Group::new().add(
        Path::new()
            .set("class", "wire-swatch")
            .set("fill", color.css())
            .set("data-color", color.to_string())
            .set("d", bar(top, SWATCH_HEIGHT)),
    );
    if let Some(tracer) = color.tracer() {
        group = group.add(
            Path::new()
                .set("class", "wire-swatch-tracer")
                .set("fill", tracer)
                .set("data-color", color.to_string())
                .set("d", bar(top + SWATCH_HEIGHT / 3.0, SWATCH_HEIGHT / 3.0)),
        );
    }
    group
}

/// An outlined box over `bounds`.
fn rectangle(bounds: &Bounds) -> Rectangle {
    Rectangle::new()
        .set("x", bounds.origin.x)
        .set("y", bounds.origin.y)
        .set("width", bounds.width)
        .set("height", bounds.height)
}

/// The cavities of `face`, its top-left corner at (`x`, `y`). A cavity no
/// port uses is drawn dashed and greyed (`cavity-unused`,
/// `cavity-pin-unused`); a used one carries its port's label, fitted
/// inside it, with the full label as its tooltip when the fit cut it.
fn render_face(face: &Face, x: f64, y: f64) -> Group {
    let mut group = Group::new().set("class", "pinout-face");
    for cell in &face.cells {
        let size = cell.side();
        let cx = x + cell.x as f64 * (CAVITY_SIZE + CAVITY_GAP);
        let cy = y + cell.y as f64 * (CAVITY_SIZE + CAVITY_GAP);
        let (cavity_class, pin_class) = if cell.is_used() {
            ("cavity", "cavity-pin")
        } else {
            ("cavity cavity-unused", "cavity-pin cavity-pin-unused")
        };
        let mut cavity = Rectangle::new()
            .set("class", cavity_class)
            .set("x", cx)
            .set("y", cy)
            .set("width", size)
            .set("height", size);
        let fitted = cell.fitted_label();
        if let (Some(label), Some(fitted)) = (&cell.label, &fitted)
            && fitted.is_cut()
        {
            cavity = cavity.add(Title::new(label.clone()));
        }
        group = group.add(cavity).add(
            Text::new(cell.pin.to_string())
                .set("class", pin_class)
                .set("x", cx + size / 2.0)
                .set("y", cy + size * 0.42),
        );
        if let Some(fitted) = fitted {
            let mut text = Text::new(fitted.text.clone())
                .set("class", "cavity-label")
                .set("x", cx + size / 2.0)
                .set("y", cy + size * 0.72);
            // An inline style, because the stylesheet's `font` shorthand
            // would override a `font-size` attribute.
            if fitted.is_shrunk() {
                text = text.set("style", format!("font-size: {}px", fitted.size));
            }
            group = group.add(text);
        }
    }
    group
}

/// The viewBox `view` is drawn in under `mode`: its tables, with room for
/// the title and the identity stamp when the mode draws them.
fn viewbox(design: &Design, view: &View, tables: &[Table], mode: SvgMode) -> ViewBox {
    let mut vb = tables_viewbox(tables, mode.shows_title(view));
    if mode.stamp(design).is_some() {
        vb.height += STAMP_HEIGHT;
    }
    vb
}

fn tables_viewbox(tables: &[Table], has_title: bool) -> ViewBox {
    if tables.is_empty() {
        return ViewBox {
            x: 0.0,
            y: 0.0,
            width: SVG_MARGIN * 2.0,
            height: SVG_MARGIN * 2.0,
        };
    }
    let boxes: Vec<Bounds> = tables.iter().map(Table::bounds).collect();
    let min_x = boxes
        .iter()
        .map(|b| b.origin.x)
        .fold(f64::INFINITY, f64::min);
    let min_y = boxes
        .iter()
        .map(|b| b.origin.y)
        .fold(f64::INFINITY, f64::min);
    let max_x = boxes
        .iter()
        .map(|b| b.origin.x + b.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let max_y = boxes
        .iter()
        .map(|b| b.origin.y + b.height)
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
    use crate::dsl::span::{FileId, Span};
    use crate::render::schematic::tests::design_from;
    use rows::Note;

    fn pinout_view(subject: &str, includes: &[(&str, f64, f64)]) -> View {
        View {
            kind: ViewKind::Pinout,
            title: "Pinout".to_string(),
            grid: None,
            span: Span::new(FileId(0), 0, 0),
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
                    span: Span::new(FileId(0), 0, 0),
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
        let tables = PinTables::from(design);
        PinoutRenderer
            .render(design, &tables, subject, view, mode)
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

    /// The first instance of `m` in `design`, and its connector `x1`.
    fn subject_x1(design: &Design) -> (&Instance, &Connector) {
        let subject = design
            .instances
            .values()
            .find(|i| i.type_name == TypeName::from("m"))
            .expect("subject");
        let connector = subject
            .connectors
            .get(&ConnectorName::from("x1"))
            .expect("connector");
        (subject, connector)
    }

    /// A design whose root `top` holds `placement` of the component `m`,
    /// which `lib` (a file of its own) defines.
    fn placed_in_top(lib: &str, placement: &str) -> Design {
        let dir = tempfile::tempdir().expect("tempdir");
        let main = format!("use m from \"lib.wb\";\ncomponent top {{ {placement} }}\n");
        std::fs::write(dir.path().join("main.wb"), main).expect("write");
        std::fs::write(dir.path().join("lib.wb"), lib).expect("write");
        let (project, _) = crate::dsl::project::load(&dir.path().join("main.wb"));
        let project = project.expect("loads");
        let resolved = crate::dsl::resolve::resolve(&project);
        let (design, problems) = crate::dsl::elaborate::elaborate(&resolved);
        assert!(problems.is_empty(), "elaboration problems: {problems:?}");
        design.expect("a design")
    }

    /// The drawing of `x1` on the first instance of `m`.
    fn table_of(design: &Design) -> Table<'_> {
        let (subject, connector) = subject_x1(design);
        let tables = PinTables::from(design);
        let table = tables.of(subject, &connector.name).expect("a pin table");
        let view = pinout_view("m", &[("x1", 0.0, 0.0)]);
        Table::new(design, table, &view.includes[0], DEFAULT_GRID)
    }

    #[test]
    fn rows_are_sorted_by_pin_whatever_the_declaration_order() {
        let design = design_from(
            r#"
component m {
    connector x1 "X" {
        pub port temp "TEMP" pin 45;
        pub port ign  "IGN"  pin 42;
        pub port b_pos "B+" pins [46, 48];
        pub port b_neg "B-" pins [47, 49];
        pub port can  "CAN"  pin 9;
    }
}
"#,
        );
        let pins: Vec<Option<u32>> = table_of(&design)
            .rows
            .iter()
            .map(|r| r.pin.map(|pin| pin.0))
            .collect();
        assert_eq!(pins, [9, 42, 45, 46, 47, 48, 49].map(Some));
    }

    #[test]
    fn header_names_the_component_the_connector_its_description_and_part() {
        let design = placed_in_top(
            r#"
connector_type Box "Box 2p" {
    part: "BX-2";
    layout grid { rows: 1; cols: 2; }
}
component m {
    connector x1: Box { pub port a "A" pin 1; }
}
"#,
            "inv: m \"Inverter\";",
        );
        let table = table_of(&design);
        assert_eq!(table.header.title, "Inverter · x1");
        assert_eq!(table.header.lines, ["Box 2p · BX-2"]);
    }

    #[test]
    fn header_names_the_type_when_it_is_placed_more_than_once() {
        // A pinout documents the type: the front unit's label would be
        // wrong on the sheet the rear unit's harness is built from.
        let design = placed_in_top(
            r#"
component m {
    connector x1 "X" { pub port a "A" pin 1; }
}
"#,
            "front: m \"BMS front\"; rear: m \"BMS rear\";",
        );
        let table = table_of(&design);
        assert_eq!(table.header.title, "m · x1");
        // The wires are one placement's: the header says whose.
        assert_eq!(
            table.header.lines,
            ["X", "Wires shown for top.front, one of 2 placements"]
        );
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        assert!(!svg.contains("BMS front") && !svg.contains("BMS rear"));
    }

    #[test]
    fn each_header_line_past_the_first_makes_the_header_taller() {
        let header = |lines: &[&str]| Header {
            title: "m · x1".to_string(),
            lines: lines.iter().map(|l| l.to_string()).collect(),
        };
        assert_eq!(header(&[]).height(), HEADER_HEIGHT);
        assert_eq!(header(&["Box"]).height(), HEADER_HEIGHT);
        assert_eq!(
            header(&["Box", "more"]).height(),
            HEADER_HEIGHT + HEADER_LINE_HEIGHT
        );
        // A line wider than the title widens the header.
        assert_eq!(header(&["Box"]).width(), header(&[]).width());
        assert!(header(&["a much longer line than the title"]).width() > header(&[]).width());
    }

    #[test]
    fn header_omits_what_is_missing_without_dangling_separators() {
        // A connector block with no description, on a component placed
        // once without a label: the component's type stands in for it.
        let design = placed_in_top(
            r#"
component m {
    connector x1 { pub port a "A" pin 1; }
}
"#,
            "inv: m;",
        );
        let table = table_of(&design);
        assert_eq!(table.header.title, "m · x1");
        assert!(table.header.lines.is_empty());
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        assert!(!svg.contains("class=\"pinout-part\""));
    }

    #[test]
    fn every_face_carries_the_harness_side_caption_below_it() {
        let design = design_from(
            r#"
connector_type Pair "Pair" { layout grid { rows: 1; cols: 2; } }
component m {
    connector x1: Pair { pub port a "A" pin 1; }
    connector x2 "Faceless" { pub port b "B" pin 1; }
}
"#,
        );
        let with_face = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        assert_eq!(with_face.matches("class=\"pinout-caption\"").count(), 1);
        assert!(with_face.contains("Viewed from harness side"));
        // A connector without a layout has no face, so no caption.
        let without = render(
            &design,
            &pinout_view("m", &[("x2", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        assert!(!without.contains("class=\"pinout-caption\""));

        // The caption has a band of its own under the face, in the face's
        // panel, which sits on top of the table.
        let table = table_of(&design);
        let face = table.face.as_ref().expect("face");
        let panel = table.face_frame.expect("a face panel");
        assert_eq!(panel.height, face.height() + FACE_PAD + CAPTION_HEIGHT);
        assert_eq!(table.frame.height, HEADER_HEIGHT + ROW_HEIGHT);
        assert_eq!(panel.origin.y + panel.height, table.frame.origin.y);
        assert_eq!(table.bounds().height, panel.height + table.frame.height);
    }

    #[test]
    fn unused_cavities_are_drawn_apart_from_used_ones() {
        let design = design_from(
            r#"
connector_type Tri "Tri" { layout grid { rows: 1; cols: 3; } }
component m {
    connector x1: Tri {
        pub port a "A" pin 1;
        pub port c "C" pin 3;
    }
}
"#,
        );
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        // Every cavity keeps the `cavity` class; the unused one adds its own.
        assert_eq!(svg.matches("class=\"cavity\"").count(), 2);
        assert_eq!(svg.matches("class=\"cavity cavity-unused\"").count(), 1);
        assert_eq!(
            svg.matches("class=\"cavity-pin cavity-pin-unused\"")
                .count(),
            1
        );
        let unused: Vec<u32> = table_of(&design)
            .face
            .expect("face")
            .cells
            .iter()
            .filter(|cell| !cell.is_used())
            .map(|cell| cell.pin.0)
            .collect();
        assert_eq!(unused, [2]);
        assert!(STYLE.contains("rect.cavity-unused"));
    }

    #[test]
    fn cavity_labels_are_fitted_inside_their_cavity() {
        let design = design_from(
            r#"
connector_type Face "Face" {
    layout face {
        cavity 1 at (0, 0);
        cavity 2 at (1, 0);
        cavity 3 at (2, 0);
        cavity 4 at (3, 0) size large;
    }
}
component m {
    connector x1: Face {
        pub port a "EX+" pin 1;
        pub port b "IGNITION" pin 2;
        pub port c "RESOLVER SHIELD DRAIN" pin 3;
        pub port d "RESOLVER" pin 4;
    }
}
"#,
        );
        let face = table_of(&design).face.expect("face");
        for cell in &face.cells {
            let fitted = cell.fitted_label().expect("a label");
            assert!(
                fitted.width() <= cell.side() - 2.0 * CAVITY_LABEL_INSET,
                "pin {}: {:?} crosses its cavity",
                cell.pin,
                fitted
            );
        }
        let fits: Vec<(String, bool, bool)> = face
            .cells
            .iter()
            .filter_map(Cell::fitted_label)
            .map(|f| (f.text.clone(), f.is_shrunk(), f.is_cut()))
            .collect();
        assert_eq!(fits[0], ("EX+".to_string(), false, false));
        assert_eq!(fits[1], ("IGNITION".to_string(), true, false));
        assert!(fits[2].2, "a long label is cut in a small cavity");
        assert_eq!(
            fits[3],
            ("RESOLVER".to_string(), false, false),
            "a large cavity holds at full size what a small one would shrink"
        );

        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        // The cut label stays whole in the cavity's tooltip and the table.
        assert!(svg.contains("<title>RESOLVER SHIELD DRAIN</title>"));
        assert!(svg.contains("c · RESOLVER SHIELD DRAIN"));
        assert!(svg.contains("style=\"font-size: 6px\""));
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
        let table = table_of(&design);
        let face = table.face.as_ref().expect("face");
        let panel = table.face_frame.expect("a face panel");

        assert_eq!(panel.width, face.width() + 2.0 * FACE_PAD);
        // Both are centred on the include's point.
        assert_eq!(panel.origin.x, -panel.width / 2.0);
        assert_eq!(table.frame.origin.x, -table.frame.width / 2.0);
    }

    #[test]
    fn a_narrow_table_is_not_stretched_to_its_wide_face() {
        let design = design_from(
            r#"
connector_type Wide "W" {
    layout grid { rows: 1; cols: 12; }
}
component m {
    connector x1: Wide { pub port a "A" pin 1; }
}
"#,
        );
        let table = table_of(&design);
        let panel = table.face_frame.expect("a face panel");
        assert_eq!(table.frame.width, table.columns.width());
        assert!(table.frame.width < panel.width - 100.0);
        assert_eq!(table.bounds().width, panel.width);

        // A face narrower than its table is not stretched to the table.
        let design = design_from(
            r#"
connector_type One "One" {
    layout grid { rows: 1; cols: 1; }
}
component m {
    connector x1: One { pub port a "A LONG DESCRIPTIVE PORT LABEL" pin 1; }
}
"#,
        );
        let table = table_of(&design);
        let panel = table.face_frame.expect("a face panel");
        let face = table.face.as_ref().expect("face");
        assert_eq!(
            panel.width,
            (face.width() + 2.0 * FACE_PAD).max(CAPTION_FONT.width(FACE_CAPTION) + 2.0 * PAD_X)
        );
        assert!(panel.width < table.frame.width);
        assert_eq!(table.bounds().width, table.frame.width);
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
        let tables = PinTables::from(&design);
        let err = PinoutRenderer
            .render(
                &design,
                &tables,
                subject,
                &pinout_view("m", &[("x1", 0.0, 0.0)]),
                SvgMode::Standalone,
            )
            .expect_err("unknown connector");
        assert!(matches!(err, ViewErrorKind::UnknownConnector { .. }));
    }

    /// A design whose root `top` places `m` (its connector `x1` holding
    /// `ports`) and an `Ecu` (whose `x1` has ports `p1`…`p6` on pins 1–6)
    /// with `body` (wires, cables, splices) between them.
    fn wired(ports: &str, body: &str) -> Design {
        wired_inside(&format!("connector x1 \"X\" {{ {ports} }}"), body)
    }

    /// As [`wired`], with `inside` as the whole body of `m`.
    fn wired_inside(inside: &str, body: &str) -> Design {
        let dir = tempfile::tempdir().expect("tempdir");
        let lib = format!(
            "component m {{ {inside} }}\n\
             component Ecu {{ connector x1 \"ECU\" {{\n\
               pub port p1 \"P1\" pin 1; pub port p2 \"P2\" pin 2; pub port p3 \"P3\" pin 3;\n\
               pub port p4 \"P4\" pin 4; pub port p5 \"P5\" pin 5; pub port p6 \"P6\" pin 6;\n\
             }} }}\n"
        );
        let main = format!(
            "use m from \"lib.wb\";\nuse Ecu from \"lib.wb\";\n\
             component top {{ dev: m; ecu: Ecu; lamp: Ecu;\n{body}\n}}\n"
        );
        std::fs::write(dir.path().join("main.wb"), main).expect("write");
        std::fs::write(dir.path().join("lib.wb"), lib).expect("write");
        let (project, _) = crate::dsl::project::load(&dir.path().join("main.wb"));
        let project = project.expect("loads");
        let resolved = crate::dsl::resolve::resolve(&project);
        assert!(resolved.problems.is_empty(), "{:?}", resolved.problems);
        let (design, problems) = crate::dsl::elaborate::elaborate(&resolved);
        assert!(problems.is_empty(), "elaboration problems: {problems:?}");
        design.expect("a design")
    }

    /// The content of every `<text>` of `svg` whose class is `class`, a
    /// tooltip left out, in document order.
    fn texts(svg: &str, class: &str) -> Vec<String> {
        use svg::node::element::tag::Type;
        use svg::parser::Event;
        let mut found = Vec::new();
        let (mut open, mut in_title) = (None::<String>, false);
        for event in svg::read(svg).expect("parses") {
            match event {
                Event::Tag("text", Type::Start, attrs) => {
                    let matches = attrs.get("class").is_some_and(|c| &**c == class);
                    open = matches.then(String::new);
                }
                Event::Tag("text", Type::End, _) => found.extend(open.take()),
                Event::Tag("title", Type::Start, _) => in_title = true,
                Event::Tag("title", Type::End, _) => in_title = false,
                Event::Text(content) if !in_title => {
                    if let Some(text) = open.as_mut() {
                        text.push_str(content.trim());
                    }
                }
                _ => {}
            }
        }
        found
    }

    /// Each row's pin, and each of its entries as `colour code · gauge ·
    /// far end` (`drain · far end` for a drain).
    fn listing(table: &Table) -> Vec<(Option<u32>, Vec<String>)> {
        table
            .rows
            .iter()
            .map(|row| {
                let entries = row
                    .entries
                    .iter()
                    .map(|entry| match entry {
                        Entry::Conductor { color, gauge, far } => {
                            format!("{} · {gauge} · {}", color.code(), far.full())
                        }
                        Entry::Drain { far } => format!("drain · {}", far.full()),
                    })
                    .collect();
                (row.pin.map(|pin| pin.0), entries)
            })
            .collect()
    }

    #[test]
    fn each_pin_lists_its_conductors_colour_gauge_and_far_end() {
        let design = wired(
            "pub port a \"A\" pin 1; pub port b \"B\" pin 2; pub port c \"C\" pin 3;",
            "wire red 1 [dev.a, ecu.p1];\n\
             wire black 0.5 [dev.a, lamp.p2];\n\
             wire green/yellow 2.5 [dev.b, ecu.p3];",
        );
        let table = table_of(&design);
        assert_eq!(
            listing(&table),
            [
                (
                    Some(1),
                    vec![
                        "RD · 1 · → top.ecu.p1 (x1:1)".to_string(),
                        "BK · 0.5 · → top.lamp.p2 (x1:2)".to_string(),
                    ]
                ),
                (
                    Some(2),
                    vec!["GN/YE · 2.5 · → top.ecu.p3 (x1:3)".to_string()]
                ),
                (Some(3), vec![]),
            ]
        );
        // A double crimp is one row, a sub-row taller per further conductor.
        let heights: Vec<f64> = table.rows.iter().map(Row::height).collect();
        assert_eq!(
            heights,
            [ROW_HEIGHT + SUB_ROW_HEIGHT, ROW_HEIGHT, ROW_HEIGHT]
        );

        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        assert_eq!(svg.matches("class=\"pin-row\"").count(), 3);
        assert_eq!(svg.matches("class=\"pin-entry\"").count(), 3);
        assert_eq!(svg.matches("class=\"wire-sep\"").count(), 1);
        assert_eq!(texts(&svg, "pin-gauge"), ["1mm²", "0.5mm²", "2.5mm²"]);
        assert_eq!(
            texts(&svg, "pin-far"),
            [
                "→ top.ecu.p1 (x1:1)",
                "→ top.lamp.p2 (x1:2)",
                "→ top.ecu.p3 (x1:3)"
            ]
        );
        assert_eq!(texts(&svg, "pin-wire"), ["RD", "BK", "GN/YE"]);
        // A pin no conductor reaches is visibly unwired.
        assert_eq!(svg.matches("class=\"pin-wire pin-unwired\"").count(), 1);
    }

    #[test]
    fn a_swatch_is_painted_as_the_harness_paints_the_wire_in_every_mode() {
        let design = wired(
            "pub port a \"A\" pin 1; pub port b \"B\" pin 2;",
            "wire purple 0.5 [dev.a, ecu.p1];\nwire green/yellow 2.5 [dev.b, ecu.p2];",
        );
        let view = pinout_view("m", &[("x1", 0.0, 0.0)]);
        for mode in [SvgMode::Standalone, SvgMode::Embed] {
            let svg = render(&design, &view, mode);
            // The base fills the swatch, the tracer a stripe over it, and
            // the canonical colour rides along as `data-color`.
            assert!(svg.contains("fill=\"violet\""), "{mode:?}");
            assert!(svg.contains("data-color=\"violet\""), "{mode:?}");
            assert!(svg.contains("fill=\"green\""), "{mode:?}");
            assert!(svg.contains("fill=\"yellow\""), "{mode:?}");
            assert_eq!(svg.matches("data-color=\"green/yellow\"").count(), 2);
            assert_eq!(svg.matches("class=\"wire-swatch\"").count(), 2);
            assert_eq!(svg.matches("class=\"wire-swatch-tracer\"").count(), 1);
            assert_eq!(texts(&svg, "pin-wire"), ["VT", "GN/YE"]);
        }
        // A swatch is not a `rect`, which the `.pinout rect` rule would
        // paint white.
        assert!(STYLE.contains(".wire-swatch {"));
        assert!(!STYLE.contains(".wire-swatch { fill"));
        // The embed stylesheet carries the new rules, scoped.
        let css = crate::render::embed_stylesheet();
        for rule in [".wire-swatch {", ".pin-far {", ".pin-note {", ".wire-sep {"] {
            assert!(css.contains(&format!(".wirebug-pinout {rule}")), "{rule}");
        }
    }

    #[test]
    fn a_drain_is_an_entry_of_its_own_with_no_colour_or_gauge() {
        let design = wired(
            "pub port s \"S\" pin 1; pub port t \"T\" pin 2;",
            "cable can \"CAN\" {\n\
               twisted { wire white 0.5 [dev.t, ecu.p1]; wire blue 0.5 [lamp.p1, ecu.p2]; }\n\
               shield [dev.s];\n\
             }\n\
             cable other \"Other\" { wire red 0.5 [lamp.p3, ecu.p3]; shield \"SCREEN\" [dev.s, ecu.p4]; }",
        );
        let table = table_of(&design);
        assert_eq!(
            listing(&table)[0],
            (
                Some(1),
                vec![
                    "drain · can shield".to_string(),
                    "drain · → top.ecu.p4 (other · SCREEN)".to_string(),
                ]
            )
        );
        assert!(
            table.rows[0]
                .entries
                .iter()
                .all(|entry| entry.gauge().is_none())
        );
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        assert_eq!(svg.matches("class=\"pin-wire pin-drain\"").count(), 2);
        assert_eq!(svg.matches("class=\"wire-swatch\"").count(), 1);
    }

    #[test]
    fn a_conductor_to_a_splice_names_the_splice_and_what_lies_beyond() {
        let design = wired(
            "pub port g \"G\" pin 1;",
            "splice sp \"GND\";\n\
             wire black 1 [dev.g, sp];\n\
             wire black 0.5 [ecu.p1, sp];\n\
             wire black 0.5 [lamp.p1, sp];\n\
             wire black 0.5 [ecu.p2, sp];",
        );
        let table = table_of(&design);
        let far = table.rows[0].entries[0].far();
        assert_eq!(far.full(), "→ top.sp (ecu.p1, lamp.p1, ecu.p2)");
        // Short of room, the list collapses before the splice is cut.
        let collapsed = far.fit(WIRE_FONT, WIRE_FONT.width("→ top.sp (ecu.p1, +2 more)"));
        assert_eq!(collapsed.text, "→ top.sp (ecu.p1, +2 more)");
        let bare = far.fit(WIRE_FONT, WIRE_FONT.width("→ top.sp (ecu"));
        assert_eq!(bare.text, "→ top.sp");
    }

    #[test]
    fn a_conductor_leaving_through_a_boundary_port_says_it_leaves_the_component() {
        // `m`'s own wires: one out through its connectorless port `out`
        // (the wire-through pattern, carried on by `top`), one to a pin of
        // its other connector — a real termination on a pin.
        let design = wired_inside(
            "pub port out \"OUT\";\n\
             connector x1 \"X\" { port a \"A\" pin 1; port b \"B\" pin 2; }\n\
             connector x2 \"Y\" { pub port c \"C\" pin 1; }\n\
             wire red 1 [a, out];\n\
             wire black 0.5 [b, c];",
            "wire green 1 [dev.out, ecu.p3];\nwire white 0.5 [dev.c, ecu.p4];",
        );
        let table = table_of(&design);
        assert_eq!(
            listing(&table),
            [
                (
                    Some(1),
                    vec!["RD · 1 · → top.dev.out (leaves m)".to_string()]
                ),
                (Some(2), vec!["BK · 0.5 · → top.dev.c (x2:1)".to_string()]),
            ]
        );
        // Short of room, the mark stays and the path shortens.
        let far = table.rows[0].entries[0].far();
        let narrow = far.fit(WIRE_FONT, WIRE_FONT.width("→ out (leaves m)"));
        assert_eq!(narrow.text, "→ out (leaves m)");
    }

    #[test]
    fn a_ganged_port_is_marked_on_each_of_its_rows() {
        let design = wired(
            "pub port pwr \"PWR\" pins [3, 1]; pub port sig \"SIG\" pin 2;",
            "wire red 2.5 [dev.pwr, ecu.p1];",
        );
        let table = table_of(&design);
        let notes: Vec<(Option<u32>, Option<String>, usize)> = table
            .rows
            .iter()
            .map(|row| {
                (
                    row.pin.map(|pin| pin.0),
                    row.note.as_ref().map(Note::to_string),
                    row.entries.len(),
                )
            })
            .collect();
        assert_eq!(
            notes,
            [
                (Some(1), Some("ganged 1+3".to_string()), 1),
                (Some(2), None, 0),
                (Some(3), Some("ganged 1+3".to_string()), 1),
            ]
        );
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        assert_eq!(svg.matches("class=\"pin-note pin-ganged\"").count(), 2);
    }

    #[test]
    fn a_port_with_no_pin_is_listed_after_the_pins_with_its_conductors() {
        let design = wired(
            "pub port spare \"SPARE\"; pub port a \"A\" pin 1;",
            "wire white 0.5 [dev.spare, ecu.p1];",
        );
        let table = table_of(&design);
        assert_eq!(
            listing(&table),
            [
                (Some(1), vec![]),
                (None, vec!["WH · 0.5 · → top.ecu.p1 (x1:1)".to_string()]),
            ]
        );
        assert_eq!(table.rows[1].note, Some(Note::Unpinned));
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        assert_eq!(svg.matches("class=\"pin-note pin-unpinned\"").count(), 1);
    }

    #[test]
    fn a_long_far_end_is_fitted_to_its_capped_column() {
        let design = wired(
            "pub port a \"A\" pin 1;",
            "splice a_splice_with_a_very_long_name_indeed;\n\
             wire red 1 [dev.a, a_splice_with_a_very_long_name_indeed];\n\
             wire red 1 [ecu.p1, a_splice_with_a_very_long_name_indeed];",
        );
        let table = table_of(&design);
        assert!(table.columns.far <= FAR_MAX_WIDTH);
        let far = table.rows[0].entries[0].far();
        assert!(WIRE_FONT.width(far.full()) > FAR_MAX_WIDTH);
        let fitted = far.fit(WIRE_FONT, table.columns.far);
        assert!(fitted.width() <= table.columns.far, "{}", fitted.text);
        assert!(table.columns.width() <= table.frame.width);
        let svg = render(
            &design,
            &pinout_view("m", &[("x1", 0.0, 0.0)]),
            SvgMode::Standalone,
        );
        // Short of room, the far end drops the splice's beyond and keeps
        // its whole text as a tooltip.
        let [drawn] = &texts(&svg, "pin-far")[..] else {
            panic!("one far end");
        };
        assert_eq!(drawn, "→ top.a_splice_with_a_very_long_name_indeed");
        assert_eq!(drawn, &fitted.text);
        assert!(svg.contains(&format!("<title>{}</title>", far.full())));
    }

    #[test]
    fn the_wire_columns_follow_their_content() {
        let narrow = wired("pub port a \"A\" pin 1;", "wire red 1 [dev.a, ecu.p1];");
        let narrow = table_of(&narrow);
        let wide = wired(
            "pub port a \"A\" pin 1;",
            "wire green/yellow 0.35 [dev.a, ecu.p1];",
        );
        let wide = table_of(&wide);
        assert!(wide.columns.wire > narrow.columns.wire);
        assert!(wide.columns.gauge > narrow.columns.gauge);
        assert_eq!(narrow.columns.wire, WIRE_MIN_WIDTH);
        // A table with no conductor keeps its columns, at their minimums.
        let unwired = wired("pub port a \"A\" pin 1;", "");
        let unwired = table_of(&unwired);
        assert_eq!(
            (
                unwired.columns.wire,
                unwired.columns.gauge,
                unwired.columns.far
            ),
            (WIRE_MIN_WIDTH, GAUGE_MIN_WIDTH, FAR_MIN_WIDTH)
        );
    }
}
