//! End-to-end check of the `pinout_project` fixture: a pinout view of two
//! connectors, one with an explicit face and one with a grid layout, whose
//! pins are declared out of order, one with a label too long for its
//! cavity, both with cavities no port uses.
//!
//! The SVG is read back into a small structure (each table's header, row
//! pins, caption and cavities) rather than snapshotted.
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

/// One cavity rectangle: its class, and its tooltip if it has one.
#[derive(Debug)]
struct Cavity {
    class: String,
    title: Option<String>,
}

/// One pin table of the drawing, in document order.
#[derive(Debug, Default)]
struct Table {
    labels: Vec<Label>,
    cavities: Vec<Cavity>,
}

impl Table {
    fn texts(&self, class: &str) -> Vec<&str> {
        self.labels
            .iter()
            .filter(|l| l.class == class)
            .map(|l| l.text.as_str())
            .collect()
    }

    fn row_pins(&self) -> Vec<u32> {
        self.texts("pin-num")
            .into_iter()
            .map(|pin| pin.parse().expect("a pin number"))
            .collect()
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
}

/// The pin tables of the fixture's one view, read back from its SVG.
fn tables() -> Vec<Table> {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pinout_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    let design = report.design.expect("a design");
    let [view] = &render_views(&design, SvgMode::Standalone)[..] else {
        panic!("one view");
    };
    let svg = &view.as_ref().expect("renders").svg;

    let mut tables: Vec<Table> = Vec::new();
    let mut open_text: Option<Label> = None;
    let mut in_title = false;
    for event in svg::read(svg).expect("parses") {
        match event {
            Event::Tag("g", Type::Start, attrs)
                if attrs.get("class").map(|c| c.to_string()) == Some("pinout".into()) =>
            {
                tables.push(Table::default());
            }
            // A cavity with a tooltip is an open `<rect>` holding a
            // `<title>`; one without is an empty element.
            Event::Tag("rect", Type::Start | Type::Empty, attrs)
                if attrs
                    .get("class")
                    .is_some_and(|c| c.to_string().starts_with("cavity")) =>
            {
                let table = tables.last_mut().expect("inside a table");
                table.cavities.push(Cavity {
                    class: attrs["class"].to_string(),
                    title: None,
                });
            }
            Event::Tag("title", Type::Start, _) => in_title = true,
            Event::Tag("title", Type::End, _) => in_title = false,
            Event::Tag("text", Type::Start, attrs) => {
                open_text = Some(Label {
                    class: attrs
                        .get("class")
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                    style: attrs.get("style").map(|s| s.to_string()),
                    text: String::new(),
                });
            }
            Event::Text(content) if in_title => {
                if let Some(cavity) = tables.last_mut().and_then(|t| t.cavities.last_mut()) {
                    cavity.title = Some(content.trim().to_string());
                }
            }
            Event::Text(content) => {
                if let Some(label) = open_text.as_mut() {
                    label.text.push_str(content.trim());
                }
            }
            Event::Tag("text", Type::End, _) => {
                if let (Some(label), Some(table)) = (open_text.take(), tables.last_mut()) {
                    table.labels.push(label);
                }
            }
            _ => {}
        }
    }
    tables
}

#[test]
fn rows_are_listed_in_pin_order() {
    let tables = tables();
    assert_eq!(tables.len(), 2);
    // Declared 45, 42, 46, 48, 47, 49 and 3, 1, 4.
    assert_eq!(tables[0].row_pins(), [42, 45, 46, 47, 48, 49]);
    assert_eq!(tables[1].row_pins(), [1, 3, 4]);
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
fn each_face_says_it_is_viewed_from_the_harness_side() {
    for table in tables() {
        assert_eq!(table.texts("pinout-caption"), ["Viewed from harness side"]);
    }
}

#[test]
fn unused_cavities_carry_a_class_of_their_own() {
    let tables = tables();
    assert_eq!(tables[0].unused_pins(), [40, 41, 43, 44]);
    assert_eq!(tables[1].unused_pins(), [2, 5, 6]);
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
    assert!(
        control
            .texts("pin-label")
            .contains(&"ignition · IGNITION SWITCHED FEED")
    );
    // Short labels keep the stylesheet's size.
    for label in control.labels.iter().filter(|l| l.class == "cavity-label") {
        if !label.text.ends_with('…') {
            assert_eq!(label.style, None, "{label:?}");
        }
    }
}
