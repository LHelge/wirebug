//! End-to-end check of the `bom_project` fixture's bill of materials: an
//! assembly placed twice (six lamps), an inline connector with two halves,
//! housings with and without a part, cables of one type summed (one
//! without a length), loose wire summed with an unmeasured conductor and
//! rails counted once, two wire part numbers of one colour and gauge, and
//! components and splices with and without a part.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::{Design, Length, LengthUnit};
use wirebug::export::{Bom, Copper, Line, LineKind};

fn design() -> Design {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bom_project");
    let report = check_project(Some(&root));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    report.design.expect("a design")
}

fn metres(value: f64) -> Length {
    Length::new(value, LengthUnit::Metre).expect("a positive length")
}

/// Each line as (kind, specified, what names it, quantity, total).
fn summary(bom: &Bom<'_>) -> Vec<(LineKind, bool, String, usize, Option<Copper>)> {
    bom.lines()
        .iter()
        .map(|line| {
            let name = match line {
                Line::Housing(l) => l
                    .part
                    .map(str::to_string)
                    .or(l.type_name.map(ToString::to_string))
                    .unwrap_or_default(),
                Line::Cable(l) => l.cable_type.unwrap_or_default().to_string(),
                Line::Wire(l) => format!(
                    "{} {} {}",
                    l.color.code(),
                    l.gauge,
                    l.part.unwrap_or_default()
                ),
                Line::Component(l) => l
                    .part
                    .number
                    .clone()
                    .unwrap_or_else(|| l.type_name.to_string()),
                Line::Splice(l) => l.part.or(l.label).unwrap_or_default().to_string(),
            };
            (
                line.kind(),
                line.is_specified(),
                name,
                line.quantity(),
                line.length(),
            )
        })
        .collect()
}

#[test]
fn the_bill_of_materials_counts_every_placement_and_lists_the_unspecified() {
    let design = design();
    let bom = Bom::from(&design);
    let copper = |length: Option<f64>, unmeasured| {
        Some(Copper {
            length: length.map(metres),
            unmeasured,
        })
    };
    use LineKind::*;
    let expect = [
        (Housing, false, "Hdr4", 2, None),
        // The relay's and the switch's connector blocks name no type.
        (Housing, false, "", 2, None),
        // Six lamps' housings and the inline's male half.
        (Housing, true, "DT04-2P", 7, None),
        (Housing, true, "DT06-2S", 1, None),
        // The cables' own 3 m; the second has no length.
        (Cable, true, "2x0.5 TP", 2, copper(Some(3.0), 1)),
        (Cable, false, "", 1, copper(Some(1.5), 0)),
        // 1 m + 0.5 m of the vehicle's own, one unmeasured, and each lamp
        // bar's 2 m rail of two segments, counted once per bar.
        (Wire, false, "RD 1 ", 7, copper(Some(5.5), 1)),
        (Wire, true, "BK 0.5 FLRY-0.5-BK", 2, copper(Some(0.3), 1)),
        (Wire, true, "BK 0.5 TXL-0.5-BK", 1, copper(Some(2.0), 0)),
        (Wire, false, "BK 0.5 ", 8, copper(None, 8)),
        (Component, true, "ECU-100", 2, None),
        (Component, false, "Relay", 1, None),
        (Component, false, "Switch", 1, None),
        (Component, true, "LMP-1", 6, None),
        // The vehicle's splice and one in each lamp bar share a part.
        (Splice, true, "D-436", 3, None),
        (Splice, false, "Feed splice", 1, None),
        (Splice, false, "sp_aux", 1, None),
    ]
    .map(|(kind, specified, name, quantity, length)| {
        (kind, specified, name.to_string(), quantity, length)
    });
    assert_eq!(summary(&bom), expect);
}

#[test]
fn a_component_line_carries_its_part_data_even_without_a_number() {
    let design = design();
    let bom = Bom::from(&design);
    let relay = bom
        .lines()
        .iter()
        .find_map(|line| match line {
            Line::Component(l) if l.type_name.as_str() == "Relay" => Some(l),
            _ => None,
        })
        .expect("the relay's line");
    assert_eq!(relay.part.manufacturer.as_deref(), Some("Generic"));
    assert_eq!(relay.part.description.as_deref(), Some("Changeover relay"));
    assert_eq!(relay.part.number, None);
}
