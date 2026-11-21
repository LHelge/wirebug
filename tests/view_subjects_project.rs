//! End-to-end check of the `view_subjects_project` fixture: a library file
//! holding two components documents each with a view `for` it, the root's
//! schematic names its subject explicitly, and its harness leaves `for`
//! out.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::Design;
use wirebug::render::{RenderedView, SvgMode, render_views};

fn design() -> Design {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/view_subjects_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    report.design.expect("a design")
}

fn rendered() -> Vec<RenderedView> {
    render_views(&design(), SvgMode::Standalone)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("renders")
}

#[test]
fn each_view_documents_the_component_it_names() {
    let design = design();
    let subjects: Vec<(&str, &str)> = design
        .views
        .iter()
        .map(|v| (v.title.as_str(), v.subject.as_str()))
        .collect();
    assert_eq!(
        subjects,
        [
            ("Overview", "Vehicle"),
            ("Loom", "Vehicle"),
            ("Sensor pinout", "Sensor"),
            ("Lamp pinout", "Lamp"),
        ]
    );
}

#[test]
fn each_pinout_draws_its_own_subjects_connector() {
    let views = rendered();
    let svg = |title: &str| {
        &views
            .iter()
            .find(|v| v.title == title)
            .unwrap_or_else(|| panic!("no view {title}"))
            .svg
    };
    let sensor = svg("Sensor pinout");
    assert!(sensor.contains("SIG") && sensor.contains("GND"));
    assert!(!sensor.contains("FEED"), "the lamp's port is not on it");
    let lamp = svg("Lamp pinout");
    assert!(lamp.contains("FEED") && lamp.contains("RET"));
    assert!(!lamp.contains("SIG"), "the sensor's port is not on it");
}
