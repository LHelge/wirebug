//! End-to-end check of the `fragment_project` fixture: `main.wb` pulls a
//! system file in with `fragment`, which pulls its loom file in with a path
//! relative to itself. Asserts every `extend` merges into the one root, the
//! views of every file render, and no name came along with the files.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::{Design, InstanceName, InstancePath};
use wirebug::render::{RenderedView, SvgMode, render_views};

fn design() -> Design {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fragment_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    report.design.expect("a design")
}

#[test]
fn fragments_merge_into_the_root_through_two_files() {
    let design = design();
    assert_eq!(design.root.to_string(), "Vehicle");
    let vehicle = InstancePath::root(InstanceName::from("Vehicle"));
    let child = |name: &str| vehicle.clone().child(InstanceName::from(name));
    // `pack` from main.wb, `inv` from systems/traction.wb.
    assert_eq!(
        design.get(&child("pack")).unwrap().type_name.as_str(),
        "Battery"
    );
    assert_eq!(
        design.get(&child("inv")).unwrap().type_name.as_str(),
        "Inverter"
    );
    // The cable from systems/hv_loom.wb, wired across both.
    let root = design.get(&design.root).unwrap();
    assert_eq!(root.wires.len(), 2);
    assert!(
        root.wires
            .iter()
            .all(|w| w.cable.as_ref().is_some_and(|c| c.as_str() == "hv_bus"))
    );
}

#[test]
fn views_of_every_pulled_in_file_render() {
    let rendered: Vec<RenderedView> = render_views(&design(), SvgMode::Standalone)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("renders");
    let titles: Vec<&str> = rendered.iter().map(|v| v.title.as_str()).collect();
    assert_eq!(titles, ["System Overview", "HV loom"]);
    let harness = &rendered[1].svg;
    assert!(harness.contains("HV bus"), "the cable box is drawn");
}
