//! End-to-end check of the `splice_project` fixture: a ground splice three
//! devices' grounds join at, run on to the battery on one heavy conductor,
//! and a butt splice a cable conductor ends on — elaborated into the IR,
//! drawn in harness views, and drawn as the nets they join in a schematic.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::{CableName, Design, InstanceName, PortName, SpliceName, WireEnd};
use wirebug::render::{RenderedView, SvgMode, embed_stylesheet, render_views};

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

    // A cable's conductor may end on a splice too: the feed changes gauge
    // at its butt splice, on to a loose wire.
    let feed = SpliceName::from("sp_feed");
    let cabled: Vec<(Option<&CableName>, f64)> = root
        .splice_conductors(&feed)
        .map(|wire| (wire.cable.as_ref(), wire.gauge))
        .collect();
    assert_eq!(cabled, [(Some(&CableName::from("feed")), 1.5), (None, 0.5)]);
}

fn views() -> Vec<RenderedView> {
    render_views(&design(), SvgMode::Standalone)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("renders")
}

fn view(views: &[RenderedView], title: &str) -> String {
    views
        .iter()
        .find(|v| v.title == title)
        .unwrap_or_else(|| panic!("view {title:?}"))
        .svg
        .clone()
}

#[test]
fn a_harness_view_draws_a_splice_as_a_labelled_joint() {
    let views = views();
    let ground = view(&views, "Ground loom");
    assert_eq!(ground.matches("class=\"splice-dot\"").count(), 1);
    for text in ["sp_gnd", "Ground splice", "D-436-36"] {
        assert!(ground.contains(&format!(">\n{text}\n</text>")), "{text}");
    }
    // Each conductor to it is drawn on its own, with its own gauge and
    // color.
    for annotation in [
        "GND · 1mm² · BK",
        "GND · 0.5mm² · BK",
        "GND · 0.5mm² · BN",
        "GND · 2.5mm² · BK",
    ] {
        assert!(ground.contains(annotation), "{annotation}");
    }

    // The feed's cable conductor runs through its cable box to the butt
    // splice, and a loose wire on from it.
    let feed = view(&views, "Lamp feed");
    assert_eq!(feed.matches("class=\"connector cable-box\"").count(), 1);
    assert!(feed.contains(">\nsp_feed\n</text>"));
    assert!(feed.contains("12V · 1.5mm² · RD") && feed.contains("12V · 0.5mm² · RD"));

    // A view without a splice draws none.
    assert!(!view(&views, "Wake").contains("class=\"splice\""));
}

#[test]
fn the_embed_stylesheet_styles_the_splice_classes() {
    let css = embed_stylesheet();
    for class in ["splice-dot", "splice-name", "splice-label", "splice-part"] {
        assert!(
            css.lines()
                .any(|rule| rule.starts_with(".wirebug-harness") && rule.contains(class)),
            "{class}"
        );
    }
}

#[test]
fn a_schematic_draws_the_ports_a_splice_joins_as_one_net() {
    let views = views();
    let grounds = view(&views, "Grounds");
    // Four grounds through `sp_gnd`, chained like a rail: three segments,
    // each in its own conductor's color; and the feed through `sp_feed`.
    assert_eq!(grounds.matches("class=\"wire\"").count(), 4);
    assert_eq!(grounds.matches("data-color=\"black\"").count(), 2);
    assert_eq!(grounds.matches("data-color=\"brown\"").count(), 1);
    assert_eq!(grounds.matches("data-color=\"red\"").count(), 1);
    // No splice appears as a box or a dot in a schematic.
    assert!(!grounds.contains("sp_gnd") && !grounds.contains("class=\"splice"));
}
