//! End-to-end check of the `shield_project` fixture: a cable whose overall
//! screen is grounded at one end, and a cable with an individually screened
//! pair grounded at both ends, a pair whose screen is cut back, and an
//! overall screen terminated nowhere — elaborated into the IR and drawn in
//! two harness views.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::{CableName, Design, InstanceName, PortName, Screened, WireEnd};
use wirebug::render::{SvgMode, embed_stylesheet, render_views};

fn design() -> Design {
    let main = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shield_project/main.wb");
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
fn shields_and_where_they_land_are_on_the_ir_cables() {
    let design = design();
    let root = design.get(&design.root).expect("root instance");

    let can = &root.cables[&CableName::from("can")];
    let screen = can.shield.as_ref().expect("the CAN cable is screened");
    assert_eq!(screen.label.as_deref(), Some("CAN SHLD"));
    assert_eq!(screen.ends(), [end("ecu", "shield_gnd")]);
    assert!(
        can.twisted_group(0)
            .is_some_and(|pair| pair.shield.is_none()),
        "its pair has no screen of its own"
    );

    let res = &root.cables[&CableName::from("res")];
    let shields: Vec<(Screened, Option<&str>, &[WireEnd])> = res
        .shields()
        .map(|(what, s)| (what, s.label.as_deref(), s.ends()))
        .collect();
    assert_eq!(
        shields,
        [
            (Screened::Cable, None, &[][..]),
            (
                Screened::Pair(0),
                Some("EX SHLD"),
                &[end("ecu", "gnd"), end("resolver", "screen")][..]
            ),
            (Screened::Pair(1), None, &[][..]),
        ]
    );

    // A shield is no conductor: the wires are the seven authored ones.
    assert_eq!(root.wires.len(), 7);
}

#[test]
fn the_loom_draws_each_screen_and_every_drain_it_includes() {
    let design = design();
    let views = render_views(&design, SvgMode::Standalone)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("renders");
    let [loom, can_only] = views.as_slice() else {
        panic!("expected two views");
    };

    // Both cables have an overall screen; two of the resolver's pairs have
    // their own.
    assert_eq!(loom.svg.matches("class=\"cable-shield\"").count(), 2);
    assert_eq!(loom.svg.matches("class=\"pair-shield\"").count(), 2);
    assert!(
        loom.svg
            .contains("Shielded twisted pair · 2 m · ×3 · shielded")
    );
    assert!(loom.svg.contains("1.5 m · ×4 · shielded"));
    // Two drains: CAN's (one lead) and the EX pair's (two leads); the cut
    // back screens have none.
    assert_eq!(loom.svg.matches("class=\"drain\"").count(), 2);
    assert_eq!(loom.svg.matches("class=\"drain-wire\"").count(), 2 + 3);
    for label in ["CAN SHLD", "EX SHLD", "SHIELD GND", "SCREEN"] {
        assert!(loom.svg.contains(label), "{label}");
    }

    // The resolver cable is not drawn without the resolver, and neither
    // are its screens.
    assert_eq!(
        can_only
            .svg
            .matches("class=\"connector cable-box\"")
            .count(),
        1
    );
    assert_eq!(can_only.svg.matches("class=\"pair-shield\"").count(), 0);
    assert_eq!(can_only.svg.matches("class=\"drain\"").count(), 1);
    assert!(!can_only.svg.contains("EX SHLD"));
}

#[test]
fn the_embed_stylesheet_styles_the_shield_classes() {
    let css = embed_stylesheet();
    for class in ["cable-shield", "pair-shield", "drain-wire", "drain-label"] {
        assert!(
            css.lines()
                .any(|rule| rule.starts_with(".wirebug-harness") && rule.contains(class)),
            "{class}"
        );
    }
}
