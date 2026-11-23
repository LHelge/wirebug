//! End-to-end check of the `inline_project` fixture: a loom split by an
//! inline connector (a mated plug/receptacle pair), rendered as one
//! harness drawing per housing half and seen through by a schematic.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::render::{RenderedView, SvgMode, render_views};

fn rendered() -> Vec<RenderedView> {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/inline_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    let design = report.design.expect("a design");
    render_views(&design, SvgMode::Standalone)
        .into_iter()
        .collect::<Result<_, _>>()
        .expect("renders")
}

#[test]
fn each_loom_drawing_shows_its_own_half() {
    let views = rendered();
    let engine = views
        .iter()
        .find(|v| v.title == "Engine bay loom")
        .expect("engine view");
    let pedal = views
        .iter()
        .find(|v| v.title == "Pedal stub")
        .expect("pedal view");

    // The engine drawing carries the female half's part identity and badge;
    // the male half stays out of it entirely.
    assert!(
        engine.svg.contains("Deutsch DT06-3S · DT06-3S"),
        "female part"
    );
    assert!(engine.svg.contains("class=\"inline-badge\""));
    assert!(engine.svg.contains(">\nF\n</text>"));
    assert!(!engine.svg.contains("DT04-3P"));

    assert!(pedal.svg.contains("Deutsch DT04-3P · DT04-3P"), "male part");
    assert!(pedal.svg.contains(">\nM\n</text>"));
    assert!(!pedal.svg.contains("DT06-3S"));

    // Auto-scoping keeps each loom's conductors to its own drawing.
    assert!(engine.svg.contains("APPS1-A"));
    assert!(!engine.svg.contains("APPS1-B"));
    assert!(pedal.svg.contains("APPS1-B"));
    assert!(!pedal.svg.contains("APPS1-A"));

    // The shared pin set appears in both, under the inline's label.
    for view in [engine, pedal] {
        assert!(view.svg.contains("APPS1"));
        assert!(view.svg.contains("Pedal branch"));
    }
}

#[test]
fn a_schematic_draws_each_signal_straight_through_the_inline() {
    let views = rendered();
    let signals = views
        .iter()
        .find(|v| v.title == "Pedal signals")
        .expect("schematic view");

    // One wire per signal from the ECU to the pedal, in the colour of the
    // conductor on the pedal's side of the inline.
    assert_eq!(signals.svg.matches("class=\"wire\"").count(), 2);
    for color in ["green", "yellow"] {
        let stroked = format!("class=\"wire\" data-color=\"{color}\"");
        assert_eq!(signals.svg.matches(&stroked).count(), 1, "{color}");
    }

    // The pedal's ground is not listed, so the ECU's ground alone gets a
    // reference: the inline's port, then what lies beyond it.
    assert_eq!(signals.svg.matches("class=\"off-view\"").count(), 1);
    assert!(signals.svg.contains("ic_pedal.gnd (pedal.gnd)"));
    assert!(!signals.svg.contains("ic_pedal.apps1"));
}
