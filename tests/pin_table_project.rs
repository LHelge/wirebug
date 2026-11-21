//! End-to-end check of the `pin_table_project` fixture: the pin tables
//! built from a checked design list every cavity of every connector, sorted
//! by pin, with what is crimped into it and where each conductor goes.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::{Design, Half, InstanceName, InstancePath, Pin};
use wirebug::export::{Cavity, CavityEntry, FarEnd, HalfFit, PinTable, PinTables};

fn checked_design() -> Design {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pin_table_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    report.design.expect("a design")
}

fn path(dotted: &str) -> InstancePath {
    let mut segments = dotted.split('.').map(InstanceName::from);
    let root = InstancePath::root(segments.next().expect("a root"));
    segments.fold(root, |path, segment| path.child(segment))
}

fn table<'t, 'd>(
    tables: &'t PinTables<'d>,
    instance: &str,
    connector: &str,
    half: Option<Half>,
) -> &'t PinTable<'d> {
    tables
        .tables()
        .iter()
        .find(|t| {
            t.instance.path == path(instance)
                && t.connector.name.as_str() == connector
                && t.half == half
        })
        .unwrap_or_else(|| panic!("a table of {instance}.{connector} ({half:?})"))
}

/// A cavity's entries, typed into what a crimp sheet prints: a conductor
/// as its label, colour code, gauge, far end and half fit; a drain as its
/// screen's label and its other end.
#[derive(Debug, Clone, PartialEq)]
enum Line {
    Conductor {
        owner: String,
        label: String,
        code: String,
        gauge: f64,
        far: String,
        half: HalfFit,
    },
    Drain {
        label: String,
        far: Option<String>,
    },
}

fn far(end: &FarEnd) -> String {
    match end {
        FarEnd::Port(port) => format!("{}.{}", port.instance.path, port.port.name),
        FarEnd::Boundary(port) => format!("boundary {}.{}", port.instance.path, port.port.name),
        FarEnd::Splice(splice) => {
            let beyond: Vec<String> = splice.beyond.iter().map(|b| b.end.to_string()).collect();
            format!("{} ({})", splice.splice.splice.name, beyond.join(", "))
        }
        FarEnd::Unresolved(end) => panic!("a checked design resolves every end: {end}"),
    }
}

fn lines(cavity: &Cavity) -> Vec<Line> {
    cavity
        .entries
        .iter()
        .map(|entry| match entry {
            CavityEntry::Conductor(c) => Line::Conductor {
                owner: c.owner.path.to_string(),
                label: c.conductor.wire.label.clone().unwrap_or_default(),
                code: c.conductor.wire.color.code(),
                gauge: c.conductor.wire.gauge,
                far: far(&c.far),
                half: c.half,
            },
            CavityEntry::Drain(d) => Line::Drain {
                label: d.shield.shield.label.clone().unwrap_or_default(),
                far: d.far.map(|end| end.to_string()),
            },
        })
        .collect()
}

fn conductor(owner: &str, label: &str, code: &str, gauge: f64, far: &str) -> Line {
    Line::Conductor {
        owner: owner.to_string(),
        label: label.to_string(),
        code: code.to_string(),
        gauge,
        far: far.to_string(),
        half: HalfFit::Assigned,
    }
}

#[test]
fn every_connector_of_every_placement_has_a_table_in_source_order() {
    let design = checked_design();
    let tables = PinTables::from(&design);
    let names: Vec<String> = tables
        .into_iter()
        .map(|t| {
            let half = t.half.map_or(String::new(), |h| format!(" {}", h.as_str()));
            format!("{}.{}{half}", t.instance.path, t.connector.name)
        })
        .collect();
    assert_eq!(
        names,
        [
            "Vehicle.ecu.x1",
            "Vehicle.bcm.x1",
            "Vehicle.pedal.x1",
            "Vehicle.front.x9",
            "Vehicle.front.lamp.x1",
            "Vehicle.rear.x9",
            "Vehicle.rear.lamp.x1",
            "Vehicle.ic.ic male",
            "Vehicle.ic.ic female",
        ]
    );
}

#[test]
fn the_ecu_connector_lists_each_cavity_by_pin_with_what_it_holds() {
    let design = checked_design();
    let tables = PinTables::from(&design);
    let ecu = table(&tables, "Vehicle.ecu", "x1", None);
    let rows: Vec<(u32, Option<&str>, Vec<Line>)> = ecu
        .cavities
        .iter()
        .map(|c| (c.pin.0, c.port.map(|p| p.label.as_str()), lines(c)))
        .collect();
    let v = "Vehicle";
    let gnd = || {
        vec![conductor(
            v,
            "GND",
            "BK",
            1.0,
            "sp_gnd (Vehicle.bcm.gnd, Vehicle.ic.ret)",
        )]
    };
    assert_eq!(
        rows,
        [
            // A double crimp: both conductors, in wire order.
            (
                1,
                Some("12V"),
                vec![
                    conductor(v, "12V", "RD", 1.5, "Vehicle.bcm.pwr"),
                    conductor(v, "12V", "RD", 0.5, "Vehicle.front.feed"),
                ]
            ),
            // A ganged port: its conductor in each of its cavities.
            (2, Some("GND"), gnd()),
            (3, Some("GND"), gnd()),
            (
                4,
                Some("CAN H"),
                vec![conductor(v, "CAN H", "YE", 0.5, "Vehicle.bcm.can_h")]
            ),
            (
                5,
                Some("CAN L"),
                vec![conductor(v, "CAN L", "GN", 0.5, "Vehicle.bcm.can_l")]
            ),
            (
                6,
                Some("SHLD"),
                vec![Line::Drain {
                    label: "CAN SHLD".to_string(),
                    far: Some("Vehicle.bcm.shld".to_string()),
                }]
            ),
            // Cavities of the housing no port uses.
            (7, None, vec![]),
            (8, None, vec![]),
            (9, None, vec![]),
            (
                10,
                Some("SIG"),
                vec![conductor(v, "SIG", "GN", 0.5, "Vehicle.ic.sig")]
            ),
            (11, None, vec![]),
            (12, None, vec![]),
        ]
    );
    let gang: Vec<(u32, bool, Vec<Pin>)> = ecu
        .cavities
        .iter()
        .take(4)
        .map(|c| (c.pin.0, c.is_ganged(), c.ganged_with().collect()))
        .collect();
    assert_eq!(
        gang,
        [
            (1, false, vec![]),
            (2, true, vec![Pin(3)]),
            (3, true, vec![Pin(2)]),
            (4, false, vec![]),
        ]
    );
    let can = ecu.cavities[3].entries.first();
    let Some(CavityEntry::Conductor(can)) = can else {
        panic!("a conductor on pin 4");
    };
    assert_eq!(can.cable.map(|c| c.name.as_str()), Some("can"));
    assert_eq!(can.conductor.wire.twisted_group, Some(0));
}

#[test]
fn a_far_end_on_a_connectorless_port_is_named() {
    let design = checked_design();
    let tables = PinTables::from(&design);
    let bcm = table(&tables, "Vehicle.bcm", "x1", None);
    let sig = bcm
        .cavities
        .iter()
        .find(|c| c.pin == Pin(10))
        .expect("pin 10");
    assert_eq!(
        lines(sig),
        [conductor(
            "Vehicle",
            "SENSE",
            "BU",
            0.35,
            "Vehicle.ecu.sense"
        )]
    );
}

#[test]
fn a_loom_placed_twice_takes_a_conductor_into_its_connector_from_each_side() {
    let design = checked_design();
    let tables = PinTables::from(&design);
    for (loom, from) in [("front", "ecu"), ("rear", "bcm")] {
        let x9 = table(&tables, &format!("Vehicle.{loom}"), "x9", None);
        let [feed] = &x9.cavities[..] else {
            panic!("one cavity");
        };
        assert_eq!(
            lines(feed),
            [
                conductor("Vehicle", "12V", "RD", 0.5, &format!("Vehicle.{from}.pwr")),
                conductor(
                    &format!("Vehicle.{loom}"),
                    "FEED",
                    "OG",
                    1.0,
                    &format!("Vehicle.{loom}.lamp.sig")
                ),
            ]
        );
        let lamp = table(&tables, &format!("Vehicle.{loom}.lamp"), "x1", None);
        assert_eq!(
            lines(&lamp.cavities[0]),
            [conductor(
                &format!("Vehicle.{loom}"),
                "FEED",
                "OG",
                1.0,
                &format!("boundary Vehicle.{loom}.feed")
            )]
        );
    }
}

#[test]
fn the_harness_views_place_each_inline_conductor_in_its_half() {
    let design = checked_design();
    let tables = PinTables::from(&design);
    let half = |half: Half| -> Vec<(u32, Vec<Line>)> {
        table(&tables, "Vehicle.ic", "ic", Some(half))
            .cavities
            .iter()
            .map(|c| (c.pin.0, lines(c)))
            .collect()
    };
    let v = "Vehicle";
    // The ground branch draws the splice's conductor into the male half,
    // the main loom into the female: it is listed under both, flagged.
    let spliced = Line::Conductor {
        owner: v.to_string(),
        label: "RET".to_string(),
        code: "BK".to_string(),
        gauge: 0.5,
        far: "sp_gnd (Vehicle.ecu.gnd, Vehicle.bcm.gnd)".to_string(),
        half: HalfFit::Undetermined,
    };
    let male = half(Half::Male);
    assert_eq!(
        male,
        [
            (1, vec![conductor(v, "SIG", "GN", 0.5, "Vehicle.pedal.sig")]),
            (
                2,
                vec![
                    spliced.clone(),
                    conductor(v, "RET", "BK", 0.5, "Vehicle.pedal.ret"),
                ]
            ),
        ]
    );
    assert_eq!(
        half(Half::Female),
        [
            (1, vec![conductor(v, "SIG", "GN", 0.5, "Vehicle.ecu.sig")]),
            (2, vec![spliced]),
        ]
    );
}
