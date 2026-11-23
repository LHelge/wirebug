//! End-to-end check of the `tied_routes_project` fixture: wires with two
//! mirror-image routes of equal cost, which the router must settle the
//! same way on every render.
//!
//! Tests never target `examples/` — that is the real, freely-changing
//! vehicle project.

use std::path::Path;

use wirebug::check_project;
use wirebug::dsl::ir::Design;
use wirebug::render::{SvgMode, render_views};

fn design() -> Design {
    let main =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tied_routes_project/main.wb");
    let report = check_project(Some(&main));
    assert!(
        report.problems.is_empty(),
        "fixture should check clean: {:?}",
        report.problems
    );
    report.design.expect("a design")
}

fn render(design: &Design) -> Vec<(String, String)> {
    render_views(design, SvgMode::Standalone)
        .into_iter()
        .map(|view| {
            let view = view.expect("view renders");
            (view.filename, view.svg)
        })
        .collect()
}

/// The `(x, y)` points of every wire polyline in `svg`.
fn wire_points(svg: &str) -> Vec<Vec<(f64, f64)>> {
    svg.split("<polyline class=\"wire\"")
        .skip(1)
        .map(|wire| {
            let points = wire
                .split("points=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .expect("a wire has points");
            points
                .split(' ')
                .map(|pair| {
                    let (x, y) = pair.split_once(',').expect("an x,y pair");
                    (x.parse().expect("x"), y.parse().expect("y"))
                })
                .collect()
        })
        .collect()
}

/// Every graph and map the router builds gets its own hash seed, so a tie
/// settled by hash order comes out differently from one render to the next
/// within a single process too.
#[test]
fn rendering_again_in_the_same_process_gives_the_same_svgs() {
    let design = design();
    let first = render(&design);
    for _ in 0..32 {
        assert_eq!(render(&design), first);
    }
}

#[test]
fn a_wire_goes_round_the_far_box_on_the_east_or_under_it() {
    let rendered = render(&design());
    let svg = |name: &str| {
        &rendered
            .iter()
            .find(|(filename, _)| filename == name)
            .expect("view rendered")
            .1
    };

    // `top` and `bottom` share a centreline, x = 0.
    let [stacked] = wire_points(svg("stacked.svg"))
        .try_into()
        .expect("one wire");
    assert!(stacked.iter().all(|&(x, _)| x >= 0.0), "{stacked:?}");
    assert!(stacked.iter().any(|&(x, _)| x > 0.0), "{stacked:?}");

    // `left` and `right` share a centreline, y = 0. Over `right` the wire
    // would have its title row to clear as well, so under is shorter.
    let [side] = wire_points(svg("side_by_side.svg"))
        .try_into()
        .expect("one wire");
    assert!(side.iter().all(|&(_, y)| y >= 0.0), "{side:?}");
    assert!(side.iter().any(|&(_, y)| y > 0.0), "{side:?}");
}
