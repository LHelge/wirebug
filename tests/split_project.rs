//! End-to-end check of a project whose root component is split across files
//! with `extend`. Asserts the fragments merge into one component sharing a
//! flat namespace (cross-fragment wires and views resolve), through the same
//! public pipeline as a single-file project.

use std::path::Path;

use wirebug::dsl::check_project;
use wirebug::dsl::ir::{InstanceName, InstancePath};

#[test]
fn split_root_merges_and_checks_clean() {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/split_project/main.wb");
    let report = check_project(Some(&main));

    assert!(
        report.problems.is_empty(),
        "split project should check clean, got: {:?}",
        report.problems
    );

    let design = report.design.expect("a design was elaborated");
    assert_eq!(design.root.to_string(), "vehicle");

    // `pack` is declared in main.wb, `inv` in traction.wb — both end up as
    // children of the one merged `vehicle`.
    let vehicle = InstancePath::root(InstanceName::from("vehicle"));
    let pack = vehicle.clone().child(InstanceName::from("pack"));
    let inv = vehicle.clone().child(InstanceName::from("inv"));
    assert_eq!(design.get(&pack).unwrap().type_name.as_str(), "battery");
    assert_eq!(design.get(&inv).unwrap().type_name.as_str(), "inverter");

    // The two HV wires, authored in traction.wb against `pack` (from main.wb),
    // land on the merged root.
    let root = design.get(&design.root).unwrap();
    assert_eq!(root.wires.len(), 2);
    assert!(
        root.wires.iter().all(|w| w.endpoints.len() == 2),
        "HV bus conductors are point-to-point"
    );

    // Views from both files come through: the overview (main.wb) and the
    // harness (traction.wb), each documenting the merged `vehicle`.
    assert_eq!(design.views.len(), 2);
    assert!(design.views.iter().any(|v| v.kind.is_harness()));
    assert!(design.views.iter().all(|v| v.subject.as_str() == "vehicle"));
}
