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
/// text, the angle it is turned by to run along the port's stub (0 when
/// level; 90 reads downward, -90 upward), and its anchor.
#[derive(Debug)]
struct Annotation {
    port: String,
    text: String,
    turn: i32,
    x: f64,
    y: f64,
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
                turn: attr(line, "transform")
                    .and_then(|t| t.strip_prefix("rotate(")?.split(' ').next()?.parse().ok())
                    .unwrap_or(0),
                x: attr(line, "x").and_then(|x| x.parse().ok()).expect("x"),
                y: attr(line, "y").and_then(|y| y.parse().ok()).expect("y"),
            });
        }
    }
    found
}

/// `(port, text, turn)` for each annotation, for comparing whole views.
fn summary(svg: &str) -> Vec<(String, String, i32)> {
    annotations(svg)
        .into_iter()
        .map(|a| (a.port, a.text, a.turn))
        .collect()
}

fn row(port: &str, text: &str, turn: i32) -> (String, String, i32) {
    (port.to_string(), text.to_string(), turn)
}

#[test]
fn pins_of_a_component_with_two_connectors_name_their_connector() {
    // `msd` has an HV and an interlock connector, each with a pin 1 and 2;
    // `lamp` and `left` have one connector, so their numbers stay bare,
    // `lamp.gnd` ganged over two pins.
    assert_eq!(
        summary(&view(SvgMode::Standalone, "MSD")),
        [
            row("msd.hv_in", "hv:1", 0),
            row("msd.hv_out", "hv:2", 0),
            // Side by side at the pitch, the two would run into each other,
            // so they turn to run along their stubs, reading up toward the
            // port so the number ends beside it.
            row("msd.intlk_in", "intlk:1", -90),
            row("msd.intlk_out", "intlk:2", -90),
            row("lamp.sig", "4", 0),
            row("lamp.pwr", "1", 0),
            row("lamp.gnd", "2,3", 0),
            row("left.pwr", "1", 0),
            row("left.sig", "4", 0),
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
            row("bms.bat", "power:1", 0),
            row("bms.gnd", "power:2,3,4", 0),
            // Turned on both sides, each reading toward its port: down on
            // the north side, up on the south.
            row("bms.can_h", "bms_signals:1", 90),
            row("bms.can_l", "bms_signals:2", 90),
            row("bms.wake", "bms_signals:3", -90),
            row("bms.fault", "bms_signals:4", -90),
            row("probe.pwr", "1", 0),
            row("probe.sig", "4", 0),
            row("probe.gnd", "2,3", 0),
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

    // The north pins sit under the box's centred title, so their texts
    // move out past it: each ends, its number nearest the port, above the
    // title's 13px glyphs rather than under them.
    let title_baseline: f64 = svg
        .lines()
        .find(|line| attr(line, "class") == Some("component-label"))
        .and_then(|line| attr(line, "y")?.parse().ok())
        .expect("the BMS title");
    for annotation in annotations(&svg)
        .into_iter()
        .filter(|a| a.port == "bms.can_h" || a.port == "bms.can_l")
    {
        assert!(
            annotation.y < title_baseline - 13.0,
            "{annotation:?} under the title at {title_baseline}"
        );
    }
}

#[test]
fn an_embedded_view_keeps_the_pin_annotation_class() {
    let standalone = summary(&view(SvgMode::Standalone, "MSD"));
    let embedded = summary(&view(SvgMode::Embed, "MSD"));
    assert_eq!(embedded, standalone);
}
