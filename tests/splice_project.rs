//! End-to-end check of the `splice_project` fixture: a ground splice three
//! devices' grounds join at, run on to the battery on one heavy conductor,
//! and a butt splice inside a cable — elaborated into the IR.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::{CableName, Design, InstanceName, PortName, SpliceName, WireEnd};

fn design() -> Design {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/splice_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    report.design.expect("a design")
}

fn end(instance: &str, port: &str) -> WireEnd {
    WireEnd::Child {
        instance: InstanceName::from(instance),
        port: PortName::from(port),
    }
}

#[test]
fn each_conductor_to_a_splice_is_a_wire_of_its_own() {
    let design = design();
    let root = design.get(&design.root).expect("root instance");
    let ground = SpliceName::from("sp_gnd");

    let splice = &root.splices[&ground];
    assert_eq!(splice.label.as_deref(), Some("Ground splice"));
    assert_eq!(splice.part.as_deref(), Some("D-436-36"));
    assert_eq!(splice.description.as_deref(), Some("Solder sleeve"));

    // One row per conductor in a wire list: its own gauge and color, the
    // splice as one end.
    let rows: Vec<(String, f64, String)> = root
        .splice_conductors(&ground)
        .map(|wire| {
            let from = wire
                .other_end(&WireEnd::Splice(ground.clone()))
                .expect("a conductor to the splice");
            (from.to_string(), wire.gauge, wire.color.to_string())
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("ecu.gnd".to_string(), 1.0, "black".to_string()),
            ("lamp.gnd".to_string(), 0.5, "black".to_string()),
            ("pump.gnd".to_string(), 0.5, "brown".to_string()),
            ("bat.neg".to_string(), 2.5, "black".to_string()),
        ]
    );

    // Electrically, the splice joins the four ports.
    let net = root.splice_net(&ground);
    let ends: Vec<&WireEnd> = net.ends.iter().map(|e| e.end).collect();
    assert_eq!(
        ends,
        [
            &end("ecu", "gnd"),
            &end("lamp", "gnd"),
            &end("pump", "gnd"),
            &end("bat", "neg"),
        ]
    );

    // A cable's conductors may end on a splice too: the feed changes gauge
    // at its butt splice.
    let feed = SpliceName::from("sp_feed");
    let cabled: Vec<(Option<&CableName>, f64)> = root
        .splice_conductors(&feed)
        .map(|wire| (wire.cable.as_ref(), wire.gauge))
        .collect();
    let cable = CableName::from("feed");
    assert_eq!(cabled, [(Some(&cable), 1.5), (Some(&cable), 0.5)]);
}
