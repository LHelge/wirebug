//! End-to-end check of the `loose_wires_project` fixture: loose wires
//! between connectors a harness view also runs cables between, one of them
//! two-tone, and a loose wire to a splice. Each is drawn through a lane of
//! its own on the spine, clear of the cable boxes (the layout's own tests
//! check the geometry), and annotated like a cable's conductor.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::render::{SvgMode, render_views};

fn view(title: &str) -> String {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/loose_wires_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    let design = report.design.expect("a design");
    render_views(&design, SvgMode::Standalone)
        .into_iter()
        .map(|view| view.expect("view renders"))
        .find(|view| view.title == title)
        .unwrap_or_else(|| panic!("view {title:?}"))
        .svg
}

/// The SVG text element carrying exactly `text`.
fn text(text: &str) -> String {
    format!(">\n{text}\n</text>")
}

#[test]
fn each_loose_wire_carries_its_label_gauge_and_colour_code() {
    let pump = view("Pump loom");
    for annotation in ["WAKE · 0.5mm² · WH", "FAULT · 0.5mm² · GN/YE"] {
        assert_eq!(pump.matches(&text(annotation)).count(), 1, "{annotation}");
    }
    // The cables' own conductors keep theirs.
    assert!(pump.contains(&text("PWR · 1.5mm² · RD")));

    let lamp = view("Lamp loom");
    assert_eq!(lamp.matches(&text("SENSE · 0.5mm² · BU")).count(), 2);
}

#[test]
fn a_loose_wire_through_its_lane_is_drawn_like_a_strand_with_no_box() {
    let pump = view("Pump loom");
    // Two loose wires, each in a `cable` group of its own beside the two
    // cable boxes.
    assert_eq!(pump.matches("<g class=\"cable\">").count(), 2);
    assert_eq!(pump.matches("class=\"connector cable-box\"").count(), 2);
    // Lead-in, run and lead-out make one path per layer: the white wire's
    // core, and the two-tone wire's core and its tracer.
    assert_eq!(pump.matches("data-color=\"white\"").count(), 1);
    assert_eq!(pump.matches("data-color=\"green/yellow\"").count(), 2);
    let fault = pump
        .split("<path")
        .find(|element| element.contains("data-color=\"green/yellow\""))
        .expect("the two-tone wire's core");
    assert_eq!(fault.matches(" C").count(), 3, "{fault}");
}
