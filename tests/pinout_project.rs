//! End-to-end check of the `pinout_project` fixture: a pinout view of two
//! connectors, one with an explicit face and one with a grid layout, whose
//! pins are declared out of order, one with a label too long for its
//! cavity, both with cavities no port uses; and a pinout of a type placed
//! twice. The design wires them, so each table lists what is crimped into
//! each cavity: a two-tone conductor, a double crimp, a ganged port, a
//! conductor to a splice, a screen's drain and a port with no pin.
//!
//! The SVG is read back into a small structure (each table's header, face,
//! rows and their entries) rather than snapshotted.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use svg::node::element::tag::Type;
use svg::parser::Event;
use wirebug::check_project;
use wirebug::render::{SvgMode, render_views};

/// One `<text>` of the drawing: its class, inline style, and content.
#[derive(Debug)]
struct Label {
    class: String,
    style: Option<String>,
    text: String,
}

/// One cavity rectangle: its class, its extent along x, and its tooltip if
/// it has one.
#[derive(Debug)]
struct Cavity {
    class: String,
    x: f64,
    width: f64,
    title: Option<String>,
}

/// A colour swatch: the paint it is filled with and its `data-color`.
#[derive(Debug, Clone, PartialEq)]
struct Paint {
    fill: String,
    data_color: String,
}

/// One entry (sub-row) of a row: a conductor or a drain.
#[derive(Debug, Default)]
struct Entry {
    swatch: Option<Paint>,
    tracer: Option<Paint>,
    /// The wire column's text and class: a colour code (`pin-wire`) or
    /// `drain` (`pin-wire pin-drain`).
    wire: Option<(String, String)>,
    gauge: Option<String>,
    far: Option<String>,
    far_title: Option<String>,
}

/// One row of a pin table.
#[derive(Debug, Default)]
struct Row {
    pin: String,
    label: String,
    note: Option<(String, String)>,
    unwired: bool,
    entries: Vec<Entry>,
}

/// A rectangle's left edge and width.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Span {
    x: f64,
    width: f64,
}

impl Span {
    fn centre(&self) -> f64 {
        self.x + self.width / 2.0
    }
}

/// One pin table of the drawing, in document order.
#[derive(Debug, Default)]
struct Table {
    labels: Vec<Label>,
    cavities: Vec<Cavity>,
    rows: Vec<Row>,
    /// The table's outline: header and rows.
    frame: Option<Span>,
    /// The face's panel, when the connector has a layout.
    panel: Option<Span>,
}

impl Table {
    fn texts(&self, class: &str) -> Vec<&str> {
        self.labels
            .iter()
            .filter(|l| l.class == class)
            .map(|l| l.text.as_str())
            .collect()
    }

    fn row_pins(&self) -> Vec<&str> {
        self.rows.iter().map(|row| row.pin.as_str()).collect()
    }

    fn row(&self, pin: &str) -> &Row {
        self.rows
            .iter()
            .find(|row| row.pin == pin)
            .unwrap_or_else(|| panic!("a row for pin {pin:?}"))
    }

    fn unused_pins(&self) -> Vec<u32> {
        let mut pins: Vec<u32> = self
            .texts("cavity-pin cavity-pin-unused")
            .into_iter()
            .map(|pin| pin.parse().expect("a pin number"))
            .collect();
        pins.sort_unstable();
        pins
    }

    /// The width the cavities span.
    fn face_extent(&self) -> f64 {
        let left = self
            .cavities
            .iter()
            .map(|c| c.x)
            .fold(f64::INFINITY, f64::min);
        let right = self
            .cavities
            .iter()
            .map(|c| c.x + c.width)
            .fold(f64::NEG_INFINITY, f64::max);
        right - left
    }
}

/// Each entry of `row` as `code gauge far` (`drain far` for a drain).
fn entries(row: &Row) -> Vec<String> {
    row.entries
        .iter()
        .map(|entry| {
            let wire = entry.wire.as_ref().map_or("", |(text, _)| text.as_str());
            [Some(wire), entry.gauge.as_deref(), entry.far.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// The pin tables of the inverter's view.
fn tables() -> Vec<Table> {
    tables_of("Inverter pinouts")
}

fn attr(attrs: &svg::node::Attributes, name: &str) -> Option<String> {
    attrs.get(name).map(|value| value.to_string())
}

fn number(attrs: &svg::node::Attributes, name: &str) -> f64 {
    attr(attrs, name)
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("a numeric {name}"))
}

/// The pin tables of the fixture's view titled `title`, read back from its
/// SVG.
fn tables_of(title: &str) -> Vec<Table> {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pinout_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    let design = report.design.expect("a design");
    let views = render_views(&design, SvgMode::Standalone)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("renders");
    let svg = &views
        .iter()
        .find(|view| view.title == title)
        .unwrap_or_else(|| panic!("no view {title}"))
        .svg;
    read_tables(svg)
}

fn read_tables(svg: &str) -> Vec<Table> {
    let mut tables: Vec<Table> = Vec::new();
    // The classes of the open groups, innermost last.
    let mut groups: Vec<String> = Vec::new();
    let mut open_text: Option<Label> = None;
    let mut in_title = false;
    for event in svg::read(svg).expect("parses") {
        match event {
            Event::Tag("g", Type::Start, attrs) => {
                let class = attr(&attrs, "class").unwrap_or_default();
                match class.as_str() {
                    "pinout" => tables.push(Table::default()),
                    "pin-row" => tables
                        .last_mut()
                        .expect("inside a table")
                        .rows
                        .push(Row::default()),
                    "pin-entry" => tables
                        .last_mut()
                        .and_then(|t| t.rows.last_mut())
                        .expect("inside a row")
                        .entries
                        .push(Entry::default()),
                    _ => {}
                }
                groups.push(class);
            }
            Event::Tag("g", Type::End, _) => {
                groups.pop();
            }
            Event::Tag("rect", Type::Start | Type::Empty, attrs) => {
                let Some(table) = tables.last_mut() else {
                    continue;
                };
                let span = Span {
                    x: number(&attrs, "x"),
                    width: number(&attrs, "width"),
                };
                match attr(&attrs, "class") {
                    // A cavity with a tooltip is an open `<rect>` holding a
                    // `<title>`; one without is an empty element.
                    Some(class) if class.starts_with("cavity") => table.cavities.push(Cavity {
                        class,
                        x: span.x,
                        width: span.width,
                        title: None,
                    }),
                    Some(class) if class == "face-panel" => table.panel = Some(span),
                    None => table.frame = Some(span),
                    Some(_) => {}
                }
            }
            Event::Tag("path", Type::Empty, attrs) => {
                let class = attr(&attrs, "class").unwrap_or_default();
                let paint = Paint {
                    fill: attr(&attrs, "fill").expect("a swatch's fill"),
                    data_color: attr(&attrs, "data-color").expect("a swatch's data-color"),
                };
                let entry = tables
                    .last_mut()
                    .and_then(|t| t.rows.last_mut())
                    .and_then(|r| r.entries.last_mut())
                    .expect("a swatch inside an entry");
                match class.as_str() {
                    "wire-swatch" => entry.swatch = Some(paint),
                    "wire-swatch-tracer" => entry.tracer = Some(paint),
                    other => panic!("an unexpected path {other}"),
                }
            }
            Event::Tag("title", Type::Start, _) => in_title = true,
            Event::Tag("title", Type::End, _) => in_title = false,
            Event::Tag("text", Type::Start, attrs) => {
                open_text = Some(Label {
                    class: attr(&attrs, "class").unwrap_or_default(),
                    style: attr(&attrs, "style"),
                    text: String::new(),
                });
            }
            Event::Text(content) if in_title => {
                let content = content.trim().to_string();
                let table = tables.last_mut().expect("inside a table");
                if open_text.is_some() {
                    let entry = table
                        .rows
                        .last_mut()
                        .and_then(|r| r.entries.last_mut())
                        .expect("a text tooltip inside an entry");
                    entry.far_title = Some(content);
                } else if let Some(cavity) = table.cavities.last_mut() {
                    cavity.title = Some(content);
                }
            }
            Event::Text(content) => {
                if let Some(label) = open_text.as_mut() {
                    label.text.push_str(content.trim());
                }
            }
            Event::Tag("text", Type::End, _) => {
                let (Some(label), Some(table)) = (open_text.take(), tables.last_mut()) else {
                    continue;
                };
                let in_entry = groups.last().is_some_and(|g| g == "pin-entry");
                let in_row = groups.iter().any(|g| g == "pin-row");
                if in_entry {
                    let entry = table
                        .rows
                        .last_mut()
                        .and_then(|r| r.entries.last_mut())
                        .expect("an entry");
                    match label.class.as_str() {
                        "pin-gauge" => entry.gauge = Some(label.text.clone()),
                        "pin-far" => entry.far = Some(label.text.clone()),
                        class if class.starts_with("pin-wire") => {
                            entry.wire = Some((label.text.clone(), class.to_string()));
                        }
                        other => panic!("an unexpected entry text {other}"),
                    }
                } else if in_row {
                    let row = table.rows.last_mut().expect("a row");
                    match label.class.as_str() {
                        "pin-num" => row.pin = label.text.clone(),
                        "pin-label" => row.label = label.text.clone(),
                        "pin-wire pin-unwired" => row.unwired = true,
                        class if class.starts_with("pin-note") => {
                            row.note = Some((label.text.clone(), class.to_string()));
                        }
                        other => panic!("an unexpected row text {other}"),
                    }
                }
                table.labels.push(label);
            }
            _ => {}
        }
    }
    tables
}

#[test]
fn rows_are_listed_in_pin_order_then_the_ports_with_no_pin() {
    let tables = tables();
    assert_eq!(tables.len(), 2);
    // Declared 45, 42, 46, 48, 47, 49 and 3, 1, 4, 2, then SPARE with none.
    assert_eq!(tables[0].row_pins(), ["42", "45", "46", "47", "48", "49"]);
    assert_eq!(tables[1].row_pins(), ["1", "2", "3", "4", ""]);
    let spare = tables[1].rows.last().expect("a row");
    assert_eq!(spare.label, "spare · SPARE");
    assert_eq!(
        spare.note,
        Some(("no pin".to_string(), "pin-note pin-unpinned".to_string()))
    );
    assert_eq!(entries(spare), ["VT 0.5mm² → Vehicle.vcm.spare (x1:7)"]);
}

#[test]
fn each_header_names_the_component_connector_description_and_part() {
    let tables = tables();
    assert_eq!(tables[0].texts("pinout-title"), ["Inverter · control"]);
    assert_eq!(tables[0].texts("pinout-part"), ["Control 6+4p · CT-10"]);
    assert_eq!(tables[1].texts("pinout-title"), ["Inverter · signal"]);
    assert_eq!(tables[1].texts("pinout-part"), ["Signal 2x3 · SG-6"]);
}

#[test]
fn a_type_placed_more_than_once_is_named_by_its_type_and_says_whose_wires_it_lists() {
    let tables = tables_of("Sensor pinout");
    let [sensor] = &tables[..] else {
        panic!("one table");
    };
    // Neither "Front sensor" nor "Rear sensor": the sheet serves both, but
    // its wires are the first placement's.
    assert_eq!(sensor.texts("pinout-title"), ["Sensor · x1"]);
    assert_eq!(
        sensor.texts("pinout-part"),
        [
            "Sensor 2p",
            "Wires shown for Vehicle.front_sensor, one of 2 placements"
        ]
    );
    assert_eq!(
        entries(sensor.row("1")),
        ["OG 0.35mm² → Vehicle.inv.wake (signal:4)"]
    );
}

#[test]
fn a_double_crimp_lists_both_conductors_under_one_pin() {
    let tables = tables();
    let wake = tables[1].row("4");
    assert_eq!(wake.label, "wake · WAKE");
    assert_eq!(
        entries(wake),
        [
            "OG 0.5mm² → Vehicle.vcm.wake (x1:3)",
            "OG 0.35mm² → Vehicle.front_sensor.sig (x1:1)",
        ]
    );
    // The pin is not repeated for the second conductor.
    assert_eq!(
        tables[1].row_pins().iter().filter(|p| **p == "4").count(),
        1
    );
}

#[test]
fn a_swatch_carries_the_wires_paint_and_a_two_tone_its_tracer() {
    let tables = tables();
    let can_h = &tables[1].row("3").entries[0];
    let paint = |fill: &str| Paint {
        fill: fill.to_string(),
        data_color: "white/blue".to_string(),
    };
    assert_eq!(can_h.swatch, Some(paint("white")));
    assert_eq!(can_h.tracer, Some(paint("blue")));
    assert_eq!(
        can_h.wire,
        Some(("WH/BU".to_string(), "pin-wire".to_string()))
    );
    let can_l = &tables[1].row("1").entries[0];
    assert_eq!(can_l.tracer, None);
    assert_eq!(
        can_l.swatch.as_ref().map(|p| p.data_color.as_str()),
        Some("white")
    );
}

#[test]
fn a_drain_is_listed_without_a_colour_or_gauge() {
    let tables = tables();
    let shld = tables[1].row("2");
    let [drain] = &shld.entries[..] else {
        panic!("one entry: {shld:?}");
    };
    assert_eq!(drain.swatch, None);
    assert_eq!(drain.gauge, None);
    assert_eq!(
        drain.wire,
        Some(("drain".to_string(), "pin-wire pin-drain".to_string()))
    );
    assert_eq!(drain.far.as_deref(), Some("can shield"));
}

#[test]
fn a_ganged_port_lists_its_conductor_on_each_pin_marked_as_ganged() {
    let tables = tables();
    let control = &tables[0];
    for (pin, gang) in [
        ("46", "46+48"),
        ("48", "46+48"),
        ("47", "47+49"),
        ("49", "47+49"),
    ] {
        let row = control.row(pin);
        assert_eq!(
            row.note,
            Some((format!("ganged {gang}"), "pin-note pin-ganged".to_string())),
            "pin {pin}"
        );
        assert_eq!(row.entries.len(), 1, "pin {pin}");
    }
    assert_eq!(
        entries(control.row("46")),
        ["RD 2.5mm² → Vehicle.vcm.feed (x1:4)"]
    );
}

#[test]
fn a_conductor_to_a_splice_names_it_and_what_it_joins_as_room_allows() {
    let tables = tables();
    let b_neg = &tables[0].row("47").entries[0];
    // The far-end column is capped short of the whole list, so it
    // collapses; the tooltip keeps it whole.
    assert_eq!(
        b_neg.far.as_deref(),
        Some("→ Vehicle.sp_gnd (front_sensor.gnd, +2 more)")
    );
    assert_eq!(
        b_neg.far_title.as_deref(),
        Some("→ Vehicle.sp_gnd (front_sensor.gnd, rear_sensor.gnd, vcm.gnd)")
    );
    // From the sensor's side the splice joins the inverter's B-.
    let sensor = tables_of("Sensor pinout");
    assert_eq!(
        sensor[0].row("2").entries[0].far.as_deref(),
        Some("→ Vehicle.sp_gnd (inv.b_neg, +2 more)")
    );
}

#[test]
fn a_pin_no_conductor_reaches_is_listed_visibly_unwired() {
    let tables = tables();
    let ignition = tables[0].row("42");
    assert!(ignition.unwired);
    assert!(ignition.entries.is_empty());
    assert!(!tables[0].row("46").unwired);
}

#[test]
fn a_conductor_leaving_through_a_boundary_port_is_marked_as_leaving() {
    // The inverter's own TEMP+ conductor leaves it at the connectorless
    // TEMP OUT, where the vehicle's wire carries it on: not a destination.
    let tables = tables();
    assert_eq!(
        entries(tables[0].row("45")),
        ["GY 0.5mm² → Vehicle.inv.temp_out (leaves Inverter)"]
    );
}

#[test]
fn the_face_panel_and_the_table_are_each_as_wide_as_their_own_content() {
    for table in tables() {
        let panel = table.panel.expect("a face panel");
        let frame = table.frame.expect("a table outline");
        // The panel is the face with its padding (or the caption's room);
        // the table is not stretched to it, nor it to the table.
        assert!(panel.width >= table.face_extent() + 24.0);
        assert_ne!(panel.width, frame.width);
        assert_eq!(panel.centre(), frame.centre());
    }
    let control = &tables()[0];
    assert_eq!(
        control.panel.expect("a face panel").width,
        control.face_extent() + 24.0
    );
}

#[test]
fn each_face_says_it_is_viewed_from_the_harness_side() {
    for table in tables() {
        assert_eq!(table.texts("pinout-caption"), ["Viewed from harness side"]);
    }
}

#[test]
fn unused_cavities_carry_a_class_of_their_own() {
    let tables = tables();
    assert_eq!(tables[0].unused_pins(), [40, 41, 43, 44]);
    assert_eq!(tables[1].unused_pins(), [5, 6]);
    for table in &tables {
        let unused = table
            .cavities
            .iter()
            .filter(|c| c.class == "cavity cavity-unused")
            .count();
        assert_eq!(unused, table.unused_pins().len());
    }
}

#[test]
fn a_label_too_long_for_its_cavity_is_cut_and_kept_whole_elsewhere() {
    let tables = tables();
    let control = &tables[0];
    let cut: Vec<&Label> = control
        .labels
        .iter()
        .filter(|l| l.class == "cavity-label" && l.text.ends_with('…'))
        .collect();
    let [label] = cut[..] else {
        panic!("one cut label: {cut:?}");
    };
    assert!(label.text.starts_with("IGNITIO"), "{}", label.text);
    assert_eq!(label.style.as_deref(), Some("font-size: 6px"));
    // The whole label is the cavity's tooltip, and the table row's text.
    let titled: Vec<&str> = control
        .cavities
        .iter()
        .filter_map(|c| c.title.as_deref())
        .collect();
    assert_eq!(titled, ["IGNITION SWITCHED FEED"]);
    assert_eq!(control.row("42").label, "ignition · IGNITION SWITCHED FEED");
    // Short labels keep the stylesheet's size.
    for label in control.labels.iter().filter(|l| l.class == "cavity-label") {
        if !label.text.ends_with('…') {
            assert_eq!(label.style, None, "{label:?}");
        }
    }
}
