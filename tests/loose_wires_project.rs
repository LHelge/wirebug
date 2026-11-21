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

/// The value of attribute `name` on the one-line SVG `element`.
fn attr(element: &str, name: &str) -> f64 {
    element
        .split(&format!(" {name}=\""))
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("no numeric {name} on {element}"))
}

/// A rectangle as `(left, top, right, bottom)`.
type Rect = (f64, f64, f64, f64);

/// The outline of every cable box in `svg`: the first `<rect>` of each
/// `cable-box` group.
fn cable_boxes(svg: &str) -> Vec<Rect> {
    svg.split("<g class=\"connector cable-box\">")
        .skip(1)
        .map(|group| {
            let rect = group
                .lines()
                .find(|l| l.starts_with("<rect"))
                .expect("rect");
            let (x, y) = (attr(rect, "x"), attr(rect, "y"));
            (x, y, x + attr(rect, "width"), y + attr(rect, "height"))
        })
        .collect()
}

/// Each loose wire's annotation in `svg`: its text, and the area the 9px
/// text covers round its centred anchor (5.5 units a character).
fn loose_annotations(svg: &str) -> Vec<(String, Rect)> {
    svg.split("<g class=\"cable\">")
        .skip(1)
        .map(|group| {
            let mut lines = group.lines().skip_while(|l| !l.starts_with("<text"));
            let element = lines.next().expect("an annotation");
            let text = lines.next().expect("its text").to_string();
            let (x, y) = (attr(element, "x"), attr(element, "y"));
            let half = text.chars().count() as f64 * 5.5 / 2.0;
            (text, (x - half, y - 9.0, x + half, y + 2.0))
        })
        .collect()
}

/// Regression: a loose wire's annotation was drawn under any cable box its
/// pin-to-pin curve passed behind, so the box's fill hid it.
#[test]
fn no_loose_wire_annotation_lies_under_a_cable_box() {
    let expected = [
        (
            "Pump loom",
            vec!["WAKE · 0.5mm² · WH", "FAULT · 0.5mm² · GN/YE"],
        ),
        (
            "Lamp loom",
            vec!["SENSE · 0.5mm² · BU", "SENSE · 0.5mm² · BU"],
        ),
        (
            "Horn loom",
            vec!["W1 · 0.5mm² · WH", "W2 · 0.5mm² · WH", "HORN · 0.5mm² · BU"],
        ),
    ];
    for (title, annotations) in expected {
        let svg = view(title);
        let boxes = cable_boxes(&svg);
        assert!(!boxes.is_empty(), "{title}");
        let found = loose_annotations(&svg);
        let texts: Vec<&str> = found.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(texts, annotations, "{title}");
        for (text, (l, t, r, b)) in &found {
            for &(bl, bt, br, bb) in &boxes {
                let covered = *l < br && bl < *r && *t < bb && bt < *b;
                assert!(!covered, "{title}: {text:?} lies under a cable box");
            }
        }
    }
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
