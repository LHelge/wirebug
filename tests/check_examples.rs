//! End-to-end check of the seed project through the public pipeline.
//!
//! The `examples/` project is the gold-standard input: `check_project`
//! must run it cleanly and elaborate the expected design tree.

use std::path::Path;

use wirebug::dsl::check_project;
use wirebug::dsl::ir::{InstanceName, InstancePath, PortName};

#[test]
fn seed_project_checks_clean_and_elaborates() {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/main.wb");
    let report = check_project(Some(&main));

    assert!(
        report.problems.is_empty(),
        "seed project should check clean, got {} problems",
        report.problems.len()
    );

    let design = report.design.expect("a design was elaborated");
    assert_eq!(design.root.to_string(), "vehicle");

    // The hierarchy is stamped out to the leaves.
    let pack = InstancePath::root(InstanceName::from("vehicle"))
        .child(InstanceName::from("front"))
        .child(InstanceName::from("module_1"))
        .child(InstanceName::from("pack"));
    let pack = design.get(&pack).expect("vehicle.front.module_1.pack");
    assert_eq!(pack.type_name.as_str(), "cell_pack");
    assert!(pack.ports.contains_key(&PortName::from("hv_pos")));

    // Both battery packs are present and distinct types.
    let front =
        InstancePath::root(InstanceName::from("vehicle")).child(InstanceName::from("front"));
    let rear = InstancePath::root(InstanceName::from("vehicle")).child(InstanceName::from("rear"));
    assert_eq!(
        design.get(&front).unwrap().type_name.as_str(),
        "front_battery"
    );
    assert_eq!(
        design.get(&rear).unwrap().type_name.as_str(),
        "rear_battery"
    );

    // Three views came through (overview + two battery details).
    assert_eq!(design.views.len(), 3);
}
