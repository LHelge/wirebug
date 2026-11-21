//! End-to-end check of the `part_data_project` fixture: component types
//! carrying `part`, `manufacturer`, `description` and `rating`, placed
//! more than once, one type with none, an instance named `part`, and a
//! root whose `extend` fragment sets a key the root left unset.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::{Design, PartMeta};
use wirebug::render::{SvgMode, render_views};

fn design() -> Design {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/part_data_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    report.design.expect("a design")
}

/// The part data of the instance at `path`.
fn part(design: &Design, path: &str) -> PartMeta {
    design
        .instances
        .values()
        .find(|i| i.path.to_string() == path)
        .unwrap_or_else(|| panic!("no instance {path}"))
        .part
        .clone()
}

fn text(value: &str) -> Option<String> {
    Some(value.to_string())
}

#[test]
fn every_placement_of_a_type_carries_its_part_data() {
    let design = design();
    let contactor = PartMeta {
        number: text("EV200AAANA"),
        manufacturer: text("TE Connectivity"),
        description: text("Main contactor, 12 V coil"),
        rating: text("500 A"),
    };
    assert_eq!(part(&design, "Vehicle.pos"), contactor);
    assert_eq!(part(&design, "Vehicle.neg"), contactor);
}

#[test]
fn an_instance_named_part_is_placed_and_carries_its_types_data() {
    let design = design();
    assert_eq!(
        part(&design, "Vehicle.part"),
        PartMeta {
            number: text("0287010.PXCN"),
            rating: text("10 A"),
            ..PartMeta::default()
        }
    );
}

#[test]
fn a_type_without_part_data_has_none() {
    assert_eq!(part(&design(), "Vehicle.lamp"), PartMeta::default());
}

#[test]
fn an_extend_fragment_adds_a_key_the_root_left_unset() {
    assert_eq!(
        part(&design(), "Vehicle"),
        PartMeta {
            manufacturer: text("Wirebug Motors"),
            description: text("Demo vehicle"),
            ..PartMeta::default()
        }
    );
}

#[test]
fn part_data_is_not_drawn_in_the_schematic() {
    let design = design();
    let views = render_views(&design, SvgMode::Standalone)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("renders");
    let [main] = views.as_slice() else {
        panic!("expected one view");
    };
    for label in ["Main +", "Main -", "F1", "Lamp"] {
        assert!(main.svg.contains(label), "{label}");
    }
    for data in ["EV200AAANA", "TE Connectivity", "500 A", "0287010.PXCN"] {
        assert!(!main.svg.contains(data), "{data} drawn");
    }
}
