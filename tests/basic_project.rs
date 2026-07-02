//! End-to-end check of the `basic_project` fixture through the public
//! pipeline.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project. Anything worth locking in end-to-end gets added to a
//! fixture under `tests/fixtures/` instead.

use std::path::Path;

use wirebug::dsl::check_project;
use wirebug::dsl::ir::{InstanceName, InstancePath, PortName};

#[test]
fn fixture_project_checks_clean_and_elaborates() {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/basic_project/main.wb");
    let report = check_project(Some(&main));

    assert!(
        report.problems.is_empty(),
        "fixture project should check clean, got {} problems",
        report.problems.len()
    );

    let design = report.design.expect("a design was elaborated");
    assert_eq!(design.root.to_string(), "Vehicle");

    // The hierarchy is stamped out to the leaves.
    let pack = InstancePath::root(InstanceName::from("Vehicle"))
        .child(InstanceName::from("pack"))
        .child(InstanceName::from("pack"));
    let pack = design.get(&pack).expect("Vehicle.pack.pack");
    assert_eq!(pack.type_name.as_str(), "CellPack");
    assert!(pack.ports.contains_key(&PortName::from("hv_pos")));

    // Imported child instances are present and distinct types.
    let battery =
        InstancePath::root(InstanceName::from("Vehicle")).child(InstanceName::from("pack"));
    let inverter =
        InstancePath::root(InstanceName::from("Vehicle")).child(InstanceName::from("inv"));
    assert_eq!(design.get(&battery).unwrap().type_name.as_str(), "Battery");
    assert_eq!(
        design.get(&inverter).unwrap().type_name.as_str(),
        "Inverter"
    );

    assert_eq!(design.views.len(), 4);
    assert_eq!(
        design.views.iter().filter(|v| v.kind.is_harness()).count(),
        1,
        "the harness view is present"
    );
    assert_eq!(
        design.views.iter().filter(|v| v.kind.is_pinout()).count(),
        1,
        "the pinout view is present"
    );
}
