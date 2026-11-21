//! End-to-end check of the `long_labels_project` fixture: a harness view
//! whose title, description line, pin labels and cable type are each too
//! long for a box sized at a flat 7 px a character. Each box is sized from
//! the bundled font's widths, so it holds its text as PNG and PDF draw it,
//! in DejaVu Sans.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::Design;
use wirebug::render::{SvgMode, render_views};

/// Inner padding either side of a harness table's text.
const PAD: f64 = 10.0;
/// The narrowest a harness table's pin-number column is drawn.
const PIN_COLUMN: f64 = 28.0;

fn design() -> Design {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/long_labels_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    report.design.expect("a design")
}

fn harness_svg() -> String {
    let views = render_views(&design(), SvgMode::Standalone);
    let [Ok(view)] = views.as_slice() else {
        panic!("one rendered view");
    };
    view.svg.clone()
}

/// The width of the box (a connector's table or a cable's box) that draws
/// `text`: the first `<rect>` of the innermost `connector` group before it.
fn box_width_around(svg: &str, text: &str) -> f64 {
    let at = svg
        .find(&format!(">\n{text}\n</text>"))
        .unwrap_or_else(|| panic!("{text:?} is drawn"));
    let group = svg[..at]
        .rfind("<g class=\"connector")
        .expect("a connector group");
    let rect = &svg[group..at];
    let width = rect
        .split("width=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("the box's width");
    width.parse().expect("a number")
}

#[test]
fn a_long_bold_title_fits_its_table() {
    let svg = harness_svg();
    // "High-mount stop lamp assembly" in DejaVu Sans Bold at 13 px.
    let drawn = 237.19;
    let width = box_width_around(&svg, "High-mount stop lamp assembly");
    assert!(width >= drawn + 2.0 * PAD, "{width}");
}

#[test]
fn a_long_description_line_fits_its_table() {
    let svg = harness_svg();
    // In DejaVu Sans at 10 px.
    let drawn = 219.10;
    let width = box_width_around(&svg, "x1 · Body controller, rear lighting connector");
    assert!(width >= drawn + 2.0 * PAD, "{width}");
}

#[test]
fn a_long_pin_label_fits_beside_its_pin_number() {
    let svg = harness_svg();
    // In DejaVu Sans at 11 px.
    let drawn = 160.36;
    let width = box_width_around(&svg, "STOP LAMP SUPPLY POSITIVE");
    assert!(width >= PIN_COLUMN + drawn + 2.0 * PAD, "{width}");
}

#[test]
fn a_long_cable_type_fits_its_cable_box() {
    let svg = harness_svg();
    // In DejaVu Sans at 10 px.
    let drawn = 330.46;
    let width = box_width_around(
        &svg,
        "Shielded twisted pair, halogen-free, 105 °C · 2.5 m · ×2 · shielded",
    );
    assert!(width >= drawn + 2.0 * PAD, "{width}");
}
