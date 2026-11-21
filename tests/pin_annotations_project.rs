//! End-to-end check of the `pin_annotations_project` fixture: a pin number
//! beside a port reads `<designator>:<pin>` on a component with more than
//! one connector and stays bare on a single-connector one, on every side of
//! a box, on the default grid and on the finest one a schematic allows.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::render::{SvgMode, render_views};

fn view(mode: SvgMode, title: &str) -> String {
    let main = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pin_annotations_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    let design = report.design.expect("a design");
    render_views(&design, mode)
        .into_iter()
        .map(|view| view.expect("view renders"))
        .find(|view| view.title == title)
        .unwrap_or_else(|| panic!("view {title:?}"))
        .svg
}

/// The value of attribute `name` on the one-line SVG `element`.
fn attr<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    element
        .split(&format!(" {name}=\""))
        .nth(1)
        .and_then(|rest| rest.split('"').next())
}

/// One drawn pin annotation: the port it labels, as `instance.port`, its
/// text, whether it is turned to run along the port's stub, and its anchor.
#[derive(Debug)]
struct Annotation {
    port: String,
    text: String,
    turned: bool,
    x: f64,
}

/// Every pin annotation of `svg`, in draw order.
fn annotations(svg: &str) -> Vec<Annotation> {
    let lines: Vec<&str> = svg.lines().collect();
    let (mut component, mut port) = ("", "");
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(name) = attr(line, "data-component") {
            component = name;
        }
        if let Some(name) = attr(line, "data-port") {
            port = name;
        }
        if attr(line, "class") == Some("port-pin") {
            found.push(Annotation {
                port: format!("{component}.{port}"),
                text: lines[i + 1].to_string(),
                turned: attr(line, "transform").is_some_and(|t| t.starts_with("rotate(-90 ")),
                x: attr(line, "x").and_then(|x| x.parse().ok()).expect("x"),
            });
        }
    }
    found
}

/// `(port, text, turned)` for each annotation, for comparing whole views.
fn summary(svg: &str) -> Vec<(String, String, bool)> {
    annotations(svg)
        .into_iter()
        .map(|a| (a.port, a.text, a.turned))
        .collect()
}

fn row(port: &str, text: &str, turned: bool) -> (String, String, bool) {
    (port.to_string(), text.to_string(), turned)
}

#[test]
fn pins_of_a_component_with_two_connectors_name_their_connector() {
    // `msd` has an HV and an interlock connector, each with a pin 1 and 2;
    // `lamp` and `left` have one connector, so their numbers stay bare,
    // `lamp.gnd` ganged over two pins.
    assert_eq!(
        summary(&view(SvgMode::Standalone, "MSD")),
        [
            row("msd.hv_in", "hv:1", false),
            row("msd.hv_out", "hv:2", false),
            // Side by side at the pitch, the two would run into each other,
            // so they turn to run along their stubs.
            row("msd.intlk_in", "intlk:1", true),
            row("msd.intlk_out", "intlk:2", true),
            row("lamp.sig", "4", false),
            row("lamp.pwr", "1", false),
            row("lamp.gnd", "2,3", false),
            row("left.pwr", "1", false),
            row("left.sig", "4", false),
        ]
    );
}

#[test]
fn long_designators_fit_at_the_minimum_grid() {
    // The fixture checks clean on grid 7.5, the minimum: the longer
    // annotations need no coarser grid.
    let svg = view(SvgMode::Standalone, "Fine grid");
    assert_eq!(
        summary(&svg),
        [
            row("bms.bat", "power:1", false),
            row("bms.gnd", "power:2,3,4", false),
            row("bms.can_h", "bms_signals:1", true),
            row("bms.can_l", "bms_signals:2", true),
            row("bms.wake", "bms_signals:3", true),
            row("bms.fault", "bms_signals:4", true),
            row("probe.pwr", "1", false),
            row("probe.sig", "4", false),
            row("probe.gnd", "2,3", false),
        ]
    );
    // Turned, neighbouring annotations stay one port pitch (two steps of
    // the 7.5 grid) apart, as their ports do.
    let along = |port: &str| {
        annotations(&svg)
            .into_iter()
            .find(|a| a.port == port)
            .map(|a| a.x)
            .expect(port)
    };
    assert_eq!(along("bms.can_l") - along("bms.can_h"), 15.0);
    assert_eq!(along("bms.fault") - along("bms.wake"), 15.0);
}

#[test]
fn an_embedded_view_keeps_the_pin_annotation_class() {
    let standalone = summary(&view(SvgMode::Standalone, "MSD"));
    let embedded = summary(&view(SvgMode::Embed, "MSD"));
    assert_eq!(embedded, standalone);
}
