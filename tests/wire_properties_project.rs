//! End-to-end check of the `wire_properties_project` fixture: wires that
//! carry a `{ length; type; part; }` body, loose and inside a cable (a
//! twisted pair included), elaborated into the IR and rendered.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::Design;
use wirebug::render::{SvgMode, render_views};

fn design() -> Design {
    let main = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/wire_properties_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    report.design.expect("a design")
}

#[test]
fn wire_bodies_land_on_the_ir_wires() {
    let design = design();
    let root = design.get(&design.root).expect("root instance");
    /// A wire as (label, length, type, part).
    type Row<'a> = (
        Option<&'a str>,
        Option<f64>,
        Option<&'a str>,
        Option<&'a str>,
    );
    let wires: Vec<Row> = root
        .wires
        .iter()
        .map(|w| {
            (
                w.label.as_deref(),
                w.length,
                w.r#type.as_deref(),
                w.part.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        wires,
        [
            (Some("12V"), Some(0.8), Some("FLRY-B"), Some("W-RD-1.5")),
            (Some("GND"), Some(2.4), None, None),
            (Some("CAN H"), None, None, Some("TP-YE")),
            (Some("CAN L"), None, None, None),
        ]
    );
}

#[test]
fn a_conductor_length_overrides_its_cables_and_the_rest_inherit_it() {
    let design = design();
    let root = design.get(&design.root).expect("root instance");
    let lengths: Vec<Option<f64>> = root
        .wires
        .iter()
        .map(|w| root.effective_length(w))
        .collect();
    assert_eq!(lengths, [Some(0.8), Some(2.4), Some(2.0), Some(2.0)]);
}

#[test]
fn a_project_with_wire_bodies_renders_its_cable() {
    let design = design();
    let views = render_views(&design, SvgMode::Standalone)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("renders");
    let [link] = views.as_slice() else {
        panic!("expected one view");
    };
    // The cable box still shows the cable's own type and length.
    assert!(link.svg.contains("ECU–BMS link"), "cable label");
    assert!(link.svg.contains("Shielded 3-core"), "cable type");
    for label in ["12V", "GND", "CAN H", "CAN L"] {
        assert!(link.svg.contains(label), "{label}");
    }
}
