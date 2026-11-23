//! End-to-end check of the `box_width_project` fixture: a schematic box is
//! sized from the ports it lists and their labels, never from its title,
//! so boxes of one type line up under titles of any length and a long
//! title overhangs its box.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::render::{SvgMode, render_views};

fn view(title: &str) -> String {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/box_width_project/main.wb");
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

/// The numeric value of attribute `name` on the one-line SVG `element`.
fn number(element: &str, name: &str) -> f64 {
    element
        .split(&format!(" {name}=\""))
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("{name} on {element}"))
}

/// One drawn component box: its rectangle, its title, and where the title
/// is centred.
#[derive(Debug, PartialEq)]
struct Drawn {
    x: f64,
    width: f64,
    height: f64,
    title: String,
    title_x: f64,
}

/// The box drawn for `instance` in `svg`.
fn drawn(svg: &str, instance: &str) -> Drawn {
    let group = format!("<g class=\"component\" data-component=\"{instance}\">");
    let mut lines = svg.lines().skip_while(|line| *line != group).skip(1);
    let rect = lines.next().expect("a rect");
    let label = lines.next().expect("a title");
    assert!(label.contains("class=\"component-label\""), "{label}");
    Drawn {
        x: number(rect, "x"),
        width: number(rect, "width"),
        height: number(rect, "height"),
        title: lines.next().expect("the title's text").to_string(),
        title_x: number(label, "x"),
    }
}

#[test]
fn fuses_of_one_type_line_up_under_titles_of_any_length() {
    let svg = view("Fuse Column");
    let [f1, f2, f3] = ["f1", "f2", "f3"].map(|name| drawn(&svg, name));

    assert_eq!(f2.title, "Cabin heater and demister supply fuse");
    assert_eq!((f1.title.as_str(), f3.title.as_str()), ("F1", "F3"));
    for fuse in [&f2, &f3] {
        assert_eq!(
            (fuse.x, fuse.width, fuse.height),
            (f1.x, f1.width, f1.height)
        );
    }
    // Each title is centred on its box, the long one too.
    for fuse in [&f1, &f2, &f3] {
        assert_eq!(fuse.title_x, fuse.x + fuse.width / 2.0);
    }
}

#[test]
fn labels_widen_a_box_and_its_title_does_not() {
    let column = view("Fuse Column");
    let relays = view("Relays");
    let fuse = drawn(&column, "f1");
    let [k1, k2] = ["k1", "k2"].map(|name| drawn(&relays, name));

    assert_eq!(k2.title, "Main contactor economiser relay");
    assert_eq!((k1.x, k1.width, k1.height), (k2.x, k2.width, k2.height));
    // `NORMALLY OPEN` beside the turned `SENSE` takes more than the
    // minimum width, which is all the fuse's `IN` and `OUT` need.
    assert!(k1.width > fuse.width, "{k1:?} {fuse:?}");
    // The controller's two long names face each other on one row.
    let ecu = drawn(&relays, "ecu");
    assert!(ecu.width > k1.width, "{ecu:?}");
}

#[test]
fn the_canvas_holds_a_title_that_overhangs_its_box() {
    let svg = view("Lone Fuse");
    let fuse = drawn(&svg, "f2");
    let root = svg.lines().next().expect("the svg element");
    let view_box: Vec<f64> = root
        .split(" viewBox=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a viewBox")
        .split(' ')
        .map(|n| n.parse().expect("a number"))
        .collect();
    let (left, width) = (view_box[0], view_box[2]);

    // With the title inside the box the canvas would be the box and a
    // 48-unit margin on each side; the title reaches further, evenly.
    assert!(width > fuse.width + 2.0 * 48.0, "{view_box:?}");
    assert_eq!(left + width / 2.0, fuse.x + fuse.width / 2.0);
}
