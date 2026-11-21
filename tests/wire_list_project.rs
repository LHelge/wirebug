//! End-to-end check of the `wire_list_project` fixture: the wire list built
//! from a checked design holds one row per physical conductor and per
//! cable screen, each end resolved to what it lands on, in source order.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::{
    ConnectorName, Design, InstanceName, InstancePath, Length, LengthUnit, Pin, Screened,
};
use wirebug::export::{Contact, Drain, End, Row, RowKind, WireList};

fn checked_design() -> Design {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wire_list_project/main.wb");
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

fn metres(value: f64) -> Option<Length> {
    Length::new(value, LengthUnit::Metre)
}

/// What an end lands on, typed: a pin (instance path, port, designator and
/// pins), a connectorless port, the owner's own port, or a splice.
#[derive(Debug, PartialEq)]
enum At {
    Pin(InstancePath, String, ConnectorName, Vec<Pin>),
    Connectorless(InstancePath, String),
    Boundary(InstancePath, String),
    Splice(InstancePath, String),
}

impl At {
    fn of(end: &End) -> Self {
        match end {
            End::Port(port) => match port.contact() {
                Contact::Connector { designator, pins } => Self::Pin(
                    port.instance.path.clone(),
                    port.port.name.to_string(),
                    designator.clone(),
                    pins.to_vec(),
                ),
                Contact::Connectorless => {
                    Self::Connectorless(port.instance.path.clone(), port.port.name.to_string())
                }
            },
            End::Boundary(port) => {
                Self::Boundary(port.instance.path.clone(), port.port.name.to_string())
            }
            End::Splice(splice) => {
                Self::Splice(splice.owner.path.clone(), splice.splice.name.to_string())
            }
            End::Unresolved(end) => panic!("a checked design resolves every end: {end}"),
        }
    }
}

fn pin(instance: &str, port: &str, pins: &[u32]) -> At {
    let designator = match instance {
        "Vehicle.ic" => "ic",
        _ => "x1",
    };
    At::Pin(
        path(instance),
        port.to_string(),
        ConnectorName::from(designator),
        pins.iter().copied().map(Pin).collect(),
    )
}

/// The conductor rows of `owner`, as their typed ends.
fn conductors_of(list: &WireList, owner: &str) -> Vec<[At; 2]> {
    list.conductors()
        .filter(|(row, _)| row.owner.path == path(owner))
        .map(|(_, conductor)| conductor.ends.each_ref().map(At::of))
        .collect()
}

#[test]
fn rows_are_listed_per_owner_in_source_order() {
    let design = checked_design();
    let list = WireList::from(&design);
    let owners: Vec<(String, bool)> = list
        .rows()
        .iter()
        .map(|row| {
            (
                row.owner.path.to_string(),
                matches!(row.kind, RowKind::Shield(_)),
            )
        })
        .collect();
    let mut expected: Vec<(String, bool)> = Vec::new();
    expected.extend(std::iter::repeat_n(("Vehicle".to_string(), false), 15));
    expected.extend(std::iter::repeat_n(("Vehicle".to_string(), true), 2));
    expected.push(("Vehicle.front".to_string(), false));
    expected.push(("Vehicle.rear".to_string(), false));
    assert_eq!(owners, expected);

    // Built again, from a design checked again, the list is the same.
    let again = checked_design();
    let ends = |list: &WireList| -> Vec<Vec<String>> {
        list.rows()
            .iter()
            .map(|row| row.ends().iter().map(ToString::to_string).collect())
            .collect()
    };
    assert_eq!(ends(&WireList::from(&again)), ends(&list));
}

#[test]
fn every_end_of_the_root_resolves_to_what_it_lands_on() {
    let design = checked_design();
    let list = WireList::from(&design);
    let feed = |loom: &str| At::Connectorless(path(loom), "feed".to_string());
    assert_eq!(
        conductors_of(&list, "Vehicle"),
        [
            // A rail of four ends: three conductors, in written order.
            [
                pin("Vehicle.ecu", "wake", &[7]),
                pin("Vehicle.bcm", "wake", &[7])
            ],
            [
                pin("Vehicle.bcm", "wake", &[7]),
                pin("Vehicle.node", "wake", &[7])
            ],
            [
                pin("Vehicle.node", "wake", &[7]),
                pin("Vehicle.lamp", "wake", &[7])
            ],
            [
                pin("Vehicle.ecu", "can_h", &[4]),
                pin("Vehicle.node", "can_h", &[4])
            ],
            [
                pin("Vehicle.ecu", "can_l", &[5]),
                pin("Vehicle.node", "can_l", &[5])
            ],
            // A double crimp: two rows into the ECU's pin 1.
            [
                pin("Vehicle.ecu", "pwr", &[1]),
                pin("Vehicle.bcm", "pwr", &[1])
            ],
            [
                pin("Vehicle.ecu", "pwr", &[1]),
                pin("Vehicle.lamp", "pwr", &[1])
            ],
            [
                At::Connectorless(path("Vehicle.ecu"), "sense".to_string()),
                pin("Vehicle.bcm", "sig", &[8]),
            ],
            // The inline connector: a pin of its designator either side.
            [
                pin("Vehicle.ecu", "sig", &[8]),
                pin("Vehicle.ic", "sig", &[1])
            ],
            [
                pin("Vehicle.ic", "sig", &[1]),
                pin("Vehicle.node", "sig", &[8])
            ],
            // Three conductors meet at the splice; a ganged port is both pins.
            [
                pin("Vehicle.ecu", "gnd", &[2, 3]),
                At::Splice(path("Vehicle"), "sp_gnd".to_string()),
            ],
            [
                pin("Vehicle.bcm", "gnd", &[2, 3]),
                At::Splice(path("Vehicle"), "sp_gnd".to_string()),
            ],
            [
                At::Splice(path("Vehicle"), "sp_gnd".to_string()),
                pin("Vehicle.ic", "gnd", &[2]),
            ],
            [pin("Vehicle.bcm", "out", &[9]), feed("Vehicle.front")],
            [pin("Vehicle.node", "out", &[9]), feed("Vehicle.rear")],
        ]
    );
}

#[test]
fn conductors_carry_their_wire_data_and_the_length_that_applies() {
    let design = checked_design();
    let list = WireList::from(&design);
    let rows: Vec<_> = list
        .conductors()
        .filter(|(row, _)| row.owner.path == path("Vehicle"))
        .map(|(row, c)| {
            (
                row.cable.map(|cable| cable.name.to_string()),
                c.wire.label.clone(),
                c.wire.color.code(),
                c.wire.gauge,
                row.length,
                c.wire.twisted_group,
            )
        })
        .collect();
    let can = Some("can".to_string());
    let label = |l: &str| Some(l.to_string());
    assert_eq!(
        rows[3],
        (
            can.clone(),
            label("CAN H"),
            "YE".into(),
            0.5,
            metres(2.15),
            Some(0)
        )
    );
    assert_eq!(
        rows[4],
        (can, label("CAN L"), "GN".into(), 0.5, metres(2.0), Some(0))
    );
    // A loose wire's own length, in centimetres; a rail's segments carry
    // their wire's (none).
    assert_eq!(
        rows[5],
        (None, label("12V"), "RD".into(), 1.5, metres(0.8), None)
    );
    assert_eq!(rows[0].4, None);

    let (_, crimp) = list.conductors().nth(5).expect("the first 12V conductor");
    assert_eq!(crimp.wire.r#type.as_deref(), Some("FLRY-B"));
    assert_eq!(crimp.wire.part.as_deref(), Some("W-RD-1.5"));
}

#[test]
fn cable_screens_are_rows_of_their_own() {
    let design = checked_design();
    let list = WireList::from(&design);
    let shields: Vec<(&Row, _)> = list.shields().collect();
    let [(overall_row, overall), (pair_row, pair)] = shields[..] else {
        panic!("the CAN cable's two screens");
    };

    assert_eq!(overall.screened, Screened::Cable);
    assert_eq!(overall.shield.label.as_deref(), Some("CAN SHLD"));
    assert!(matches!(overall.drain, Drain::Single(_)));
    assert_eq!(
        overall_row.ends().iter().map(At::of).collect::<Vec<_>>(),
        [pin("Vehicle.ecu", "shld", &[6])]
    );
    assert_eq!(overall_row.length, metres(2.0));

    assert_eq!(pair.screened, Screened::Pair(0));
    assert!(matches!(pair.drain, Drain::NoPin));
    assert!(pair_row.ends().is_empty());
    assert_eq!(
        pair_row.cable.map(|c| c.name.to_string()),
        Some("can".into())
    );
}

#[test]
fn a_loom_placed_twice_lists_its_own_wire_once_per_placement() {
    let design = checked_design();
    let list = WireList::from(&design);
    for loom in ["Vehicle.front", "Vehicle.rear"] {
        assert_eq!(
            conductors_of(&list, loom),
            [[
                At::Boundary(path(loom), "feed".to_string()),
                pin(&format!("{loom}.lamp"), "pwr", &[1]),
            ]],
            "{loom}"
        );
    }
    let lengths: Vec<_> = list
        .conductors()
        .filter(|(row, _)| row.owner.path != path("Vehicle"))
        .map(|(row, _)| row.length)
        .collect();
    assert_eq!(lengths, [metres(0.5), metres(0.5)]);
}
