//! End-to-end: parse the worked example, validate, render, and check
//! the SVG carries the structural fragments we expect.
//!
//! We deliberately do *not* snapshot the raw SVG string — layout pixels
//! change too easily. Fragment asserts plus a snapshot of the parsed
//! model lock the contract down without locking down the pixels.

use wirebug::{Model, View, render};

const MODEL: &str = "examples/model.yaml";
const VIEW: &str = "examples/views/hv_overview.yaml";

#[test]
fn example_renders_with_expected_fragments() {
    let model = Model::load(MODEL).expect("model parses");
    let view = View::load(VIEW).expect("view parses");

    let report = model.validate().expect("model validates");
    // Two warnings: contactor.coil.{pos,neg} are intentionally dangling.
    assert_eq!(report.warnings.len(), 2);

    let view_report = view.validate(&model).expect("view validates");
    assert!(view_report.is_empty());

    let svg = render::render(&model, &view).expect("renders");

    assert!(svg.contains("<svg"));
    assert!(svg.contains("viewBox="));
    assert!(svg.contains("HV Power Path"));

    // Component labels
    for label in [
        "400 V Battery",
        "Main Contactor",
        "EM57 Inverter",
        "EM57 Motor",
    ] {
        assert!(svg.contains(label), "expected SVG to mention {label:?}");
    }

    assert_eq!(svg.matches("class=\"component\"").count(), 4);
    assert_eq!(svg.matches("class=\"wire\"").count(), 12);
    assert_eq!(svg.matches("class=\"port\"").count(), 26);

    // Every placed port carries a pin label (every port in the example
    // has a pin number).
    assert_eq!(svg.matches("class=\"port-pin\"").count(), 26);

    // The SVG emitter wraps text content across lines, so collapse
    // inter-tag whitespace before asserting on specific pin labels.
    let normalised: String = svg.lines().map(str::trim).collect();
    for pin in [">B+<", ">B-<", ">A1<", ">A2<", ">U<", ">V<", ">W<"] {
        assert!(normalised.contains(pin), "expected pin marker {pin:?}");
    }
}

#[test]
fn example_warns_about_unconnected_contactor_coil() {
    let model = Model::load(MODEL).expect("model parses");
    let report = model.validate().expect("validates");

    let warnings: Vec<String> = report.warnings.iter().map(ToString::to_string).collect();
    assert!(
        warnings.iter().any(|w| w.contains("contactor.coil.pos")),
        "expected warning for contactor.coil.pos in {warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| w.contains("contactor.coil.neg")),
        "expected warning for contactor.coil.neg in {warnings:?}"
    );
}

#[test]
fn parsed_example_model_snapshot() {
    let model = Model::load(MODEL).expect("model parses");
    insta::assert_debug_snapshot!(model);
}

/// End-to-end through the library entry point used by the CLI binary.
/// Future e2e tests should look like this — no filesystem write, no
/// `assert_cmd` spin-up.
#[test]
fn render_paths_returns_svg_and_warnings() {
    let result = wirebug::render_paths(MODEL, VIEW).expect("renders");
    assert!(result.svg.contains("<svg"));
    assert!(result.svg.contains("HV Power Path"));
    assert_eq!(result.warnings.len(), 2);
}
